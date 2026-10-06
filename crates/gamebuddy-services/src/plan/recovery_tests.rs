use super::*;
use gamebuddy_core::{PlayerGoal, import_markdown};

#[test]
fn fulfilled_side_goal_recovers_without_losing_facts_or_unfinished_goals() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let store = Store::open(dir.path())?;
    let mut guide = import_markdown(
        "Game",
        "Route",
        "# Shop\nBuy boots.\n# Road\nWalk right.\n# Tower\nEnter the tower.",
    )?;
    guide.goals = vec![
        PlayerGoal {
            id: "boots".into(),
            description: "Buy replacement boots".into(),
            step_ids: vec!["step-1".into()],
        },
        PlayerGoal {
            id: "tower".into(),
            description: "Reach the tower".into(),
            step_ids: vec!["step-3".into()],
        },
    ];
    store.save_guide(&guide)?;
    let mut progress = Progress {
        current_step: Some("step-1".into()),
        ..Default::default()
    };
    progress.reported_completed.insert("step-1".into());
    store.save_progress(&guide, &progress, "fixture")?;
    let widgets: Vec<gamebuddy_core::Widget> = serde_json::from_value(
        json!([{"id":"notes","title":"Notes","content":{"kind":"text","body":"Keep this clue"}}]),
    )?;
    store.put(
        &format!("widgets:{}:{}", guide.id, guide.revision),
        &widgets,
    )?;
    let mut tracker = gamebuddy_core::PlaceTracker::default();
    tracker.report_place("shop", progress.epoch, 1, "I am in the shop");
    store.put(&crate::navigation::tracker_key(&guide), &tracker)?;
    let updated = retire_completed(&store, &guide)?.expect("side goal retired");
    assert_eq!(
        json!(store.get::<Vec<gamebuddy_core::Widget>>(&format!(
            "widgets:{}:{}",
            updated.id, updated.revision
        ))?),
        json!(widgets)
    );
    assert_eq!(
        store
            .get::<gamebuddy_core::PlaceTracker>(&crate::navigation::tracker_key(&updated))?
            .unwrap()
            .place_id,
        tracker.place_id
    );
    assert_eq!(
        store
            .get::<gamebuddy_core::PlaceTracker>(&crate::navigation::tracker_key(&guide))?
            .unwrap()
            .epoch,
        progress.epoch,
        "repair preserves the previous revision's location record"
    );
    assert_eq!(updated.goals.len(), 1);
    assert_eq!(updated.goals[0].id, "tower");
    assert_eq!(updated.steps, guide.steps);
    let after = store.progress(&updated)?;
    assert_eq!(after.reported_completed, progress.reported_completed);
    assert_eq!(after.world, progress.world);
    assert_eq!(after.epoch, progress.epoch + 1);
    let advice = gamebuddy_core::guidance(&updated, &after, &Default::default());
    assert!(advice.now.is_some());
    assert!(
        retire_completed(&store, &updated)?.is_none(),
        "unfinished goal is never retired"
    );
    assert_eq!(store.revision_history(&guide.id)?.len(), 2);
    store.journal_sync(&updated)?;
    assert_eq!(
        store
            .journal_entries(
                &store.journal_active(&updated)?,
                "Completed goal",
                None,
                false,
                30
            )?
            .len(),
        1
    );
    assert!(
        store
            .recover_plan(&guide, progress.epoch, guide.clone(), None, false, "stale")
            .is_err()
    );
    let mut invalid = updated.clone();
    invalid.steps[0].instructions = "Invented replacement".into();
    assert!(
        store
            .recover_plan(
                &updated,
                after.epoch,
                invalid,
                None,
                false,
                "rewrite history"
            )
            .is_err()
    );
    assert_eq!(store.guide(&guide.id)?, updated);
    Ok(())
}

#[test]
fn terminal_goal_and_stale_background_work_do_not_restart_the_game() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let store = Store::open(dir.path())?;
    let mut guide = import_markdown("Game", "Ending", "# Finale\nFinish the game.")?;
    guide.goals = vec![PlayerGoal {
        id: "end".into(),
        description: "Finish".into(),
        step_ids: vec!["step-1".into()],
    }];
    store.save_guide(&guide)?;
    let progress = store.report_complete(&guide, 0, "step-1", "I finished the game")?;
    assert!(retire_completed(&store, &guide)?.is_none());
    assert!(
        store
            .recover_plan(&guide, 0, guide.clone(), None, false, "old capture")
            .is_err()
    );
    assert_eq!(json!(store.progress(&guide)?), json!(progress));
    struct Never;
    impl ReasoningProvider for Never {
        fn complete(&self, _: ModelRequest) -> Result<String> {
            panic!("terminal goal must not spend a model request")
        }
    }
    assert_eq!(repair(&store, &guide, &Never)?["status"], "complete");
    Ok(())
}

#[test]
fn unsupported_disabled_goal_yields_one_specific_question_without_mutation() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let store = Store::open(dir.path())?;
    let mut guide = import_markdown("Game", "Route", "# Gate\nEnter the gate.")?;
    guide
        .reference_preferences
        .push(gamebuddy_core::ReferencePreference {
            source_id: guide.sources[0].id.clone(),
            enabled: false,
            preference: String::new(),
        });
    guide.goals.push(PlayerGoal {
        id: "gate".into(),
        description: "Enter gate".into(),
        step_ids: vec!["step-1".into()],
    });
    store.save_guide(&guide)?;
    struct Ask;
    impl ReasoningProvider for Ask {
        fn complete(&self, request: ModelRequest) -> Result<String> {
            assert!(request.prompt.contains("Gate"));
            Ok(json!({"summary":"The goal uses a disabled source.","steps":[],"rules":null,"evidence_step_ids":[],"question":"Which enabled reference should I use for the gate route?"}).to_string())
        }
    }
    assert_eq!(repair(&store, &guide, &Ask)?["status"], "needs_information");
    assert_eq!(
        context(&store, &guide)?["status"],
        "needs_information",
        "same input does not reschedule repeatedly"
    );
    assert_eq!(store.guide(&guide.id)?, guide);
    assert_eq!(store.progress(&guide)?.epoch, 0);
    Ok(())
}

#[test]
fn a_satisfied_inventory_goal_resumes_without_forgetting_the_inventory() -> Result<()> {
    use gamebuddy_core::outcomes::{Condition, KnownFact, OutcomeGoal};
    let dir = tempfile::tempdir()?;
    let store = Store::open(dir.path())?;
    let guide = import_markdown("Game", "Route", "# Shop\nBuy boots.\n# Exit\nWalk right.")?;
    store.save_guide(&guide)?;
    let mut progress = Progress {
        current_step: Some("step-1".into()),
        ..Default::default()
    };
    progress.completed.insert("step-1".into());
    progress.world.facts.insert(
        "boots".into(),
        KnownFact {
            value: true,
            evidence: "I bought the boots".into(),
            source: "player_report".into(),
            step_id: None,
            at_ms: 1,
        },
    );
    progress.world.goal = Some(OutcomeGoal {
        description: "Get boots".into(),
        satisfied_when: Condition::Fact {
            id: "boots".into(),
            value: true,
        },
    });
    store.save_progress(&guide, &progress, "fixture")?;
    let updated = retire_completed(&store, &guide)?.expect("satisfied outcome retired");
    let after = store.progress(&updated)?;
    assert!(after.world.goal.is_none());
    assert_eq!(after.world.facts, progress.world.facts);
    assert_eq!(after.completed, progress.completed);
    assert_eq!(
        gamebuddy_core::guidance(&updated, &after, &Default::default())
            .now
            .unwrap()
            .id,
        "step-2"
    );
    Ok(())
}

#[test]
fn source_backed_rule_repair_unblocks_guidance_without_inventing_completion() -> Result<()> {
    use gamebuddy_core::outcomes::{ActionRule, Condition, FactDefinition};
    let dir = tempfile::tempdir()?;
    let store = Store::open(dir.path())?;
    let mut guide = import_markdown(
        "Game",
        "Route",
        "# Chest\nThis optional chest contains a key for a different room.\n# Open gate\nWalk right through the open gate. No key is needed.",
    )?;
    guide.goals.push(PlayerGoal {
        id: "gate".into(),
        description: "Pass the open gate".into(),
        step_ids: vec!["step-2".into()],
    });
    store.save_guide(&guide)?;
    let rules = OutcomeModel {
        facts: vec![FactDefinition {
            id: "key".into(),
            description: "Has the key".into(),
            possession_names: vec![],
            evidence_step_ids: vec!["step-1".into()],
        }],
        actions: vec![ActionRule {
            step_id: "step-2".into(),
            requires: Some(Condition::Fact {
                id: "key".into(),
                value: true,
            }),
            reason: "Incorrect gate requirement".into(),
            evidence_step_ids: vec!["step-2".into()],
            ..Default::default()
        }],
        ..Default::default()
    };
    store.put(
        &format!("outcomes:{}", guide.id),
        &outcomes::Prepared {
            guide_revision: guide.revision,
            reviewed_scopes: vec![],
            model: rules,
            provider_model: "fixture".into(),
            elapsed_ms: 0,
        },
    )?;
    assert_eq!(context(&store, &guide)?["guidance_status"], "blocked");
    struct Fix;
    impl ReasoningProvider for Fix {
        fn complete(&self, request: ModelRequest) -> Result<String> {
            assert!(request.prompt.contains("No key is needed"));
            Ok(json!({"summary":"Removed an incorrect key requirement: the source explicitly says the gate is open.","steps":[],"rules":{"facts":[],"actions":[{"step_id":"step-2","requires":{"op":"always"},"reason":"The gate is open; the key belongs to another room.","evidence_step_ids":["step-2"]}],"bypasses":[]},"evidence_step_ids":["step-2"],"question":null}).to_string())
        }
    }
    assert_eq!(repair(&store, &guide, &Fix)?["status"], "repaired");
    let revised = store.guide(&guide.id)?;
    let progress = store.progress(&revised)?;
    assert_eq!(revised.goals, guide.goals);
    assert_eq!(revised.steps, guide.steps);
    assert!(progress.world.facts.is_empty());
    assert!(progress.completed.is_empty());
    assert!(progress.reported_completed.is_empty());
    assert_eq!(
        gamebuddy_core::guidance_with_outcomes(
            &revised,
            &progress,
            &Default::default(),
            None,
            &model(&store, &revised)?
        )
        .now
        .unwrap()
        .id,
        "step-2"
    );
    Ok(())
}

#[test]
fn periodic_repair_retires_a_completed_goal_without_waiting_for_a_player_request() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let store = Store::open(dir.path())?;
    let mut guide = import_markdown("Game", "Route", "# Shop\nBuy boots.\n# Road\nWalk right.")?;
    guide.goals.push(PlayerGoal {
        id: "boots".into(),
        description: "Get boots".into(),
        step_ids: vec!["step-1".into()],
    });
    store.save_guide(&guide)?;
    store.report_complete(&guide, 0, "step-1", "I bought them")?;
    schedule(&store, &guide, false);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while store.guide(&guide.id)?.revision == guide.revision {
        ensure!(
            std::time::Instant::now() < deadline,
            "Background repair did not finish"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let revised = store.guide(&guide.id)?;
    assert!(revised.goals.is_empty());
    assert!(
        store
            .progress(&revised)?
            .reported_completed
            .contains("step-1")
    );
    Ok(())
}
