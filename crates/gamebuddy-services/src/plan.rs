//! Revisioned player plans. Model edits never turn advice into verified progress.
use crate::Store;
use anyhow::{Result, ensure};
use gamebuddy_core::{Guide, PlayerGoal, ReferencePreference, Step};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanEdit {
    pub expected_revision: u64,
    pub summary: String,
    #[serde(default)]
    pub upsert_steps: Vec<Step>,
    #[serde(default)]
    pub remove_step_ids: Vec<String>,
    pub goals: Option<Vec<PlayerGoal>>,
    pub reference_preferences: Option<Vec<ReferencePreference>>,
}

pub fn edit_plan(store: &Store, id: &str, edit: PlanEdit) -> Result<Guide> {
    let mut guide = store.guide(id)?;
    let original = guide.clone();
    ensure!(
        guide.revision == edit.expected_revision,
        "Plan changed; read the latest outline before editing"
    );
    ensure!(!edit.summary.trim().is_empty(), "Explain the plan change");
    let removals: HashSet<_> = edit.remove_step_ids.iter().collect();
    ensure!(
        removals.len() == edit.remove_step_ids.len(),
        "Duplicate removal IDs"
    );
    ensure!(
        removals.iter().all(|id| guide.step(id).is_some()),
        "Cannot remove an unknown step"
    );
    let mut upserts = HashSet::new();
    for step in &edit.upsert_steps {
        ensure!(
            upserts.insert(&step.id) && !removals.contains(&step.id),
            "Conflicting step edits"
        );
    }
    guide.steps.retain(|s| !removals.contains(&s.id));
    for step in edit.upsert_steps {
        if let Some(existing) = guide.steps.iter_mut().find(|s| s.id == step.id) {
            *existing = step;
        } else {
            guide.steps.push(step);
        }
    }
    if let Some(goals) = edit.goals {
        guide.goals = goals;
    }
    if let Some(preferences) = edit.reference_preferences {
        guide.reference_preferences = preferences;
    }
    if guide == original {
        return Ok(guide);
    }
    guide.revision += 1;
    guide.validate()?;
    store.revise_guide(&guide, edit.expected_revision, &edit.summary)?;
    Ok(guide)
}

pub fn restore_plan(
    store: &Store,
    id: &str,
    expected_revision: u64,
    restore_revision: u64,
) -> Result<Guide> {
    ensure!(
        restore_revision < expected_revision,
        "Choose an earlier revision"
    );
    let mut guide = store.guide_revision(id, restore_revision)?;
    guide.revision = expected_revision + 1;
    store.revise_guide(
        &guide,
        expected_revision,
        &format!("Restore outline revision {restore_revision}"),
    )?;
    Ok(guide)
}

/// Merge a separately imported reference with collision-free IDs. Keep its
/// structure available to the coach; a subsequent edit can reconcile routes.
pub fn attach_guide(
    store: &Store,
    id: &str,
    expected_revision: u64,
    imported: &Guide,
) -> Result<Guide> {
    let mut guide = store.guide(id)?;
    ensure!(
        guide.game.eq_ignore_ascii_case(&imported.game),
        "Reference belongs to a different game"
    );
    let prefix = format!("ref-{}-", uuid::Uuid::new_v4());
    let remap = |id: &str| format!("{prefix}{id}");
    for source in &imported.sources {
        let mut source = source.clone();
        source.id = remap(&source.id);
        guide.sources.push(source);
    }
    for step in &imported.steps {
        let mut step = step.clone();
        step.id = remap(&step.id);
        step.parent_id = step.parent_id.map(|id| remap(&id));
        step.prerequisites = step.prerequisites.iter().map(|id| remap(id)).collect();
        step.next = step.next.iter().map(|id| remap(id)).collect();
        for anchor in &mut step.anchors {
            anchor.source_id = remap(&anchor.source_id);
        }
        guide.steps.push(step);
    }
    guide
        .coverage_notes
        .extend(imported.coverage_notes.iter().cloned());
    guide.revision = expected_revision + 1;
    store.revise_guide(
        &guide,
        expected_revision,
        &format!("Add reference: {}", imported.title),
    )?;
    Ok(guide)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gamebuddy_core::{Progress, import_markdown};
    fn edit(revision: u64) -> PlanEdit {
        PlanEdit {
            expected_revision: revision,
            summary: "Player goal change".into(),
            upsert_steps: vec![],
            remove_step_ids: vec![],
            goals: None,
            reference_preferences: None,
        }
    }
    #[test]
    fn edits_are_atomic_revision_checked_and_preserve_only_unchanged_evidence() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path()).unwrap();
        let guide = import_markdown(
            "Faxanadu",
            "Route",
            "# Town\nBuy equipment\n# Tower\nFind the key",
        )
        .unwrap();
        store.save_guide(&guide).unwrap();
        store
            .save_progress(
                &guide,
                &Progress {
                    current_step: Some("step-1".into()),
                    completed: HashSet::from(["step-1".into(), "step-2".into()]),
                    ..Default::default()
                },
                "user_confirmation",
            )
            .unwrap();
        let mut change = edit(1);
        change.goals = Some(vec![PlayerGoal {
            id: "finish".into(),
            description: "Finish without optional collectibles".into(),
            step_ids: vec!["step-2".into()],
        }]);
        let revised = edit_plan(&store, &guide.id, change).unwrap();
        assert_eq!(store.progress(&revised).unwrap().completed.len(), 2);
        assert!(edit_plan(&store, &guide.id, edit(1)).is_err());
        let mut invalid = edit(2);
        invalid.remove_step_ids.push("step-2".into());
        assert!(edit_plan(&store, &guide.id, invalid).is_err());
        assert_eq!(store.guide(&guide.id).unwrap().revision, 2);
        let mut changed_step = guide.steps[1].clone();
        changed_step.instructions = "Get two different keys".into();
        let mut change = edit(2);
        change.upsert_steps.push(changed_step);
        let revised = edit_plan(&store, &guide.id, change).unwrap();
        assert_eq!(
            store.progress(&revised).unwrap().completed,
            HashSet::from(["step-1".into()])
        );
        let restored = restore_plan(&store, &guide.id, 3, 1).unwrap();
        assert_eq!(restored.steps, guide.steps);
        assert!(restored.goals.is_empty());
        assert_eq!(restored.revision, 4);
        assert_eq!(store.guide_revision(&guide.id, 2).unwrap().goals.len(), 1);
    }
    #[test]
    fn attaching_and_disabling_sources_retains_provenance() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path()).unwrap();
        let guide = import_markdown("Faxanadu", "Original", "# Town\nGo east").unwrap();
        store.save_guide(&guide).unwrap();
        let other = import_markdown("Faxanadu", "Preferred", "# Town\nGo west").unwrap();
        let combined = attach_guide(&store, &guide.id, 1, &other).unwrap();
        assert_eq!(combined.sources.len(), 2);
        assert_eq!(combined.steps.len(), 2);
        assert_ne!(combined.steps[0].id, combined.steps[1].id);
        let mut change = edit(2);
        change.reference_preferences = Some(vec![ReferencePreference {
            source_id: "text-1".into(),
            enabled: false,
            preference: "Use the new reference".into(),
        }]);
        let revised = edit_plan(&store, &guide.id, change).unwrap();
        assert!(!revised.step_enabled(&revised.steps[0]));
        assert!(revised.step_enabled(&revised.steps[1]));
        assert_eq!(revised.sources[0], guide.sources[0]);
    }
}
