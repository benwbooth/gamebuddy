//! Benchmark the old UI-thread index count against the shared reference cache.
//! Use a private fixture copied by scripts/copy-ui-fixture.mjs.
use anyhow::{Context, Result, ensure};
use gamebuddy_services::Store;
use std::{path::PathBuf, time::Instant};

fn main() -> Result<()> {
    let root = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .context("Private fixture directory")?,
    );
    ensure!(
        root.starts_with("/tmp"),
        "Benchmark only a private /tmp fixture"
    );
    let store = Store::open(root)?;
    let guide = store.guide(
        &store
            .get::<String>("active_guide")?
            .context("Active guide")?,
    )?;
    for pass in 0..5 {
        let before = Instant::now();
        let count = store.index(&guide)?.map_or(0, |i| i.frames.len());
        let legacy = before.elapsed();
        let before = Instant::now();
        ensure!(
            store.reference_count(&guide)? == count,
            "Cached count changed"
        );
        println!(
            "pass={pass} references={count} old_index_ms={:.2} cached_index_ms={:.2}",
            legacy.as_secs_f64() * 1000.,
            before.elapsed().as_secs_f64() * 1000.
        );
    }
    Ok(())
}
