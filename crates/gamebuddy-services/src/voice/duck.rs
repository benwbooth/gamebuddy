//! Conversation timing and a supervised, game-only audio volume lease.
//! The helper owns restoration, so a killed Qt process closes its pipe and
//! restores the surviving game's streams. No device/master volume is touched.
use std::time::{Duration, Instant};

#[derive(Default)]
pub struct Conversation {
    last_activity: Option<Instant>,
}
impl Conversation {
    pub fn update(&mut self, active: bool, now: Instant) -> bool {
        if active {
            self.last_activity = Some(now);
        }
        self.last_activity
            .is_some_and(|last| now.saturating_duration_since(last) < Duration::from_millis(1800))
    }
}

#[cfg(target_os = "linux")]
pub mod linux;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn conversation_holds_between_words_but_releases_silence() {
        let mut c = Conversation::default();
        let now = Instant::now();
        assert!(!c.update(false, now));
        assert!(c.update(true, now));
        assert!(c.update(false, now + Duration::from_secs(1)));
        assert!(!c.update(false, now + Duration::from_secs(2)));
    }
}
