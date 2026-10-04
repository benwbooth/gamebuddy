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
                "mcp_servers={}",
                "-c",
                "web_search=\"disabled\"",
            ])
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
                    bail!("Codex {method}: {error}");
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
        self.thread_with_tools_at_tier(model, instructions, tools, None, deadline)
    }
    pub fn voice_thread(
        &mut self,
        model: &str,
        instructions: &str,
        tools: Value,
        fast: bool,
        deadline: Instant,
    ) -> Result<String> {
        let priority = fast
            && self
                .request("model/list", json!({}), deadline)
                .ok()
                .and_then(|models| {
                    models["data"]
                        .as_array()?
                        .iter()
                        .find(|m| {
                            if model.is_empty() {
                                m["isDefault"] == true
                            } else {
                                m["model"] == model || m["id"] == model
                            }
                        })
                        .map(|m| {
                            m["serviceTiers"].as_array().is_some_and(|tiers| {
                                tiers.iter().any(|tier| tier["id"] == "priority")
                            })
                        })
                })
                .unwrap_or(false);
        self.thread_with_tools_at_tier(
            model,
            instructions,
            tools,
            priority.then_some("priority"),
            deadline,
        )
    }
    fn thread_with_tools_at_tier(
        &mut self,
        model: &str,
        instructions: &str,
        tools: Value,
        tier: Option<&str>,
        deadline: Instant,
    ) -> Result<String> {
        let mut params = json!({"ephemeral":true,"cwd":self._directory.path(),"environments":[],"approvalPolicy":"never","sandbox":"read-only","baseInstructions":instructions,"config":{"model_reasoning_effort":"low"}});
        if let Some(tier) = tier {
            params["serviceTier"] = json!(tier)
        }
        params["dynamicTools"] = tools;
        if !model.is_empty() {
            params["model"] = json!(model);
        }
        let value = self.request("thread/start", params, deadline)?;
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

pub fn codex_complete(model: &str, r: ModelRequest) -> Result<String> {
    let deadline = Instant::now() + Duration::from_secs(180);
    let mut client = CodexClient::start()?;
    let thread = client.thread(model, &r.system, deadline)?;
    let mut input = vec![json!({"type":"text","text":r.prompt})];
    input.extend(
        r.images
            .into_iter()
            .map(|data| json!({"type":"image","url":format!("data:image/jpeg;base64,{data}")})),
    );
    client.request(
        "turn/start",
        json!({"threadId":thread,"input":input,"effort":"low","environments":[]}),
        deadline,
    )?;
    let mut answer = String::new();
    loop {
        let event = client.receive(deadline)?;
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
                ensure!(
                    turn["status"] == "completed",
                    "Codex turn failed: {}",
                    turn["error"]
                );
                ensure!(!answer.trim().is_empty(), "Codex returned no answer");
                return Ok(answer);
            }
            _ => {}
        }
    }
}

pub fn claude_complete(model: &str, r: ModelRequest) -> Result<String> {
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
        "The Claude subscription provider requires a Claude account login; API-key authentication uses a separate provider"
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
        .stderr(Stdio::null());
    if !model.is_empty() {
        command.args(["--model", model]);
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
    ensure!(value["is_error"] != true, "Claude: {}", value["result"]);
    let answer = value["result"]
        .as_str()
        .context("Claude returned no result")?;
    ensure!(!answer.trim().is_empty(), "Claude returned an empty answer");
    Ok(answer.into())
}
