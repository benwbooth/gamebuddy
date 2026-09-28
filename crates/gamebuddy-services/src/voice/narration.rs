//! Session-local spoken initiative. Visual guesses never become completion claims.
use serde_json::Value;
use std::time::{Duration, Instant};

#[derive(Default)]
pub struct Narrator {
    game: String,
    introduced: bool,
    last_action: String,
    pending: String,
    pending_since: Option<Instant>,
    last_spoken: Option<Instant>,
    last_scene_at: u64,
    pending_event: String,
    event_votes: u8,
    spoken_event: String,
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
        let guidance = &context["guidance"];
        let ready = guidance["status"] == "ready" && context["location_status"] == "estimated";
        let action = if ready {
            field(&guidance["now"], "id")
        } else {
            String::new()
        };
        if !self.introduced {
            self.introduced = true;
            self.last_spoken = Some(now);
            self.last_action = action;
            let state = if context["preparing"] == true {
                "I'm putting your walkthrough together while you play."
            } else {
                "I've got your guide open. I'll keep the next steps and maps alongside your game."
            };
            let next = if ready {
                cue(context)
            } else {
                "I'm watching for a reliable location match before giving directions.".into()
            };
            return Some(format!(
                "Hey, I'm GameBuddy, your companion for {game}. {state} {next} Unmute your mic or choose Listen whenever you want to talk."
            ));
        }
        let scene = &context["screen"]["scene"];
        if context["screen"]["status"] == "live" && scene["fresh"] == true {
            let at = scene["captured_at_ms"].as_u64().unwrap_or_default();
            let event = field(scene, "event");
            if at > self.last_scene_at {
                self.last_scene_at = at;
                if event == "none" {
                    self.spoken_event.clear();
                }
                if event == self.pending_event {
                    self.event_votes = self.event_votes.saturating_add(1);
                } else {
                    self.pending_event = event.clone();
                    self.event_votes = 1;
                }
            }
            let feedback = field(scene, "feedback");
            if ["death", "danger", "dialogue", "menu", "new_area", "victory"]
                .contains(&event.as_str())
                && self.event_votes >= 2
                && event != self.spoken_event
                && !feedback.is_empty()
                && scene["confidence"].as_f64().unwrap_or_default() >= 0.8
                && self
                    .last_spoken
                    .is_none_or(|t| now.duration_since(t) >= Duration::from_secs(25))
            {
                self.spoken_event = event;
                self.last_spoken = Some(now);
                return Some(feedback);
            }
        } else {
            self.pending_event.clear();
            self.event_votes = 0;
        }
        if action != self.pending {
            self.pending = action.clone();
            self.pending_since = Some(now);
        }
        if action.is_empty()
            || action == self.last_action
            || self
                .pending_since
                .is_none_or(|t| now.duration_since(t) < Duration::from_secs(5))
            || self
                .last_spoken
                .is_some_and(|t| now.duration_since(t) < Duration::from_secs(25))
        {
            return None;
        }
        self.last_action = action;
        self.last_spoken = Some(now);
        Some(cue(context))
    }
}

fn cue(context: &Value) -> String {
    let guidance = &context["guidance"];
    let title = field(&guidance["now"], "title");
    let place = field(&context["current_place"], "label");
    let location = if place.is_empty() {
        String::new()
    } else {
        format!("Looks like you're at {place}. ")
    };
    if guidance["requirements"]
        .as_array()
        .is_some_and(|r| !r.is_empty())
    {
        format!(
            "{location}Before moving on, let's check this prerequisite: {title}. Tell me if you've already done it, and I'll update the plan."
        )
    } else {
        format!("{location}Your next step is: {title}. I've put the details in your guide.")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn introduces_once_without_a_location_then_debounces_advice() {
        let mut n = Narrator::default();
        let t = Instant::now();
        let mut c = json!({"game":"Test game","preparing":true});
        assert!(n.next(&c, t).unwrap().contains("I'm GameBuddy"));
        assert!(n.next(&c, t + Duration::from_secs(30)).is_none());
        c["location_status"] = json!("estimated");
        c["guidance"] =
            json!({"status":"ready","now":{"id":"a","title":"Buy the key"},"requirements":[{}]});
        assert!(n.next(&c, t + Duration::from_secs(31)).is_none());
        assert!(
            n.next(&c, t + Duration::from_secs(37))
                .unwrap()
                .contains("check this prerequisite")
        );
        assert!(n.next(&c, t + Duration::from_secs(70)).is_none());
        c["location_status"] = json!("uncertain");
        c["guidance"]["now"]["id"] = json!("b");
        assert!(n.next(&c, t + Duration::from_secs(90)).is_none());
        assert!(n.next(&c, t + Duration::from_secs(100)).is_none());
        c["game"] = json!("Another game");
        assert!(
            n.next(&c, t + Duration::from_secs(110))
                .unwrap()
                .contains("I'm GameBuddy")
        );
    }
    #[test]
    fn unsolicited_feedback_needs_two_fresh_observations_and_does_not_repeat() {
        let mut n = Narrator::default();
        let t = Instant::now();
        let mut c = json!({"game":"Game","screen":{"status":"live","scene":{"fresh":true,"captured_at_ms":1,"event":"death","confidence":0.9,"feedback":"Looks like a game-over screen. We can plan the next attempt."}}});
        n.next(&c, t);
        assert!(n.next(&c, t + Duration::from_secs(26)).is_none());
        assert!(n.next(&c, t + Duration::from_secs(27)).is_none());
        c["screen"]["scene"]["captured_at_ms"] = json!(2);
        assert!(
            n.next(&c, t + Duration::from_secs(28))
                .unwrap()
                .contains("game-over")
        );
        assert!(n.next(&c, t + Duration::from_secs(60)).is_none());
        c["screen"]["status"] = json!("paused");
        c["screen"]["scene"]["event"] = json!("danger");
        c["screen"]["scene"]["captured_at_ms"] = json!(3);
        assert!(n.next(&c, t + Duration::from_secs(90)).is_none());
    }
}
