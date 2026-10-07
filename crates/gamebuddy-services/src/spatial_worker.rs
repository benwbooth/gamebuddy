//! A bounded, persistent optional ML worker. No shell interpolation, and model
//! failures stay visible rather than degrading silently to a 2D matcher.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    io::{BufRead, BufReader, Write},
    process::{Child, ChildStdin, Command, Stdio},
    sync::{Arc, Mutex, OnceLock, mpsc},
    time::{Duration, Instant},
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorkerConfig {
    pub executable: String,
    pub arguments: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Startup {
    pub status: String,
    pub message: String,
    pub completed: u32,
    pub total: u32,
    pub started_at_ms: u64,
    pub updated_at_ms: u64,
}
impl Startup {
    fn starting() -> Self {
        let now = crate::screen::now_ms();
        Self {
            status: "loading".into(),
            message: "Starting visual tracking…".into(),
            completed: 0,
            total: 0,
            started_at_ms: now,
            updated_at_ms: now,
        }
    }
    fn finish(&mut self, error: Option<&str>) {
        self.status = if error.is_some() { "failed" } else { "ready" }.into();
        self.message = error
            .unwrap_or("Tracker loaded · locating your game view…")
            .into();
        if error.is_none() {
            self.completed = self.total;
        }
        self.updated_at_ms = crate::screen::now_ms();
    }
}
#[derive(Deserialize)]
struct StartupEvent {
    message: String,
    completed: u32,
    total: u32,
}

/// Prebuilt Python wheels (numpy, torch) on NixOS load system libraries such
/// as libz through nix-ld. nix-ld only supplies NIX_LD_LIBRARY_PATH when
/// LD_LIBRARY_PATH is unset, and GameBuddy's own runtime sets it, so the
/// worker would crash on import. Append nix-ld's libraries after ours.
fn foreign_library_path(
    current: Option<std::ffi::OsString>,
    nix_ld: Option<std::ffi::OsString>,
) -> Option<std::ffi::OsString> {
    let nix_ld = nix_ld.filter(|p| !p.is_empty())?;
    let current = current.filter(|p| !p.is_empty())?;
    let mut paths: Vec<_> = std::env::split_paths(&current).collect();
    for path in std::env::split_paths(&nix_ld) {
        if !paths.contains(&path) {
            paths.push(path);
        }
    }
    std::env::join_paths(paths).ok()
}

pub struct Worker {
    child: Child,
    input: ChildStdin,
    output: mpsc::Receiver<Result<String, String>>,
    startup: Arc<Mutex<Startup>>,
}
impl Worker {
    pub fn start(config: &WorkerConfig) -> Result<Self> {
        Self::start_with_status(config, Arc::new(Mutex::new(Startup::starting())))
    }
    fn start_with_status(config: &WorkerConfig, startup: Arc<Mutex<Startup>>) -> Result<Self> {
        let mut command = Command::new(&config.executable);
        if let Some(path) = foreign_library_path(
            std::env::var_os("LD_LIBRARY_PATH"),
            std::env::var_os("NIX_LD_LIBRARY_PATH"),
        ) {
            command.env("LD_LIBRARY_PATH", path);
        }
        let mut child = command
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
            startup,
        })
    }
    pub fn call(&mut self, request: Value) -> Result<Value> {
        serde_json::to_writer(&mut self.input, &request)?;
        self.input.write_all(b"\n")?;
        self.input.flush()?;
        let deadline = Instant::now() + Duration::from_secs(60);
        let reply = loop {
            ensure!(Instant::now() < deadline, "Spatial worker timed out");
            let line = self
                .output
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .context("Spatial worker timed out or exited")?
                .map_err(anyhow::Error::msg)?;
            let reply: Value = serde_json::from_str(&line)?;
            if let Some(event) = reply.get("startup") {
                let event: StartupEvent = serde_json::from_value(event.clone())?;
                ensure!(
                    event.total > 0 && event.total <= 32 && event.completed < event.total,
                    "Invalid startup progress"
                );
                if let Ok(mut state) = self.startup.lock()
                    && state.status == "loading"
                {
                    state.message = event.message;
                    state.completed = event.completed;
                    state.total = event.total;
                    state.updated_at_ms = crate::screen::now_ms();
                }
                continue;
            }
            break reply;
        };
        ensure!(
            reply.get("error").is_none(),
            "Spatial worker: {}",
            reply["error"]
        );
        let result = reply
            .get("result")
            .cloned()
            .context("Spatial worker returned no result")?;
        if let Ok(mut state) = self.startup.lock()
            && state.status == "loading"
        {
            state.finish(None);
        }
        Ok(result)
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct Slot {
    worker: Mutex<Option<Worker>>,
    startup: Arc<Mutex<Startup>>,
}
impl Default for Slot {
    fn default() -> Self {
        Self {
            worker: Mutex::new(None),
            startup: Arc::new(Mutex::new(Startup::starting())),
        }
    }
}
type Pool = std::collections::HashMap<String, Arc<Slot>>;
static POOL: OnceLock<Mutex<Pool>> = OnceLock::new();

fn pool_key(config: &WorkerConfig, slot: &str) -> String {
    format!(
        "{slot}:{}",
        serde_json::to_string(config).expect("Worker configuration serializes")
    )
}

/// Read only the small status lock, never the worker lock held during inference.
/// This process-local state cannot show a previous launch's saved completion.
pub fn startup(config: &WorkerConfig, slot: &str) -> Option<Startup> {
    let handle = POOL
        .get()?
        .lock()
        .ok()?
        .get(&pool_key(config, slot))?
        .clone();
    let value = handle.startup.lock().ok()?.clone();
    Some(value)
}

pub fn call(config: &WorkerConfig, slot: &str, request: Value) -> Result<Value> {
    let key = pool_key(config, slot);
    let handle = POOL
        .get_or_init(Default::default)
        .lock()
        .map_err(|_| anyhow::anyhow!("Spatial pool lock poisoned"))?
        .entry(key)
        .or_default()
        .clone();
    let mut worker = handle
        .worker
        .lock()
        .map_err(|_| anyhow::anyhow!("Spatial worker lock poisoned"))?;
    let result: Result<Value> = (|| {
        if worker.is_none() {
            let mut state = handle
                .startup
                .lock()
                .map_err(|_| anyhow::anyhow!("Startup status lock poisoned"))?;
            // Keep a startup error readable and avoid spawning a broken worker
            // for every captured frame. A later request can retry normally.
            ensure!(
                state.status != "failed"
                    || crate::screen::now_ms().saturating_sub(state.updated_at_ms) >= 5000,
                "{}",
                state.message
            );
            *state = Startup::starting();
            drop(state);
            *worker = Some(Worker::start_with_status(config, handle.startup.clone())?);
        }
        worker.as_mut().unwrap().call(request)
    })();
    if let Err(error) = &result {
        if let Ok(mut state) = handle.startup.lock()
            && state.status == "loading"
        {
            state.finish(Some(&format!(
                "Could not load visual tracking: {:#}",
                error
            )));
        }
        *worker = None;
    }
    result
}

// nix-ld and these colon-separated paths exist only on Unix.
#[cfg(all(test, unix))]
mod library_path_tests {
    use super::foreign_library_path;
    #[test]
    fn nix_ld_libraries_follow_the_app_runtime_libraries() {
        let joined = foreign_library_path(Some("/app/lib".into()), Some("/nix-ld/lib".into()));
        assert_eq!(joined, Some("/app/lib:/nix-ld/lib".into()));
        // nix-ld handles an unset LD_LIBRARY_PATH itself; non-NixOS has no NIX_LD.
        assert_eq!(foreign_library_path(None, Some("/nix-ld/lib".into())), None);
        assert_eq!(foreign_library_path(Some("/app/lib".into()), None), None);
        assert_eq!(
            foreign_library_path(
                Some("/app/lib:/nix-ld/lib".into()),
                Some("/nix-ld/lib".into())
            ),
            Some("/app/lib:/nix-ld/lib".into())
        );
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn progress_is_readable_during_loading_and_preserves_rpc_replies() {
        let directory = tempfile::tempdir().unwrap();
        let gate = directory.path().join("continue");
        let config = WorkerConfig {
            executable: "sh".into(),
            arguments: vec!["-c".into(), r#"
                read -r request
                printf '%s\n' '{"startup":{"message":"Loading test models","completed":1,"total":3}}'
                while [ ! -e "$1" ]; do sleep 0.01; done
                printf '%s\n' '{"result":{"model":"test"}}'
                read -r request
                printf '%s\n' '{"result":{"registered":true}}'
            "#.into(), "worker".into(), gate.to_string_lossy().into_owned()],
        };
        assert!(startup(&config, "test").is_none());
        let worker_config = config.clone();
        let work = std::thread::spawn(move || call(&worker_config, "test", json!({"op":"info"})));
        let deadline = Instant::now() + Duration::from_secs(3);
        let state = loop {
            if let Some(state) = startup(&config, "test")
                && state.completed == 1
            {
                break state;
            }
            assert!(
                Instant::now() < deadline,
                "Loading progress was not observable"
            );
            std::thread::sleep(Duration::from_millis(5));
        };
        assert_eq!(state.status, "loading");
        assert!(!work.is_finished());
        std::fs::write(gate, "").unwrap();
        assert_eq!(work.join().unwrap().unwrap(), json!({"model":"test"}));
        let ready = startup(&config, "test").unwrap();
        assert_eq!(ready.status, "ready");
        assert_eq!(ready.completed, 3);
        assert_eq!(ready.started_at_ms, state.started_at_ms);
        assert_eq!(
            call(&config, "test", json!({"op":"register"})).unwrap(),
            json!({"registered":true})
        );
        assert_eq!(
            startup(&config, "test").unwrap().started_at_ms,
            ready.started_at_ms
        );
    }

    #[test]
    fn startup_failure_is_visible_and_never_reported_as_ready() {
        let directory = tempfile::tempdir().unwrap();
        let config = WorkerConfig {
            executable: directory
                .path()
                .join("missing-worker")
                .to_string_lossy()
                .into_owned(),
            arguments: vec![],
        };
        assert!(call(&config, "test", json!({"op":"info"})).is_err());
        let state = startup(&config, "test").unwrap();
        assert_eq!(state.status, "failed");
        assert!(state.message.contains("Could not load visual tracking"));
        assert_eq!(state.completed, 0);
        assert!(call(&config, "test", json!({"op":"info"})).is_err());
        assert_eq!(
            startup(&config, "test").unwrap().started_at_ms,
            state.started_at_ms
        );
    }

    #[test]
    fn workers_without_progress_events_still_load() {
        let config = WorkerConfig {
            executable: "sh".into(),
            arguments: vec![
                "-c".into(),
                "read -r request; printf '%s\\n' '{\"result\":{\"model\":\"legacy-test\"}}'".into(),
            ],
        };
        assert_eq!(
            call(&config, "legacy-test", json!({"op":"info"})).unwrap(),
            json!({"model":"legacy-test"})
        );
        let state = startup(&config, "legacy-test").unwrap();
        assert_eq!(state.status, "ready");
        assert_eq!(state.total, 0);
    }
}
