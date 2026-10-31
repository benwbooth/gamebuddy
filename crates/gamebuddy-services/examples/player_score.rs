//! Where a recorded practice run got to: places the frames of each run on the
//! area maps and prints, one JSON line per frame looked at, how many screens
//! of the room graph's route to <toward> were still ahead.
//! player_score <data dir> <toward map title> <seconds between frames> <run dir>...
//! Frames are PNGs named by their milliseconds into the run.
use anyhow::{Context, Result};
use gamebuddy_services::navigation::{atlas, room_graph};
use serde_json::json;
use std::path::PathBuf;

fn main() -> Result<()> {
    let a: Vec<String> = std::env::args().collect();
    let store = gamebuddy_services::Store::open(a.get(1).context("Data directory")?)?;
    let toward_title = a.get(2).context("Toward map title")?;
    let every: f64 = a.get(3).context("Seconds between frames")?.parse()?;
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
    for run in &a[4..] {
        let mut frames: Vec<PathBuf> = std::fs::read_dir(run)?
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "png"))
            .collect();
        frames.sort();
        let mut next = 0.0;
        for path in frames {
            let ms: f64 = path
                .file_stem()
                .and_then(|s| s.to_str())
                .context("Frame name")?
                .parse()?;
            if ms / 1000.0 < next {
                continue;
            }
            next = ms / 1000.0 + every;
            let frame = image::open(&path)?;
            let marker = gamebuddy_services::spatial_maps::locate(&store, &guide, &world, &frame)?;
            let Some(marker) = marker else {
                println!("{}", json!({"run": run, "ms": ms, "map": null}));
                continue;
            };
            let map = marker["map_id"].as_str().unwrap_or_default().to_owned();
            let r = &marker["region"];
            let at = [
                (r["x"].as_f64().unwrap_or(0.) + r["width"].as_f64().unwrap_or(0.) / 2.) as f32,
                (r["y"].as_f64().unwrap_or(0.) + r["height"].as_f64().unwrap_or(0.) / 2.) as f32,
            ];
            let left = if map == toward {
                Some(0)
            } else {
                room_graph::way_out(&store, &guide, &map, at, &toward)?
                    .map(|m| m.iter().map(|s| s.screens).sum::<u32>() + 1)
            };
            let title = atlas.maps.iter().find(|m| m.id == map).map(|m| &m.title);
            println!(
                "{}",
                json!({"run": run, "ms": ms, "map": title, "at": at, "left": left})
            );
        }
    }
    Ok(())
}
