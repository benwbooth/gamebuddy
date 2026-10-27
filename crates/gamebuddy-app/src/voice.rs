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
        #[qproperty(bool, speaking)]
        #[qproperty(QString, status)]
        #[qproperty(QString, error)]
        #[qproperty(QString, device_name)]
        type VoiceInput = super::VoiceInputRust;
        /// The player started talking over GameBuddy's spoken reply.
        #[qsignal]
        fn barged_in(self: Pin<&mut VoiceInput>);
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
            realtime: bool,
        ) -> QString;
    }
}
use cxx_qt::CxxQtType;
use cxx_qt_lib::QString;
use gamebuddy_services::voice::{BargeIn, MicGate, Segmenter, mute::Monitor, save_turn};
use std::{
    path::Path,
    pin::Pin,
    time::{Duration, Instant},
};

pub struct VoiceInputRust {
    armed: bool,
    active: bool,
    listening: bool,
    speaking: bool,
    status: QString,
    error: QString,
    device_name: QString,
    gate: MicGate,
    segmenter: Segmenter,
    barge: BargeIn,
    monitor: Option<Monitor>,
    native: cxx::UniquePtr<ffi::Microphone>,
    resume_after: Instant,
    pcm: Vec<u8>,
}
impl Default for VoiceInputRust {
    fn default() -> Self {
        Self {
            armed: false,
            active: false,
            listening: false,
            speaking: false,
            status: QString::from("Waiting for microphone"),
            error: QString::default(),
            device_name: QString::default(),
            gate: MicGate::default(),
            segmenter: Segmenter::default(),
            barge: BargeIn::default(),
            monitor: None,
            native: cxx::UniquePtr::null(),
            resume_after: Instant::now(),
            pcm: Vec::new(),
        }
    }
}
impl ffi::VoiceInput {
    pub fn toggle_listening(mut self: Pin<&mut Self>) {
        if self.rust().gate.active {
            self.as_mut().rust_mut().gate.stop();
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
        self.as_mut().set_speaking(false);
    }
    pub fn poll(
        mut self: Pin<&mut Self>,
        directory: QString,
        busy: bool,
        speaking: bool,
        push_to_talk: bool,
        realtime: bool,
    ) -> QString {
        let armed = *self.armed();
        self.as_mut().rust_mut().gate.set_hands_free(armed);
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
            if update.muted == Some(true)
                && *self.listening()
                && !busy
                && !speaking
                && !push_to_talk
            {
                finished = self.as_mut().rust_mut().segmenter.finish();
            }
            self.as_mut().rust_mut().gate.observe(&id, update.muted);
            self.as_mut().set_error(QString::from(update.error));
        }
        let active = self.rust().gate.active;
        self.as_mut().set_active(active);
        if realtime {
            // Chromium owns capture, echo cancellation and playback for WebRTC.
            // Keep the same system-mute gate, without opening a second recorder.
            self.as_mut().suspend_capture();
            self.set_status(QString::from(if active {
                "Live Codex voice"
            } else {
                "Unmute your mic or choose Listen"
            }));
            return QString::default();
        }
        // While GameBuddy speaks, keep listening for the player talking over
        // it; otherwise capture pauses while a turn is being answered.
        let blocked = (busy && !speaking) || push_to_talk;
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
                self.as_mut().rust_mut().gate.stop();
                self.as_mut().set_active(false);
            }
            self.as_mut().set_listening(started);
        }
        if *self.listening() {
            let error = self.rust().native.error();
            if !error.is_empty() {
                self.as_mut().set_error(error);
                self.as_mut().rust_mut().gate.stop();
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
                if speaking {
                    // A short tail after GameBuddy stops is still its echo.
                    self.as_mut().rust_mut().resume_after =
                        Instant::now() + Duration::from_millis(400);
                    // GameBuddy's own voice must not become a turn: only an
                    // interruption starts one, with the audio that caused it.
                    if self.as_mut().rust_mut().barge.feed(&samples) {
                        let audio = self.as_mut().rust_mut().barge.take_audio();
                        let _ = self.as_mut().rust_mut().segmenter.feed(&audio);
                        self.as_mut().barged_in();
                    }
                } else if Instant::now() < self.rust().resume_after
                    && !self.rust().segmenter.speaking()
                {
                    self.as_mut().rust_mut().barge.reset();
                } else {
                    self.as_mut().rust_mut().barge.reset();
                    if let Some(turn) = self.as_mut().rust_mut().segmenter.feed(&samples) {
                        finished = Some(turn);
                    }
                }
                let speaking = self.rust().segmenter.speaking();
                self.as_mut().set_speaking(speaking);
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
