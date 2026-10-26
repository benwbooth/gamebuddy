//! The fused location estimate, the map's dot and the arrow for the active
//! guide (or the one given): estimate_probe <data dir> [guide id]
fn main() -> anyhow::Result<()> {
    use anyhow::Context;
    let store =
        gamebuddy_services::Store::open(std::env::args().nth(1).context("Data directory")?)?;
    let id: String = match std::env::args().nth(2) {
        Some(id) => id,
        None => store.get("active_guide")?.context("No active guide")?,
    };
    let guide = store.guide(&id)?;
    let progress = store.progress(&guide)?;
    let started = std::time::Instant::now();
    let value =
        gamebuddy_services::navigation::context(&store, &guide, &progress, &Default::default())?;
    println!(
        "{}",
        serde_json::json!({
            "game": guide.game,
            "context_ms": started.elapsed().as_millis(),
            "step": value["guidance"]["now"]["title"],
            "location": {"status": value["location"]["status"], "place_id": value["location"]["place_id"]},
            "location_estimate": value["location_estimate"],
            "map": {"selected_map_id": value["map"]["selected_map_id"], "position": value["map"]["position"],
                    "estimate_status": value["map"]["estimate_status"], "current_place": value["map"]["current_place"]["label"],
                    "target_place": value["map"]["target_place"]["label"]},
            "direction": value["direction"],
        })
    );
    Ok(())
}
