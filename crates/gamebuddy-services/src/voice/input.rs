//! Match streamed transcript fragments to the player's current utterance.
//! A short playback pause and a complete request have different endpoints.
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

#[derive(Default)]
pub struct PlayerInput {
    utterance: u64,
    speaking: bool,
    last_change: Option<Instant>,
    started: Option<Instant>,
    chunks: VecDeque<(Instant, String)>,
    partial_start: Option<Instant>,
}
impl PlayerInput {
    pub fn activity(&mut self, speaking: bool, now: Instant) {
        if speaking
            && !self.speaking
            && self.last_change.is_none_or(|last| {
                now.saturating_duration_since(last) > Duration::from_millis(1200)
            })
        {
            self.utterance += 1;
            // The browser's echo check can confirm speech just after its first
            // transcript fragment arrives on the app-server connection.
            self.started = now.checked_sub(Duration::from_millis(500));
        }
        if speaking != self.speaking {
            self.last_change = Some(now)
        }
        self.speaking = speaking;
    }
    pub fn utterance(&self) -> u64 {
        self.utterance
    }
    pub fn append(&mut self, text: &str, now: Instant) {
        self.partial_start.get_or_insert(now);
        self.push(text.to_owned(), now);
    }
    fn push(&mut self, text: String, now: Instant) {
        self.chunks.push_back((now, text));
        while self.chunks.len() > 256
            || self.chunks.front().is_some_and(|(at, _)| {
                now.saturating_duration_since(*at) > Duration::from_secs(120)
            })
        {
            self.chunks.pop_front();
        }
    }
    /// Final transcripts can correct or replace streamed words, and can arrive
    /// after the backend has already received an early delegation.
    pub fn finish(&mut self, text: &str, now: Instant) {
        if let Some(start) = self.partial_start.take() {
            self.chunks.retain(|(at, _)| *at < start);
        }
        let separator = if self
            .chunks
            .back()
            .is_some_and(|(at, _)| self.started.is_none_or(|start| *at >= start))
        {
            " "
        } else {
            ""
        };
        self.push(format!("{separator}{text}"), now);
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
            .trim()
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
    fn final_transcript_padding_does_not_change_a_dispatched_request() {
        let mut input = PlayerInput::default();
        let now = Instant::now();
        input.activity(true, now);
        input.append("What next?", now);
        input.activity(false, now + Duration::from_secs(1));
        let sent = input.text(now + Duration::from_secs(2));
        input.finish(" What next? ", now + Duration::from_secs(3));
        assert_eq!(input.text(now + Duration::from_secs(3)), sent);
    }
    #[test]
    fn final_transcript_replaces_partial_delegation_without_losing_correction() {
        let mut input = PlayerInput::default();
        let now = Instant::now();
        input.activity(true, now);
        input.append("Where do I", now);
        input.activity(false, now + Duration::from_secs(1));
        assert_eq!(input.text(now + Duration::from_secs(2)), "Where do I");
        input.finish(
            "I already did that. I'm at the restoration vendors.",
            now + Duration::from_secs(3),
        );
        assert_eq!(
            input.text(now + Duration::from_secs(3)),
            "I already did that. I'm at the restoration vendors."
        );
        input.append("Where", now + Duration::from_millis(3100));
        input.finish("Where else do I go?", now + Duration::from_secs(4));
        assert_eq!(
            input.text(now + Duration::from_secs(4)),
            "I already did that. I'm at the restoration vendors. Where else do I go?"
        );
        input.activity(true, now + Duration::from_secs(6));
        input.finish("What next?", now + Duration::from_secs(7));
        assert_eq!(input.text(now + Duration::from_secs(8)), "What next?");
    }
    #[test]
    fn a_followup_clause_is_kept_until_the_request_finishes() {
        let mut input = PlayerInput::default();
        let now = Instant::now();
        input.activity(true, now);
        assert_eq!(input.utterance(), 1);
        input.append("What color is it?", now);
        input.activity(false, now + Duration::from_secs(2));
        assert!(!input.quiet(now + Duration::from_millis(2250)));
        input.activity(true, now + Duration::from_millis(2350));
        assert_eq!(
            input.utterance(),
            1,
            "a follow-up clause is the same request"
        );
        input.append(" Put it in a widget.", now + Duration::from_secs(3));
        input.activity(false, now + Duration::from_secs(4));
        assert!(input.quiet(now + Duration::from_millis(4500)));
        assert_eq!(
            input.text(now + Duration::from_secs(5)),
            "What color is it? Put it in a widget."
        );
        input.activity(true, now + Duration::from_secs(8));
        assert_eq!(
            input.utterance(),
            2,
            "an interruption supersedes the old request"
        );
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
