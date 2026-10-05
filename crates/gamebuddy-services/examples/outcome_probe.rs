//! Real provider/source compilation and local replanning in a private database.
use anyhow::{Context, Result};
use gamebuddy_core::{PlaceTracker, guidance_with_outcomes};
use gamebuddy_services::{HttpProvider, Settings, Store, outcomes};
use serde_json::json;
fn main() -> Result<()> {
    let root = std::env::args()
        .nth(1)
        .context("Private data directory required")?;
    let store = Store::open(root)?;
    let id: String = store
        .get("active_guide")?
        .context("Active guide required")?;
    let guide = store.guide(&id)?;
    let settings: Settings = store.get("settings")?.unwrap_or_default();
    let mut config = settings.planning_config();
    if let Ok(model) = std::env::var("GAMEBUDDY_PROBE_MODEL") {
        config.model = model;
    }
    if let Ok(effort) = std::env::var("GAMEBUDDY_PROBE_EFFORT") {
        config.reasoning_effort = effort;
    }
    let p = outcomes::compile(
        &store,
        &guide,
        &HttpProvider::new(config.clone())?,
        &config.model,
    )?;
    let progress = store.progress(&guide)?;
    let started = std::time::Instant::now();
    let advice =
        guidance_with_outcomes(&guide, &progress, &PlaceTracker::default(), None, &p.model);
    let local_us = started.elapsed().as_micros();
    println!(
        "{}",
        json!({"model":p.provider_model,"compile_ms":p.elapsed_ms,"local_replan_us":local_us,"facts":progress.world.facts,"guidance":advice,"planning":p.model})
    );
    if let Some(report) = std::env::args().nth(2) {
        let at = std::time::Instant::now();
        let answer = outcomes::quick_report(&store, &guide.game, &report)?;
        println!(
            "{}",
            json!({"local_report_ms":at.elapsed().as_millis(),"answer":answer,"progress":store.progress(&guide)?})
        );
    }
    Ok(())
}
