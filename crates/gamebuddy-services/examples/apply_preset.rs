//! Apply a one-click setup to saved settings, as the Models page does:
//! apply_preset <data dir> <device|local|openai|gemini>
fn main() -> anyhow::Result<()> {
    use anyhow::Context;
    let args: Vec<String> = std::env::args().collect();
    let store = gamebuddy_services::Store::open(args.get(1).context("Data directory")?)?;
    let settings: gamebuddy_services::Settings = store.get("settings")?.unwrap_or_default();
    let next = settings.with_preset(args.get(2).context("Preset")?)?;
    store.put("settings", &next)?;
    println!(
        "{}",
        serde_json::json!({"reasoning": next.reasoning, "speech": next.speech, "brain": next.brain, "voice": next.prepared_speech})
    );
    Ok(())
}
