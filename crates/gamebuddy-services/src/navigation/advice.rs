//! One persisted recommendation shared by conversation, the outline and Now card.
use crate::{Store, screen};
use anyhow::{Result, ensure};
use gamebuddy_core::{Guide, PlaceTracker, Progress, SyncEstimate, guidance_from};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NextAction {
    pub expected_revision: u64,
    pub epoch: u64,
    pub step_id: String,
    /// One immediate action. For supported screen-relative movement, start with
    /// "Walk right", "Climb up", etc. This drives the OSD arrow only with fresh
    /// matching place evidence. Never guess a direction or add it to non-movement.
    pub instructions: String,
}
#[derive(Serialize, Deserialize)]
pub struct Recommendation {
    pub revision: u64,
    pub epoch: u64,
    pub session: String,
    pub updated_at_ms: u64,
    pub step_id: String,
    pub instructions: String,
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
        "Progress changed or task already complete; read get_game_context"
    );
    let guidance = guidance_from(
        guide,
        &progress,
        &PlaceTracker::default(),
        Some(&action.step_id),
    );
    ensure!(
        guidance
            .now
            .as_ref()
            .is_some_and(|s| s.id == action.step_id),
        "This action has unconfirmed prerequisites; resolve them or edit the plan first"
    );
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
    };
    store.put_for_progress(guide, progress.epoch, &key(guide), &recommendation)?;
    Ok(
        json!({"recommendation":recommendation,"navigation":super::context(store, guide, &progress, &SyncEstimate::default())?,"completion_changed":false}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
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
