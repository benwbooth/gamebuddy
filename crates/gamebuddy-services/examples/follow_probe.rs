//! Show what following the latest spatial observation would do for a
//! profile: follow_probe <data directory>
use anyhow::{Context, Result};
use gamebuddy_services::{Store, navigation::follow};

fn main() -> Result<()> {
    let store = Store::open(std::env::args().nth(1).context("Data directory")?)?;
    let id: String = store.get("active_guide")?.context("No active guide")?;
    let guide = store.guide(&id)?;
    let observation: serde_json::Value = store
        .get(&format!("world-observation:{id}"))?
        .context("No spatial observation")?;
    let matches = follow::video_matches(&observation);
    let map = follow::map_title(&store, &guide, &observation["map_registration"]);
    let epoch = store.progress(&guide)?.epoch;
    println!(
        "before: {:?} matches: {matches:?} map: {map:?}",
        store.progress(&guide)?.current_step
    );
    follow::observed(&store, &guide, epoch, &matches, map.as_deref(), 1_000)?;
    let result = follow::observed(&store, &guide, epoch, &matches, map.as_deref(), 5_000)?;
    println!("result: {}", serde_json::to_string(&result)?);
    println!("after: {:?}", store.progress(&guide)?.current_step);
    Ok(())
}
