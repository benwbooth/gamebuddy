//! Exercise the configured reasoning provider against a private copy of an
//! existing app-built guide. Never changes the player's live progress.
use anyhow::{Context, Result, ensure};
use gamebuddy_core::SyncEstimate;
use gamebuddy_services::{HttpProvider, Settings, Store, harness, navigation};
use std::path::PathBuf;

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let id = args.next().context("guide ID")?;
    let place = args.next().context("expected place ID")?;
    let utterance = args.next().context("player statement")?;
    let output = PathBuf::from(args.next().context("new output directory")?);
    ensure!(!output.exists(), "Use a new output directory");
    let source = Store::open(gamebuddy_services::data_dir()?)?;
    let guide = source.guide(&id)?;
    let atlas = navigation::atlas(&source, &guide)?;
    let settings: Settings = source.get("settings")?.unwrap_or_default();
    let store = Store::open(output.join("data"))?;
    store.save_guide(&guide)?;
    store.put("active_guide", &guide.id)?;
    store.put("settings", &settings)?;
    store.put(&format!("atlas:{}", guide.id), &atlas)?;
    if let Some(index) = source.index(&guide)? {
        store.save_index(&index)?;
    }
    std::fs::create_dir_all(store.root.join("maps"))?;
    for map in &atlas.maps {
        let path = navigation::cached_map_path(&source, map);
        if path.exists() {
            std::fs::copy(path, navigation::cached_map_path(&store, map))?;
        }
    }
    println!(
        "Calling {:?} with a player location report in an isolated store",
        settings.reasoning.kind
    );
    let response = harness::coach(&HttpProvider::new(settings.reasoning)?, &store, &utterance)?;
    let state = navigation::context(
        &store,
        &guide,
        &store.progress(&guide)?,
        &SyncEstimate::default(),
    )?;
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&state)?,
    )?;
    println!("Agent: {response}");
    ensure!(
        state["location"]["place_id"] == place && state["location"]["status"] == "reported",
        "Agent did not persist the reported location"
    );
    ensure!(
        state["cursor"]["id"].is_string(),
        "Outline cursor was not updated"
    );
    println!("Report and outline cursor verified; registering map landmarks");
    navigation::placement::prepare(&store, &guide, &place)?;
    let state = navigation::context(
        &store,
        &guide,
        &store.progress(&guide)?,
        &SyncEstimate::default(),
    )?;
    std::fs::write(
        output.join("placement.json"),
        serde_json::to_vec_pretty(&state)?,
    )?;
    let placement =
        navigation::placement::get(&store, &guide, &place)?.context("Placement result")?;
    println!("{}", serde_json::to_string_pretty(&placement)?);
    ensure!(
        placement.status == "ready",
        "No validated area registration was produced"
    );
    println!("GAMEBUDDY_LOCATION_PROVIDER_PASS {}", output.display());
    Ok(())
}
