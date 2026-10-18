//! Stitch one stretch of a walkthrough video into map pieces and save them:
//! level_map_probe <data dir> <start s> <end s> <out dir>
fn main() -> anyhow::Result<()> {
    use anyhow::Context;
    let a: Vec<String> = std::env::args().collect();
    let store = gamebuddy_services::Store::open(&a[1])?;
    let id: String = store.get("active_guide")?.context("No active guide")?;
    let guide = store.guide(&id)?;
    let source = guide.sources.iter().find(|s| s.media_path.is_some()).context("No video")?;
    let profile = gamebuddy_services::spatial::profile(&store, &guide)?;
    let c = profile.source_crops.get(&source.id).copied().unwrap_or_default();
    let b = profile.background;
    let crop = gamebuddy_core::Crop { x: c.x + b.x * c.width, y: c.y + b.y * c.height, width: b.width * c.width, height: b.height * c.height };
    let started = std::time::Instant::now();
    let pieces = gamebuddy_services::level_maps::stitch(
        std::path::Path::new(source.media_path.as_ref().unwrap()),
        crop,
        a[2].parse::<u64>()? * 1000,
        a[3].parse::<u64>()? * 1000,
    )?;
    std::fs::create_dir_all(&a[4])?;
    for (i, p) in pieces.iter().enumerate() {
        let path = format!("{}/piece-{i}.jpg", a[4]);
        p.image.save(&path)?;
        println!("{path} {}x{} {}..{}s", p.image.width(), p.image.height(), p.start_ms / 1000, p.end_ms / 1000);
    }
    println!("{} pieces in {:.1}s", pieces.len(), started.elapsed().as_secs_f32());
    Ok(())
}
