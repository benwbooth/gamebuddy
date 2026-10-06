//! The realtime voice owns conversation; only its delegated requests own tools.
//! Backchannels must not replace an in-flight game request or replay its actions.
use super::input::PlayerInput;
use std::time::Instant;

#[derive(Default)]
pub struct Requests {
    active: Option<(u64, String)>,
    delegation_id: Option<String>,
    seen: std::collections::VecDeque<String>,
}

impl Requests {
    pub fn delegate(&mut self, id: String, utterance: u64, request: String) {
        if self.seen.contains(&id) {
            return;
        }
        self.seen.push_back(id.clone());
        if self.seen.len() > 64 {
            self.seen.pop_front();
        }
        self.delegation_id = Some(id);
        self.active = Some((utterance, request));
    }

    pub fn delegation_id(&self) -> Option<&str> {
        self.delegation_id.as_deref()
    }

    /// Preserve the delegated task across unrelated conversation. The original
    /// utterance may still gain a final clause or corrected transcript.
    pub fn snapshot(&mut self, input: &PlayerInput, now: Instant) -> (bool, String, u64) {
        let quiet = input.settled(now);
        let utterance = input.utterance();
        let text = input.text(now);
        if quiet && is_stop(&text) {
            self.active = Some((utterance, text.clone()));
        }
        if let Some((id, request)) = &mut self.active {
            if (*id == utterance || *id == 0) && !text.is_empty() {
                *id = utterance;
                *request = text;
            }
            (quiet, request.clone(), *id)
        } else {
            (quiet, String::new(), 0)
        }
    }

    pub fn utterance(&self) -> u64 {
        self.active.as_ref().map_or(0, |(id, _)| *id)
    }
}

fn is_stop(text: &str) -> bool {
    matches!(
        text.trim()
            .trim_end_matches(['.', '!', '?'])
            .to_lowercase()
            .as_str(),
        "stop" | "stop talking" | "stop speaking" | "be quiet"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn say(input: &mut PlayerInput, text: &str, now: Instant) {
        input.activity(true, now);
        input.finish(text, now + Duration::from_millis(100));
        input.activity(false, now + Duration::from_millis(200));
    }

    #[test]
    fn ordinary_conversation_never_starts_a_text_request() {
        let t = Instant::now();
        let mut input = PlayerInput::default();
        let mut requests = Requests::default();
        say(&mut input, "How are you doing?", t);
        assert_eq!(
            requests.snapshot(&input, t + Duration::from_secs(1)),
            (true, String::new(), 0)
        );
    }

    #[test]
    fn a_delegated_partial_keeps_the_final_widget_clause() {
        let t = Instant::now();
        let mut input = PlayerInput::default();
        let mut requests = Requests::default();
        input.activity(true, t);
        input.append("What color is it?", t);
        requests.delegate("one".into(), input.utterance(), input.text(t));
        assert!(!requests.snapshot(&input, t).0);
        input.finish(
            "What color is it? Put the answer in a widget.",
            t + Duration::from_secs(1),
        );
        input.activity(false, t + Duration::from_secs(1));
        assert_eq!(
            requests.snapshot(&input, t + Duration::from_secs(2)),
            (
                true,
                "What color is it? Put the answer in a widget.".into(),
                1
            )
        );
    }

    #[test]
    fn backchannels_keep_the_task_but_delegated_corrections_replace_it() {
        let t = Instant::now();
        let mut input = PlayerInput::default();
        let mut requests = Requests::default();
        say(&mut input, "Show the town map.", t);
        requests.delegate(
            "one".into(),
            input.utterance(),
            input.text(t + Duration::from_secs(1)),
        );
        say(&mut input, "Mm-hmm, thanks.", t + Duration::from_secs(4));
        assert_eq!(
            requests.snapshot(&input, t + Duration::from_secs(5)),
            (true, "Show the town map.".into(), 1)
        );
        say(
            &mut input,
            "Actually show the forest map.",
            t + Duration::from_secs(8),
        );
        requests.delegate(
            "two".into(),
            input.utterance(),
            input.text(t + Duration::from_secs(9)),
        );
        assert_eq!(
            requests.snapshot(&input, t + Duration::from_secs(9)),
            (true, "Actually show the forest map.".into(), 3)
        );
    }

    #[test]
    fn explicit_stop_cancels_without_waiting_for_delegation() {
        let t = Instant::now();
        let mut input = PlayerInput::default();
        let mut requests = Requests::default();
        say(&mut input, "Show a map.", t);
        requests.delegate(
            "one".into(),
            input.utterance(),
            input.text(t + Duration::from_secs(1)),
        );
        say(&mut input, "Stop.", t + Duration::from_secs(4));
        assert_eq!(
            requests.snapshot(&input, t + Duration::from_secs(5)),
            (true, "Stop.".into(), 2)
        );
    }
    #[test]
    fn a_repeated_handoff_cannot_rebind_to_a_later_backchannel() {
        let t = Instant::now();
        let mut input = PlayerInput::default();
        let mut requests = Requests::default();
        say(&mut input, "Show the town map.", t);
        requests.delegate(
            "one".into(),
            input.utterance(),
            input.text(t + Duration::from_secs(1)),
        );
        say(&mut input, "Mm-hmm, thanks.", t + Duration::from_secs(4));
        requests.delegate(
            "one".into(),
            input.utterance(),
            input.text(t + Duration::from_secs(5)),
        );
        assert_eq!(
            requests.snapshot(&input, t + Duration::from_secs(5)),
            (true, "Show the town map.".into(), 1)
        );
    }
}
