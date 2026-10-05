//! One persisted recommendation shared by conversation, the outline and Now card.
use crate::{Store, screen};
use anyhow::{Result, ensure};
use gamebuddy_core::navigation::direction::{DirectionCue, ScreenDirection, movement};
use gamebuddy_core::{Guide, PlaceTracker, Progress, SyncEstimate, guidance_from};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NextAction {
    pub expected_revision: u64,
    pub epoch: u64,
    pub step_id: String,
    /// One immediate action. For movement, include the same supported direction
    /// in screen_direction. Never guess a direction or add it to non-movement.
    pub instructions: String,
    /// Immediate screen-relative movement supported by the latest screenshot
    /// and source route, even when instructions use natural wording such as
    /// "Enter the door beside you". Use null for dialogue, purchases, uncertain
    /// routes or a direction inferred only from map orientation.
    #[serde(default)]
    pub screen_direction: Option<ScreenDirection>,
}
#[derive(Serialize, Deserialize)]
pub struct Recommendation {
    pub revision: u64,
    pub epoch: u64,
    pub session: String,
    pub updated_at_ms: u64,
    pub step_id: String,
    pub instructions: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location_evidence: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub movement: Option<ObservedMovement>,
}

#[derive(Serialize, Deserialize)]
pub struct ObservedMovement {
    pub direction: ScreenDirection,
    pub ticket: screen::Ticket,
    pub captured_at_ms: u64,
    pub frame_signature: Vec<u8>,
}

pub(crate) fn public(recommendation: &Recommendation) -> Value {
    let mut value = json!(recommendation);
    if let Some(movement) = value["movement"].as_object_mut() {
        // Capture signatures are local freshness checks, not useful model input.
        movement.remove("frame_signature");
        movement.remove("ticket");
    }
    value
}

// Screen-relative advice does not require resolving one semantic place among
// several shops/doors sharing the same street. Bind explicit directions to the
// actual view and expire them on a scene, capture or session change.
pub(crate) fn direction(
    store: &Store,
    guide: &Guide,
    guidance: &gamebuddy_core::Guidance,
    recommendation: Option<&Recommendation>,
    spatial: &Value,
    now: u64,
) -> Result<Option<DirectionCue>> {
    let Some(step) = guidance
        .now
        .as_ref()
        .filter(|_| ["ready", "unlocated"].contains(&guidance.status.as_str()))
    else {
        return Ok(None);
    };
    let explicit = recommendation
        .filter(|r| r.step_id == step.id)
        .and_then(|r| r.movement.as_ref());
    let (direction, at) = if let Some(explicit) = explicit {
        if now < explicit.captured_at_ms
            || now - explicit.captured_at_ms >= screen::FRESH_MS
            || !screen::matches_movement(store, &explicit.ticket, &explicit.frame_signature)?
        {
            return Ok(None);
        }
        (explicit.direction, explicit.captured_at_ms)
    } else if supports_action(guide, &step.id, spatial) {
        let Some(direction) = movement(&step.instructions) else {
            return Ok(None);
        };
        let at = spatial["observation"]["tracker"]["observed_at_ms"]
            .as_u64()
            .unwrap_or_default();
        if at == 0 || now < at || now - at >= screen::FRESH_MS {
            return Ok(None);
        }
        (direction, at)
    } else {
        return Ok(None);
    };
    Ok(Some(DirectionCue {
        direction,
        step_id: step.id.clone(),
        label: step.title.clone(),
        instruction: step.instructions.clone(),
        valid_until_ms: at.saturating_add(screen::FRESH_MS),
    }))
}

// Walkthrough ordering is not proof that the player is still behind a gate.
// A current, accepted source match can support advice at the present location
// without claiming that every earlier battle, purchase or side task was done.
pub(crate) fn supports_action(guide: &Guide, step_id: &str, spatial: &Value) -> bool {
    let observation = &spatial["observation"];
    if observation["tracker"]["status"] != "tracked" {
        return false;
    }
    let mut at = Some(step_id);
    while let Some(id) = at {
        if observation["related_step_ids"]
            .as_array()
            .is_some_and(|ids| ids.iter().any(|v| v.as_str() == Some(id)))
        {
            return true;
        }
        at = guide.step(id).and_then(|s| s.parent_id.as_deref());
    }
    false
}

pub(crate) fn apply_location_advice(
    guide: &Guide,
    recommendation: &Recommendation,
    guidance: &mut gamebuddy_core::Guidance,
) {
    if recommendation.location_evidence.is_some()
        && guidance
            .now
            .as_ref()
            .is_none_or(|step| step.id != recommendation.step_id)
        && let Some(step) = guide.step(&recommendation.step_id)
    {
        guidance.now = Some(step.clone());
        guidance.target = Some(step.clone());
        guidance.next.clear();
        guidance.status = "ready".into();
        guidance.reason = "Advice for the currently observed area. Earlier walkthrough prerequisites remain unverified; they do not send you back to an area you have already left.".into();
    }
}

fn key(guide: &Guide) -> String {
    format!("next-action:{}", guide.id)
}
pub fn current(
    store: &Store,
    guide: &Guide,
    progress: &Progress,
    session: &str,
) -> Result<Option<Recommendation>> {
    Ok(store.get::<Recommendation>(&key(guide))?.filter(|r| {
        r.revision == guide.revision
            && r.epoch == progress.epoch
            && r.session == session
            && guide
                .step(&r.step_id)
                .is_some_and(|s| guide.step_enabled(s))
            && !gamebuddy_core::step_complete(guide, progress, &r.step_id)
    }))
}
pub fn publish(store: &Store, guide: &Guide, action: NextAction) -> Result<Value> {
    ensure!(
        action.expected_revision == guide.revision,
        "Outline changed; read get_game_context"
    );
    ensure!(
        !action.instructions.trim().is_empty() && action.instructions.len() <= 1600,
        "Give a short next instruction"
    );
    ensure!(
        gamebuddy_core::player_outline(guide)
            .iter()
            .any(|s| s.id == action.step_id)
            && !guide
                .steps
                .iter()
                .any(|s| s.parent_id.as_deref() == Some(&action.step_id)),
        "Choose an enabled player action, not a chapter"
    );
    let progress = store.progress(guide)?;
    ensure!(
        progress.epoch == action.epoch
            && !gamebuddy_core::step_complete(guide, &progress, &action.step_id),
        "Progress changed or task already complete. Current progress epoch is {}; read get_game_context before retrying.",
        progress.epoch
    );
    let guidance = guidance_from(
        guide,
        &progress,
        &PlaceTracker::default(),
        Some(&action.step_id),
    );
    let spatial = crate::spatial::context(store, guide)?;
    let location_supported = supports_action(guide, &action.step_id, &spatial);
    ensure!(
        location_supported
            || guidance
                .now
                .as_ref()
                .is_some_and(|s| s.id == action.step_id),
        "This action has unconfirmed prerequisites and no current source match supporting its location: {}. Do not infer the player's location from this checklist. Inspect the current screen and location_actions; choose an action supported there. Record explicit completion reports, and ask about a prerequisite only when it is actually needed for the current action. Current progress epoch is {}.",
        serde_json::to_string(&guidance.requirements)?,
        progress.epoch
    );
    let movement = action
        .screen_direction
        .map(|direction| -> Result<_> {
            use base64::Engine;
            let frame = screen::snapshot(store)?;
            let image = image::load_from_memory(
                &base64::engine::general_purpose::STANDARD.decode(&frame.jpeg)?,
            )?;
            if let Some(parsed) = movement(&action.instructions) {
                ensure!(
                    parsed == direction,
                    "Screen direction contradicts the movement instruction"
                );
            }
            Ok(ObservedMovement {
                direction,
                ticket: frame.ticket,
                captured_at_ms: frame.captured_at_ms,
                frame_signature: screen::frame_signature(&image),
            })
        })
        .transpose()?;
    let session = screen::status(store)?["session_id"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let recommendation = Recommendation {
        revision: guide.revision,
        epoch: progress.epoch,
        session,
        updated_at_ms: screen::now_ms(),
        step_id: action.step_id,
        instructions: action.instructions.trim().into(),
        movement,
        location_evidence: location_supported.then(|| {
            json!({
                "kind":"current_source_match_not_completion",
                "room_id":spatial["observation"]["tracker"]["room_id"],
                "observed_at_ms":spatial["observation"]["tracker"]["observed_at_ms"],
                "related_step_ids":spatial["observation"]["related_step_ids"]
            })
        }),
    };
    store.put_for_progress(guide, progress.epoch, &key(guide), &recommendation)?;
    Ok(
        json!({"recommendation":public(&recommendation),"navigation":super::context(store, guide, &progress, &SyncEstimate::default())?,"completion_changed":false}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_movement_works_without_a_named_place_and_expires_with_the_view() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let store = Store::open(dir.path())?;
        let guide =
            gamebuddy_core::import_markdown("Game", "Route", "# Visit church\nEnter the door.")?;
        store.save_guide(&guide)?;
        let ticket = screen::start(&store, "Game", true)?;
        let path = dir.path().join("screen.png");
        image::RgbImage::from_pixel(96, 72, image::Rgb([30, 160, 40])).save(&path)?;
        screen::publish(&store, &ticket, &path, "test")?;
        let action = |instructions: &str, screen_direction| NextAction {
            expected_revision: guide.revision,
            epoch: 0,
            step_id: "step-1".into(),
            instructions: instructions.into(),
            screen_direction,
        };
        assert!(
            publish(
                &store,
                &guide,
                action("Walk right to the door.", Some(ScreenDirection::Left))
            )
            .is_err()
        );
        let result = publish(
            &store,
            &guide,
            action("Enter the door beside you.", Some(ScreenDirection::Up)),
        )?;
        assert!(result["navigation"]["location"]["place_id"].is_null());
        assert_eq!(result["navigation"]["direction"]["direction"], "up");
        assert!(
            result["recommendation"]["movement"]
                .get("frame_signature")
                .is_none()
        );
        let progress = store.progress(&guide)?;
        let context = || super::super::context(&store, &guide, &progress, &SyncEstimate::default());
        let mut saved: Recommendation = store.get(&key(&guide))?.unwrap();
        saved.movement.as_mut().unwrap().captured_at_ms -= screen::FRESH_MS;
        store.put(&key(&guide), &saved)?;
        assert!(
            context()?["direction"].is_null(),
            "expired advice cannot revive through a fallback"
        );
        publish(
            &store,
            &guide,
            action("Walk right to the door.", Some(ScreenDirection::Right)),
        )?;
        image::RgbImage::from_pixel(96, 72, image::Rgb([180, 20, 10])).save(&path)?;
        screen::publish(&store, &ticket, &path, "changed_scene")?;
        assert!(context()?["direction"].is_null());
        publish(
            &store,
            &guide,
            action("Enter the door beside you.", Some(ScreenDirection::Up)),
        )?;
        screen::set_enabled(&store, &ticket.session, "Game", false)?;
        assert!(context()?["direction"].is_null());
        assert!(
            publish(
                &store,
                &guide,
                action("Enter the door beside you.", Some(ScreenDirection::Up))
            )
            .is_err()
        );
        screen::start(&store, "Game", true)?;
        assert!(context()?["direction"].is_null());
        assert!(progress.completed.is_empty());
        assert!(progress.reported_completed.is_empty());
        Ok(())
    }
    #[test]
    fn direction_tracks_shared_advice_and_hides_without_live_capture() {
        use gamebuddy_core::{Atlas, Place, PlaceBinding};
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let guide = gamebuddy_core::import_markdown(
            "Game",
            "Route",
            "# Leave town\nWalk right to the gate.",
        )
        .unwrap();
        store.save_guide(&guide).unwrap();
        let atlas = Atlas {
            guide_id: guide.id.clone(),
            guide_revision: guide.revision,
            places: vec![Place {
                id: "town".into(),
                label: "Town".into(),
                map_id: None,
                landmarks: "Gate".into(),
            }],
            bindings: vec![PlaceBinding {
                step_id: "step-1".into(),
                place_id: "town".into(),
                evidence: "Route starts at the gate".into(),
            }],
            ..Default::default()
        };
        super::super::save(&store, &guide, &atlas).unwrap();
        let ticket = screen::start(&store, "Game", true).unwrap();
        let frame = dir.path().join("frame.png");
        image::RgbImage::new(32, 32).save(&frame).unwrap();
        screen::publish(&store, &ticket, &frame, "test").unwrap();
        let mut location = PlaceTracker::for_session(ticket.session.clone());
        location.report("town", "step-1", 0, screen::now_ms(), "I am at the gate");
        store
            .put(&super::super::tracker_key(&guide), &location)
            .unwrap();
        let progress = store.progress(&guide).unwrap();
        let before =
            super::super::context(&store, &guide, &progress, &SyncEstimate::default()).unwrap();
        assert_eq!(before["direction"]["direction"], "right");
        let result = publish(
            &store,
            &guide,
            NextAction {
                expected_revision: guide.revision,
                epoch: progress.epoch,
                step_id: "step-1".into(),
                instructions: "Walk left to the other door.".into(),
                screen_direction: None,
            },
        )
        .unwrap();
        assert_eq!(result["navigation"]["direction"]["direction"], "left");
        assert_eq!(
            result["navigation"]["direction"]["step_id"],
            result["navigation"]["guidance"]["now"]["id"]
        );
        assert_eq!(
            serde_json::to_value(store.progress(&guide).unwrap()).unwrap(),
            serde_json::to_value(&progress).unwrap()
        );
        screen::set_enabled(&store, &ticket.session, "Game", false).unwrap();
        let paused =
            super::super::context(&store, &guide, &progress, &SyncEstimate::default()).unwrap();
        assert!(paused["direction"].is_null());
        screen::start(&store, "Game", true).unwrap();
        let replaced =
            super::super::context(&store, &guide, &progress, &SyncEstimate::default()).unwrap();
        assert!(replaced["direction"].is_null());
    }
    #[test]
    fn advice_rejects_stale_edits_and_unconfirmed_prerequisites() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let mut guide = gamebuddy_core::import_markdown(
            "Game",
            "Guide",
            "# Get key\nBuy it\n# Open door\nUse key",
        )
        .unwrap();
        guide.steps[1].prerequisites = vec!["step-1".into()];
        store.save_guide(&guide).unwrap();
        let action = |revision, epoch, step: &str| NextAction {
            expected_revision: revision,
            epoch,
            step_id: step.into(),
            instructions: "Take the next action.".into(),
            screen_direction: None,
        };
        assert!(publish(&store, &guide, action(guide.revision, 0, "step-2")).is_err());
        assert!(publish(&store, &guide, action(guide.revision + 1, 0, "step-1")).is_err());
        assert!(publish(&store, &guide, action(guide.revision, 1, "step-1")).is_err());
        assert!(store.get::<Recommendation>(&key(&guide)).unwrap().is_none());
        let stale_guide = guide.clone();
        guide.revision += 1;
        store.save_guide(&guide).unwrap();
        assert!(
            publish(
                &store,
                &stale_guide,
                action(stale_guide.revision, 0, "step-1")
            )
            .is_err()
        );
        assert!(store.get::<Recommendation>(&key(&guide)).unwrap().is_none());
    }
    #[test]
    fn advice_updates_shared_outline_without_claiming_progress_and_expires_on_reset() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let guide = gamebuddy_core::import_markdown(
            "Game",
            "Guide",
            "# Old advice\nWait here\n# New action\nTalk to the king",
        )
        .unwrap();
        store.save_guide(&guide).unwrap();
        let progress = store.progress(&guide).unwrap();
        let action = NextAction {
            expected_revision: guide.revision,
            epoch: progress.epoch,
            step_id: "step-2".into(),
            instructions: "Enter the castle and speak to the king.".into(),
            screen_direction: None,
        };
        let result = publish(&store, &guide, action).unwrap();
        assert_eq!(result["navigation"]["guidance"]["now"]["id"], "step-2");
        assert_eq!(
            result["navigation"]["cursor"]["instructions"],
            "Enter the castle and speak to the king."
        );
        assert_eq!(
            serde_json::to_value(store.progress(&guide).unwrap()).unwrap(),
            serde_json::to_value(&progress).unwrap()
        );
        assert!(
            current(&store, &guide, &progress, "new-session")
                .unwrap()
                .is_none()
        );
        let mut changed = progress;
        changed.epoch += 1;
        assert!(current(&store, &guide, &changed, "").unwrap().is_none());
    }
}
