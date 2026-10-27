//! Run the brain once on recorded frames, in a copy of the data directory:
//! brain_probe <data dir copy> <guide id> <provider: ollama:MODEL | current> <frame.jpg>...
//! The first frame is "now"; any others are earlier moments, oldest first.
//! Set GAMEBUDDY_QUESTION to ask a spoken question.
//! Writes the situation and timing as JSON. Never use the live data directory:
//! the brain records facts it reads.
use anyhow::{Context, Result};
use gamebuddy_services::{ProviderConfig, ProviderKind, Settings, Store, brain, screen};

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let store = Store::open(args.get(1).context("Data directory copy")?)?;
    let guide = store.guide(args.get(2).context("Guide id")?)?;
    let mut settings: Settings = store.get("settings")?.unwrap_or_default();
    if let Some(model) = args.get(3).and_then(|p| p.strip_prefix("ollama:")) {
        settings.brain.provider = Some(ProviderConfig {
            kind: ProviderKind::Ollama,
            endpoint: "http://127.0.0.1:11434".into(),
            model: model.into(),
            api_key_env: String::new(),
            reasoning_effort: String::new(),
        });
    }
    settings.brain.enabled = true;
    store.put("settings", &settings)?;
    let frames = &args[4..];
    let ticket = screen::start(&store, &guide.game, true)?;
    let now = screen::now_ms();
    let mut moments = vec![];
    for (i, path) in frames.iter().enumerate().skip(1) {
        let at = now - 1_000 * (frames.len() - i) as u64;
        screen::publish_at(&store, &ticket, std::path::Path::new(path), "window", at)?;
        moments.push(screen::snapshot(&store)?);
    }
    screen::publish_at(
        &store,
        &ticket,
        std::path::Path::new(frames.first().context("A frame")?),
        "window",
        now,
    )?;
    let started = std::time::Instant::now();
    // GAMEBUDDY_QUESTION asks the brain a spoken question, as local voice does.
    let question = std::env::var("GAMEBUDDY_QUESTION").ok();
    let (situation, answer) = brain::think_with(&store, &guide, question.as_deref(), &moments)?;
    println!(
        "{}",
        serde_json::json!({"seconds": started.elapsed().as_secs_f64(), "answer": answer, "situation": situation})
    );
    Ok(())
}
