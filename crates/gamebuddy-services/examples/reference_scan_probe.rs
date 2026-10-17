//! Candidate-frame recall for the reference scan: `reference_scan_probe
//! <video> [known.json]` prints the frames the scan would show the model and,
//! given transcribed cards with times, how many of them fall near a candidate.
fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let video = std::path::Path::new(&args[1]);
    let interval = 2u64;
    let started = std::time::Instant::now();
    let frames = gamebuddy_services::reference_scan::thumbnails(video, interval)?;
    let picks = gamebuddy_services::reference_scan::candidates(&frames);
    println!(
        "{} frames, {} candidates ({} sheets) in {:.1}s",
        frames.len(),
        picks.len(),
        picks.len().div_ceil(12),
        started.elapsed().as_secs_f32()
    );
    if let Some(known) = args.get(2) {
        let cards: Vec<serde_json::Value> = serde_json::from_slice(&std::fs::read(known)?)?;
        let times: Vec<u64> = picks.iter().map(|&i| i as u64 * interval).collect();
        let mut hit = 0;
        let mut missed = vec![];
        let cards: Vec<_> = cards.iter().filter(|c| c["is_card"] == true).collect();
        for card in &cards {
            let t = card["t"].as_u64().unwrap_or(0);
            let end = card["end_s"].as_u64().unwrap_or(t + 4);
            let start = card["start_s"].as_u64().unwrap_or(t.saturating_sub(4));
            if times.iter().any(|&x| x + 2 >= start && x <= end + 2) {
                hit += 1
            } else {
                missed.push(format!("{} {}", t, card["name"]))
            }
        }
        println!("recall {hit}/{}; missed {missed:?}", cards.len());
    }
    Ok(())
}
