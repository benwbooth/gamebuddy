//! Neural (Piper) speech for local voice mode. Replies are synthesized one
//! sentence at a time so the first sentence plays while later ones render;
//! QML plays the WAV files in order and falls back to system speech on error.
#[cxx_qt::bridge]
pub mod ffi {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }
    unsafe extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qproperty(bool, available)]
        #[qproperty(QString, event_json)]
        type NeuralVoice = super::NeuralVoiceRust;
        #[qinvokable]
        fn configure(self: Pin<&mut NeuralVoice>, directory: QString) -> bool;
        #[qinvokable]
        fn speak(self: Pin<&mut NeuralVoice>, text: QString) -> bool;
        #[qinvokable]
        fn stop(self: Pin<&mut NeuralVoice>);
    }
    impl cxx_qt::Threading for NeuralVoice {}
}
use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::QString;
use gamebuddy_services::{
    Settings, Store,
    voice::speech::{Config, Speech, cached},
};
use serde_json::json;
use std::{path::PathBuf, pin::Pin, sync::Arc};

#[derive(Default)]
pub struct NeuralVoiceRust {
    available: bool,
    event_json: QString,
    speech: Option<Speech>,
    config: Option<Config>,
    output: PathBuf,
    sequence: u64,
}

/// Piper is usable in local mode whenever it is installed; the settings
/// checkbox only controls prepared speech inside Codex voice.
fn installed(store: &Store) -> Option<Config> {
    let mut config = store
        .get::<Settings>("settings")
        .ok()
        .flatten()?
        .prepared_speech?;
    config.enabled = true;
    config.validate().ok().map(|_| config)
}

impl ffi::NeuralVoice {
    pub fn configure(mut self: Pin<&mut Self>, directory: QString) -> bool {
        let Ok(store) = Store::open(directory.to_string()) else {
            return false;
        };
        let config = installed(&store);
        if config != self.rust().config {
            let output = store.root.join("speech").join("replies");
            // Replies are disposable; keep only what this session produces.
            let _ = std::fs::remove_dir_all(&output);
            let _ = std::fs::create_dir_all(&output);
            let speech = config
                .clone()
                .map(|c| Speech::start(store.root.join("speech"), c));
            let mut this = self.as_mut().rust_mut();
            this.output = output;
            this.speech = speech;
            this.config = config;
        }
        let available = self.rust().speech.is_some();
        self.as_mut().set_available(available);
        available
    }
    pub fn speak(mut self: Pin<&mut Self>, text: QString) -> bool {
        if self.rust().speech.is_none() {
            return false;
        }
        self.as_mut().rust_mut().sequence += 1;
        let sequence = self.rust().sequence;
        let output = self.rust().output.clone();
        let ui = self.qt_thread();
        let text = text.to_string();
        let done = Arc::new(move |index: usize, result: anyhow::Result<_>| {
            let event = match result.and_then(|chunk: gamebuddy_services::voice::speech::Chunk| {
                let path = output.join(format!("{}.wav", chunk.key));
                if !path.is_file() {
                    let wav = cached(&chunk.key).ok_or_else(|| anyhow::anyhow!("Synthesized audio expired"))?;
                    std::fs::write(&path, wav.as_slice())?;
                }
                Ok(json!({"kind":"chunk","sequence":sequence,"index":chunk.index,"last":chunk.last,"text":chunk.text,"url":url::Url::from_file_path(&path).map(|u| u.to_string()).unwrap_or_default()}))
            }) {
                Ok(event) => event,
                Err(error) => json!({"kind":"error","sequence":sequence,"index":index,"error":format!("{error:#}")}),
            }
            .to_string();
            let _ = ui.queue(move |mut object| {
                if object.rust().sequence == sequence {
                    object.as_mut().set_event_json(QString::from(event));
                }
            });
        });
        self.rust().speech.as_ref().unwrap().speak(text, done);
        true
    }
    pub fn stop(mut self: Pin<&mut Self>) {
        self.as_mut().rust_mut().sequence += 1;
        if let Some(speech) = &self.rust().speech {
            speech.cancel();
        }
    }
}
