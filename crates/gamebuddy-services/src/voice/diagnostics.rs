//! A bounded local lifecycle record survives closing a game. No speech text,
//! audio, SDP or images are stored here, and disk writes never block voice.
use crate::Store;
use serde_json::{Value, json};
use std::{path::Path, sync::mpsc};

pub struct Diagnostics(mpsc::SyncSender<Value>);
impl Diagnostics {
    pub fn start(root: &Path) -> Self {
        let root = root.to_owned();
        let (tx, rx) = mpsc::sync_channel::<Value>(64);
        std::thread::spawn(move || {
            let Ok(store) = Store::open(root) else { return };
            let mut recent = store
                .get::<Vec<Value>>("voice-diagnostics")
                .ok()
                .flatten()
                .unwrap_or_default();
            let mut states = std::collections::HashMap::new();
            while let Ok(event) = rx.recv() {
                let kind = event["kind"].as_str().unwrap_or_default().to_owned();
                if kind.starts_with("browser/") && states.get(&kind) == Some(&event["value"]) {
                    continue;
                }
                states.insert(kind, event["value"].clone());
                recent.push(event);
                if recent.len() > 512 {
                    recent.drain(..recent.len() - 512);
                }
                let _ = store.put("voice-diagnostics", &recent);
            }
        });
        Self(tx)
    }
    pub fn record(&self, kind: &str, value: Value) {
        let _ = self
            .0
            .try_send(json!({"at_ms":crate::screen::now_ms(),"kind":kind,"value":value}));
    }
}
