//! Gemini Live speech-to-speech. The page owns audio and the WebSocket; this
//! object serves it from localhost, builds its session setup, and records the
//! conversation. Game questions arrive as `ask_gamebuddy` tool calls that QML
//! answers with the configured reasoning provider.
#[cxx_qt::bridge]
pub mod ffi {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }
    unsafe extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qproperty(QString, tool_event_json)]
        type GeminiVoice = super::GeminiVoiceRust;
        #[qinvokable]
        fn page_url(self: Pin<&mut GeminiVoice>) -> QString;
        #[qinvokable]
        fn session(
            self: &GeminiVoice,
            directory: QString,
            game: QString,
            context: QString,
        ) -> QString;
        #[qinvokable]
        fn record(
            self: &GeminiVoice,
            directory: QString,
            game: QString,
            role: QString,
            text: QString,
        );
        #[qinvokable]
        fn save_api_key(self: &GeminiVoice, key: QString) -> QString;
        #[qinvokable]
        fn has_api_key(self: &GeminiVoice, directory: QString) -> bool;
        #[qinvokable]
        fn call_tool(
            self: Pin<&mut GeminiVoice>,
            directory: QString,
            id: QString,
            name: QString,
            arguments: QString,
        );
        #[qinvokable]
        fn latest_frame(self: &GeminiVoice, directory: QString, after_ms: f64) -> QString;
        #[qinvokable]
        fn scene_changed(self: Pin<&mut GeminiVoice>, directory: QString) -> bool;
        #[qinvokable]
        fn note(self: &GeminiVoice, directory: QString, kind: QString, value: QString);
        #[qinvokable]
        fn player_said(self: &GeminiVoice, directory: QString, text: QString) -> QString;
        #[qinvokable]
        fn located_area(self: &GeminiVoice, directory: QString) -> QString;
    }
    impl cxx_qt::Threading for GeminiVoice {}
}
use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::QString;
use gamebuddy_services::{LiveVoice, Settings, Store};
use serde_json::{Value, json};
use std::{pin::Pin, sync::Arc, time::Duration};

#[derive(Default)]
pub struct GeminiVoiceRust {
    page_server: Option<Arc<()>>,
    tool_event_json: QString,
    scene: Option<Vec<u8>>,
}

/// Recent tool failures by tool name. A live model retries a failing tool
/// in a tight loop (seventy calls in forty seconds were seen) and never
/// answers; after two failures in a minute the call is refused unrun.
static FAILURES: std::sync::Mutex<Vec<(String, std::time::Instant)>> =
    std::sync::Mutex::new(Vec::new());

fn recent_failures(name: &str) -> usize {
    let mut failures = FAILURES.lock().unwrap_or_else(|e| e.into_inner());
    failures.retain(|(_, at)| at.elapsed() < Duration::from_secs(60));
    failures.iter().filter(|(n, _)| n == name).count()
}

fn record_failure(name: &str) {
    FAILURES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push((name.to_owned(), std::time::Instant::now()));
}

fn live_settings(directory: &str) -> anyhow::Result<(Store, LiveVoice)> {
    let store = Store::open(directory)?;
    let settings = store.get::<Settings>("settings")?.unwrap_or_default();
    Ok((store, settings.live_voice))
}

pub fn tools() -> Value {
    gamebuddy_services::voice::prompts::gemini_tools()
}

impl ffi::GeminiVoice {
    pub fn page_url(mut self: Pin<&mut Self>) -> QString {
        let Ok(server) = tiny_http::Server::http("127.0.0.1:0") else {
            return QString::default();
        };
        let url = format!("http://{}/gemini", server.server_addr());
        let guard = Arc::new(());
        let alive = Arc::downgrade(&guard);
        self.as_mut().rust_mut().page_server = Some(guard);
        std::thread::spawn(move || {
            while alive.upgrade().is_some() {
                if let Ok(Some(request)) = server.recv_timeout(Duration::from_millis(100)) {
                    let response = if request.url() == "/gemini"
                        && request.method() == &tiny_http::Method::Get
                    {
                        tiny_http::Response::from_string(include_str!("../assets/gemini.html"))
                            .with_header(
                                tiny_http::Header::from_bytes(
                                    "Content-Type",
                                    "text/html; charset=utf-8",
                                )
                                .unwrap(),
                            )
                            .with_header(
                                tiny_http::Header::from_bytes("Cache-Control", "no-store").unwrap(),
                            )
                    } else {
                        tiny_http::Response::from_string("Not found").with_status_code(404)
                    };
                    let _ = request.respond(response);
                }
            }
        });
        QString::from(url)
    }
    /// Everything the page needs to open a session, or an error message.
    pub fn session(&self, directory: QString, game: QString, context: QString) -> QString {
        let result = Store::open(directory.to_string()).and_then(|store| {
            gamebuddy_services::voice::prompts::gemini_session(
                &store,
                &game.to_string(),
                &context.to_string(),
            )
        });
        QString::from(
            match result {
                Ok(value) => value,
                Err(error) => json!({"error": format!("{error:#}")}),
            }
            .to_string(),
        )
    }
    pub fn record(&self, directory: QString, game: QString, role: QString, text: QString) {
        let text = text.to_string();
        if text.trim().is_empty() {
            return;
        }
        if let Ok(store) = Store::open(directory.to_string()) {
            let id = format!("gemini-{}", gamebuddy_services::screen::now_ms());
            // The brain answers the player's latest words when Gemini next
            // looks up the game (see call_tool).
            if role.to_string() == "user" {
                let _ = store.put(
                    "last-player-utterance",
                    &json!({"text":text.trim(),"at_ms":gamebuddy_services::screen::now_ms()}),
                );
            }
            let _ = store.append_conversation(
                &game.to_string(),
                "gemini-live",
                &id,
                &role.to_string(),
                text.trim(),
            );
        }
    }
    pub fn save_api_key(&self, key: QString) -> QString {
        QString::from(match LiveVoice::save_api_key(&key.to_string()) {
            Ok(()) => String::new(),
            Err(error) => format!("{error:#}"),
        })
    }
    /// Run one GameBuddy tool off the UI thread and return its exact images,
    /// including reference maps and historical journal screenshots.
    pub fn call_tool(
        self: Pin<&mut Self>,
        directory: QString,
        id: QString,
        name: QString,
        arguments: QString,
    ) {
        let (directory, id, name, arguments) = (
            directory.to_string(),
            id.to_string(),
            name.to_string(),
            arguments.to_string(),
        );
        let ui = self.qt_thread();
        std::thread::spawn(move || {
            let started = std::time::Instant::now();
            let mut images = Vec::new();
            let repeated = recent_failures(&name) >= 2;
            let result = (|| -> anyhow::Result<Value> {
                anyhow::ensure!(
                    !repeated,
                    "{name} already failed twice in the last minute, so it was not run again"
                );
                anyhow::ensure!(
                    tools()[0]["functionDeclarations"]
                        .as_array()
                        .is_some_and(|d| d.iter().any(|t| t["name"] == name)),
                    "That tool is not available in live voice"
                );
                let store = Store::open(&directory)?;
                let settings: Settings = store.get("settings")?.unwrap_or_default();
                let native_reasoning = settings.live_voice.extended_thinking();
                anyhow::ensure!(
                    !(native_reasoning && name == "ask_brain"),
                    "Gemini Extended Thinking reasons directly from game context and images"
                );
                let arguments: Value =
                    serde_json::from_str(&arguments).unwrap_or_else(|_| json!({}));
                let output =
                    gamebuddy_services::harness::call_tool_with_images(&store, &name, arguments)?;
                images = output.images;
                let mut value = output.value;
                if name == "get_game_context" {
                    let guide = store
                        .get::<String>("active_guide")?
                        .and_then(|id| store.guide(&id).ok());
                    // The player's latest words, answered by the brain (a
                    // stronger model reading the screen with the plan) rather
                    // than left to the live voice's own reasoning.
                    let brain_answer = guide.as_ref().and_then(|guide| {
                        let said: Value = store.get("last-player-utterance").ok()??;
                        let at = said["at_ms"].as_u64()?;
                        let text = said["text"].as_str()?;
                        let answered: u64 = store.get("brain-answered-at").ok()?.unwrap_or(0);
                        (!native_reasoning
                            && settings.brain.enabled
                            && at > answered
                            && gamebuddy_services::screen::now_ms().saturating_sub(at) < 30_000
                            && text.split_whitespace().count() >= 3)
                            .then_some(())?;
                        let _ = store.put("brain-answered-at", &at);
                        gamebuddy_services::brain::think(&store, guide, Some(text))
                            .ok()
                            .and_then(|(_, answer)| answer)
                            .map(|answer| json!({"question":text,"answer":answer}))
                    });
                    value = if native_reasoning {
                        gamebuddy_services::voice::prompts::gemini_thinking_context(
                            &value,
                            guide.as_ref().map(|g| (&store, g)),
                        )
                    } else {
                        gamebuddy_services::voice::prompts::gemini_context(
                            &value,
                            guide.as_ref().map(|g| (&store, g)),
                        )
                    };
                    if let Some(answer) = brain_answer {
                        value["brain_answer"] = answer;
                        value["instruction"] = json!(
                            "brain_answer is GameBuddy's brain answering what the player just said, from the screen and plan. Say it to the player in your own words; it takes precedence over your own reading."
                        );
                    }
                }
                if name == "get_game_screen"
                    && let Some(object) = value.as_object_mut()
                {
                    object.insert("image".into(), json!("The current screenshot is the image attached just before this result. Describe the screen and the player's position from that image."));
                }
                Ok(value)
            })();
            let response = match result {
                Ok(Value::Object(object)) => Value::Object(object),
                Ok(other) => json!({"result": other}),
                Err(error) => {
                    record_failure(&name);
                    json!({"error": format!("{error:#}"), "retry": false,
                        "instruction": "This tool failed. Do not call it again for this question. Answer the player now from get_game_context and what you can see, and say briefly what is unavailable."})
                }
            };
            let event = json!({"id":id,"name":name,"response":response,"images":images,"elapsed_ms":started.elapsed().as_millis() as u64}).to_string();
            let _ = ui
                .queue(move |mut object| object.as_mut().set_tool_event_json(QString::from(event)));
        });
    }
    /// The current game frame as base64 JPEG when it is newer than after_ms.
    pub fn latest_frame(&self, directory: QString, after_ms: f64) -> QString {
        let store = Store::open(directory.to_string()).ok();
        let frame = store
            .as_ref()
            .and_then(|store| gamebuddy_services::screen::snapshot(store).ok())
            .filter(|s| s.captured_at_ms as f64 > after_ms);
        // The screenshot widget shows exactly what the live voice was sent.
        if let (Some(store), Some(frame)) = (&store, &frame) {
            let _ = gamebuddy_services::screen::seen::record(store, frame, "live");
        }
        QString::from(
            frame
                .map(|s| json!({"at":s.captured_at_ms,"jpeg":s.jpeg}).to_string())
                .unwrap_or_default(),
        )
    }
    /// True once per scene change since the last call (first call too).
    pub fn scene_changed(mut self: Pin<&mut Self>, directory: QString) -> bool {
        let Ok(store) = Store::open(directory.to_string()) else {
            return false;
        };
        match gamebuddy_services::screen::scene_change(&store, self.rust().scene.as_deref()) {
            Ok((signature, changed)) => {
                if changed {
                    self.as_mut().rust_mut().scene = Some(signature);
                }
                changed
            }
            Err(_) => false,
        }
    }
    /// Keep the last 200 live-session events (frames sent, tool calls) for
    /// diagnosing a session afterwards.
    pub fn note(&self, directory: QString, kind: QString, value: QString) {
        if let Ok(store) = Store::open(directory.to_string()) {
            let mut notes: Vec<Value> = store
                .get("gemini-diagnostics")
                .ok()
                .flatten()
                .unwrap_or_default();
            notes.push(json!({"at_ms":gamebuddy_services::screen::now_ms(),"kind":kind.to_string(),"value":serde_json::from_str::<Value>(&value.to_string()).unwrap_or(Value::Null)}));
            let excess = notes.len().saturating_sub(200);
            notes.drain(..excess);
            let _ = store.put("gemini-diagnostics", &notes);
        }
    }
    /// Record "I'm in <area>" straight away, so the route follows the
    /// player's word before the live model looks anything up.
    pub fn player_said(&self, directory: QString, text: QString) -> QString {
        let result = (|| -> anyhow::Result<Option<String>> {
            let store = Store::open(directory.to_string())?;
            let Some(id) = store.get::<String>("active_guide")? else {
                return Ok(None);
            };
            let guide = store.guide(&id)?;
            gamebuddy_services::navigation::tracking::report_spoken_location(
                &store,
                &guide,
                &text.to_string(),
            )
        })();
        QString::from(result.ok().flatten().unwrap_or_default())
    }
    /// The player's area once this session has evidence for it (screen map
    /// match, their report); empty while visual tracking is still starting.
    pub fn located_area(&self, directory: QString) -> QString {
        let result = (|| -> anyhow::Result<Option<String>> {
            let store = Store::open(directory.to_string())?;
            let Some(id) = store.get::<String>("active_guide")? else {
                return Ok(None);
            };
            let guide = store.guide(&id)?;
            let atlas = gamebuddy_services::navigation::atlas(&store, &guide)?;
            let model = gamebuddy_services::outcomes::prepared(&store, &guide)?
                .map(|p| p.model.clone())
                .unwrap_or_default();
            let graph = gamebuddy_core::planner::PlanGraph::build(&guide, &atlas, &model);
            let (area, confirmed) =
                gamebuddy_services::navigation::planning::locate(&store, &guide, &graph)?;
            Ok(area.filter(|_| confirmed))
        })();
        QString::from(result.ok().flatten().unwrap_or_default())
    }
    pub fn has_api_key(&self, directory: QString) -> bool {
        live_settings(&directory.to_string()).is_ok_and(|(_, live)| live.api_key().is_ok())
    }
}
