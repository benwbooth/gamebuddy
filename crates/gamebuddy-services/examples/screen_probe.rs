//! Inspect a saved game image with the configured local observer in a private
//! capture store. Does not modify the player's live session or game state.
use anyhow::{Context, Result};
use gamebuddy_services::{HttpProvider, ProviderConfig, Store, screen};
use serde_json::json;
use std::{path::PathBuf, time::Instant};

fn main() -> Result<()> {
    let frame = PathBuf::from(
        std::env::args()
            .nth(1)
            .context("Cropped image path required")?,
    );
    let directory = tempfile::tempdir()?;
    let store = Store::open(directory.path())?;
    let ticket = screen::start(&store, "Faxanadu", true)?;
    let provider = HttpProvider::new(ProviderConfig::default())?;
    let mut previous = None;
    for _ in 0..3 {
        screen::publish(&store, &ticket, &frame, "saved_capture_observer_probe")?;
        let snapshot = screen::snapshot(&store)?;
        let started = Instant::now();
        let scene = screen::observe(
            &provider,
            &snapshot,
            previous.as_ref(),
            &json!({"places":[]}),
        )?;
        println!(
            "{}",
            json!({"elapsed_ms":started.elapsed().as_millis(),"scene":scene})
        );
        previous = Some(serde_json::to_value(scene)?);
    }
    Ok(())
}
