//! Where a frame is and the route's next move, for a player running on its
//! own clock: reads one frame path per line, answers one JSON line.
//! player_route <data dir> <toward map title>
use anyhow::{Context, Result};
use gamebuddy_services::navigation::{atlas, room_graph};
use serde_json::json;
use std::io::BufRead;

fn main() -> Result<()> {
    let a: Vec<String> = std::env::args().collect();
    let store = gamebuddy_services::Store::open(a.get(1).context("Data directory")?)?;
    let toward_title = a.get(2).context("Toward map title")?;
    let id: String = store.get("active_guide")?.context("No active guide")?;
    let guide = store.guide(&id)?;
    let atlas = atlas(&store, &guide)?;
    let toward = atlas
        .maps
        .iter()
        .find(|m| m.title == *toward_title)
        .context("Unknown map")?
        .id
        .clone();
    let world = gamebuddy_services::spatial::load_cached(&store, &guide)?.context("No world")?;
    for line in std::io::stdin().lock().lines() {
        let frame = image::open(line?.trim())?;
        let marker = gamebuddy_services::spatial_maps::locate(&store, &guide, &world, &frame)?;
        let Some(marker) = marker else {
            println!("{}", json!({"map": null}));
            continue;
        };
        let map = marker["map_id"].as_str().unwrap_or_default().to_owned();
        let r = &marker["region"];
        let at = [
            (r["x"].as_f64().unwrap_or(0.) + r["width"].as_f64().unwrap_or(0.) / 2.) as f32,
            (r["y"].as_f64().unwrap_or(0.) + r["height"].as_f64().unwrap_or(0.) / 2.) as f32,
        ];
        let moves = if map == toward {
            Some(vec![])
        } else {
            room_graph::way_out(&store, &guide, &map, at, &toward)?
        };
        let list: Option<Vec<_>> = moves.map(|m| {
            m.iter()
                .map(|s| json!([format!("{:?}", s.direction), s.screens]))
                .collect()
        });
        println!(
            "{}",
            json!({"map": map, "arrived": map == toward, "at": at, "moves": list})
        );
    }
    Ok(())
}
