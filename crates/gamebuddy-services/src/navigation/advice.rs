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
