//! Source-backed responses selected locally. Preparing a branch never commits
//! its hypothetical facts; every reply is selected again against current state.
use crate::{Store, navigation, outcomes, screen};
use anyhow::Result;
use gamebuddy_core::{Guide, Progress, outcomes::KnownFact};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Response {
    pub text: String,
    pub kind: String,
    pub step_id: Option<String>,
}

pub fn brief(text: &str) -> String {
    let sentences = text
        .split_inclusive(['.', '!', '?'])
        .take(2)
        .collect::<String>();
    let words: Vec<_> = sentences.split_whitespace().collect();
    if words.len() <= 55 {
        return sentences.trim().to_owned();
    }
    // Do not cut a conditional or instruction in the middle of a sentence.
    let first = text.split_inclusive(['.', '!', '?']).next().unwrap_or(text);
    if first.split_whitespace().count() <= 55 {
        first.trim().into()
    } else {
        text.trim().into()
    }
}

fn query(text: &str) -> String {
    let mut spoken = text.trim();
    // Recognition can prefix a complete question with a breath annotation.
    // Only known acoustic tags are ignored, never words or unknown directions.
    while let Some(tag) = spoken.strip_prefix('[').and_then(|s| s.split_once(']')) {
        if !matches!(
            tag.0.to_ascii_lowercase().as_str(),
            "sigh" | "sighs" | "yawn" | "yawns"
        ) {
            break;
        }
        spoken = tag.1.trim_start();
    }
    let mut value = spoken.to_lowercase().replace('’', "'");
    value = value.trim_end_matches(['.', '?', '!']).trim().to_owned();
    for prefix in ["hey gamebuddy, ", "gamebuddy, ", "please "] {
        if let Some(rest) = value.strip_prefix(prefix) {
            value = rest.into();
        }
    }
    value
}

/// Exact complete intents only. Compound requests and corrections must retain
/// their full meaning and go to the structured or general reasoning path.
pub fn intent(text: &str) -> Option<&'static str> {
    match query(text).as_str() {
        "what now"
        | "now what"
        | "what next"
        | "what's next"
        | "what do i do"
        | "what do i do now"
        | "what should i do"
        | "what should i do next"
        | "where do i go"
        | "where do i go now"
        | "which way"
        | "where next"
        | "what's my next step"
        | "where should i go"
        | "where should i go now"
        | "where to now"
        | "where am i supposed to go"
        | "what am i supposed to do"
        | "what do i need to do"
        | "what do i need to do now"
        | "what's the plan"
        | "okay what now"
        | "ok what now"
        | "so what now" => Some("next"),
        "why"
        | "why that"
        | "why this"
        | "why should i do that"
        | "what's the point"
        | "why am i doing this"
        | "why do i need to do that" => Some("why"),
        "repeat"
        | "repeat that"
        | "say that again"
        | "what did you say"
        | "repeat the instruction" => Some("repeat"),
        "where am i" | "what room am i in" | "what area am i in" => Some("location"),
        _ => None,
    }
}

pub fn for_context(context: &Value, request: &str) -> Option<Response> {
    let kind = intent(request)?;
    // The text cache has never inspected pixels. Live navigation and location
    // questions require the image-aware path, even if the saved plan is ready.
    if context["screen"]["status"] == "live" && matches!(kind, "next" | "location") {
        return None;
    }
    let nav = &context["navigation"];
    let guidance = &nav["guidance"];
    let action = &guidance["now"];
    let text = if kind == "location" {
        if nav["location"]["status"] == "confirmed"
            || nav["spatial"]["observation"]["tracker"]["status"] == "tracked"
        {
            format!(
                "The current location match is {}.",
                nav["map"]["current_place"]["label"].as_str()?
            )
        } else {
            return None;
        }
    } else if nav["planning"]["goal_complete"] == true {
        "Your selected goal is complete. You can choose another goal or explore optional rewards."
            .into()
    } else if matches!(guidance["status"].as_str(), Some("ready" | "unlocated")) {
        let instructions = action["instructions"]
            .as_str()
            .filter(|s| !s.trim().is_empty())?;
        if kind == "why" {
            let cursor = nav["cursor"]["step_id"]
                .as_str()
                .or_else(|| context["progress"]["current_step"].as_str());
            let skipped = nav["planning"]["not_needed"].as_array().and_then(|rows| {
                rows.iter().find(|row| {
                    cursor.is_some_and(|id| {
                        row["step_id"] == id
                            || row["step_ids"]
                                .as_array()
                                .is_some_and(|ids| ids.iter().any(|v| v == id))
                    })
                })
            });
            brief(skipped.and_then(|r| r["reason"].as_str())?)
        } else if guidance["status"] == "unlocated" {
            format!(
                "The next step in your plan is: {} I haven't confirmed your current location.",
                brief(instructions)
            )
        } else {
            brief(instructions)
        }
    } else {
        return None;
    };
    Some(Response {
        text,
        kind: kind.into(),
        step_id: action["id"].as_str().map(str::to_owned),
    })
}

pub fn context(store: &Store, game: &str) -> Result<Option<Value>> {
    let Some(id) = store.get::<String>("active_guide")? else {
        return Ok(None);
    };
    let guide = store.guide(&id)?;
    if !guide.game.eq_ignore_ascii_case(game) {
        return Ok(None);
    }
    let progress = store.progress(&guide)?;
    let nav = navigation::context(store, &guide, &progress, &Default::default())?;
    Ok(Some(
        json!({"game":game,"guide_id":guide.id,"revision":guide.revision,"progress":progress,"screen":screen::status(store)?,"navigation":nav}),
    ))
}

pub fn reply(store: &Store, game: &str, request: &str) -> Result<Option<Response>> {
    if intent(request).is_none() {
        return Ok(None);
    }
    let response = context(store, game)?.and_then(|c| for_context(&c, request));
    if let Some(response) = &response
        && response.kind == "repeat"
        && !store
            .conversation_messages(game, "", None, 8)?
            .iter()
            .rev()
            .find(|m| m.role == "assistant")
            .is_some_and(|m| m.text.trim() == response.text)
    {
        // "Repeat that" refers to the preceding reply, which might be a
        // warning or an explanation. Only a still-current instruction can be
        // repeated locally; ordinary conversation recall handles other cases.
        return Ok(None);
    }
    Ok(response)
}

/// A bounded frontier for speech preparation, never a list of instructions to
/// play automatically. Actual navigation still chooses the current eligible leaf.
pub fn frontier(
    store: &Store,
    guide: &Guide,
    progress: &Progress,
    nav: &Value,
) -> Result<Vec<String>> {
    let mut phrases = vec![];
    let mut push = |text: &str| {
        let text = text.trim().to_owned();
        if !text.is_empty() && !phrases.contains(&text) && phrases.len() < 8 {
            phrases.push(text);
        }
    };
    for request in ["What next?", "Why?"] {
        if let Some(reply) = for_context(&json!({"navigation":nav}), request) {
            push(&reply.text);
        }
    }
    push("The screen says game over. Let's work through the next attempt.");
    push("Your health is low. Watch your health before the next fight.");
    if let Some(text) = nav["guidance"]["now"]["instructions"].as_str() {
        push(&brief(text));
    }
    for next in nav["guidance"]["next"]
        .as_array()
        .into_iter()
        .flatten()
        .take(2)
    {
        if let Some(text) = next["instructions"].as_str() {
            push(&brief(text));
        }
    }
    if let Some(prepared) = outcomes::prepared(store, guide)? {
        let current = nav["guidance"]["now"]["id"].as_str().unwrap_or_default();
        for rule in prepared
            .model
            .actions
            .iter()
            .filter(|r| r.step_id == current)
            .take(2)
        {
            let mut possible = progress.clone();
            possible.completed.insert(rule.step_id.clone());
            for effect in &rule.effects {
                possible.world.facts.insert(
                    effect.id.clone(),
                    KnownFact {
                        value: effect.value,
                        evidence: "Preparation only".into(),
                        source: "hypothesis".into(),
                        step_id: None,
                        at_ms: 0,
                    },
                );
            }
            let next = gamebuddy_core::guidance_with_outcomes(
                guide,
                &possible,
                &Default::default(),
                None,
                &prepared.model,
            );
            if let Some(step) = next.now {
                push(&brief(&step.instructions));
            }
        }
    }
    Ok(phrases)
}

/// Audio bytes can be reused across sessions; permission to play those words
/// cannot. Keep capture and progress scope with each individual request.
pub struct PlaybackGuard {
    guide: Option<(String, u64, u64)>,
    frame: Option<(screen::Ticket, Vec<u8>)>,
}
impl PlaybackGuard {
    pub fn capture(store: &Store, visual: bool) -> Result<Self> {
        let guide = store
            .get::<String>("active_guide")?
            .map(|id| -> Result<_> {
                let g = store.guide(&id)?;
                Ok((id, g.revision, store.progress(&g)?.epoch))
            })
            .transpose()?;
        Ok(Self {
            guide,
            frame: if visual {
                use base64::Engine;
                let frame = screen::snapshot(store)?;
                let image = image::load_from_memory(
                    &base64::engine::general_purpose::STANDARD.decode(frame.jpeg)?,
                )?;
                Some((frame.ticket, screen::frame_signature(&image)))
            } else {
                None
            },
        })
    }
    pub fn current(&self, store: &Store) -> bool {
        if let Some((id, revision, epoch)) = &self.guide {
            if !store
                .get::<String>("active_guide")
                .is_ok_and(|v| v.as_ref() == Some(id))
            {
                return false;
            }
            let Ok(g) = store.guide(id) else {
                return false;
            };
            if g.revision != *revision || !store.progress(&g).is_ok_and(|p| p.epoch == *epoch) {
                return false;
            }
        }
        self.frame.as_ref().is_none_or(|(ticket, signature)| {
            screen::matches_latest_view(store, ticket, signature, screen::FRESH_MS).unwrap_or(false)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn complete_intents_do_not_swallow_corrections_or_extra_work() {
        assert_eq!(intent("What should I do next?"), Some("next"));
        assert_eq!(intent("[yawn] [sigh] Now what?"), Some("next"));
        for q in [
            "What next? But I already have that.",
            "What next and show me a map",
            "I don't want to repeat that",
            "Where do I go if I have no key?",
            "[turn left] Now what?",
        ] {
            assert_eq!(intent(q), None);
        }
    }
    #[test]
    fn local_instructions_preserve_location_uncertainty_and_completed_goals() {
        let mut c = json!({"navigation":{"guidance":{"status":"unlocated","now":{"id":"a","instructions":"Enter the door."}}}});
        assert!(
            for_context(&c, "What next?")
                .unwrap()
                .text
                .contains("haven't confirmed")
        );
        assert!(for_context(&c, "Where am I?").is_none());
        assert_eq!(
            for_context(&c, "Now what?").unwrap().text,
            for_context(&c, "What next?").unwrap().text
        );
        c["navigation"]["planning"] = json!({"goal_complete":true});
        assert!(
            for_context(&c, "What next?")
                .unwrap()
                .text
                .contains("goal is complete")
        );
    }

    #[test]
    fn live_next_and_location_questions_require_image_inspection() {
        let c = json!({"screen":{"status":"live"}, "navigation":{
            "location":{"status":"confirmed"}, "map":{"current_place":{"label":"Town"}},
            "guidance":{"status":"ready","now":{"id":"fight","instructions":"Fight every enemy."}}
        }});
        assert!(for_context(&c, "What next?").is_none());
        assert!(for_context(&c, "Where am I?").is_none());
        assert!(for_context(&c, "Repeat that").is_some());
    }

    #[test]
    fn prepared_playback_loses_permission_after_a_reset_or_capture_pause() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let store = Store::open(dir.path())?;
        let guide = gamebuddy_core::import_markdown("Fixture", "Guide", "# Door\nEnter the door.")?;
        store.save_guide(&guide)?;
        store.put("active_guide", &guide.id)?;
        store.save_progress(&guide, &Default::default(), "fixture")?;
        assert!(PlaybackGuard::capture(&store, true).is_err());
        let ticket = screen::start(&store, &guide.game, true)?;
        let path = dir.path().join("screen.png");
        image::RgbImage::from_pixel(96, 64, image::Rgb([80, 30, 150])).save(&path)?;
        screen::publish(&store, &ticket, &path, "fixture")?;
        let guard = PlaybackGuard::capture(&store, true)?;
        assert!(guard.current(&store));
        let mut progress = store.progress(&guide)?;
        progress.epoch += 1;
        store.save_progress(&guide, &progress, "reset")?;
        assert!(!guard.current(&store));
        let guard = PlaybackGuard::capture(&store, true)?;
        screen::start(&store, &guide.game, false)?;
        assert!(!guard.current(&store));
        Ok(())
    }

    #[test]
    fn repeating_an_explanation_must_not_return_unrelated_plan_instructions() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let store = Store::open(dir.path())?;
        let guide = gamebuddy_core::import_markdown("Fixture", "Guide", "# Door\nEnter the door.")?;
        store.save_guide(&guide)?;
        store.put("active_guide", &guide.id)?;
        store.save_progress(&guide, &Default::default(), "fixture")?;
        store.append_conversation(
            "Fixture",
            "test",
            "explanation",
            "assistant",
            "The red meter is health.",
        )?;
        assert!(reply(&store, "Fixture", "Repeat that")?.is_none());
        let current = reply(&store, "Fixture", "What next?")?.unwrap();
        store.append_conversation("Fixture", "test", "instruction", "assistant", &current.text)?;
        assert_eq!(
            reply(&store, "Fixture", "Repeat that")?.unwrap().text,
            current.text
        );
        Ok(())
    }
}
