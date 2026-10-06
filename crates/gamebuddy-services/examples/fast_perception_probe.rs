//! Real local OCR timings. The input is a test image, never a gameplay mutation.
use anyhow::{Context, Result};
use gamebuddy_services::{Store, fast_perception, screen};
use std::time::Instant;
fn main() -> Result<()> {
    let image = std::env::args().nth(1).context("Test image required")?;
    let directory = tempfile::tempdir()?;
    let store = Store::open(directory.path())?;
    let ticket = screen::start(&store, "Detector fixture", true)?;
    for _ in 0..8 {
        screen::publish(
            &store,
            &ticket,
            std::path::Path::new(&image),
            "detector_fixture",
        )?;
        let frame = screen::snapshot(&store)?;
        let start = Instant::now();
        let result = fast_perception::inspect(&frame)?;
        println!(
            "{}",
            serde_json::json!({"elapsed_ms":start.elapsed().as_millis(),"event":result})
        );
    }
    Ok(())
}
