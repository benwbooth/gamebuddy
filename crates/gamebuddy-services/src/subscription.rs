//! Official installed clients own login/refresh. GameBuddy never reads OAuth
//! tokens or calls subscription endpoints directly. No coding tools are needed.
use crate::ModelRequest;
use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Write},
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc::{self, Receiver},
    time::{Duration, Instant},
};

pub mod claude;
pub mod session;

#[derive(Debug)]
struct RpcError {
    method: String,
    error: Value,
}
impl std::fmt::Display for RpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Codex {}: {}", self.method, self.error)
    }
}
impl std::error::Error for RpcError {}

/// Interactive coaching must not inherit a heavyweight coding model from the CLI.
pub const DEFAULT_CODEX_MODEL: &str = "gpt-6-luna";

fn codex_model(model: &str) -> &str {
    if model.trim().is_empty() {
        DEFAULT_CODEX_MODEL
    } else {
        model.trim()
    }
}

struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// JSON-lines transport with one total deadline, including initialization and
/// inference. Dropping it closes the private client even on parse/timeout errors.
pub struct CodexClient {
    _process: Process,
    input: ChildStdin,
    output: Receiver<Result<Value>>,
    next_id: u64,
    _directory: tempfile::TempDir,
    pub resolved_model: String,
    // Process is dropped first, before releasing the saved-thread leases.
    sessions: std::collections::HashMap<std::path::PathBuf, session::Lease>,
}
/// Overrides turning off every MCP server in the player's own Codex config.
/// `mcp_servers={}` merges rather than clears, so each app server launched
/// the player's tools (Blender, desktop control…) and they piled up as
/// hundreds of busy background processes. Only servers defined in
/// config.toml are named: disabling a plugin's server (plugins are already
/// off) would create an entry without a transport, which Codex rejects.
fn mcp_overrides() -> Vec<String> {
    static NAMES: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    NAMES
        .get_or_init(|| {
            let home = std::env::var_os("CODEX_HOME")
                .map(std::path::PathBuf::from)
                .or_else(|| {
                    std::env::var_os("HOME").map(|h| std::path::Path::new(&h).join(".codex"))
                });
            let config = home
                .and_then(|h| std::fs::read_to_string(h.join("config.toml")).ok())
                .unwrap_or_default();
            let mut names: Vec<String> = config
                .lines()
                .filter_map(|line| {
                    line.trim()
                        .strip_prefix("[mcp_servers.")?
                        .strip_suffix(']')
                        .map(str::to_owned)
                })
                // Sub-tables (`name.env`) and quoted names are not server names.
                .filter(|n| {
                    n.chars()
                        .all(|c| c.is_ascii_alphanumeric() || "-_".contains(c))
                })
                .collect();
            names.dedup();
            names
        })
        .iter()
        .flat_map(|name| ["-c".to_owned(), format!("mcp_servers.{name}.enabled=false")])
        .collect()
}

impl CodexClient {
    pub fn start() -> Result<Self> {
        let directory = tempfile::tempdir()?;
        let mut child = Command::new("codex")
            .args([
                "app-server",
                "-c",
                "features.shell_tool=false",
                "-c",
                "features.exec_tool=false",
                "-c",
                "features.apply_patch_freeform=false",
                "-c",
                "features.plugins=false",
                "-c",
                "features.memories=false",
                "-c",
                "features.hooks=false",
                "-c",
                "web_search=\"disabled\"",
            ])
            .args(mcp_overrides())
            .env_remove("OPENAI_API_KEY")
            .current_dir(directory.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .context("Install Codex CLI and run `codex login` with your ChatGPT account")?;
        let input = child.stdin.take().context("Codex stdin unavailable")?;
        let output = child.stdout.take().context("Codex stdout unavailable")?;
        let (tx, rx) = mpsc::sync_channel(128);
        std::thread::spawn(move || {
            for line in BufReader::new(output).lines() {
                let value = line
                    .map_err(anyhow::Error::from)
                    .and_then(|s| Ok(serde_json::from_str(&s)?));
                if tx.send(value).is_err() {
                    break;
                }
            }
        });
        let mut client = Self {
            _process: Process(child),
            input,
            output: rx,
            next_id: 1,
            _directory: directory,
            resolved_model: String::new(),
            sessions: Default::default(),
        };
        let deadline = Instant::now() + Duration::from_secs(20);
        client.request("initialize", json!({"clientInfo":{"name":"gamebuddy","title":"GameBuddy","version":"0.1.0"},"capabilities":{"experimentalApi":true}}), deadline)?;
        client.send(json!({"method":"initialized","params":{}}))?;
        let account = client.request("account/read", json!({}), deadline)?;
        ensure!(
            account["account"]["type"] == "chatgpt",
            "Codex subscription provider needs ChatGPT sign-in. Run `codex login`; API-key authentication is a separate provider."
        );
        Ok(client)
    }
    pub fn send(&mut self, value: Value) -> Result<()> {
        serde_json::to_writer(&mut self.input, &value)?;
        self.input.write_all(b"\n")?;
        self.input.flush()?;
        Ok(())
    }
    pub fn receive(&mut self, deadline: Instant) -> Result<Value> {
        let value = self
            .output
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .context("Codex stopped responding or reached the request deadline")??;
        Ok(value)
    }
    pub fn poll(&mut self) -> Result<Option<Value>> {
        match self.output.try_recv() {
            Ok(value) => value.map(Some),
            Err(mpsc::TryRecvError::Empty) => Ok(None),
            Err(mpsc::TryRecvError::Disconnected) => bail!("Codex disconnected"),
        }
    }
    pub fn reject(&mut self, event: &Value) -> Result<()> {
        if event.get("id").is_some() && event.get("method").is_some() {
            self.send(json!({"id":event["id"],"error":{"code":-32601,"message":"Only GameBuddy tools are available"}}))?;
        }
        Ok(())
    }
    pub fn request(&mut self, method: &str, params: Value, deadline: Instant) -> Result<Value> {
        self.request_with_notifications(method, params, deadline, |_| Ok(()))
    }
    /// Startup/shutdown can interleave transcript/SDP notifications with RPC
    /// responses. Give realtime a chance to persist/deliver every notification.
    pub fn request_with_notifications(
        &mut self,
        method: &str,
        params: Value,
        deadline: Instant,
        mut notification: impl FnMut(&Value) -> Result<()>,
    ) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        self.send(json!({"id":id,"method":method,"params":params}))?;
        loop {
            let value = self.receive(deadline)?;
            self.reject(&value)?;
            if value["id"] == id {
                if let Some(error) = value.get("error") {
                    return Err(RpcError {
                        method: method.into(),
                        error: error.clone(),
                    }
                    .into());
                }
                return Ok(value["result"].clone());
            }
            if value.get("id").is_none() {
                notification(&value)?;
            }
        }
    }
    pub fn thread(&mut self, model: &str, instructions: &str, deadline: Instant) -> Result<String> {
        self.thread_with_tools(model, instructions, json!([]), deadline)
    }
    pub fn thread_with_tools(
        &mut self,
        model: &str,
        instructions: &str,
        tools: Value,
        deadline: Instant,
    ) -> Result<String> {
        self.thread_with_options(model, instructions, tools, None, "", deadline)
    }
    pub fn voice_thread(
        &mut self,
        model: &str,
        instructions: &str,
        tools: Value,
        fast: bool,
        effort: &str,
        deadline: Instant,
    ) -> Result<String> {
        let tier = self.voice_tier(model, fast, deadline);
        self.thread_with_options(model, instructions, tools, Some(tier), effort, deadline)
    }
    pub fn voice_tier(&mut self, model: &str, fast: bool, deadline: Instant) -> &'static str {
        let model = codex_model(model);
        let priority = fast
            && self
                .request("model/list", json!({}), deadline)
                .ok()
                .and_then(|models| {
                    models["data"]
                        .as_array()?
                        .iter()
                        .find(|m| m["model"] == model || m["id"] == model)
                        .map(|m| {
                            m["serviceTiers"].as_array().is_some_and(|tiers| {
                                tiers.iter().any(|tier| tier["id"] == "priority")
                            })
                        })
                })
                .unwrap_or(false);
        // Explicit opt-out also overrides a globally configured Fast tier.
        if priority { "priority" } else { "default" }
    }
    pub fn thread_with_options(
        &mut self,
        model: &str,
        instructions: &str,
        tools: Value,
        tier: Option<&str>,
        effort: &str,
        deadline: Instant,
    ) -> Result<String> {
        let mut params = json!({"ephemeral":true,"cwd":self._directory.path(),"environments":[],"approvalPolicy":"never","sandbox":"read-only","baseInstructions":instructions});
        if effort != "auto" {
            params["config"] =
                json!({"model_reasoning_effort":if effort.is_empty() { "low" } else { effort }});
        }
        if let Some(tier) = tier {
            params["serviceTier"] = json!(tier)
        }
        params["dynamicTools"] = tools;
        params["model"] = json!(codex_model(model));
        let value = self.request("thread/start", params, deadline)?;
        self.resolved_model = value["model"].as_str().unwrap_or_default().into();
        if std::env::args().any(|a| a == "--voice-trace") {
            eprintln!(
                "GAMEBUDDY_CODEX_THREAD model={} effort={} service_tier={}",
                value["model"].as_str().unwrap_or_default(),
                value["reasoningEffort"].as_str().unwrap_or_default(),
                value["serviceTier"].as_str().unwrap_or("default")
            );
        }
        Ok(value["thread"]["id"]
            .as_str()
            .context("Codex did not return a thread ID")?
            .into())
    }
}

pub fn codex_complete(
    client: &mut CodexClient,
    model: &str,
    effort: &str,
    r: ModelRequest,
) -> Result<String> {
    codex_complete_scoped(client, model, effort, r, None)
}

pub(crate) fn codex_complete_scoped(
    client: &mut CodexClient,
    model: &str,
    effort: &str,
    r: ModelRequest,
    scope: Option<&(std::path::PathBuf, String)>,
) -> Result<String> {
    let started = Instant::now();
    let trace = std::env::var_os("GAMEBUDDY_PROVIDER_TRACE").is_some();
    if trace {
        eprintln!(
            "GAMEBUDDY_PROVIDER_START model={model} effort={effort} prompt_bytes={} images={}",
            r.prompt.len(),
            r.images.len()
        );
    }
    let deadline = Instant::now() + Duration::from_secs(180);
    let thread = if let Some((root, game)) = scope {
        let store = crate::Store::open(root)?;
        client
            .saved_thread(
                &store,
                game,
                session::Role::Chat,
                session::Options {
                    model,
                    instructions: &r.system,
                    tools: json!([]),
                    tier: None,
                    effort,
                },
                deadline,
            )?
            .id
    } else {
        client.thread_with_options(model, &r.system, json!([]), None, effort, deadline)?
    };
    let mut input = vec![json!({"type":"text","text":r.prompt})];
    input.extend(
        r.images
            .into_iter()
            .map(|data| json!({"type":"image","url":format!("data:image/jpeg;base64,{data}")})),
    );
    let mut params = json!({"threadId":thread,"input":input,"environments":[]});
    if effort != "auto" {
        params["effort"] = json!(if effort.is_empty() { "low" } else { effort });
    }
    client.request("turn/start", params, deadline)?;
    let mut answer = String::new();
    let mut first_output = false;
    loop {
        let event = client.receive(deadline)?;
        if trace && !first_output && event["method"] == "item/agentMessage/delta" {
            first_output = true;
            eprintln!(
                "GAMEBUDDY_PROVIDER_FIRST_OUTPUT model={model} elapsed_ms={}",
                started.elapsed().as_millis()
            );
        }
        client.reject(&event)?;
        if event["params"]["threadId"] != thread {
            continue;
        }
        match event["method"].as_str() {
            Some("item/completed") if event["params"]["item"]["type"] == "agentMessage" => {
                answer = event["params"]["item"]["text"]
                    .as_str()
                    .unwrap_or_default()
                    .into();
            }
            Some("turn/completed") => {
                let turn = &event["params"]["turn"];
                if turn["status"] != "completed" {
                    let detail = turn["error"].to_string();
                    // An exhausted plan is told to the player and pauses
                    // background work; other failures are ordinary errors.
                    if turn["error"]["codexErrorInfo"] == "usageLimitExceeded"
                        || crate::providers::is_quota_error(&detail)
                    {
                        return Err(crate::providers::out_of_tokens(
                            "Codex",
                            turn["error"]["message"].as_str().unwrap_or(&detail),
                        ));
                    }
                    bail!("Codex turn failed: {detail}");
                }
                ensure!(!answer.trim().is_empty(), "Codex returned no answer");
                if trace {
                    eprintln!(
                        "GAMEBUDDY_PROVIDER_COMPLETE model={model} elapsed_ms={} answer_bytes={}",
                        started.elapsed().as_millis(),
                        answer.len()
                    );
                }
                return Ok(answer);
            }
            _ => {}
        }
    }
}

pub fn claude_complete(model: &str, effort: &str, r: ModelRequest) -> Result<String> {
    claude_complete_structured(model, effort, r, None)
}

pub fn claude_complete_structured(
    model: &str,
    effort: &str,
    r: ModelRequest,
    schema: Option<Value>,
) -> Result<String> {
    let directory = tempfile::tempdir()?;
    let login = Command::new("claude")
        .args(["--safe-mode", "auth", "status"])
        .env_remove("ANTHROPIC_API_KEY")
        .env_remove("ANTHROPIC_AUTH_TOKEN")
        .current_dir(directory.path())
        .output()
        .context("Install Claude Code and sign in with `claude auth login`")?;
    let login: Value =
        serde_json::from_slice(&login.stdout).context("Cannot read Claude sign-in status")?;
    ensure!(
        login["loggedIn"] == true && login["authMethod"] == "claude.ai",
        "Sign in to your Claude subscription with `claude auth login`, then test again. For an API key, select Anthropic API instead."
    );
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
            &r.system,
        ])
        .env_remove("ANTHROPIC_API_KEY")
        .env_remove("ANTHROPIC_AUTH_TOKEN")
        .current_dir(directory.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(if std::env::var_os("GAMEBUDDY_TRACE_CLAUDE").is_some() {
            Stdio::inherit()
        } else {
            Stdio::null()
        });
    if !model.is_empty() {
        command.args(["--model", model]);
    }
    if !matches!(effort, "" | "auto") {
        command.args(["--effort", effort]);
    }
    if let Some(schema) = &schema {
        // Claude Code rejects a schema that names a draft by URL ("$schema"),
        // and then exits before reading the prompt (a broken pipe here).
        let mut schema = schema.clone();
        if let Some(object) = schema.as_object_mut() {
            object.remove("$schema");
        }
        command.args(["--json-schema", &schema.to_string()]);
    }
    let mut process = Process(
        command
            .spawn()
            .context("Install Claude Code and run `claude auth login` with your Claude account")?,
    );
    let mut input = process.0.stdin.take().context("Claude stdin unavailable")?;
    let mut content = vec![json!({"type":"text","text":r.prompt})];
    content.extend(r.images.into_iter().map(|data| json!({"type":"image","source":{"type":"base64","media_type":"image/jpeg","data":data}})));
    serde_json::to_writer(
        &mut input,
        &json!({"type":"user","message":{"role":"user","content":content}}),
    )?;
    input.write_all(b"\n")?;
    drop(input);
    let output = process
        .0
        .stdout
        .take()
        .context("Claude stdout unavailable")?;
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        use std::io::Read;
        let mut text = String::new();
        let result = BufReader::new(output)
            .take(4_000_000)
            .read_to_string(&mut text)
            .map(|_| text);
        let _ = tx.send(result);
    });
    let output = rx
        .recv_timeout(Duration::from_secs(180))
        .context("Claude response timed out")??;
    let value: Value = output
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .find(|v| v["type"] == "result")
        .context("Claude returned no result; check `claude auth status`")?;
    let failure = value["result"].as_str().unwrap_or_default();
    if value["is_error"] == true && crate::is_quota_error(failure) {
        return Err(crate::out_of_tokens("Claude", failure));
    }
    ensure!(value["is_error"] != true, "Claude: {}", value["result"]);
    if schema.is_some() {
        let output = value
            .get("structured_output")
            .filter(|v| v.is_object())
            .context("Claude returned no structured output")?;
        return Ok(output.to_string());
    }
    let answer = value["result"]
        .as_str()
        .context("Claude returned no result")?;
    ensure!(!answer.trim().is_empty(), "Claude returned an empty answer");
    Ok(answer.into())
}
