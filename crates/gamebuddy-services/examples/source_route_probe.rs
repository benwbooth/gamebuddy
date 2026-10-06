//! Replay the entire authored route and rejected forward skips in a private
//! store. Simulated completion is never written to the player's save.
use anyhow::{Context, Result, ensure};
use gamebuddy_core::{Progress, outcomes::PlanningView, route::authored_route};
use gamebuddy_services::{
    Store,
    navigation::advice::{NextAction, publish},
};
use serde_json::json;

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let source = Store::open(args.next().context("Source data directory")?)?;
    let output = std::path::PathBuf::from(args.next().context("New private output directory")?);
    let stop_before = args.next();
    ensure!(
        output.starts_with("/tmp") && !output.exists(),
        "Use a new private /tmp directory"
    );
    let id: String = source.get("active_guide")?.context("Active guide")?;
    let mut guide = source.guide(&id)?;
    // This replay verifies the supplied source route independently of live
    // goals, world facts and semantic preparation.
    guide.goals.clear();
    let store = Store::open(&output)?;
    store.save_guide(&guide)?;
    store.put("active_guide", &id)?;
    if let Some(settings) = source.get::<serde_json::Value>("settings")? {
        store.put("settings", &settings)?;
    }
    let first = guide
        .steps
        .iter()
        .find(|s| s.parent_id.is_none() && guide.step_enabled(s))
        .context("Source root")?;
    let mut progress = Progress::default();
    progress.world.include_optional = true;
    let route = authored_route(
        &guide,
        &progress,
        &PlanningView::default(),
        &first.id,
        guide.steps.len() + 1,
    );
    ensure!(
        route.stop == "end" && !route.step_ids.is_empty(),
        "Source route ended with {}",
        route.stop
    );
    let mut rejected = 0;
    let mut snapshot = None;
    let started = std::time::Instant::now();
    for (index, id) in route.step_ids.iter().enumerate() {
        store.save_progress(&guide, &progress, "private_route_simulation")?;
        if stop_before.as_ref() == Some(id) {
            snapshot = Some(progress.clone());
        }
        let action = |id: &str| NextAction {
            expected_revision: guide.revision,
            epoch: progress.epoch,
            step_id: id.into(),
            instructions: guide
                .step(id)
                .unwrap()
                .instructions
                .chars()
                .take(1500)
                .collect(),
            screen_direction: None,
        };
        if let Some(later) = route.step_ids.get(index + 1) {
            ensure!(
                publish(&store, &guide, action(later)).is_err(),
                "Skipped {id} by advising {later}"
            );
            rejected += 1;
        }
        let result = publish(&store, &guide, action(id))
            .with_context(|| format!("Source action {index}: {id}"))?;
        ensure!(
            result["navigation"]["guidance"]["now"]["id"] == *id,
            "Now card left the source route"
        );
        progress.completed.insert(id.clone());
        progress.current_step = Some(id.clone());
        progress.epoch += 1;
    }
    store.save_progress(&guide, &progress, "private_route_simulation")?;
    let finished =
        gamebuddy_services::navigation::context(&store, &guide, &progress, &Default::default())?;
    ensure!(
        finished["guidance"]["now"].is_null(),
        "Route end restarted earlier tasks"
    );
    if let Some(mut saved) = snapshot {
        // Use a fresh epoch so the final replay recommendation cannot survive.
        saved.epoch = progress.epoch + 1;
        store.save_progress(&guide, &saved, "private_voice_fixture")?;
    } else {
        ensure!(
            stop_before.is_none(),
            "Requested action was not on the source route"
        );
    }
    println!(
        "{}",
        json!({"game":guide.game,"source":guide.sources.iter().map(|s| &s.title).collect::<Vec<_>>(),"source_actions":route.step_ids.len(),"forward_skips_rejected":rejected,"route_end_did_not_restart":true,"outcome_rules_used":false,"simulated_only":true,"elapsed_ms":started.elapsed().as_millis(),"voice_fixture_before":stop_before,"output":output})
    );
    Ok(())
}
