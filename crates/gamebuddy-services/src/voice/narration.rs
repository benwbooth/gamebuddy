//! Session-local spoken initiative. Visual guesses never become completion claims.
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

#[derive(Default)]
pub struct Narrator {
    session: String,
    introduced: bool,
    spoken_actions: HashSet<String>,
    acknowledged_actions: HashSet<String>,
    spoken_events: HashMap<String, Instant>,
    last_conversation: Option<Instant>,
    last_spoken: Option<Instant>,
    last_coaching: Option<Instant>,
    spoken_coaching: HashSet<String>,
    last_scene_at: u64,
    pending_event: String,
    event_votes: u8,
    urgent: bool,
    warned_hazards: HashSet<String>,
    /// Greeted the player as starting a new game while the guide was built.
    welcomed_new_game: bool,
    announced_ready: bool,
}

fn field<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or_default()
}

impl Narrator {
    pub fn heard_advice(&mut self, context: &Value, now: Instant) {
        let action = &context["guidance"]["now"];
        if !field(action, "id").is_empty() {
            self.acknowledged_actions.insert(field(action, "id").into());
            self.spoken_actions
                .insert(format!("{}:{}", field(action, "id"), cue(context)));
            self.last_spoken = Some(now);
        }
    }
    pub fn next_cue(&mut self, context: &Value, now: Instant) -> Option<Value> {
        self.next(context, now)
            .map(|text| serde_json::json!({"text":text,"urgent":self.urgent}))
    }
    pub fn next(&mut self, context: &Value, now: Instant) -> Option<String> {
        self.urgent = false;
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
        let scene = &context["screen"]["scene"];
        if context["screen"]["status"] == "live"
            && scene["fresh"] == true
            && scene["confidence"].as_f64().unwrap_or_default() >= 0.8
        {
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
        } else {
            self.pending_event.clear();
            self.event_votes = 0;
        }
        // Keep the latest action pending while the player or Buddy is speaking.
        // Seeing an action during a conversation is not proof it was spoken.
        let live = context["screen"]["status"] == "live";
        let fast = &context["fast_event"];
        let event_available = |event: &str, seconds| {
            self.spoken_events
                .get(event)
                .is_none_or(|t| now.duration_since(*t) >= Duration::from_secs(seconds))
        };
        let fast_event = live
            && fast["fresh"] == true
            && matches!(field(fast, "event"), "death" | "danger" | "victory")
            && event_available(field(fast, "event"), 30);
        let coaching = &context["coaching"];
        let warning = &context["guidance"]["warning"];
        let warning_id = field(warning, "id");
        // Losing location for a frame, refreshing progress, or editing a guide
        // does not mean the player entered a new encounter.
        let warning_key = format!(
            "{}:{}",
            field(warning, "evidence_kind"),
            warning["encounter_step_id"].as_str().unwrap_or(warning_id)
        );
        let upcoming_danger = live
            && !warning_id.is_empty()
            && !self.warned_hazards.contains(&warning_key)
            && !field(warning, "text").is_empty();
        if field(coaching, "kind") == "in_progress" {
            self.acknowledged_actions
                .insert(field(coaching, "step_id").into());
        }
        let urgent = matches!(field(coaching, "kind"), "danger" | "recovery");
        let coach_id = field(coaching, "id");
        let can_coach = live
            && !matches!(field(coaching, "kind"), "next_action" | "in_progress")
            && !coach_id.is_empty()
            && !field(coaching, "text").is_empty()
            && !self.spoken_coaching.contains(coach_id)
            && self.last_coaching.is_none_or(|t| {
                now.duration_since(t) >= Duration::from_secs(if urgent { 12 } else { 30 })
            });
        let urgent_coaching = can_coach && urgent;
        let urgent_event = (fast_event && matches!(field(fast, "event"), "death" | "danger"))
            || (self.event_votes >= 2
                && matches!(field(scene, "event"), "death" | "danger")
                && scene["fresh"] == true
                && scene["age_ms"].as_u64().unwrap_or(0) <= 3000
                && event_available(field(scene, "event"), 30));
        let can_interrupt =
            context["player_speaking"] == false && (urgent_coaching || urgent_event);
        if context["conversation_active"] == true && !can_interrupt {
            self.last_conversation = Some(now);
            return None;
        }
        if self
            .last_conversation
            .is_some_and(|t| now.duration_since(t) < Duration::from_millis(250))
            && !can_interrupt
        {
            return None;
        }
        let guidance = &context["guidance"];
        let live = context["screen"]["status"] == "live";
        let place_conflict = context["current_place"]["id"]
            .as_str()
            .zip(context["target_place"]["id"].as_str())
            .is_some_and(|(current, target)| current != target);
        let ready = !place_conflict
            && !self
                .acknowledged_actions
                .contains(field(&guidance["now"], "id"))
            && matches!(guidance["status"].as_str(), Some("ready" | "unlocated"))
            && live
            && (context["screen"]["scene"]["fresh"] == true
                || context["spatial"]["tracker"]["status"] == "tracked"
                || context["spatial"]["observation"]["tracker"]["status"] == "tracked"
                || (context["coaching"]["kind"] == "next_action"
                    && context["coaching"]["step_id"] == guidance["now"]["id"]));
        let action = &guidance["now"];
        let action_key = format!("{}:{}", field(action, "id"), cue(context));
        // Urgent, eligible cues precede greetings and may interrupt thinking.
        if urgent_coaching && context["player_speaking"] != true {
            self.urgent = true;
            return Some(self.coach(coaching, now));
        }
        if fast_event && context["player_speaking"] != true {
            self.spoken_events.insert(field(fast, "event").into(), now);
            self.last_spoken = Some(now);
            self.urgent = matches!(field(fast, "event"), "death" | "danger");
            return Some(field(fast, "text").to_owned());
        }
        if live && scene["fresh"] == true && scene["age_ms"].as_u64().unwrap_or(0) <= 3000 {
            let event = field(scene, "event");
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
                    .is_none_or(|t| now.duration_since(t) >= Duration::from_millis(250))
            {
                self.spoken_events.insert(event.into(), now);
                self.last_spoken = Some(now);
                self.urgent = matches!(event, "death" | "danger");
                return Some(brief(feedback));
            }
        }
        if upcoming_danger && context["player_speaking"] != true {
            self.warned_hazards.insert(warning_key);
            self.last_spoken = Some(now);
            // A source preview is not an observed emergency. In particular it
            // must never cancel a request correcting our location or route.
            return Some(field(warning, "text").to_owned());
        }
        // Already talking with the player: a greeting now would talk over
        // (and once replaced) the answer they just asked for.
        if !self.introduced && self.last_conversation.is_some() {
            self.introduced = true;
        }
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
            } else if context["new_game"] == true && context["preparation_status"] == "running" {
                self.welcomed_new_game = true;
                let game = Some(field(context, "game"))
                    .filter(|g| !g.is_empty())
                    .unwrap_or("this game");
                format!(
                    "Looks like we're starting a new game of {game}. Let me gather the walkthrough, charts and maps for it first. Go ahead and play; I'll tell you when it's ready."
                )
            } else if context["preparing"] == true && context["guide_ready"] != true {
                "I'm preparing your guide. I'll talk you through the next steps as you play.".into()
            } else {
                "I'll talk you through the next steps as you play.".into()
            };
            return Some(format!("{greeting} {next}"));
        }

        if self.welcomed_new_game
            && !self.announced_ready
            && matches!(field(context, "preparation_status"), "ready" | "partial")
            && context["player_speaking"] != true
        {
            self.announced_ready = true;
            self.last_spoken = Some(now);
            let gathered: Vec<&str> = context["gathered"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .filter(|l| !l.starts_with("Walkthrough:"))
                .collect();
            let game = Some(field(context, "game"))
                .filter(|g| !g.is_empty())
                .unwrap_or("the game");
            let mut line = format!("Okay, I'm all set for {game}");
            if !gathered.is_empty() {
                line.push_str(&format!(": {}", gathered.join(", ")));
            }
            line.push('.');
            if ready && !field(action, "id").is_empty() && self.spoken_actions.insert(action_key) {
                line.push(' ');
                line.push_str(&cue(context));
            }
            return Some(line);
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
    super::prepared::brief(text)
}

fn cue(context: &Value) -> String {
    // The errand the step belongs to, when there is one: a step's own words
    // narrate the video's moment ("use your sword against the enemies"),
    // which was said with no enemy on the player's screen.
    let errand = &context["guidance"]["errand"];
    let action = if errand.is_object() {
        errand
    } else {
        &context["guidance"]["now"]
    };
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
    fn route_warning_waits_for_corrections_and_survives_context_refreshes() {
        let mut n = Narrator::default();
        let t = Instant::now();
        let mut c = action("entrance");
        n.next(&c, t);
        c["conversation_active"] = json!(true);
        c["player_speaking"] = json!(false);
        c["guidance"]["warning"] = json!({"id":"guide:1:chamber", "encounter_step_id":"chamber", "text":"Danger ahead: a boss waits inside."});
        assert!(n.next_cue(&c, t + Duration::from_secs(1)).is_none());
        c["conversation_active"] = json!(false);
        let cue = n.next_cue(&c, t + Duration::from_secs(2)).unwrap();
        assert_eq!(cue["text"], c["guidance"]["warning"]["text"]);
        assert_eq!(cue["urgent"], false);
        for i in 3..6 {
            c["guidance"]["warning"] = Value::Null;
            n.next_cue(&c, t + Duration::from_secs(i * 2));
            c["guidance"]["warning"] = json!({"id":format!("guide:{i}:chamber"), "encounter_step_id":"chamber", "text":"Danger ahead: a boss waits inside."});
            assert!(n.next_cue(&c, t + Duration::from_secs(i * 2 + 1)).is_none());
        }
        c["guidance"]["warning"]["evidence_kind"] = json!("source_route_requirement");
        assert!(
            n.next_cue(&c, t + Duration::from_secs(18)).is_some(),
            "A distinct gate requirement is not the same warning as combat"
        );
        c["guidance"]["warning"]["evidence_kind"] = Value::Null;
        assert!(n.next_cue(&c, t + Duration::from_secs(19)).is_none());
        c["screen"]["session_id"] = json!("new-session");
        assert_eq!(
            n.next_cue(&c, t + Duration::from_secs(20)).unwrap()["text"],
            c["guidance"]["warning"]["text"]
        );
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
    fn progress_updates_do_not_repeat_unchanged_advice() {
        let mut n = Narrator::default();
        let t = Instant::now();
        let mut c = action("a");
        c["progress_epoch"] = json!(1);
        assert!(n.next(&c, t).is_some());
        c["progress_epoch"] = json!(2);
        assert!(n.next(&c, t + Duration::from_secs(2)).is_none());
        c["guidance"]["now"]["instructions"] = json!("Use the other door.");
        assert_eq!(
            n.next(&c, t + Duration::from_secs(3)).as_deref(),
            Some("Use the other door.")
        );
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

    #[test]
    fn verified_events_precede_new_objectives_and_survive_conversation() {
        let mut n = Narrator::default();
        let t = Instant::now();
        n.next(&action("a"), t);
        let mut c = action("b");
        c["conversation_active"] = json!(true);
        for at in [1, 2] {
            c["screen"]["scene"] = json!({"fresh":true,"confidence":0.95,"event":"danger","feedback":"Watch the enemy ahead.","captured_at_ms":at});
            assert!(n.next(&c, t + Duration::from_millis(at * 100)).is_none());
        }
        c["conversation_active"] = json!(false);
        assert!(n.next(&c, t + Duration::from_millis(400)).is_none());
        assert_eq!(
            n.next(&c, t + Duration::from_millis(450)).as_deref(),
            Some("Watch the enemy ahead.")
        );
        assert_eq!(
            n.next(&c, t + Duration::from_millis(700)).as_deref(),
            Some("Walk right to b. Climb the ladder.")
        );
    }

    #[test]
    fn urgent_event_interrupts_thinking_once_but_never_the_player() {
        let mut n = Narrator::default();
        let t = Instant::now();
        let mut c = action("a");
        c["conversation_active"] = json!(true);
        c["player_speaking"] = json!(true);
        c["fast_event"] = json!({"fresh":true,"event":"death","text":"The screen says game over."});
        assert!(n.next_cue(&c, t).is_none());
        c["player_speaking"] = json!(false);
        let cue = n.next_cue(&c, t + Duration::from_secs(1)).unwrap();
        assert_eq!(cue["urgent"], true);
        assert!(cue["text"].as_str().unwrap().contains("game over"));
        assert!(
            n.next_cue(&c, t + Duration::from_secs(2)).is_none(),
            "cooldown cannot allow an ordinary cue through a conversation"
        );
        c["fast_event"]["fresh"] = json!(false);
        assert!(n.next_cue(&c, t + Duration::from_secs(40)).is_none());
    }
    #[test]
    fn old_vision_danger_does_not_interrupt_on_its_second_vote() {
        let mut n = Narrator::default();
        let t = Instant::now();
        let mut c = action("a");
        n.next(&c, t);
        c["conversation_active"] = json!(true);
        c["player_speaking"] = json!(false);
        c["screen"]["scene"] = json!({"fresh":true,"confidence":0.95,"event":"danger","feedback":"Jump now!","age_ms":6000,"captured_at_ms":1});
        assert!(n.next(&c, t + Duration::from_secs(1)).is_none());
        c["screen"]["scene"]["captured_at_ms"] = json!(2);
        assert!(n.next(&c, t + Duration::from_secs(2)).is_none());
    }

    #[test]
    fn uncertain_event_is_not_a_confirmation() {
        let mut n = Narrator::default();
        let t = Instant::now();
        let mut c = action("a");
        n.next(&c, t);
        c["screen"]["scene"] =
            json!({"fresh":true,"confidence":0.4,"event":"death","captured_at_ms":1});
        assert!(n.next(&c, t + Duration::from_secs(1)).is_none());
        c["screen"]["scene"] =
            json!({"fresh":true,"confidence":0.95,"event":"death","captured_at_ms":2});
        assert!(n.next(&c, t + Duration::from_secs(2)).is_none());
        c["screen"]["scene"]["captured_at_ms"] = json!(3);
        assert!(
            n.next(&c, t + Duration::from_secs(3))
                .unwrap()
                .contains("died")
        );
    }

    #[test]
    fn ongoing_action_and_conversational_acknowledgment_suppress_reworded_reminders() {
        for observed in [true, false] {
            let mut n = Narrator::default();
            let now = Instant::now();
            let mut c = action("farm");
            n.next(&c, now);
            if observed {
                c["coaching"] = json!({"id":"seen-working","kind":"in_progress","step_id":"farm","text":"The player is already farming."});
            } else {
                n.heard_advice(&c, now + Duration::from_secs(1));
            }
            c["guidance"]["now"]["instructions"] =
                json!("Keep farming until you have enough gold.");
            assert!(n.next(&c, now + Duration::from_secs(10)).is_none());
            c["coaching"] = Value::Null;
            c["guidance"]["now"]["instructions"] = json!("Fight here for the replacement boots.");
            assert!(n.next(&c, now + Duration::from_secs(40)).is_none());
            let mut danger = c.clone();
            danger["fast_event"] =
                json!({"fresh":true,"event":"danger","text":"Your health is low."});
            assert_eq!(
                n.next(&danger, now + Duration::from_secs(41)).as_deref(),
                Some("Your health is low.")
            );
            assert!(
                n.next(&action("buy"), now + Duration::from_secs(42))
                    .unwrap()
                    .contains("buy")
            );
        }
    }
}
