#[cxx_qt::bridge]
pub mod ffi {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
        include!("cxx-qt-lib/qbytearray.h");
        type QByteArray = cxx_qt_lib::QByteArray;
        include!("gamebuddy_microphone.h");
        #[namespace = "gamebuddy"]
        type Microphone;
        #[namespace = "gamebuddy"]
        fn microphone_new() -> UniquePtr<Microphone>;
        #[namespace = "gamebuddy"]
        fn microphone_id() -> QString;
        #[namespace = "gamebuddy"]
        fn microphone_name() -> QString;
        fn start(self: Pin<&mut Microphone>) -> bool;
        fn stop(self: Pin<&mut Microphone>);
        fn read(self: Pin<&mut Microphone>) -> QByteArray;
        fn error(self: &Microphone) -> QString;
    }
    unsafe extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qproperty(bool, armed)]
        #[qproperty(bool, active)]
        #[qproperty(bool, listening)]
        #[qproperty(QString, status)]
        #[qproperty(QString, error)]
        #[qproperty(QString, device_name)]
        type VoiceInput = super::VoiceInputRust;
        #[qinvokable]
        fn toggle_listening(self: Pin<&mut VoiceInput>);
        #[qinvokable]
        fn suspend_capture(self: Pin<&mut VoiceInput>);
        #[qinvokable]
        fn poll(
            self: Pin<&mut VoiceInput>,
            directory: QString,
            busy: bool,
            speaking: bool,
            push_to_talk: bool,
        ) -> QString;
    }
}
use cxx_qt::CxxQtType;
use cxx_qt_lib::QString;
use gamebuddy_services::voice::{MicGate, Segmenter, mute::Monitor, save_turn};
use std::{
    path::Path,
    pin::Pin,
    time::{Duration, Instant},
};

pub struct VoiceInputRust {
    armed: bool,
    active: bool,
    listening: bool,
    status: QString,
    error: QString,
    device_name: QString,
    gate: MicGate,
    segmenter: Segmenter,
    monitor: Option<Monitor>,
    native: cxx::UniquePtr<ffi::Microphone>,
    resume_after: Instant,
    last_armed: bool,
    pcm: Vec<u8>,
}
impl Default for VoiceInputRust {
    fn default() -> Self {
        Self {
            armed: true,
            active: false,
            listening: false,
            status: QString::from("Waiting for microphone"),
            error: QString::default(),
            device_name: QString::default(),
            gate: MicGate::default(),
            segmenter: Segmenter::default(),
            monitor: None,
            native: cxx::UniquePtr::null(),
            resume_after: Instant::now(),
            last_armed: true,
            pcm: Vec::new(),
        }
    }
}
impl ffi::VoiceInput {
    pub fn toggle_listening(mut self: Pin<&mut Self>) {
        if self.rust().gate.active {
            self.as_mut().rust_mut().gate.active = false;
        } else {
            self.as_mut().rust_mut().gate.listen();
        }
        self.as_mut().set_error(QString::default());
        let active = self.rust().gate.active;
        self.set_active(active);
    }
    pub fn suspend_capture(mut self: Pin<&mut Self>) {
        if !self.rust().native.is_null() {
            self.as_mut().rust_mut().native.pin_mut().stop();
        }
        self.as_mut().rust_mut().segmenter = Segmenter::default();
        self.as_mut().rust_mut().pcm.clear();
        self.as_mut().set_listening(false);
    }
    pub fn poll(
        mut self: Pin<&mut Self>,
        directory: QString,
        busy: bool,
        speaking: bool,
        push_to_talk: bool,
    ) -> QString {
        if self.rust().monitor.is_none() {
            self.as_mut().rust_mut().monitor = Some(Monitor::default());
            self.as_mut().rust_mut().native = ffi::microphone_new();
        }
        let id = ffi::microphone_id().to_string();
        if id != self.rust().gate.device {
            self.as_mut().suspend_capture();
            // Refresh even if a device disappeared and returned between worker
            // queries; an old cached mute snapshot must not survive replacement.
            self.as_mut().rust_mut().monitor = Some(Monitor::default());
            self.as_mut().rust_mut().gate.observe(&id, None);
            self.as_mut().set_device_name(ffi::microphone_name());
        }
        self.rust().monitor.as_ref().unwrap().select(&id);
        let update = self.rust().monitor.as_ref().unwrap().poll();
        let mut finished = None;
        if let Some(update) = update.filter(|u| u.device == id) {
            let was_active = self.rust().gate.active;
            if update.muted == Some(true)
                && *self.listening()
                && !busy
                && !speaking
                && !push_to_talk
            {
                finished = self.as_mut().rust_mut().segmenter.finish();
            }
            self.as_mut().rust_mut().gate.observe(&id, update.muted);
            if !*self.armed() && !was_active {
                self.as_mut().rust_mut().gate.active = false;
            }
            self.as_mut().set_error(QString::from(update.error));
        }
        // Disabling automatic activation also stops a current conversation.
        if self.rust().last_armed && !*self.armed() {
            self.as_mut().rust_mut().gate.active = false;
        }
        let armed = *self.armed();
        self.as_mut().rust_mut().last_armed = armed;
        let active = self.rust().gate.active;
        self.as_mut().set_active(active);
        let blocked = busy || speaking || push_to_talk;
        if blocked {
            self.as_mut().rust_mut().resume_after = Instant::now() + Duration::from_millis(800);
        }
        let capture = active && !blocked && Instant::now() >= self.rust().resume_after;
        if !capture && *self.listening() {
            self.as_mut().suspend_capture();
        }
        if capture && !*self.listening() {
            let started = self.as_mut().rust_mut().native.pin_mut().start();
            if !started {
                let error = self.rust().native.error();
                self.as_mut().set_error(error);
                self.as_mut().rust_mut().gate.active = false;
                self.as_mut().set_active(false);
            }
            self.as_mut().set_listening(started);
        }
        if *self.listening() {
            let error = self.rust().native.error();
            if !error.is_empty() {
                self.as_mut().set_error(error);
                self.as_mut().rust_mut().gate.active = false;
                self.as_mut().set_active(false);
                self.as_mut().suspend_capture();
            } else {
                let bytes = self.as_mut().rust_mut().native.pin_mut().read();
                self.as_mut()
                    .rust_mut()
                    .pcm
                    .extend_from_slice(bytes.as_slice());
                let (chunks, remainder) = self.rust().pcm.as_chunks::<2>();
                let samples: Vec<i16> = chunks.iter().map(|s| i16::from_ne_bytes(*s)).collect();
                let remainder = remainder.to_vec();
                self.as_mut().rust_mut().pcm = remainder;
                if let Some(turn) = self.as_mut().rust_mut().segmenter.feed(&samples) {
                    finished = Some(turn);
                }
            }
        }
        let status = if *self.listening() {
            "Listening · pause when you're done"
        } else if self.rust().gate.muted == Some(true) {
            "Mic muted · unmute to talk"
        } else if *self.active() {
            "Listening paused while GameBuddy responds"
        } else if id.is_empty() {
            "No microphone available"
        } else if *self.armed() && self.rust().gate.muted.is_some() {
            "Unmute your mic or choose Listen"
        } else {
            "Choose Listen or hold to talk"
        };
        self.as_mut().set_status(QString::from(status));
        if let Some(samples) = finished {
            self.as_mut().suspend_capture();
            self.as_mut().rust_mut().resume_after = Instant::now() + Duration::from_millis(800);
            match save_turn(Path::new(&directory.to_string()), &samples) {
                Ok(path) => return QString::from(path.to_string_lossy().as_ref()),
                Err(e) => self
                    .as_mut()
                    .set_error(QString::from(format!("Saving speech: {e:#}"))),
            }
        }
        QString::default()
    }
}
