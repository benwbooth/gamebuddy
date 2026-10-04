//! A bounded, persistent optional ML worker. No shell interpolation, and model
//! failures stay visible rather than degrading silently to a 2D matcher.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    io::{BufRead, BufReader, Write},
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc,
    time::Duration,
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorkerConfig {
    pub executable: String,
    pub arguments: Vec<String>,
}

pub struct Worker {
    child: Child,
    input: ChildStdin,
    output: mpsc::Receiver<Result<String, String>>,
}
impl Worker {
    pub fn start(config: &WorkerConfig) -> Result<Self> {
        let mut child = Command::new(&config.executable)
            .args(&config.arguments)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .context("Start learned spatial worker")?;
        let input = child.stdin.take().context("Worker input unavailable")?;
        let stdout = child.stdout.take().context("Worker output unavailable")?;
        let (tx, output) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                if tx.send(line.map_err(|e| e.to_string())).is_err() {
                    break;
                }
            }
        });
        Ok(Self {
            child,
            input,
            output,
        })
    }
    pub fn call(&mut self, request: Value) -> Result<Value> {
        serde_json::to_writer(&mut self.input, &request)?;
        self.input.write_all(b"\n")?;
        self.input.flush()?;
        let line = self
            .output
            .recv_timeout(Duration::from_secs(60))
            .context("Spatial worker timed out or exited")?
            .map_err(anyhow::Error::msg)?;
        let reply: Value = serde_json::from_str(&line)?;
        ensure!(
            reply.get("error").is_none(),
            "Spatial worker: {}",
            reply["error"]
        );
        reply
            .get("result")
            .cloned()
            .context("Spatial worker returned no result")
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub fn call(config: &WorkerConfig, slot: &str, request: Value) -> Result<Value> {
    type Slot = std::sync::Arc<std::sync::Mutex<Option<Worker>>>;
    type Pool = std::collections::HashMap<String, Slot>;
    static POOL: std::sync::OnceLock<std::sync::Mutex<Pool>> = std::sync::OnceLock::new();
    let key = format!("{slot}:{}", serde_json::to_string(config)?);
    let handle = POOL
        .get_or_init(Default::default)
        .lock()
        .map_err(|_| anyhow::anyhow!("Spatial pool lock poisoned"))?
        .entry(key)
        .or_default()
        .clone();
    let mut worker = handle
        .lock()
        .map_err(|_| anyhow::anyhow!("Spatial worker lock poisoned"))?;
    if worker.is_none() {
        *worker = Some(Worker::start(config)?);
    }
    let result = worker.as_mut().unwrap().call(request);
    if result.is_err() {
        *worker = None;
    }
    result
}
