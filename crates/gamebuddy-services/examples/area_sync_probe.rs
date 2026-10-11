//! Feed a held area-map match to sync_area and show the route cursor:
//! area_sync_probe <data directory> <map title>
fn main() -> anyhow::Result<()> {
    use anyhow::Context;
    let mut args = std::env::args().skip(1);
    let store = gamebuddy_services::Store::open(args.next().context("Data directory")?)?;
    let title = args.next().context("Map title")?;
    let id: String = store.get("active_guide")?.context("No active guide")?;
    let guide = store.guide(&id)?;
    let atlas = gamebuddy_services::navigation::atlas(&store, &guide)?;
    let map = atlas
        .maps
        .iter()
        .find(|m| m.title == title)
        .context("Unknown map")?;
    let session = gamebuddy_services::screen::status(&store)?["session_id"].clone();
    let key = format!("world-observation:{id}");
    let mut observation: serde_json::Value = store.get(&key)?.unwrap_or_default();
    observation["tracker"]["session"] = session;
    observation["map_registration"]["map_id"] = serde_json::json!(map.id);
    store.put(&key, &observation)?;
    println!("before: {:?}", store.progress(&guide)?.current_step);
    for at in [0, 5_000] {
        gamebuddy_services::navigation::planning::sync_area(&store, &guide, at)?;
    }
    println!("after: {:?}", store.progress(&guide)?.current_step);
    Ok(())
}
