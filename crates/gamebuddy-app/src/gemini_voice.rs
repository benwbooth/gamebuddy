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
    /// Run one GameBuddy tool off the UI thread; the result arrives as
    /// tool_event_json. Images are left out: Gemini already sees the screen.
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
            let result = (|| -> anyhow::Result<Value> {
                anyhow::ensure!(
                    tools()[0]["functionDeclarations"]
                        .as_array()
                        .is_some_and(|d| d.iter().any(|t| t["name"] == name)),
                    "That tool is not available in live voice"
                );
                let store = Store::open(&directory)?;
                let arguments: Value =
                    serde_json::from_str(&arguments).unwrap_or_else(|_| json!({}));
                let output =
                    gamebuddy_services::harness::call_tool_with_images(&store, &name, arguments)?;
                let mut value = output.value;
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
                Err(error) => json!({"error": format!("{error:#}")}),
            };
            let event = json!({"id":id,"name":name,"response":response,"elapsed_ms":started.elapsed().as_millis() as u64}).to_string();
            let _ = ui
                .queue(move |mut object| object.as_mut().set_tool_event_json(QString::from(event)));
        });
    }
    /// The current game frame as base64 JPEG when it is newer than after_ms.
    pub fn latest_frame(&self, directory: QString, after_ms: f64) -> QString {
        let frame = Store::open(directory.to_string())
            .ok()
            .and_then(|store| gamebuddy_services::screen::snapshot(&store).ok())
            .filter(|s| s.captured_at_ms as f64 > after_ms);
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
    pub fn has_api_key(&self, directory: QString) -> bool {
        live_settings(&directory.to_string()).is_ok_and(|(_, live)| live.api_key().is_ok())
    }
}
