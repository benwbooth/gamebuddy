//! Which map the map widget would select right now: map_selection_probe <data dir>
fn main() -> anyhow::Result<()> {
    use anyhow::Context;
    let store =
        gamebuddy_services::Store::open(std::env::args().nth(1).context("Data directory")?)?;
    let id: String = store.get("active_guide")?.context("No active guide")?;
    let guide = store.guide(&id)?;
    let progress = store.progress(&guide)?;
    let value =
        gamebuddy_services::navigation::context(&store, &guide, &progress, &Default::default())?;
    let map = &value["map"];
    println!(
        "step={} selected={} current_place={} target_place={} maps={}",
        value["guidance"]["now"]["title"],
        map["selected_map_id"],
        map["current_place"]["label"],
        map["target_place"]["label"],
        map["maps"].as_array().map_or(0, Vec::len)
    );
    Ok(())
}
