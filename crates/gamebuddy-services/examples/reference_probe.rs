//! Replay a cropped screenshot through reference retrieval without modifying
//! live progress. Optionally run the configured local vision provider.
use anyhow::{Context, Result, ensure};
use gamebuddy_core::{Crop, Synchronizer, fingerprint, fingerprint_has_detail};
use gamebuddy_services::{HttpProvider, ProviderKind, Settings, Store};
use serde_json::json;

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let id = args
        .next()
        .context("Usage: reference_probe GUIDE_ID CROPPED_IMAGE OUTPUT_JSON [--vision]")?;
    let image = args.next().context("Cropped gameplay image")?;
    let output = args.next().context("Output JSON")?;
    let vision = args.next().as_deref() == Some("--vision");
    let store = Store::open(gamebuddy_services::data_dir()?)?;
    let guide = store.guide(&id)?;
    let index = store.index(&guide)?.context("No visual index")?;
    let progress = store.progress(&guide)?;
    let epoch = progress.epoch;
    let vector = fingerprint(&image::open(&image)?, Crop::default())?;
    let estimate = Synchronizer::new(progress).observe(
        &guide,
        &index,
        &vector,
        gamebuddy_services::screen::now_ms(),
        epoch,
    )?;
    let mut result = json!({
        "guide_id":id,"guide_revision":guide.revision,"image":image,
        "reference_count":index.frames.len(),
        "excluded_low_detail_frames":index.frames.iter().filter(|f|!fingerprint_has_detail(&f.fingerprint)).count(),
        "candidates":estimate.candidates.iter().map(|c|json!({"candidate":c,"title":guide.step(&c.step_id).map(|s|&s.title)})).collect::<Vec<_>>(),
        "estimate":estimate
    });
    std::fs::write(&output, serde_json::to_vec_pretty(&result)?)?;
    println!(
        "{} references, {} low-detail frames excluded; {} candidate steps",
        index.frames.len(),
        result["excluded_low_detail_frames"],
        estimate.candidates.len()
    );
    if vision {
        let settings: Settings = store.get("settings")?.unwrap_or_default();
        ensure!(
            settings.vision.kind == ProviderKind::Ollama,
            "This diagnostic only calls a local Ollama provider"
        );
        result["model"] = json!(settings.vision.model);
        let started = std::time::Instant::now();
        let verdict = gamebuddy_services::compare_candidates(
            &HttpProvider::new(settings.vision)?,
            &guide,
            &image,
            &estimate.candidates,
        );
        result["vision_elapsed_ms"] = json!(started.elapsed().as_millis());
        match verdict {
            Ok(v) => {
                println!("{}", serde_json::to_string_pretty(&v)?);
                result["vision_verdict"] = json!(v);
            }
            Err(e) => {
                result["vision_error"] = json!(e.to_string());
                std::fs::write(&output, serde_json::to_vec_pretty(&result)?)?;
                return Err(e);
            }
        }
        std::fs::write(&output, serde_json::to_vec_pretty(&result)?)?;
    }
    println!("Reference diagnostic: {output}");
    Ok(())
}
