//! Build level maps for the active guide: level_maps_build <data directory>
fn main() -> anyhow::Result<()> {
    use anyhow::Context;
    let root = std::env::args().nth(1).context("Data directory")?;
    let store = gamebuddy_services::Store::open(root)?;
    let id: String = store.get("active_guide")?.context("No active guide")?;
    let guide = store.guide(&id)?;
    let added = gamebuddy_services::level_maps::build(&store, &guide, &|m| eprintln!("{m}"))?;
    let atlas = gamebuddy_services::navigation::atlas(&store, &guide)?;
    for map in &atlas.maps {
        println!(
            "{} | {} | {}",
            map.id,
            map.title,
            gamebuddy_services::navigation::cached_map_path(&store, map).display()
        );
    }
    println!(
        "{added} maps added; {} places on a map",
        atlas.places.iter().filter(|p| p.map_id.is_some()).count()
    );
    Ok(())
}
