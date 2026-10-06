//! Durable Codex conversations, separate from disposable perception/planning jobs.
//! Codex owns the rollout; SQLite stores its identity before the first turn.
use super::{CodexClient, RpcError, codex_model};
use crate::{Store, conversation};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    path::Path,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug)]
pub enum Role {
    Reasoner,
    Speaker,
    Chat,
}
impl Role {
    fn name(self) -> &'static str {
        match self {
            Self::Reasoner => "reasoner",
            Self::Speaker => "speaker",
            Self::Chat => "chat",
        }
    }
}

pub struct Options<'a> {
    pub model: &'a str,
    pub instructions: &'a str,
    pub tools: Value,
    pub tier: Option<&'a str>,
    pub effort: &'a str,
}

#[derive(Clone, Debug)]
pub struct Opened {
    pub id: String,
    pub resumed: bool,
    /// A missing rollout or incompatible tool catalog requires a fresh thread.
    pub replacement: Option<&'static str>,
}

pub(super) struct Lease {
    opened: Opened,
    _file: File,
}

#[derive(Serialize, Deserialize)]
struct Saved {
    thread_id: String,
    tools_sha256: String,
    previous_thread_id: Option<String>,
}

fn key(game: &str, role: Role) -> Result<String> {
    let game = conversation::game_key(game);
    ensure!(
        !game.is_empty(),
        "Load a game before opening its saved session"
    );
    Ok(format!("llm-session:codex:{}:{game}", role.name()))
}

fn acquire(path: &Path, deadline: Instant) -> Result<File> {
    let file = File::options()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)?;
    loop {
        match fs2::FileExt::try_lock_exclusive(&file) {
            Ok(()) => return Ok(file),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                ensure!(
                    Instant::now() < deadline,
                    "This game's saved LLM session is still in use"
                );
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => return Err(e.into()),
        }
    }
}

fn missing_rollout(error: &anyhow::Error) -> bool {
    error.downcast_ref::<RpcError>().is_some_and(|e| {
        e.method == "thread/resume"
            && e.error["code"] == -32600
            && e.error["message"]
                .as_str()
                .is_some_and(|m| m.starts_with("no rollout found for thread id "))
    })
}

fn open(
    store: &Store,
    key: &str,
    options: Options<'_>,
    cwd: &Path,
    mut request: impl FnMut(&str, Value) -> Result<Value>,
) -> Result<(Opened, String)> {
    let saved = store.get::<Saved>(key)?;
    let tools_sha256 = format!("{:x}", Sha256::digest(serde_json::to_vec(&options.tools)?));
    let mut params = json!({"cwd":cwd,"approvalPolicy":"never","sandbox":"read-only",
        "model":codex_model(options.model),"baseInstructions":format!("{}\n{}", options.instructions, conversation::MEMORY_RULES)});
    if options.effort != "auto" {
        params["config"] = json!({"model_reasoning_effort":if options.effort.is_empty() { "low" } else { options.effort }});
    }
    if let Some(tier) = options.tier {
        params["serviceTier"] = json!(tier);
    }
    let mut replacement = None;
    if let Some(saved) = &saved {
        // 0.160 cannot replace dynamicTools on resume. Keep the previous rollout
        // and bootstrap from the dialogue archive after a tool-schema upgrade.
        if saved.tools_sha256 == tools_sha256 {
            let mut resume = params.clone();
            resume["threadId"] = json!(saved.thread_id);
            resume["excludeTurns"] = json!(true);
            match request("thread/resume", resume) {
                Ok(value) => {
                    ensure!(
                        value["thread"]["id"] == saved.thread_id,
                        "Codex resumed a different thread"
                    );
                    return Ok((
                        Opened {
                            id: saved.thread_id.clone(),
                            resumed: true,
                            replacement: None,
                        },
                        value["model"].as_str().unwrap_or_default().into(),
                    ));
                }
                Err(error) if missing_rollout(&error) => replacement = Some("missing_rollout"),
                // Don't silently abandon history on auth, timeout, or other errors.
                Err(error) => return Err(error),
            }
        } else {
            replacement = Some("tools_changed");
        }
    }
    params["ephemeral"] = json!(false);
    params["environments"] = json!([]);
    params["dynamicTools"] = options.tools;
    let value = request("thread/start", params)?;
    let id = value["thread"]["id"]
        .as_str()
        .filter(|s| !s.is_empty())
        .context("Codex did not return a thread ID")?
        .to_owned();
    store.put(
        key,
        &Saved {
            thread_id: id.clone(),
            tools_sha256,
            previous_thread_id: saved.map(|s| s.thread_id),
        },
    )?;
    Ok((
        Opened {
            id,
            resumed: false,
            replacement,
        },
        value["model"].as_str().unwrap_or_default().into(),
    ))
}

impl CodexClient {
    pub fn saved_thread(
        &mut self,
        store: &Store,
        game: &str,
        role: Role,
        options: Options<'_>,
        deadline: Instant,
    ) -> Result<Opened> {
        let key = key(game, role)?;
        let directory = store.root.canonicalize()?.join("llm-sessions");
        std::fs::create_dir_all(&directory)?;
        let path = directory.join(format!("{:x}.lock", Sha256::digest(key.as_bytes())));
        if let Some(lease) = self.sessions.get(&path) {
            return Ok(lease.opened.clone());
        }
        // Old voice workers finish asynchronously. Never let two app servers
        // append to the same rollout, including across simultaneous launches.
        let file = acquire(&path, deadline)?;
        let cwd = self._directory.path().to_owned();
        let (opened, model) = open(store, &key, options, &cwd, |method, params| {
            self.request(method, params, deadline)
        })?;
        self.resolved_model = model;
        if std::env::args().any(|a| a == "--voice-trace")
            || std::env::var_os("GAMEBUDDY_PROVIDER_TRACE").is_some()
        {
            eprintln!(
                "GAMEBUDDY_CODEX_SESSION role={} resumed={} replacement={} thread={}",
                role.name(),
                opened.resumed,
                opened.replacement.unwrap_or("none"),
                opened.id
            );
        }
        self.sessions.insert(
            path,
            Lease {
                opened: opened.clone(),
                _file: file,
            },
        );
        Ok(opened)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options() -> Options<'static> {
        Options {
            model: "gpt-6.1-sol",
            instructions: "Use current pixels.",
            tools: json!([]),
            tier: Some("priority"),
            effort: "low",
        }
    }
    fn response(id: &str) -> Value {
        json!({"thread":{"id":id},"model":"gpt-6.1-sol"})
    }
    fn seed(store: &Store, key: &str) {
        open(store, key, options(), &store.root, |method, params| {
            assert_eq!(method, "thread/start");
            assert_eq!(params["ephemeral"], false);
            Ok(response("original"))
        })
        .unwrap();
    }

    #[test]
    fn restart_resumes_same_game_and_overrides_current_settings() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let first_key = key(" Faxanadu ", Role::Reasoner).unwrap();
        seed(&store, &first_key);
        drop(store);
        let store = Store::open(dir.path()).unwrap();
        let mut opts = options();
        opts.model = "gpt-6-luna";
        opts.instructions = "New live session: current screen overrides old location.";
        opts.tier = Some("default");
        opts.effort = "medium";
        let (opened, _) = open(
            &store,
            &key("FAXANADU", Role::Reasoner).unwrap(),
            opts,
            dir.path(),
            |method, params| {
                assert_eq!(method, "thread/resume");
                assert_eq!(params["threadId"], "original");
                assert_eq!(params["model"], "gpt-6-luna");
                assert_eq!(params["serviceTier"], "default");
                assert_eq!(params["config"]["model_reasoning_effort"], "medium");
                assert!(
                    params["baseInstructions"]
                        .as_str()
                        .unwrap()
                        .contains("current screen overrides")
                );
                assert_eq!(params["excludeTurns"], true);
                for unsupported in ["dynamicTools", "ephemeral", "environments", "history"] {
                    assert!(params.get(unsupported).is_none());
                }
                Ok(response("original"))
            },
        )
        .unwrap();
        assert!(opened.resumed);
        assert_eq!(opened.id, "original");
        assert_ne!(first_key, key("Zelda", Role::Reasoner).unwrap());
        for role in [Role::Speaker, Role::Chat] {
            assert_ne!(first_key, key("Faxanadu", role).unwrap());
        }
        let (other, _) = open(
            &store,
            &key("Zelda", Role::Reasoner).unwrap(),
            options(),
            dir.path(),
            |method, _| {
                assert_eq!(method, "thread/start");
                Ok(response("other-game"))
            },
        )
        .unwrap();
        assert!(!other.resumed);
    }

    #[test]
    fn lost_rollout_replaces_record_but_transient_errors_preserve_it() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let key = key("Faxanadu", Role::Reasoner).unwrap();
        seed(&store, &key);
        let result = open(&store, &key, options(), dir.path(), |method, _| {
            assert_eq!(method, "thread/resume");
            Err(RpcError {
                method: method.into(),
                error: json!({"code":-32000,"message":"Authentication temporarily unavailable"}),
            }
            .into())
        });
        assert!(result.is_err());
        assert_eq!(
            store.get::<Saved>(&key).unwrap().unwrap().thread_id,
            "original"
        );
        let mut calls = 0;
        let (replacement, _) = open(&store, &key, options(), dir.path(), |method, _| {
            calls += 1;
            if calls == 1 {
                assert_eq!(method, "thread/resume");
                Err(RpcError { method: method.into(), error: json!({"code":-32600,"message":"no rollout found for thread id original"}) }.into())
            } else { assert_eq!(method, "thread/start"); Ok(response("replacement")) }
        }).unwrap();
        assert_eq!(calls, 2);
        assert_eq!(replacement.replacement, Some("missing_rollout"));
        let saved = store.get::<Saved>(&key).unwrap().unwrap();
        assert_eq!(saved.thread_id, "replacement");
        assert_eq!(saved.previous_thread_id.as_deref(), Some("original"));
    }

    #[test]
    fn tool_upgrade_starts_compatible_session_and_retains_previous_identity() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let key = key("Faxanadu", Role::Reasoner).unwrap();
        seed(&store, &key);
        let mut opts = options();
        opts.tools = json!([{"name":"new_tool"}]);
        let (opened, _) = open(&store, &key, opts, dir.path(), |method, params| {
            assert_eq!(method, "thread/start");
            assert_eq!(params["dynamicTools"][0]["name"], "new_tool");
            Ok(response("upgraded"))
        })
        .unwrap();
        assert_eq!(opened.replacement, Some("tools_changed"));
        assert_eq!(
            store
                .get::<Saved>(&key)
                .unwrap()
                .unwrap()
                .previous_thread_id
                .as_deref(),
            Some("original")
        );
    }

    #[test]
    fn simultaneous_workers_cannot_write_the_same_rollout() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.lock");
        let lease = acquire(&path, Instant::now()).unwrap();
        assert!(acquire(&path, Instant::now()).is_err());
        drop(lease);
        assert!(acquire(&path, Instant::now()).is_ok());
    }
}
