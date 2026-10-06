//! Signed-in provider benchmark against a private snapshot. No progress writes.
use anyhow::{Context, Result, ensure};
use gamebuddy_services::{Settings, Store, screen, voice::decision::Lane};
use std::time::{Duration, Instant};
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    ensure!(
        args.len() >= 4,
        "decision_probe PRIVATE_STORE IMAGE REQUEST [COUNT]"
    );
    ensure!(args[1].starts_with("/tmp/"), "Use a private fixture store");
    let store = Store::open(&args[1])?;
    let guide = store.guide(
        &store
            .get::<String>("active_guide")?
            .context("Guide missing")?,
    )?;
    let ticket = screen::start(&store, &guide.game, true)?;
    let settings: Settings = store.get("settings")?.unwrap_or_default();
    let mut lane = Lane::start(settings);
    let count = args.get(4).and_then(|s| s.parse::<u64>().ok()).unwrap_or(3);
    for n in 1..=count {
        screen::publish(
            &store,
            &ticket,
            std::path::Path::new(&args[2]),
            "decision_fixture",
        )?;
        let evidence = gamebuddy_services::voice::decision::Evidence::capture(&store, &guide.game)?;
        std::fs::write(
            store.root.join("decision-context.json"),
            serde_json::to_vec_pretty(&evidence.value)?,
        )?;
        let at = Instant::now();
        lane.propose(&store, &guide.game, n, &args[3])?;
        loop {
            if let Some(p) = lane.poll() {
                println!(
                    "{}",
                    serde_json::json!({"attempt":n,"elapsed_ms":at.elapsed().as_millis(),"context_bytes":p.evidence.value.to_string().len(),"decision":p.decision.map_err(|e|e.to_string())})
                );
                break;
            }
            ensure!(at.elapsed() < Duration::from_secs(30), "Decision timed out");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    Ok(())
}
