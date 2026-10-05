//! Replay a saved crop through concurrent capture, semantic vision and spatial
//! matching. Requires a disposable copy of a prepared store as its first arg.
use anyhow::{Context, Result};
use gamebuddy_services::{HttpProvider, ProviderConfig, Store, screen, spatial};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
fn main() -> Result<()> {
    let root = PathBuf::from(
        std::env::args()
            .nth(1)
            .context("Disposable prepared store required")?,
    );
    anyhow::ensure!(root.starts_with("/tmp/"), "Use a disposable /tmp store");
    let path = PathBuf::from(std::env::args().nth(2).context("Cropped frame required")?);
    let mut frames = if path.is_dir() {
        std::fs::read_dir(&path)?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|e| e == "png"))
            .collect::<Vec<_>>()
    } else {
        vec![path.clone()]
    };
    frames.sort();
    anyhow::ensure!(!frames.is_empty(), "No frames to replay");
    let store = Store::open(&root)?;
    let id: String = store.get("active_guide")?.context("Guide required")?;
    let guide = store.guide(&id)?;
    let ticket = screen::start(&store, &guide.game, true)?;
    let running = Arc::new(AtomicBool::new(true));
    let capture = {
        let running = running.clone();
        let root = root.clone();
        let guide = guide.clone();
        std::thread::spawn(move || -> Result<()> {
            let store = Store::open(root)?;
            let started = Instant::now();
            let mut index = 0;
            while running.load(Ordering::Relaxed) {
                let frame = &frames[index.min(frames.len() - 1)];
                if started.elapsed() >= Duration::from_secs(25) {
                    index += 1;
                }
                screen::publish(&store, &ticket, frame, "concurrent_saved_capture_replay")?;
                spatial::schedule(
                    &store,
                    &guide,
                    screen::snapshot(&store)?,
                    store.progress(&guide)?.epoch,
                );
                std::thread::sleep(Duration::from_secs(1));
            }
            Ok(())
        })
    };
    let vision = {
        let running = running.clone();
        let root = root.clone();
        std::thread::spawn(move || -> Result<()> {
            let store = Store::open(root)?;
            let provider = HttpProvider::new(ProviderConfig::default())?;
            while running.load(Ordering::Relaxed) {
                if let Ok(snapshot) = screen::snapshot(&store) {
                    let start = Instant::now();
                    let result = screen::observe(&provider, &snapshot, None, &json!({"places":[]}));
                    let saved = screen::save_observation(&store, &snapshot, &result);
                    println!(
                        "vision_ms={} parsed={} saved={}",
                        start.elapsed().as_millis(),
                        result.is_ok(),
                        saved.is_ok()
                    );
                }
                std::thread::sleep(Duration::from_secs(3));
            }
            Ok(())
        })
    };
    let start = Instant::now();
    let mut visible = 0;
    let mut hidden = 0;
    let mut started = false;
    let mut states = vec![];
    while start.elapsed() < Duration::from_secs(75) {
        let context = spatial::context(&store, &guide)?;
        let state = gamebuddy_services::navigation::context(
            &store,
            &guide,
            &store.progress(&guide)?,
            &Default::default(),
        )?;
        let has = state["map"]["geometric_marker"].is_object()
            || state["map"]["updating_camera_area"].is_object();
        if started && !state["map"]["geometric_marker"].is_object() {
            println!(
                "pending age={} visible={has}",
                screen::now_ms().saturating_sub(
                    state["map"]["updating_camera_area"]["observed_at_ms"]
                        .as_u64()
                        .unwrap_or_default()
                )
            );
        }
        if has {
            visible += 1;
            started = true
        } else if started {
            hidden += 1;
            let raw: Value = store
                .get(&format!("world-observation:{id}"))?
                .unwrap_or_default();
            println!(
                "hidden {}",
                json!({"capture":screen::status(&store)?["captured_at_ms"],"observed":raw["tracker"]["observed_at_ms"],"raw_marker":raw["map_registration"],"context_observation":!context["observation"].is_null(),"error":context["error"]})
            );
        }
        if started {
            states.push(state);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    running.store(false, Ordering::Relaxed);
    capture.join().unwrap()?;
    vision.join().unwrap()?;
    std::fs::write(
        root.join("perception-states.json"),
        serde_json::to_vec(&states)?,
    )?;
    println!("perception visible={visible} hidden={hidden}");
    anyhow::ensure!(
        visible > 200 && hidden == 0,
        "Stable captured view lost its camera marker"
    );
    Ok(())
}
