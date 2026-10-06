//! Publish the final answer and its optional next action in one model pass.
//! The caller still owns endpointing, superseded-request checks and delivery.
use crate::{Store, harness::ToolOutput, navigation::advice::NextAction, screen};
use anyhow::{Result, ensure};
use serde::Deserialize;
use serde_json::{Value, json};
use std::hash::{DefaultHasher, Hash, Hasher};

pub const TOOL: &str = "finish_voice_reply";
pub const RETRY_INSTRUCTIONS: &str = "The prior answer was rejected. This is the replacement evidence for the SAME live request. Use the supplied current context and any new pixels directly, then finish the corrected answer; no separate screen fetch is needed. Preserve successful actions already completed in this turn.";
pub const INSTRUCTIONS: &str = "For live voice, finish with finish_voice_reply as soon as the complete player request is satisfied. It delivers speech immediately. Copy request_id from the latest application request; a follow-up or corrected transcript replaces that ID. For a new gameplay instruction, include next_action in that same call instead of a separate set_next_action; its instructions will be the spoken text. Complete any other requested tool actions and check their successful results first. Do not batch this final tool with other actions. Set screen_grounded=true whenever the answer depends on the supplied screenshot, such as identifying this door or directing an immediate movement. Set it false for instructions or facts supported entirely by the guide and confirmed progress; ordinary screen movement does not invalidate those. Use next_action=null for factual answers, clarifications, uncertainty or conversation. After success, end the turn without repeating the answer or calling more tools. On failure, correct only the reported problem and retry. Never claim an action succeeded before its tool succeeded.";

pub fn request_id(utterance: u64, request: &str) -> String {
    let mut hash = DefaultHasher::new();
    request.hash(&mut hash);
    format!("{utterance}-{:016x}", hash.finish())
}

/// A failed answer remains failed, but its next pass already has the current
/// image, epoch and request identity. No context-fetch round trip is needed.
pub fn retry_context(
    store: &Store,
    game: &str,
    error: &str,
    request_id: &str,
    request: &str,
) -> ToolOutput {
    let mut output = ToolOutput {
        value: json!({"error":error,"live_player_request":request,"request_id":request_id}),
        images: vec![],
    };
    if let Ok(context) = crate::harness::context_for_request(store, request)
        && context.value["game"]
            .as_str()
            .is_some_and(|g| g.eq_ignore_ascii_case(game))
    {
        output.value["current_game_context"] = context.value;
        output.value["retry_instruction"] = json!(
            "Use the supplied current context and images to revise the answer. Do not repeat successful actions or fetch this same context again. Nothing from the failed answer was published."
        );
        output.images = context.images;
    }
    output
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Reply {
    /// Copy the latest request_id, including after a follow-up or correction.
    pub request_id: String,
    /// The entire useful spoken answer, usually one or two sentences.
    pub text: String,
    pub screen_grounded: bool,
    /// New immediate advice only. Its instructions must match text exactly.
    pub next_action: Option<NextAction>,
}

impl Reply {
    pub fn requires_current_screen(&self) -> bool {
        self.screen_grounded
            || self
                .next_action
                .as_ref()
                .is_some_and(|a| a.screen_direction.is_some())
    }
}

pub fn tools() -> Value {
    let mut tools = crate::harness::tool_definitions();
    let mut schema = json!(schemars::schema_for!(Reply));
    schema["required"] = json!(["request_id", "text", "screen_grounded", "next_action"]);
    tools.as_array_mut().unwrap().push(json!({
        "name":TOOL, "description":INSTRUCTIONS, "inputSchema":schema
    }));
    tools
}

pub fn apply(
    store: &Store,
    arguments: Value,
    request_id: &str,
    frame: Option<&screen::Snapshot>,
) -> Result<ToolOutput> {
    let reply: Reply = serde_json::from_value(arguments)?;
    ensure!(
        reply.request_id == request_id,
        "This answer belongs to an older request. Complete the latest live_player_request using request_id {request_id} before finishing. Nothing was published or spoken."
    );
    ensure!(!reply.text.trim().is_empty(), "Spoken answer is empty");
    // Source instructions remain valid while a menu opens or the camera moves.
    // Only visual claims and immediate screen-relative directions require the
    // original pixels. Revision/epoch/route eligibility are still checked below.
    if reply.requires_current_screen() {
        ensure!(
            frame.is_some_and(|frame| screen::still_current(store, frame).unwrap_or(false)),
            "The supplied screen is no longer current. Reassess the replacement image attached to this rejection before finishing. If no replacement context is attached, read get_game_context. No advice was published."
        );
    }
    let step_id = reply
        .next_action
        .as_ref()
        .map(|action| action.step_id.clone());
    if let Some(action) = reply.next_action {
        ensure!(
            action.instructions == reply.text,
            "Use the same text for speech and next_action.instructions"
        );
        crate::harness::call_tool(store, "set_next_action", json!(action))?;
    }
    Ok(ToolOutput {
        value: json!({"delivered":true,"voice_reply":{"text":reply.text,"step_id":step_id}}),
        images: vec![],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejected_answer_returns_exact_replacement_pixels_without_publishing() -> Result<()> {
        let root = tempfile::tempdir()?;
        let store = Store::open(root.path())?;
        let guide =
            gamebuddy_core::import_markdown("Game", "Guide", "# Continue\nTalk to the guard.")?;
        store.save_guide(&guide)?;
        store.put("active_guide", &guide.id)?;
        let ticket = screen::start(&store, "Game", true)?;
        let path = root.path().join("frame.png");
        image::RgbImage::from_pixel(64, 64, image::Rgb([200, 30, 40])).save(&path)?;
        screen::publish(&store, &ticket, &path, "fixture")?;
        let old = screen::snapshot(&store)?;
        image::RgbImage::from_pixel(64, 64, image::Rgb([20, 30, 240])).save(&path)?;
        screen::publish(&store, &ticket, &path, "fixture")?;
        let answer = json!({"request_id":"current","text":"It is blue.","screen_grounded":true,"next_action":null});
        let error = apply(&store, answer.clone(), "current", Some(&old))
            .err()
            .expect("Old pixels must fail");
        let output = retry_context(
            &store,
            "Game",
            &error.to_string(),
            "current",
            "What color is it now?",
        );
        let fresh = output.screen_frame().expect("Retry has its actual image");
        assert_eq!(fresh.jpeg, output.images[0]);
        assert_ne!(fresh.jpeg, old.jpeg);
        assert_eq!(output.value["request_id"], "current");
        assert!(
            store
                .get::<Value>(&format!("next-action:{}", guide.id))?
                .is_none()
        );
        assert!(apply(&store, answer, "current", Some(&fresh)).is_ok());
        assert!(
            retry_context(&store, "Other game", "failure", "current", "question")
                .images
                .is_empty()
        );
        Ok(())
    }

    #[test]
    fn final_reply_publishes_matching_advice_and_rejects_stale_or_invalid_changes() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let guide = gamebuddy_core::import_markdown("Game", "Guide", "# Begin\nTalk to the guard.")
            .unwrap();
        store.save_guide(&guide).unwrap();
        store.put("active_guide", &guide.id).unwrap();
        let ticket = screen::start(&store, "Game", true).unwrap();
        let path = dir.path().join("screen.png");
        image::RgbImage::from_pixel(64, 64, image::Rgb([20, 180, 90]))
            .save(&path)
            .unwrap();
        screen::publish(&store, &ticket, &path, "test").unwrap();
        let frame = screen::snapshot(&store).unwrap();
        let progress = store.progress(&guide).unwrap();
        let reply = json!({"request_id":"request-1","text":"Talk to the guard.","screen_grounded":true,"next_action":{
            "expected_revision":guide.revision,"epoch":progress.epoch,"step_id":guide.steps[0].id,
            "instructions":"Talk to the guard.","screen_direction":null
        }});
        let key = format!("next-action:{}", guide.id);
        assert!(apply(&store, reply.clone(), "request-2", Some(&frame)).is_err());
        assert!(store.get::<Value>(&key).unwrap().is_none());
        let output = apply(&store, reply.clone(), "request-1", Some(&frame)).unwrap();
        let saved = store.get::<Value>(&key).unwrap().unwrap();
        assert_eq!(output.value["voice_reply"]["text"], saved["instructions"]);
        assert_eq!(output.value["voice_reply"]["step_id"], saved["step_id"]);
        assert_eq!(store.progress(&guide).unwrap().epoch, progress.epoch);

        let mut invalid = reply.clone();
        invalid["next_action"]["epoch"] = json!(progress.epoch + 1);
        assert!(apply(&store, invalid, "request-1", Some(&frame)).is_err());
        let mut mismatch = reply.clone();
        mismatch["text"] = json!("Go somewhere else.");
        assert!(apply(&store, mismatch, "request-1", Some(&frame)).is_err());
        screen::start(&store, "Game", true).unwrap();
        assert!(apply(&store, reply, "request-1", Some(&frame)).is_err());
        assert_eq!(store.get::<Value>(&key).unwrap().unwrap(), saved);
    }

    #[test]
    fn final_transcript_corrections_revoke_an_earlier_reply_id() {
        assert_ne!(
            request_id(2, "What color is it?"),
            request_id(2, "What color is it? Put it in a widget.")
        );
        assert_ne!(request_id(2, "Where next?"), request_id(3, "Where next?"));
    }

    #[test]
    fn factual_reply_needs_no_action_but_visual_claims_require_current_evidence() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let mut reply = json!({"request_id":"request-1","text":"The target was 2,200 experience.","screen_grounded":false,"next_action":null});
        assert!(apply(&store, reply.clone(), "request-1", None).is_ok());
        reply["screen_grounded"] = json!(true);
        assert!(apply(&store, reply, "request-1", None).is_err());
    }

    #[test]
    fn source_advice_survives_camera_changes_but_visual_directions_do_not() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let guide = gamebuddy_core::import_markdown(
            "Game",
            "Guide",
            "# Use the key\nUse the key to open the door.",
        )
        .unwrap();
        store.save_guide(&guide).unwrap();
        store.put("active_guide", &guide.id).unwrap();
        let mut reply = json!({"request_id":"current","text":"Use the key to open the door.","screen_grounded":false,"next_action":{
            "expected_revision":guide.revision,"epoch":0,"step_id":guide.steps[0].id,
            "instructions":"Use the key to open the door.","screen_direction":null
        }});
        assert!(apply(&store, reply.clone(), "current", None).is_ok());
        reply["screen_grounded"] = json!(true);
        assert!(apply(&store, reply.clone(), "current", None).is_err());
        reply["screen_grounded"] = json!(false);
        reply["next_action"]["screen_direction"] = json!("right");
        assert!(apply(&store, reply, "current", None).is_err());
    }
}
