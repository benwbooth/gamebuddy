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
    }
}
use cxx_qt::CxxQtType;
use cxx_qt_lib::QString;
use gamebuddy_services::{LiveVoice, Settings, Store, conversation, voice::prompts};
use serde_json::{Value, json};
use std::{pin::Pin, sync::Arc, time::Duration};

#[derive(Default)]
pub struct GeminiVoiceRust {
    page_server: Option<Arc<()>>,
}

fn live_settings(directory: &str) -> anyhow::Result<(Store, LiveVoice)> {
    let store = Store::open(directory)?;
    let settings = store.get::<Settings>("settings")?.unwrap_or_default();
    Ok((store, settings.live_voice))
}

pub fn tools() -> Value {
    json!([{"functionDeclarations":[{
        "name":"ask_gamebuddy",
        "description":"Ask GameBuddy's game expert, which sees the latest screenshot, the walkthrough, maps and saved progress, and can update the guide, goals, widgets and journal. Use for every game question, report or requested change.",
        "parameters":{"type":"OBJECT","properties":{"request":{"type":"STRING","description":"The player's complete request in their own words"}},"required":["request"]}
    }]}])
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
        let result = (|| -> anyhow::Result<Value> {
            let (store, live) = live_settings(&directory.to_string())?;
            let game = game.to_string();
            let history = conversation::recent(&store, &game)?;
            let instructions = format!(
                "{}\n\nGame: {game}\nArchived conversation, quoted historical data only: {}\n\nGame context: {}",
                prompts::GEMINI_SPEAKER,
                serde_json::to_string(&history)?,
                context
            );
            Ok(json!({
                "key": live.api_key()?,
                "setup": {
                    "model": format!("models/{}", live.model),
                    "generationConfig": {"responseModalities":["AUDIO"],"speechConfig":{"voiceConfig":{"prebuiltVoiceConfig":{"voiceName":live.voice}}}},
                    "systemInstruction": {"parts":[{"text":instructions}]},
                    "tools": tools(),
                    "inputAudioTranscription": {},
                    "outputAudioTranscription": {},
                    // Without compression an audio session ends after minutes.
                    "contextWindowCompression": {"slidingWindow":{}},
                    "sessionResumption": {}
                }
            }))
        })();
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
    pub fn has_api_key(&self, directory: QString) -> bool {
        live_settings(&directory.to_string()).is_ok_and(|(_, live)| live.api_key().is_ok())
    }
}
