//! Replay captured game images through a private store; never alter player progress.
use anyhow::{Context, Result, ensure};
use gamebuddy_services::{Store, navigation, screen, spatial};
use serde_json::{Value, json};
use std::{path::PathBuf, time::Instant};

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let source = Store::open(args.next().context("source store directory")?)?;
    let output = PathBuf::from(args.next().context("new private output directory")?);
    ensure!(!output.exists(), "Use a new private output directory");
    let frames: Vec<_> = args.map(PathBuf::from).collect();
    ensure!(!frames.is_empty(), "Supply cropped gameplay frames");
    let id = source
        .get::<String>("active_guide")?
        .context("active guide")?;
    let guide = source.guide(&id)?;
    let store = Store::open(&output)?;
    store.save_guide(&guide)?;
    store.put("active_guide", &id)?;
    for key in [
        "spatial-worker".into(),
        spatial::key(&guide),
        format!("world-profile:{id}"),
        format!("world-status:{id}"),
        format!("spatial-maps:{id}"),
        format!("atlas:{id}"),
    ] {
        store.put(
            &key,
            &source.get::<Value>(&key)?.with_context(|| key.clone())?,
        )?;
    }
    std::fs::create_dir_all(output.join("maps"))?;
    for map in navigation::atlas(&store, &guide)?.maps {
        let path = navigation::cached_map_path(&source, &map);
        if path.exists() {
            std::fs::copy(path, navigation::cached_map_path(&store, &map))?;
        }
    }
    let ticket = screen::start(&store, &guide.game, true)?;
    let mut results = vec![];
    for pass in 0..3 {
        for path in &frames {
            screen::publish(&store, &ticket, path, "saved_gameplay_replay")?;
            let snapshot = screen::snapshot(&store)?;
            let started = Instant::now();
            let result = spatial::observe(&store, &guide, &snapshot, 0);
            let elapsed_ms = started.elapsed().as_millis();
            let value = match result {
                Ok(value) => json!({"pass":pass,"frame":path,"elapsed_ms":elapsed_ms,
                    "status":value["tracker"]["status"],"room_id":value["tracker"]["room_id"],
                    "map_registration":value["map_registration"],"timing":value["timing"]}),
                Err(error) => {
                    json!({"pass":pass,"frame":path,"elapsed_ms":elapsed_ms,"error":error.to_string()})
                }
            };
            println!("{value}");
            results.push(value);
        }
    }
    std::fs::write(
        output.join("latency.json"),
        serde_json::to_vec_pretty(&results)?,
    )?;
    Ok(())
}
