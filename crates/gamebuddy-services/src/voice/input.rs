//! Match streamed transcript fragments to the player's current utterance.
//! A short playback pause and a complete request have different endpoints.
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

#[derive(Default)]
pub struct PlayerInput {
    speaking: bool,
    last_change: Option<Instant>,
    started: Option<Instant>,
    chunks: VecDeque<(Instant, String)>,
}
impl PlayerInput {
    pub fn activity(&mut self, speaking: bool, now: Instant) {
        if speaking
            && !self.speaking
            && self.last_change.is_none_or(|last| {
                now.saturating_duration_since(last) > Duration::from_millis(1200)
            })
        {
            // The browser's echo check can confirm speech just after its first
            // transcript fragment arrives on the app-server connection.
            self.started = now.checked_sub(Duration::from_millis(500));
        }
        if speaking != self.speaking {
            self.last_change = Some(now)
        }
        self.speaking = speaking;
    }
    pub fn append(&mut self, text: &str, now: Instant) {
        self.chunks.push_back((now, text.to_owned()));
        while self.chunks.len() > 256
            || self.chunks.front().is_some_and(|(at, _)| {
                now.saturating_duration_since(*at) > Duration::from_secs(120)
            })
        {
            self.chunks.pop_front();
        }
    }
    pub fn quiet(&self, now: Instant) -> bool {
        !self.speaking
            && self.last_change.is_none_or(|last| {
                now.saturating_duration_since(last) >= Duration::from_millis(500)
            })
    }
    pub fn text(&self, now: Instant) -> String {
        if self
            .chunks
            .back()
            .is_none_or(|(at, _)| now.saturating_duration_since(*at) > Duration::from_secs(30))
        {
            return String::new();
        }
        let value: String = self
            .chunks
            .iter()
            .filter(|(at, _)| self.started.is_none_or(|start| *at >= start))
            .map(|(_, text)| text.as_str())
            .collect();
        // Bound a deliberately long dictated request without splitting UTF-8.
        value
            .chars()
            .rev()
            .take(8000)
            .collect::<String>()
            .chars()
            .rev()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_followup_clause_is_kept_until_the_request_finishes() {
        let mut input = PlayerInput::default();
        let now = Instant::now();
        input.activity(true, now);
        input.append("What color is it?", now);
        input.activity(false, now + Duration::from_secs(2));
        assert!(!input.quiet(now + Duration::from_millis(2250)));
        input.activity(true, now + Duration::from_millis(2350));
        input.append(" Put it in a widget.", now + Duration::from_secs(3));
        input.activity(false, now + Duration::from_secs(4));
        assert!(input.quiet(now + Duration::from_millis(4500)));
        assert_eq!(
            input.text(now + Duration::from_secs(5)),
            "What color is it? Put it in a widget."
        );
        input.activity(true, now + Duration::from_secs(8));
        input.append("Stop that. New question.", now + Duration::from_secs(8));
        assert_eq!(
            input.text(now + Duration::from_secs(9)),
            "Stop that. New question."
        );
        assert!(
            input.text(now + Duration::from_secs(40)).is_empty(),
            "Old requests must not be attached to later background work"
        );
    }
    #[test]
    fn late_echo_confirmation_keeps_the_first_recognized_word() {
        let mut input = PlayerInput::default();
        let now = Instant::now();
        input.append("Don't", now);
        input.activity(true, now + Duration::from_millis(100));
        input.append(" buy that.", now + Duration::from_millis(400));
        assert_eq!(input.text(now + Duration::from_secs(1)), "Don't buy that.");
    }
}
