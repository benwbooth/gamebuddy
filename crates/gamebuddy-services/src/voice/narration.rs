//! Session-local spoken initiative. Visual guesses never become completion claims.
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

#[derive(Default)]
pub struct Narrator {
    game: String,
    introduced: bool,
    spoken_actions: HashSet<String>,
    spoken_events: HashMap<String, Instant>,
    last_conversation: Option<Instant>,
    pending: String,
    pending_since: Option<Instant>,
    last_spoken: Option<Instant>,
    last_scene_at: u64,
    pending_event: String,
    event_votes: u8,
}

fn field(value: &Value, key: &str) -> String {
    value[key]
        .as_str()
        .unwrap_or_default()
        .chars()
        .take(220)
        .collect()
}

impl Narrator {
    pub fn next(&mut self, context: &Value, now: Instant) -> Option<String> {
        let game = field(context, "game");
        if game.is_empty() {
            return None;
        }
        if self.game != game {
            *self = Self {
                game: game.clone(),
                ..Self::default()
            };
        }
        // Observations keep flowing during a conversation, but unsolicited
        // speech must not compete with the player or a requested answer.
        if context["conversation_active"] == true {
            self.last_conversation = Some(now);
            // Leave the visible task on the OSD; do not repeat it immediately
            // after a requested answer or turn it into a delayed interruption.
            let action = field(&context["guidance"]["now"], "id");
            if !action.is_empty() {
                self.spoken_actions.insert(action);
            }
            self.pending.clear();
            self.pending_since = None;
            self.pending_event.clear();
            self.event_votes = 0;
            return None;
        }
        let guidance = &context["guidance"];
        let ready = guidance["status"] == "ready"
            && context["screen"]["status"] == "live"
            && context["screen"]["scene"]["fresh"] == true
            && matches!(
                context["location_status"].as_str(),
                Some("estimated" | "reported")
            );
        let action = if ready {
            field(&guidance["now"], "id")
        } else {
            String::new()
        };
        let recently_talking = self
            .last_conversation
            .is_some_and(|t| now.duration_since(t) < Duration::from_secs(45));
        if !self.introduced && recently_talking {
            return None;
        }
        if !self.introduced {
            self.introduced = true;
            self.last_spoken = Some(now);
            let greeting = if context["returning_player"] == true {
                "Welcome back."
            } else {
                "Hey, I'm GameBuddy."
            };
            let next = if ready {
                self.spoken_actions.insert(action);
                cue(context)
            } else if context["preparing"] == true {
                "I'm preparing your guide. Speak whenever you need me.".into()
            } else {
                "Your guide is open. Speak whenever you need me.".into()
            };
            return Some(format!("{greeting} {next}"));
        }

        let scene = &context["screen"]["scene"];
        if context["screen"]["status"] == "live" && scene["fresh"] == true {
            let at = scene["captured_at_ms"].as_u64().unwrap_or_default();
            let event = field(scene, "event");
            if at > self.last_scene_at {
                self.last_scene_at = at;
                if event == self.pending_event {
                    self.event_votes = self.event_votes.saturating_add(1);
                } else {
                    self.pending_event = event.clone();
                    self.event_votes = 1;
                }
            }
            let supplied_feedback = field(scene, "feedback");
            // An observed outcome can be acknowledged without inventing a
            // recovery strategy when the semantic model supplies no advice.
            let feedback = if supplied_feedback.trim().is_empty() {
                match event.as_str() {
                    "death" => "It looks like you died.",
                    "victory" => "That looks like a victory screen.",
                    _ => "",
                }
            } else {
                supplied_feedback.as_str()
            };
            if ["death", "danger", "victory"].contains(&event.as_str())
                && self.event_votes >= 2
                && self
                    .spoken_events
                    .get(&event)
                    .is_none_or(|t| now.duration_since(*t) >= Duration::from_secs(300))
                && !feedback.is_empty()
                && scene["confidence"].as_f64().unwrap_or_default() >= 0.8
                && self
                    .last_spoken
                    .is_none_or(|t| now.duration_since(t) >= Duration::from_secs(8))
                && self
                    .last_conversation
                    .is_none_or(|t| now.duration_since(t) >= Duration::from_secs(3))
            {
                self.spoken_events.insert(event, now);
                self.last_spoken = Some(now);
                return Some(brief(feedback));
            }
        } else {
            self.pending_event.clear();
            self.event_votes = 0;
        }
        if self
            .last_conversation
            .is_some_and(|t| now.duration_since(t) < Duration::from_secs(45))
        {
            return None;
        }
        if action != self.pending {
            self.pending = action.clone();
            self.pending_since = Some(now);
        }
        if action.is_empty()
            || self.spoken_actions.contains(&action)
            || self
                .pending_since
                .is_none_or(|t| now.duration_since(t) < Duration::from_secs(12))
            || self
                .last_spoken
                .is_some_and(|t| now.duration_since(t) < Duration::from_secs(90))
        {
            return None;
        }
        self.spoken_actions.insert(action);
        self.last_spoken = Some(now);
        Some(cue(context))
    }
}

fn brief(text: &str) -> String {
    text.split_whitespace()
        .take(24)
        .collect::<Vec<_>>()
        .join(" ")
}

fn cue(context: &Value) -> String {
    let guidance = &context["guidance"];
    let title = brief(&field(&guidance["now"], "title"));
    if guidance["requirements"]
        .as_array()
        .is_some_and(|r| !r.is_empty())
    {
        format!("First, {}.", title.trim_end_matches(['.', '!', '?']))
    } else {
        format!("Next, {}.", title.trim_end_matches(['.', '!', '?']))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn action(id: &str) -> Value {
        json!({"game":"Game","screen":{"status":"live","scene":{"fresh":true}},"location_status":"estimated","guidance":{"status":"ready","now":{"id":id,"title":"Buy the key"}}})
    }
    #[test]
    fn outdated_scene_cannot_trigger_a_next_action_cue() {
        let mut n = Narrator::default();
        let t = Instant::now();
        let mut c = action("a");
        c["screen"]["scene"]["fresh"] = serde_json::json!(false);
        assert!(!n.next(&c, t).unwrap().contains("Buy the key"));
        assert!(n.next(&c, t + Duration::from_secs(120)).is_none());
    }
    #[test]
    fn short_introduction_does_not_claim_an_uncertain_location() {
        let mut n = Narrator::default();
        let context = json!({"game":"Game","returning_player":true,"location_status":"uncertain","current_place":{"label":"Church"}});
        let greeting = n.next(&context, Instant::now()).unwrap();
        assert!(greeting.starts_with("Welcome back."));
        assert!(greeting.split_whitespace().count() <= 20);
        assert!(!greeting.contains("Church"));
    }
    #[test]
    fn repeated_outcomes_can_be_acknowledged_without_generated_advice() {
        for event in ["death", "victory"] {
            let mut n = Narrator::default();
            let t = Instant::now();
            let mut c = json!({"game":"Game","screen":{"status":"live","scene":{
                "fresh":true,"confidence":0.95,"event":event,"feedback":"","captured_at_ms":1
            }}});
            n.next(&c, t);
            assert!(n.next(&c, t + Duration::from_secs(100)).is_none());
            c["screen"]["scene"]["captured_at_ms"] = json!(2);
            assert!(n.next(&c, t + Duration::from_secs(110)).is_some());
            c["screen"]["scene"]["captured_at_ms"] = json!(3);
            assert!(n.next(&c, t + Duration::from_secs(210)).is_none());
        }
    }
    #[test]
    fn ordinary_observations_stay_silent_and_events_have_a_long_cooldown() {
        let mut n = Narrator::default();
        let t = Instant::now();
        let mut c = json!({"game":"Game","screen":{"status":"live","scene":{"fresh":true,"confidence":0.9,"feedback":"Watch the spikes."}}});
        n.next(&c, t);
        for (i, event) in ["menu", "dialogue", "new_area", "menu"].iter().enumerate() {
            c["screen"]["scene"]["event"] = json!(event);
            for j in 0..2 {
                c["screen"]["scene"]["captured_at_ms"] = json!(i * 2 + j + 1);
                assert!(
                    n.next(&c, t + Duration::from_secs(100 + i as u64 * 30 + j as u64))
                        .is_none()
                );
            }
        }
        c["screen"]["scene"]["event"] = json!("danger");
        c["screen"]["scene"]["captured_at_ms"] = json!(10);
        assert!(n.next(&c, t + Duration::from_secs(250)).is_none());
        // Polling a cached observation does not establish two-frame agreement.
        assert!(n.next(&c, t + Duration::from_secs(260)).is_none());
        c["screen"]["scene"]["captured_at_ms"] = json!(11);
        assert_eq!(
            n.next(&c, t + Duration::from_secs(261)).as_deref(),
            Some("Watch the spikes.")
        );
        for (at, event) in [(262, "none"), (370, "danger"), (371, "danger")] {
            c["screen"]["scene"]["event"] = json!(event);
            c["screen"]["scene"]["captured_at_ms"] = json!(at);
            assert!(n.next(&c, t + Duration::from_secs(at)).is_none());
        }
    }
    #[test]
    fn a_confirmed_threat_is_not_silenced_for_45_seconds_after_guidance() {
        let mut n = Narrator::default();
        let t = Instant::now();
        let mut c = action("a");
        n.next(&c, t);
        c["conversation_active"] = json!(true);
        assert!(n.next(&c, t + Duration::from_secs(100)).is_none());
        c["conversation_active"] = json!(false);
        c["screen"]["scene"]["event"] = json!("danger");
        c["screen"]["scene"]["feedback"] = json!("Watch the enemy ahead.");
        c["screen"]["scene"]["confidence"] = json!(0.95);
        c["screen"]["scene"]["captured_at_ms"] = json!(101);
        assert!(n.next(&c, t + Duration::from_secs(101)).is_none());
        c["screen"]["scene"]["captured_at_ms"] = json!(104);
        assert_eq!(
            n.next(&c, t + Duration::from_secs(104)).as_deref(),
            Some("Watch the enemy ahead.")
        );
    }
    #[test]
    fn action_jitter_cannot_repeat_a_hint_and_new_hints_are_spaced_out() {
        let mut n = Narrator::default();
        let t = Instant::now();
        let mut c = action("a");
        assert!(n.next(&c, t).unwrap().contains("Buy the key"));
        c = action("b");
        assert!(n.next(&c, t + Duration::from_secs(10)).is_none());
        assert!(n.next(&c, t + Duration::from_secs(30)).is_none());
        assert!(n.next(&c, t + Duration::from_secs(91)).is_some());
        c = action("a");
        assert!(n.next(&c, t + Duration::from_secs(200)).is_none());
        assert!(n.next(&c, t + Duration::from_secs(220)).is_none());
        c["game"] = json!("Another game");
        assert!(n.next(&c, t + Duration::from_secs(240)).is_some());
    }
    #[test]
    fn conversation_and_its_afterglow_block_unsolicited_speech() {
        let mut n = Narrator::default();
        let t = Instant::now();
        let mut c = action("a");
        n.next(&c, t);
        c = action("b");
        c["conversation_active"] = json!(true);
        assert!(n.next(&c, t + Duration::from_secs(100)).is_none());
        c["conversation_active"] = json!(false);
        assert!(n.next(&c, t + Duration::from_secs(130)).is_none());
        assert!(n.next(&c, t + Duration::from_secs(146)).is_none());
        assert!(n.next(&c, t + Duration::from_secs(159)).is_none());
        c = action("c");
        assert!(n.next(&c, t + Duration::from_secs(200)).is_none());
        assert!(n.next(&c, t + Duration::from_secs(213)).is_some());
    }
}
