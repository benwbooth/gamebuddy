//! One Claude Code process per coaching request. Each tool round is a new
//! user turn in the same conversation, so later rounds send only new evidence
//! and skip the CLI's startup. A spare process is started ahead of time so
//! even the first round does not wait for startup.
use super::Process;
use crate::ModelRequest;
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Write},
    process::{ChildStdin, Command, Stdio},
    sync::{Mutex, mpsc},
    time::{Duration, Instant},
};

// Partial messages stream continuously while Claude thinks or writes. The
// service sometimes stops mid-reply; a fresh process then answers in seconds.
// Detect that silence quickly and let the caller retry, keeping a voice
// answer inside its time limit.
const TURN_TIMEOUT: Duration = Duration::from_secs(60);
const SILENCE_TIMEOUT: Duration = Duration::from_secs(12);
const RUNAWAY_BYTES: usize = 6000;
const RUNAWAY_TIME: Duration = Duration::from_secs(8);

/// A complete tool call or answer is already present in the streamed text.
fn holds_reply(text: &str) -> bool {
    text.contains("</invoke>")
        || text.lines().map(str::trim).any(|line| {
            line.starts_with('{')
                && serde_json::from_str::<Value>(line)
                    .is_ok_and(|v| v.get("tool").is_some() || v.get("answer").is_some())
        })
}

#[derive(Clone, PartialEq, Eq)]
struct Key {
    model: String,
    effort: String,
    system: String,
}
pub struct Session {
    key: Key,
    process: Process,
    input: ChildStdin,
    output: mpsc::Receiver<std::io::Result<String>>,
    _directory: tempfile::TempDir,
    started: Instant,
    /// A runaway reply was cut off; this process is still mid-turn.
    pub finished: bool,
}
static SPARE: Mutex<Option<Session>> = Mutex::new(None);

fn signed_in(directory: &std::path::Path) -> Result<()> {
    let login = Command::new("claude")
        .args(["--safe-mode", "auth", "status"])
        .env_remove("ANTHROPIC_API_KEY")
        .env_remove("ANTHROPIC_AUTH_TOKEN")
        .current_dir(directory)
        .output()
        .context("Install Claude Code and sign in with `claude auth login`")?;
    let login: Value =
        serde_json::from_slice(&login.stdout).context("Cannot read Claude sign-in status")?;
    ensure!(
        login["loggedIn"] == true && login["authMethod"] == "claude.ai",
        "Sign in to your Claude subscription with `claude auth login`, then test again. For an API key, select Anthropic API instead."
    );
    Ok(())
}

impl Session {
    fn spawn(key: Key) -> Result<Self> {
        let directory = tempfile::tempdir()?;
        signed_in(directory.path())?;
        let mut command = Command::new("claude");
        command
            .args([
                "--print",
                "--verbose",
                "--safe-mode",
                "--no-session-persistence",
                "--include-partial-messages",
                "--input-format",
                "stream-json",
                "--output-format",
                "stream-json",
                "--tools",
                "",
                "--strict-mcp-config",
                "--mcp-config",
                "{\"mcpServers\":{}}",
                "--system-prompt",
                &key.system,
            ])
            .env_remove("ANTHROPIC_API_KEY")
            .env_remove("ANTHROPIC_AUTH_TOKEN")
            .current_dir(directory.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        if !key.model.is_empty() {
            command.args(["--model", &key.model]);
        }
        if !matches!(key.effort.as_str(), "" | "auto") {
            command.args(["--effort", &key.effort]);
        }
        let mut process =
            Process(command.spawn().context(
                "Install Claude Code and run `claude auth login` with your Claude account",
            )?);
        let input = process.0.stdin.take().context("Claude stdin unavailable")?;
        let stdout = process
            .0
            .stdout
            .take()
            .context("Claude stdout unavailable")?;
        let (tx, output) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        Ok(Self {
            key,
            process,
            input,
            output,
            _directory: directory,
            started: Instant::now(),
            finished: false,
        })
    }
    fn alive(&mut self) -> bool {
        // Idle spares are recycled so a long pause never hands over a stale one.
        self.started.elapsed() < Duration::from_secs(30 * 60)
            && self
                .process
                .0
                .try_wait()
                .is_ok_and(|status| status.is_none())
    }
    /// A ready process for this system prompt, then a replacement in the background.
    pub fn take(model: &str, effort: &str, system: &str) -> Result<Self> {
        let key = Key {
            model: model.into(),
            effort: effort.into(),
            system: system.into(),
        };
        let spare = SPARE
            .lock()
            .map_err(|_| anyhow::anyhow!("Claude session lock poisoned"))?
            .take()
            .filter(|s| s.key == key);
        let session = match spare {
            Some(mut session) => {
                if session.alive() {
                    session
                } else {
                    Self::spawn(key.clone())?
                }
            }
            None => Self::spawn(key.clone())?,
        };
        std::thread::spawn(move || prepare_key(key));
        Ok(session)
    }
    /// Send one user turn and wait for its result.
    pub fn send(&mut self, request: ModelRequest) -> Result<String> {
        let mut content = vec![json!({"type":"text","text":request.prompt})];
        content.extend(request.images.into_iter().map(|data| json!({"type":"image","source":{"type":"base64","media_type":"image/jpeg","data":data}})));
        serde_json::to_writer(
            &mut self.input,
            &json!({"type":"user","message":{"role":"user","content":content}}),
        )?;
        self.input.write_all(b"\n")?;
        self.input.flush()?;
        ensure!(!self.finished, "Claude session ended");
        let deadline = Instant::now() + TURN_TIMEOUT;
        let mut written = String::new();
        let mut text_started = None;
        loop {
            let wait = deadline
                .saturating_duration_since(Instant::now())
                .min(SILENCE_TIMEOUT);
            let line = self
                .output
                .recv_timeout(wait)
                .context("Claude stopped responding or the process exited")??;
            if let Some(path) = std::env::var_os("GAMEBUDDY_CLAUDE_TRACE") {
                let _ = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)
                    .and_then(|mut f| {
                        writeln!(
                            f,
                            "{} {}",
                            crate::screen::now_ms(),
                            line.chars().take(400).collect::<String>()
                        )
                    });
            }
            let Ok(value) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if let Some(delta) = value["event"]["delta"]["text"].as_str() {
                written.push_str(delta);
                // A reply is a short JSON object. Claude occasionally repeats
                // itself until its output limit, which takes minutes. Stop at
                // a runaway length; the caller reads the first call from it.
                let started = *text_started.get_or_insert_with(Instant::now);
                if written.len() > RUNAWAY_BYTES
                    || (started.elapsed() > RUNAWAY_TIME || written.len() > 2000)
                        && holds_reply(&written)
                {
                    self.finished = true;
                    return Ok(written);
                }
            }
            if value["type"] == "rate_limit_event"
                && value["rate_limit_info"]["status"] == "rejected"
            {
                return Err(crate::out_of_tokens(
                    "Claude",
                    &format!(
                        "Your Claude plan's limit resets at Unix time {}.",
                        value["rate_limit_info"]["resetsAt"]
                    ),
                ));
            }
            if value["type"] != "result" {
                continue;
            }
            let failure = value["result"].as_str().unwrap_or_default();
            if value["is_error"] == true && crate::is_quota_error(failure) {
                return Err(crate::out_of_tokens("Claude", failure));
            }
            ensure!(value["is_error"] != true, "Claude: {}", value["result"]);
            let answer = value["result"]
                .as_str()
                .context("Claude returned no result")?;
            ensure!(!answer.trim().is_empty(), "Claude returned an empty answer");
            return Ok(answer.into());
        }
    }
}

fn prepare_key(key: Key) {
    let mut spare = match SPARE.lock() {
        Ok(spare) => spare,
        Err(_) => return,
    };
    if spare.as_mut().is_some_and(|s| s.key == key && s.alive()) {
        return;
    }
    drop(spare);
    if let Ok(session) = Session::spawn(key)
        && let Ok(mut spare) = SPARE.lock()
    {
        *spare = Some(session);
    }
}

/// Start a spare process so the next question skips Claude Code's startup.
pub fn prepare(model: &str, effort: &str, system: &str) {
    let key = Key {
        model: model.into(),
        effort: effort.into(),
        system: system.into(),
    };
    std::thread::spawn(move || prepare_key(key));
}
