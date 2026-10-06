//! Request delivery is driven by recognized player speech, not by the live
//! speaker deciding to acknowledge or delegate it.
#[derive(Default)]
pub struct Delivery {
    started: Option<u64>,
    delivered: Option<u64>,
    completed: Option<(u64, String, String)>,
}

impl Delivery {
    pub fn finished(&self, utterance: u64) -> bool {
        self.delivered == Some(utterance)
    }
    pub fn started(&mut self, utterance: u64) {
        self.started = Some(utterance);
    }

    pub fn dispatch(&mut self, utterance: u64, request: &str, backend_active: bool) -> bool {
        if utterance == 0
            || backend_active
            || self.started == Some(utterance)
            || self.delivered == Some(utterance)
            || request.trim().is_empty()
            || matches!(
                request
                    .trim()
                    .trim_end_matches(['.', '!', '?'])
                    .to_lowercase()
                    .as_str(),
                "stop" | "stop talking" | "stop speaking" | "be quiet"
            )
        {
            return false;
        }
        self.started(utterance);
        true
    }

    pub fn retry(&mut self, utterance: u64) {
        if self.started == Some(utterance) {
            self.started = None;
        }
        self.completed = None;
    }

    pub fn abandon(&mut self, utterance: u64) {
        self.delivered = Some(utterance);
        self.completed = None;
    }

    pub fn complete(&mut self, utterance: u64, request: &str, answer: &str) {
        if self.delivered != Some(utterance) && !answer.trim().is_empty() {
            self.completed = Some((utterance, request.into(), answer.into()));
        }
    }

    pub fn take_reply(&mut self, utterance: u64, request: &str, quiet: bool) -> Option<String> {
        let (completed, supplied, _) = self.completed.as_ref()?;
        if *completed != utterance {
            self.completed = None;
            return None;
        }
        if !quiet {
            return None;
        }
        // A final transcript can correct the same utterance after reasoning
        // completes. Reassess that correction instead of silently dropping it.
        // Empty text after the input-history timeout is normal for long replies.
        if !request.is_empty() && request != supplied {
            self.completed = None;
            self.started = None;
            return None;
        }
        let (_, _, answer) = self.completed.take()?;
        self.delivered = Some(utterance);
        Some(answer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::voice::input::PlayerInput;
    use std::time::{Duration, Instant};

    #[test]
    fn silent_speaker_cannot_block_a_recognized_request_or_a_followup() {
        let mut input = PlayerInput::default();
        let mut delivery = Delivery::default();
        let now = Instant::now();
        input.activity(true, now);
        input.append("How do I leave town?", now);
        input.activity(false, now + Duration::from_secs(1));
        let ready = now + Duration::from_secs(2);
        assert!(input.quiet(ready));
        // No assistant/delegation event has occurred at all.
        assert!(delivery.dispatch(input.utterance(), &input.text(ready), false));
        assert!(!delivery.dispatch(input.utterance(), &input.text(ready), false));
        delivery.complete(input.utterance(), &input.text(ready), "Use the gate key.");
        assert_eq!(
            delivery
                .take_reply(input.utterance(), &input.text(ready), true)
                .as_deref(),
            Some("Use the gate key.")
        );
        input.activity(true, now + Duration::from_secs(5));
        input.append("Are you there?", now + Duration::from_secs(5));
        input.activity(false, now + Duration::from_secs(6));
        assert!(delivery.dispatch(
            input.utterance(),
            &input.text(now + Duration::from_secs(7)),
            false
        ));
    }

    #[test]
    fn completion_during_overlap_is_delivered_after_the_player_finishes() {
        let mut d = Delivery::default();
        d.started(1);
        d.complete(1, "Which key?", "A Key of Jack.");
        assert!(d.take_reply(1, "Which key?", false).is_none());
        assert_eq!(
            d.take_reply(1, "Which key?", true).as_deref(),
            Some("A Key of Jack.")
        );
        assert!(d.take_reply(1, "Which key?", true).is_none());
        assert!(!d.dispatch(1, "Which key?", false));
    }

    #[test]
    fn failed_request_does_not_retry_mutations_or_block_the_next_question() {
        let mut d = Delivery::default();
        assert!(d.dispatch(1, "Change my goal.", false));
        d.abandon(1);
        d.complete(1, "Change my goal.", "Old completion");
        assert!(d.take_reply(1, "Change my goal.", true).is_none());
        assert!(!d.dispatch(1, "Change my goal.", false));
        assert!(d.dispatch(2, "Are you there?", false));
    }

    #[test]
    fn late_corrections_and_new_questions_cannot_receive_an_old_answer() {
        let mut d = Delivery::default();
        d.started(1);
        d.complete(1, "Go right?", "Go right.");
        assert!(
            d.take_reply(1, "Go right? There is a wall.", true)
                .is_none()
        );
        assert!(d.dispatch(1, "Go right? There is a wall.", false));
        d.complete(1, "Go right? There is a wall.", "Use the gate key.");
        assert!(d.take_reply(2, "Stop.", true).is_none());
        assert!(!d.dispatch(2, "Stop.", false));
        assert!(d.dispatch(3, "What should I do?", false));
    }
}
