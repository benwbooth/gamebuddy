//! Place a stretch of the walkthrough video on the area maps and keep it as
//! the run's path through the rooms (`navigation::room_graph`):
//! video_trace <data dir> <start s> <end s> <every ms>
//! Everything not traced yet, as GameBuddy does in the background: video_trace <data dir> all
//! Then the way out of a screen: video_trace <data dir> route <map title> <x> <y> <toward map title>
use anyhow::Context;

fn main() -> anyhow::Result<()> {
    use gamebuddy_services::navigation::{atlas, room_graph};
    let a: Vec<String> = std::env::args().collect();
    let store = gamebuddy_services::Store::open(a.get(1).context("Data directory")?)?;
    let id: String = store.get("active_guide")?.context("No active guide")?;
    let guide = store.guide(&id)?;
    if a.get(2).map(String::as_str) == Some("route") {
        let atlas = atlas(&store, &guide)?;
        let map = |title: &str| {
            atlas
                .maps
                .iter()
                .find(|m| m.title == title)
                .map(|m| m.id.clone())
                .context("Unknown map")
        };
        let at = [a[4].parse()?, a[5].parse()?];
        let moves = room_graph::way_out(&store, &guide, &map(&a[3])?, at, &map(&a[6])?)?;
        println!("{moves:?}");
        println!(
            "{}",
            moves.map(|m| room_graph::words(&m)).unwrap_or_default()
        );
        return Ok(());
    }
    if a.get(2).map(String::as_str) == Some("all") {
        let started = std::time::Instant::now();
        room_graph::trace_remaining(&store, &guide, &|m| eprintln!("{m}"), &|| {})?;
        println!("traced in {:.0} s", started.elapsed().as_secs_f32());
        return Ok(());
    }
    let start = a.get(2).context("Start seconds")?.parse::<u64>()? * 1000;
    let end = a.get(3).context("End seconds")?.parse::<u64>()? * 1000;
    let every = a.get(4).context("Every ms")?.parse::<u64>()?;
    let started = std::time::Instant::now();
    let placed = room_graph::trace_video(
        &store,
        &guide,
        start,
        end,
        every,
        &|m| eprintln!("{m}"),
        &|| {},
    )?;
    println!(
        "{placed} frames placed in {:.0} s",
        started.elapsed().as_secs_f32()
    );
    Ok(())
}
