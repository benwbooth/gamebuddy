//! Start the configured learned spatial worker the way GameBuddy does and
//! report whether its models load. Usage: tracker_probe <data directory>
use anyhow::{Context, Result};
use gamebuddy_services::{Store, spatial_worker};

fn main() -> Result<()> {
    let root = std::env::args_os().nth(1).context("Data directory")?;
    let store = Store::open(root)?;
    let config: spatial_worker::WorkerConfig = store
        .get("spatial-worker")?
        .context("No spatial worker configured; run scripts/setup-spatial-worker")?;
    let started = std::time::Instant::now();
    let info = spatial_worker::call(&config, "probe", serde_json::json!({"op":"info"}))?;
    println!(
        "TRACKER_PROBE_PASS ms={} {info}",
        started.elapsed().as_millis()
    );
    Ok(())
}
