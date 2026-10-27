//! Microphone activation and bounded, local speech turn detection.
pub mod briefing;
pub mod decision;
pub mod delivery;
pub mod diagnostics;
pub mod duck;
pub mod input;
pub mod live;
pub mod mute;
pub mod narration;
pub mod prefetch;
pub mod prepared;
pub mod priority;
pub mod prompts;
pub mod provider;
pub mod reply;
pub mod speech;
pub mod talk;
use std::{collections::VecDeque, path::Path};
use webrtc_vad::{SampleRate, Vad, VadMode};

/// Hands-free mode starts on the first confirmed unmuted microphone, then on
/// mute/unmute gestures. Stop and capture failures consume the startup request;
/// repeated snapshots must never reopen capture behind the user's back.
#[derive(Default, Debug)]
pub struct MicGate {
    pub device: String,
    pub muted: Option<bool>,
    pub active: bool,
    hands_free: bool,
    startup_pending: bool,
}
impl MicGate {
    pub fn set_hands_free(&mut self, enabled: bool) {
        if self.hands_free == enabled {
            return;
        }
        self.hands_free = enabled;
        self.stop();
        self.startup_pending = enabled;
        self.try_start();
    }
    fn try_start(&mut self) {
        if self.startup_pending && !self.device.is_empty() && self.muted.is_some() {
            self.startup_pending = false;
            self.active = self.muted == Some(false);
        }
    }
    pub fn observe(&mut self, device: &str, muted: Option<bool>) {
        if device != self.device || muted.is_none() {
            self.active = false;
        } else if self.hands_free && self.muted == Some(true) && muted == Some(false) {
            self.active = true;
        }
        if muted == Some(true) {
            self.active = false;
        }
        self.device = device.into();
        self.muted = muted;
        self.try_start();
    }
    pub fn listen(&mut self) {
        self.startup_pending = false;
        self.active = !self.device.is_empty() && self.muted != Some(true);
    }
    pub fn stop(&mut self) {
        self.startup_pending = false;
        self.active = false;
    }
}

pub const SAMPLE_RATE: u32 = 16_000;
const FRAME: usize = 320; // 20 ms, a supported WebRTC VAD frame size.
const PREROLL: usize = 10;
const MIN_VOICED: usize = 15;
const END_SILENCE: usize = 45;
const MAX_FRAMES: usize = 1250; // 25 seconds, safely inside Whisper's 45s limit.

pub struct Segmenter {
    vad: Vad,
    pending: VecDeque<i16>,
    preroll: VecDeque<i16>,
    utterance: Vec<i16>,
    voiced: usize,
    silence: usize,
}
impl Default for Segmenter {
    fn default() -> Self {
        Self {
            vad: Vad::new_with_rate_and_mode(SampleRate::Rate16kHz, VadMode::Aggressive),
            pending: VecDeque::new(),
            preroll: VecDeque::new(),
            utterance: Vec::new(),
            voiced: 0,
            silence: 0,
        }
    }
}
impl Segmenter {
    pub fn speaking(&self) -> bool {
        self.voiced >= 3 && self.silence < END_SILENCE
    }
    /// Streaming voice already has server-side endpointing. Release its local
    /// playback mask promptly so the first words of a quick reply are audible;
    /// the longer pause used to finalize a Whisper recording is independent.
    pub fn live_speaking(&self) -> bool {
        self.voiced >= 3 && self.silence < 10
    }
    pub fn feed(&mut self, samples: &[i16]) -> Option<Vec<i16>> {
        self.pending.extend(samples);
        while self.pending.len() >= FRAME {
            let frame: Vec<_> = self.pending.drain(..FRAME).collect();
            let speech = self.vad.is_voice_segment(&frame).unwrap_or(false);
            if let Some(turn) = self.frame(&frame, speech) {
                // Never queue speech recorded while a previous turn is being
                // transcribed. The caller pauses capture before processing it.
                self.pending.clear();
                return Some(turn);
            }
        }
        None
    }
    fn frame(&mut self, frame: &[i16], speech: bool) -> Option<Vec<i16>> {
        if self.utterance.is_empty() && !speech {
            self.preroll.extend(frame);
            while self.preroll.len() > PREROLL * FRAME {
                self.preroll.pop_front();
            }
            return None;
        }
        if self.utterance.is_empty() {
            self.utterance.extend(self.preroll.drain(..));
        }
        self.utterance.extend_from_slice(frame);
        if speech {
            self.voiced += 1;
            self.silence = 0;
        } else {
            self.silence += 1;
        }
        if self.silence >= END_SILENCE || self.utterance.len() >= MAX_FRAMES * FRAME {
            return self.finish();
        }
        None
    }
    pub fn finish(&mut self) -> Option<Vec<i16>> {
        let audio = std::mem::take(&mut self.utterance);
        let valid = self.voiced >= MIN_VOICED;
        self.voiced = 0;
        self.silence = 0;
        self.preroll.clear();
        self.pending.clear();
        valid.then_some(audio)
    }
}

pub fn save_turn(directory: &Path, samples: &[i16]) -> anyhow::Result<std::path::PathBuf> {
    std::fs::create_dir_all(directory)?;
    let file = tempfile::Builder::new()
        .prefix("utterance-")
        .suffix(".wav")
        .tempfile_in(directory)?;
    let mut wav = hound::WavWriter::new(
        file.as_file(),
        hound::WavSpec {
            channels: 1,
            sample_rate: SAMPLE_RATE,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )?;
    for sample in samples {
        wav.write_sample(*sample)?;
    }
    wav.finalize()?;
    Ok(file.into_temp_path().keep()?)
}

/// Focus changes require explicit commands, not arbitrary words in a strategy
/// question. Values 3/4/6 match the overlay actions; 7/8 open guide/arrangement.
pub fn overlay_command(text: &str) -> Option<i32> {
    let words = command_words(text);
    match words.as_str() {
        "open the guide" | "open guide" | "show the guide" => Some(7),
        "arrange widgets" | "edit widgets" => Some(8),
        "close the guide" | "close guide" | "back to the game" | "release overlay" => Some(6),
        "hide overlay" | "hide the overlay" => Some(4),
        "show overlay" | "show the overlay" => Some(3),
        _ => None,
    }
}

/// Connection checks and short social turns need neither screenshots nor a
/// reasoning round trip. Exact whole requests only; compound questions go to
/// the backend so this never swallows a gameplay question or requested action.
pub fn quick_reply(text: &str) -> Option<&'static str> {
    if let Some(reply) = presence_reply(text) {
        return Some(reply);
    }
    match command_words(text).as_str() {
        "thanks" | "thank you" => Some("You're welcome."),
        _ => None,
    }
}

pub fn presence_reply(text: &str) -> Option<&'static str> {
    match command_words(text).as_str() {
        "hello" | "hi" | "hey" => Some("Hi. I'm listening."),
        "are you there"
        | "are you still there"
        | "can you hear me"
        | "hello are you there"
        | "hello are you still there"
        | "hello can you hear me" => Some("I'm here. I can hear you."),
        _ => None,
    }
}

fn command_words(text: &str) -> String {
    let normalized = text
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c.is_whitespace() {
                c
            } else {
                ' '
            }
        })
        .collect::<String>();
    let words = normalized.split_whitespace().collect::<Vec<_>>().join(" ");
    let words = words.strip_prefix("gamebuddy ").unwrap_or(&words);
    let words = words.strip_prefix("please ").unwrap_or(words);
    words.strip_suffix(" gamebuddy").unwrap_or(words).to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hands_free_starts_on_an_already_unmuted_microphone() {
        let mut gate = MicGate::default();
        gate.set_hands_free(true);
        gate.observe("mic1", None); // Qt selection precedes the async mute query.
        assert!(!gate.active);
        gate.observe("mic1", Some(false));
        assert!(gate.active);
        gate.stop();
        for _ in 0..10 {
            gate.set_hands_free(true);
            gate.observe("mic1", Some(false));
            assert!(!gate.active, "Stop must survive ordinary polling");
        }
        gate.listen();
        assert!(gate.active);
    }
    #[test]
    fn muted_startup_waits_for_unmute_and_device_loss_revokes_capture() {
        let mut gate = MicGate::default();
        gate.set_hands_free(true);
        gate.observe("mic1", Some(true));
        assert!(!gate.active);
        gate.observe("mic1", Some(false));
        assert!(gate.active);
        gate.observe("mic2", Some(false));
        assert!(!gate.active);
        gate.listen();
        assert!(gate.active);
        gate.observe("mic2", None);
        assert!(!gate.active);
        gate.observe("mic2", Some(false));
        assert!(!gate.active);
        gate.observe("mic2", Some(true));
        gate.listen();
        assert!(!gate.active);
    }
    #[test]
    fn hands_free_can_be_disabled_and_reenabled_without_defeating_manual_controls() {
        let mut gate = MicGate::default();
        gate.observe("mic1", Some(false));
        assert!(!gate.active, "Manual mode does not open capture");
        gate.set_hands_free(true);
        assert!(
            gate.active,
            "Enabling hands-free with an unmuted mic starts it"
        );
        gate.set_hands_free(false);
        gate.observe("mic1", Some(true));
        gate.observe("mic1", Some(false));
        assert!(!gate.active);
        gate.listen();
        gate.set_hands_free(false);
        gate.observe("mic1", Some(false));
        assert!(gate.active, "Manual Listen works with hands-free disabled");
        gate.observe("mic1", Some(true));
        assert!(!gate.active);
        gate.set_hands_free(true);
        assert!(!gate.active, "Enabling hands-free must not bypass mute");
        gate.observe("mic1", Some(false));
        assert!(gate.active);
        gate.stop();
        gate.observe("mic1", None);
        gate.observe("mic1", Some(false));
        assert!(
            !gate.active,
            "Recovery must not defeat Stop or capture errors"
        );
        gate.observe("mic1", Some(true));
        gate.observe("mic1", Some(false));
        assert!(gate.active, "An explicit unmute resumes hands-free mode");
    }
    #[test]
    fn live_voice_releases_playback_before_the_recording_endpoint() {
        let mut s = Segmenter::default();
        let frame = [1000; FRAME];
        for _ in 0..MIN_VOICED {
            s.frame(&frame, true);
        }
        assert!(s.live_speaking());
        for _ in 0..9 {
            assert!(s.frame(&frame, false).is_none());
        }
        assert!(s.live_speaking());
        assert!(s.frame(&frame, false).is_none());
        assert!(
            !s.live_speaking(),
            "streaming replies can start after 200 ms"
        );
        assert!(
            s.speaking(),
            "local recording still tolerates a longer pause"
        );
        assert!(s.frame(&frame, true).is_none());
        assert!(s.live_speaking(), "continued speech resumes immediately");
    }
    #[test]
    fn silence_and_clicks_do_not_submit_but_a_pause_ends_speech() {
        let mut s = Segmenter::default();
        assert!(s.feed(&vec![0; 16000 * 10]).is_none());
        let frame = [1000; FRAME];
        s.frame(&frame, true);
        for _ in 0..END_SILENCE {
            assert!(s.frame(&frame, false).is_none());
        }
        for _ in 0..MIN_VOICED {
            assert!(s.frame(&frame, true).is_none());
        }
        for _ in 1..END_SILENCE {
            assert!(s.frame(&frame, false).is_none());
        }
        assert!(s.frame(&frame, false).is_some());
        assert!(s.finish().is_none());
    }
    #[test]
    fn continuous_speech_is_bounded_and_mute_can_finish_a_turn() {
        let mut s = Segmenter::default();
        let frame = [1000; FRAME];
        for _ in 1..MAX_FRAMES {
            assert!(s.frame(&frame, true).is_none());
        }
        assert_eq!(s.frame(&frame, true).unwrap().len(), MAX_FRAMES * FRAME);
        for _ in 0..MIN_VOICED {
            s.frame(&frame, true);
        }
        let samples = s.finish().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = save_turn(dir.path(), &samples).unwrap();
        let read = hound::WavReader::open(path).unwrap();
        assert_eq!(read.spec().sample_rate, SAMPLE_RATE);
        assert_eq!(read.len() as usize, samples.len());
    }
    #[test]
    fn only_explicit_overlay_commands_take_focus() {
        assert_eq!(overlay_command("Please open the guide."), Some(7));
        assert_eq!(overlay_command("Arrange widgets!"), Some(8));
        assert_eq!(overlay_command("How do I open the guide door?"), None);
        assert_eq!(overlay_command("Don't hide the overlay"), None);
        assert_eq!(
            quick_reply("Are you still there, GameBuddy?"),
            Some("I'm here. I can hear you.")
        );
        assert_eq!(quick_reply("Hello. Where should I go?"), None);
        assert_eq!(quick_reply("Thanks, now open the guide."), None);
    }
}
