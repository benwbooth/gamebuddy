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

const TURN_TIMEOUT: Duration = Duration::from_secs(120);

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
        let deadline = Instant::now() + TURN_TIMEOUT;
        loop {
            let line = self
                .output
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .context("Claude response timed out or the process exited")??;
            let Ok(value) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if value["type"] != "result" {
                continue;
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
