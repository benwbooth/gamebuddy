//! Session-local spoken initiative. Visual guesses never become completion claims.
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

#[derive(Default)]
pub struct Narrator {
    session: String,
    introduced: bool,
    spoken_actions: HashSet<String>,
    spoken_events: HashMap<String, Instant>,
    last_conversation: Option<Instant>,
    last_spoken: Option<Instant>,
    last_coaching: Option<Instant>,
    spoken_coaching: HashSet<String>,
    last_scene_at: u64,
    pending_event: String,
    event_votes: u8,
}

fn field<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or_default()
}

impl Narrator {
    pub fn next(&mut self, context: &Value, now: Instant) -> Option<String> {
        let game = field(context, "game");
        if game.is_empty() {
            return None;
        }
        let session = format!(
            "{game}:{}:{}",
            field(context, "guide_id"),
            field(&context["screen"], "session_id")
        );
        if self.session != session {
            *self = Self {
                session,
                ..Self::default()
            };
        }
        // Keep the latest action pending while the player or Buddy is speaking.
        // Seeing an action during a conversation is not proof it was spoken.
        if context["conversation_active"] == true {
            self.last_conversation = Some(now);
            return None;
        }
        if self
            .last_conversation
            .is_some_and(|t| now.duration_since(t) < Duration::from_secs(1))
        {
            return None;
        }
        let guidance = &context["guidance"];
        let live = context["screen"]["status"] == "live";
        let ready = matches!(guidance["status"].as_str(), Some("ready" | "unlocated"))
            && live
            && (context["screen"]["scene"]["fresh"] == true
                || context["spatial"]["tracker"]["status"] == "tracked"
                || context["spatial"]["observation"]["tracker"]["status"] == "tracked"
                || (context["coaching"]["kind"] == "next_action"
                    && context["coaching"]["step_id"] == guidance["now"]["id"]));
        let action = &guidance["now"];
        let action_key = format!("{}:{}", field(action, "id"), cue(context));
        if !self.introduced {
            self.introduced = true;
            self.last_spoken = Some(now);
            let greeting = if context["returning_player"] == true {
                "Welcome back."
            } else {
                "Hey, I'm GameBuddy."
            };
            let next = if ready && !field(action, "id").is_empty() {
                self.spoken_actions.insert(action_key);
                cue(context)
            } else if context["preparing"] == true && context["guide_ready"] != true {
                "I'm preparing your guide. I'll talk you through the next steps as you play.".into()
            } else {
                "I'll talk you through the next steps as you play.".into()
            };
            return Some(format!("{greeting} {next}"));
        }

        // Source-grounded coaching has already passed capture and progress
        // freshness checks. Urgent recovery can precede a changed objective.
        let coaching = &context["coaching"];
        let urgent = matches!(field(coaching, "kind"), "danger" | "recovery");
        let coach_id = field(coaching, "id");
        let can_coach = live
            && field(coaching, "kind") != "next_action"
            && !coach_id.is_empty()
            && !field(coaching, "text").is_empty()
            && !self.spoken_coaching.contains(coach_id)
            && self.last_coaching.is_none_or(|t| {
                now.duration_since(t) >= Duration::from_secs(if urgent { 12 } else { 30 })
            });
        if can_coach && urgent {
            return Some(self.coach(coaching, now));
        }
        if ready && !field(action, "id").is_empty() && self.spoken_actions.insert(action_key) {
            self.last_spoken = Some(now);
            // Actual instructions, immediately on the next poll. No named-place
            // requirement or minute-long delay after the previous objective.
            return Some(cue(context));
        }
        if can_coach
            && self
                .last_spoken
                .is_none_or(|t| now.duration_since(t) >= Duration::from_secs(3))
        {
            return Some(self.coach(coaching, now));
        }

        let scene = &context["screen"]["scene"];
        if live && scene["fresh"] == true {
            let at = scene["captured_at_ms"].as_u64().unwrap_or_default();
            let event = field(scene, "event");
            if at > self.last_scene_at {
                self.last_scene_at = at;
                if event == self.pending_event {
                    self.event_votes = self.event_votes.saturating_add(1);
                } else {
                    self.pending_event = event.into();
                    self.event_votes = 1;
                }
            }
            let supplied = field(scene, "feedback");
            let feedback = if supplied.trim().is_empty() {
                match event {
                    "death" => "It looks like you died. Let's work through the next attempt.",
                    "victory" => "That looks like a victory screen.",
                    _ => "",
                }
            } else {
                supplied
            };
            if ["death", "danger", "victory"].contains(&event)
                && self.event_votes >= 2
                && self
                    .spoken_events
                    .get(event)
                    .is_none_or(|t| now.duration_since(*t) >= Duration::from_secs(30))
                && !feedback.is_empty()
                && scene["confidence"].as_f64().unwrap_or_default() >= 0.8
                && self
                    .last_spoken
                    .is_none_or(|t| now.duration_since(t) >= Duration::from_secs(8))
            {
                self.spoken_events.insert(event.into(), now);
                self.last_spoken = Some(now);
                return Some(brief(feedback));
            }
        } else {
            self.pending_event.clear();
            self.event_votes = 0;
        }
        None
    }

    fn coach(&mut self, coaching: &Value, now: Instant) -> String {
        self.spoken_coaching.insert(field(coaching, "id").into());
        self.last_coaching = Some(now);
        self.last_spoken = Some(now);
        brief(field(coaching, "text"))
    }
}

fn brief(text: &str) -> String {
    let sentences: Vec<_> = text.split_inclusive(['.', '!', '?']).take(2).collect();
    sentences
        .concat()
        .split_whitespace()
        .take(65)
        .collect::<Vec<_>>()
        .join(" ")
}

fn cue(context: &Value) -> String {
    let action = &context["guidance"]["now"];
    let instructions = field(action, "instructions");
    if !instructions.trim().is_empty() {
        brief(instructions)
    } else {
        format!(
            "Next, {}.",
            brief(field(action, "title")).trim_end_matches(['.', '!', '?'])
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn action(id: &str) -> Value {
        json!({"game":"Game","screen":{"status":"live","session_id":"s","scene":{"fresh":true}},"location_status":"uncertain","guidance":{"status":"ready","now":{"id":id,"title":"Next task","instructions":format!("Walk right to {id}. Climb the ladder.")}}})
    }
    #[test]
    fn new_steps_speak_instructions_immediately_without_a_named_location() {
        let mut n = Narrator::default();
        let t = Instant::now();
        assert!(
            n.next(&action("a"), t)
                .unwrap()
                .contains("Climb the ladder.")
        );
        assert_eq!(
            n.next(&action("b"), t + Duration::from_secs(1)).as_deref(),
            Some("Walk right to b. Climb the ladder.")
        );
        assert!(n.next(&action("b"), t + Duration::from_secs(100)).is_none());
        assert!(n.next(&action("a"), t + Duration::from_secs(110)).is_none());
    }
    #[test]
    fn conversation_queues_only_the_latest_step_and_does_not_consume_it() {
        let mut n = Narrator::default();
        let t = Instant::now();
        n.next(&action("a"), t);
        for (i, id) in ["b", "c"].iter().enumerate() {
            let mut c = action(id);
            c["conversation_active"] = json!(true);
            assert!(n.next(&c, t + Duration::from_secs(i as u64 + 1)).is_none());
        }
        assert!(
            n.next(&action("c"), t + Duration::from_secs(3))
                .unwrap()
                .contains("right to c")
        );
        assert!(n.next(&action("c"), t + Duration::from_secs(4)).is_none());
    }
    #[test]
    fn fresh_geometry_can_speak_before_the_slow_scene_description() {
        let mut n = Narrator::default();
        let mut c = action("a");
        c["screen"]["scene"]["fresh"] = json!(false);
        c["guidance"]["status"] = json!("unlocated");
        c["spatial"] = json!({"tracker":{"status":"tracked"}});
        assert!(n.next(&c, Instant::now()).unwrap().contains("Walk right"));
    }
    #[test]
    fn stale_or_paused_capture_does_not_give_gameplay_instructions() {
        for change in ["stale", "paused"] {
            let mut n = Narrator::default();
            let t = Instant::now();
            let mut c = action("a");
            if change == "stale" {
                c["screen"]["scene"]["fresh"] = json!(false);
            } else {
                c["screen"]["status"] = json!("paused");
            }
            assert!(!n.next(&c, t).unwrap().contains("Walk right"));
            assert!(n.next(&c, t + Duration::from_secs(120)).is_none());
        }
    }
    #[test]
    fn grounded_strategy_is_spoken_once_and_waits_for_player_speech() {
        let mut n = Narrator::default();
        let t = Instant::now();
        let mut c = action("a");
        n.next(&c, t);
        c["coaching"] = json!({"id":"tip","kind":"strategy","text":"Take a few steps before jumping to clear the gap."});
        c["conversation_active"] = json!(true);
        assert!(n.next(&c, t + Duration::from_secs(4)).is_none());
        c["conversation_active"] = json!(false);
        assert!(
            n.next(&c, t + Duration::from_secs(5))
                .unwrap()
                .contains("before jumping")
        );
        assert!(n.next(&c, t + Duration::from_secs(100)).is_none());
    }
    #[test]
    fn cached_outcome_is_not_two_observations() {
        let mut n = Narrator::default();
        let t = Instant::now();
        let mut c = action("a");
        n.next(&c, t);
        c["screen"]["scene"] = json!({"fresh":true,"confidence":0.95,"event":"danger","feedback":"Watch the enemy ahead.","captured_at_ms":1});
        assert!(n.next(&c, t + Duration::from_secs(10)).is_none());
        assert!(n.next(&c, t + Duration::from_secs(11)).is_none());
        c["screen"]["scene"]["captured_at_ms"] = json!(2);
        assert_eq!(
            n.next(&c, t + Duration::from_secs(12)).as_deref(),
            Some("Watch the enemy ahead.")
        );
    }
}
