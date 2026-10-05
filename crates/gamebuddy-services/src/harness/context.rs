//! Small, current evidence for interactive coaching. The full atlas, outline
//! and geometry diagnostics remain available through their dedicated tools.
use super::*;
use gamebuddy_core::{PlaceTracker, Progress, guidance_from};

pub(super) fn navigation(value: &Value) -> Value {
    json!({
        "location":value["location"], "cursor":value["cursor"],
        "guidance":value["guidance"], "direction":value["direction"], "recommendation":value["recommendation"],
        "map":{
            "status":value["map"]["status"], "current_place":value["map"]["current_place"],
            "target_place":value["map"]["target_place"], "precision":value["map"]["precision"]
            ,"selected_map_id":value["map"]["selected_map_id"],
            "geometric_marker":value["map"]["geometric_marker"],
            "observed_at_ms":value["spatial"]["observation"]["tracker"]["observed_at_ms"],
            "image_tool":"get_location_map"
        },
        "spatial":{
            "preparation_status":value["spatial"]["preparation"]["status"],
            "tracker":value["spatial"]["observation"]["tracker"],
            "reference":value["spatial"]["observation"]["reference"]
        }
    })
}

pub(super) fn action_status(guide: &Guide, progress: &Progress, id: &str) -> Value {
    let complete = gamebuddy_core::step_complete(guide, progress, id);
    let leaf = !guide
        .steps
        .iter()
        .any(|s| s.parent_id.as_deref() == Some(id));
    let advice = guidance_from(guide, progress, &PlaceTracker::default(), Some(id));
    json!({"complete":complete,"eligible_next_action":!complete && leaf && advice.now.as_ref().is_some_and(|s|s.id==id),
        "unconfirmed_prerequisites":advice.requirements.iter().map(|s|&s.id).collect::<Vec<_>>()})
}

pub(super) fn nearby_actions(guide: &Guide, progress: &Progress, current: &Value) -> Value {
    let actions: Vec<_> = gamebuddy_core::player_outline(guide)
        .into_iter()
        .filter(|step| {
            !guide
                .steps
                .iter()
                .any(|s| s.parent_id.as_deref() == Some(&step.id))
        })
        .collect();
    let at = actions
        .iter()
        .position(|s| Some(s.id.as_str()) == current["id"].as_str())
        .unwrap_or(0);
    let start = at.saturating_sub(3);
    json!(
        actions
            .iter()
            .skip(start)
            .take(12)
            .filter_map(|s| guide.step(&s.id))
            .map(|step| { json!({"step":step,"status":action_status(guide,progress,&step.id)}) })
            .collect::<Vec<_>>()
    )
}

pub(super) fn outline(store: &Store, guide: &Guide, arguments: &Value) -> Result<Value> {
    let Some(ids) = arguments.get("step_ids") else {
        return Ok(
            json!({"guide":guide,"scope":"complete","history":store.revision_history(&guide.id)?}),
        );
    };
    let ids: Vec<String> = serde_json::from_value(ids.clone())?;
    ensure!(
        !ids.is_empty() && ids.len() <= 20,
        "Request between one and twenty step IDs, or omit step_ids for the complete outline"
    );
    let mut selected = std::collections::HashSet::new();
    for id in ids {
        let mut cursor = Some(id.as_str());
        while let Some(id) = cursor {
            let step = guide.step(id).context("Unknown outline step")?;
            if !selected.insert(step.id.clone()) {
                break;
            }
            cursor = step.parent_id.as_deref();
        }
    }
    let mut excerpt = guide.clone();
    excerpt.steps.retain(|s| selected.contains(&s.id));
    Ok(
        json!({"guide":excerpt,"scope":"selected_steps_and_ancestors","total_steps":guide.steps.len(),
        "instruction":"This is a partial view. Unlisted steps still exist. Use complete step objects for a targeted edit; do not remove or replace unlisted nodes."}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nearby_actions_keep_source_evidence_and_prerequisite_guards() {
        let mut guide = gamebuddy_core::import_markdown(
            "Game",
            "Guide",
            "# Get key\nBuy key\n# Open door\nUse key",
        )
        .unwrap();
        guide.steps[1].prerequisites = vec!["step-1".into()];
        let actions = nearby_actions(&guide, &Progress::default(), &json!({"id":"step-1"}));
        assert_eq!(actions[0]["status"]["eligible_next_action"], true);
        assert_eq!(actions[1]["status"]["eligible_next_action"], false);
        assert_eq!(
            actions[1]["status"]["unconfirmed_prerequisites"][0],
            "step-1"
        );
        assert_eq!(
            actions[1]["step"],
            serde_json::to_value(&guide.steps[1]).unwrap()
        );
    }
    #[test]
    fn compact_context_keeps_current_evidence_without_duplicating_the_atlas() {
        let value = json!({"location":{"status":"uncertain"},"guidance":{"now":{"id":"key"}},
            "map":{"current_place":{"label":"Town"},"places":["large atlas"],"spatial":{"dense":"geometry"}},
            "spatial":{"preparation":{"status":"ready"},"observation":{"tracker":{"status":"lost"}}}});
        let compact = navigation(&value);
        assert_eq!(compact["location"]["status"], "uncertain");
        assert_eq!(compact["map"]["current_place"]["label"], "Town");
        assert_eq!(compact["spatial"]["tracker"]["status"], "lost");
        assert!(compact["map"].get("places").is_none());
        assert!(compact["map"].get("spatial").is_none());
    }
    #[test]
    fn targeted_outline_keeps_complete_objects_and_ancestors() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let guide = gamebuddy_core::import_markdown(
            "Game",
            "Guide",
            "# Town\nExplore\n## Shop\nBuy key\n## Church\nVisit guru",
        )
        .unwrap();
        store.save_guide(&guide).unwrap();
        let result = outline(&store, &guide, &json!({"step_ids":["step-2"]})).unwrap();
        assert_eq!(result["scope"], "selected_steps_and_ancestors");
        assert_eq!(result["total_steps"], 3);
        assert_eq!(result["guide"]["steps"].as_array().unwrap().len(), 2);
        assert_eq!(
            result["guide"]["steps"][1],
            serde_json::to_value(&guide.steps[1]).unwrap()
        );
        assert!(outline(&store, &guide, &json!({"step_ids":["missing"]})).is_err());
    }
}
