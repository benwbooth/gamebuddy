//! The realtime voice owns conversation; only its delegated requests own tools.
//! Backchannels must not replace an in-flight game request or replay its actions.
use super::input::PlayerInput;
use std::time::{Duration, Instant};

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

    /// Recover a recognized request if the live model neither answered nor
    /// delegated it. A later handoff binds to this work instead of replaying it.
    pub fn recover(&mut self, utterance: u64, request: String) {
        self.active = Some((utterance, request));
        self.delegation_id = None;
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

    pub fn stopped(&self) -> bool {
        self.active.as_ref().is_some_and(|(_, text)| is_stop(text))
    }
}

/// Normal conversation stays native. Only otherwise unanswered speech takes
/// this path; explicit presence checks must also work during a slow tool turn.
#[derive(Default)]
pub struct Responsiveness {
    last_output: Option<Instant>,
    recovered: Option<(u64, String)>,
}

pub enum Recovery {
    Presence(&'static str),
    Request { utterance: u64, text: String },
}

impl Responsiveness {
    pub fn output(&mut self, now: Instant) {
        self.last_output = Some(now);
    }

    pub fn poll(&mut self, input: &PlayerInput, delegated: u64, now: Instant) -> Option<Recovery> {
        if !input.settled(now) {
            return None;
        }
        let at = input.last_words_at()?;
        if self.last_output.is_some_and(|output| output >= at) {
            return None;
        }
        let utterance = input.utterance();
        let text = input.text(now);
        if text.is_empty()
            || is_stop(&text)
            || self.recovered.as_ref() == Some(&(utterance, text.clone()))
        {
            return None;
        }
        let presence = super::presence_reply(&text);
        // Native replies normally start within a second. Recover a silent
        // handoff after two seconds instead of adding five seconds of idle time.
        let delay = 2;
        if now.saturating_duration_since(at) < Duration::from_secs(delay) {
            return None;
        }
        let recovery = if let Some(reply) = presence {
            Recovery::Presence(reply)
        } else {
            // Acknowledgments can intentionally end without a spoken reply.
            // Compound requests still go through, including ones starting "OK".
            let words = super::command_words(&text);
            if utterance == delegated
                || matches!(
                    words.as_str(),
                    "yes"
                        | "no"
                        | "okay"
                        | "ok"
                        | "yeah"
                        | "right"
                        | "mm hmm"
                        | "uh huh"
                        | "thanks"
                        | "thank you"
                )
            {
                return None;
            }
            Recovery::Request {
                utterance,
                text: text.clone(),
            }
        };
        self.recovered = Some((utterance, text));
        Some(recovery)
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

    #[test]
    fn missing_handoff_recovers_once_and_late_delegation_reuses_the_request() {
        let now = Instant::now();
        let mut input = PlayerInput::default();
        let mut watch = Responsiveness::default();
        let mut requests = Requests::default();
        let mut delivery = super::super::delivery::Delivery::default();
        say(&mut input, "Which item can I clear for the boots?", now);
        assert!(
            watch
                .poll(&input, 0, now + Duration::from_secs(1))
                .is_none()
        );
        let Some(Recovery::Request { utterance, text }) =
            watch.poll(&input, 0, now + Duration::from_millis(2200))
        else {
            panic!("silent question was not recovered")
        };
        requests.recover(utterance, text.clone());
        assert!(delivery.dispatch(utterance, &text, false));
        requests.delegate("late".into(), utterance, text.clone());
        assert!(!delivery.dispatch(utterance, &text, false));
        assert_eq!(requests.delegation_id(), Some("late"));
        assert!(
            watch
                .poll(&input, utterance, now + Duration::from_secs(10))
                .is_none()
        );
    }

    #[test]
    fn hello_during_work_does_not_replace_or_restart_the_inventory_question() {
        let now = Instant::now();
        let mut input = PlayerInput::default();
        let mut requests = Requests::default();
        let mut watch = Responsiveness::default();
        say(&mut input, "Which item can I clear for the boots?", now);
        requests.delegate(
            "inventory".into(),
            1,
            input.text(now + Duration::from_secs(1)),
        );
        say(&mut input, "Hello?", now + Duration::from_secs(10));
        assert!(
            watch
                .poll(&input, 1, now + Duration::from_secs(11))
                .is_none()
        );
        assert!(matches!(
            watch.poll(&input, 1, now + Duration::from_secs(13)),
            Some(Recovery::Presence(_))
        ));
        assert_eq!(
            requests.snapshot(&input, now + Duration::from_secs(13)).1,
            "Which item can I clear for the boots?"
        );
        assert_eq!(requests.delegation_id(), Some("inventory"));
        assert!(
            watch
                .poll(&input, 1, now + Duration::from_secs(20))
                .is_none()
        );
    }

    #[test]
    fn normal_native_answers_and_backchannels_need_no_recovery() {
        let now = Instant::now();
        for text in ["Okay.", "Mm hmm.", "Thanks.", "Stop."] {
            let mut input = PlayerInput::default();
            say(&mut input, text, now);
            assert!(
                Responsiveness::default()
                    .poll(&input, 0, now + Duration::from_secs(8))
                    .is_none()
            );
        }
        let mut input = PlayerInput::default();
        say(&mut input, "Hello?", now);
        let mut watch = Responsiveness::default();
        watch.output(now + Duration::from_secs(1));
        assert!(
            watch
                .poll(&input, 0, now + Duration::from_secs(8))
                .is_none()
        );
        say(
            &mut input,
            "Hello, which item can I clear?",
            now + Duration::from_secs(10),
        );
        assert!(matches!(
            watch.poll(&input, 0, now + Duration::from_secs(16)),
            Some(Recovery::Request { .. })
        ));
        say(
            &mut input,
            "Hello? Can you hear me?",
            now + Duration::from_secs(20),
        );
        assert!(matches!(
            watch.poll(&input, 0, now + Duration::from_secs(23)),
            Some(Recovery::Presence(_))
        ));
    }

    #[test]
    fn native_completed_result_reaches_its_handoff_during_a_backchannel() {
        let now = Instant::now();
        let mut input = PlayerInput::default();
        let mut requests = Requests::default();
        let mut delivery = super::super::delivery::Delivery::default();
        say(&mut input, "Which item can I clear?", now);
        requests.delegate(
            "inventory".into(),
            1,
            input.text(now + Duration::from_secs(1)),
        );
        assert!(delivery.dispatch(1, "Which item can I clear?", false));
        input.activity(true, now + Duration::from_secs(4));
        input.append("Hello?", now + Duration::from_secs(4));
        let (quiet, request, utterance) = requests.snapshot(&input, now + Duration::from_secs(4));
        assert!(!quiet);
        delivery.complete(
            1,
            "Which item can I clear?",
            "Keep the key; clear one spare potion.",
        );
        assert_eq!(
            delivery.take_reply(utterance, &request, true).as_deref(),
            Some("Keep the key; clear one spare potion.")
        );
        assert_eq!(requests.delegation_id(), Some("inventory"));
    }
}
