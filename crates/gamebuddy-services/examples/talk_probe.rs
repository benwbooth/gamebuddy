//! Time one spoken turn the way local voice answers it, on a recorded frame,
//! in a copy of the data directory (answers are written to its conversation):
//! talk_probe <data dir copy> <guide id> <ollama:MODEL | claude:MODEL | openai:ENDPOINT|MODEL | current> <frame.jpg> <question>...
//! GAMEBUDDY_PROBE_ESTIMATE=<file> holds a recorded location estimate (a
//! flight recorder `location-estimate` value) in place for every question,
//! so a session's turns replay where the player was.
use anyhow::{Context, Result};
use gamebuddy_services::{ProviderConfig, ProviderKind, Settings, Store, screen, voice::talk};

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let store = Store::open(args.get(1).context("Data directory copy")?)?;
    let guide = store.guide(args.get(2).context("Guide id")?)?;
    let mut settings: Settings = store.get("settings")?.unwrap_or_default();
    let choice = args.get(3).context("Provider")?;
    if let Some(model) = choice.strip_prefix("ollama:") {
        settings.reasoning = ProviderConfig {
            kind: ProviderKind::Ollama,
            endpoint: "http://127.0.0.1:11434".into(),
            model: model.into(),
            api_key_env: String::new(),
            reasoning_effort: String::new(),
        };
    } else if let Some((endpoint, model)) = choice
        .strip_prefix("openai:")
        .and_then(|p| p.split_once('|'))
    {
        settings.reasoning = ProviderConfig {
            kind: ProviderKind::OpenAiCompatible,
            endpoint: endpoint.into(),
            model: model.into(),
            api_key_env: String::new(),
            reasoning_effort: String::new(),
        };
    } else if let Some(model) = choice.strip_prefix("claude:") {
        settings.reasoning = ProviderConfig {
            kind: ProviderKind::ClaudeCode,
            endpoint: String::new(),
            model: model.into(),
            api_key_env: String::new(),
            reasoning_effort: "low".into(),
        };
    }
    store.put("settings", &settings)?;
    store.put("active_guide", &guide.id)?;
    let ticket = screen::start(&store, &guide.game, true)?;
    let frame = std::path::Path::new(args.get(4).context("Frame")?);
    for question in &args[5..] {
        screen::publish_at(&store, &ticket, frame, "window", screen::now_ms())?;
        if let Some(path) = std::env::var_os("GAMEBUDDY_PROBE_ESTIMATE") {
            let recorded: serde_json::Value = serde_json::from_slice(&std::fs::read(path)?)?;
            store.put(
                &gamebuddy_services::navigation::estimate::key(&guide),
                &recorded,
            )?;
        }
        let started = std::time::Instant::now();
        let mut first = None;
        let answer = talk::answer_streaming(&store, &guide, question, &mut |_| {
            first.get_or_insert(started.elapsed().as_secs_f64());
        });
        println!(
            "{}",
            serde_json::json!({"question": question, "first_sentence_seconds": first, "seconds": started.elapsed().as_secs_f64(),
                "answer": answer.as_ref().map_err(|e| format!("{e:#}"))})
        );
    }
    let request = talk::request(&store, &guide, "what now")?;
    eprintln!("prompt bytes: {}", request.prompt.len());
    if std::env::var_os("GAMEBUDDY_SHOW_COMPACT").is_some() {
        eprintln!(
            "{}",
            talk::compact_request(&store, &guide, "what now")?.prompt
        );
    }
    Ok(())
}
