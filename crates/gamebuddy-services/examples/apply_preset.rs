//! Apply a one-click setup to saved settings, as the Models page does:
//! apply_preset <data dir> <device|local|openai|gemini>
//! Or only the thinking model, through Claude Code, keeping the screen reader
//! and voice on this computer: apply_preset <data dir> claude:<model>:<effort>
fn main() -> anyhow::Result<()> {
    use anyhow::Context;
    let args: Vec<String> = std::env::args().collect();
    let store = gamebuddy_services::Store::open(args.get(1).context("Data directory")?)?;
    let settings: gamebuddy_services::Settings = store.get("settings")?.unwrap_or_default();
    let preset = args.get(2).context("Preset")?;
    let next = match preset.strip_prefix("claude:").and_then(|r| r.split_once(':')) {
        Some((model, effort)) => {
            let mut next = settings.clone();
            next.reasoning = gamebuddy_services::ProviderConfig {
                kind: gamebuddy_services::ProviderKind::ClaudeCode,
                endpoint: String::new(),
                model: model.into(),
                api_key_env: String::new(),
                reasoning_effort: effort.into(),
            };
            next.single_provider = false;
            // Spoken answers read the screen themselves; a cloud brain on
            // every scene change would compete with them for the quota.
            next.brain.enabled = false;
            next
        }
        None => settings.with_preset(preset)?,
    };
    store.put("settings", &next)?;
    println!(
        "{}",
        serde_json::json!({"reasoning": next.reasoning, "speech": next.speech, "brain": next.brain, "voice": next.prepared_speech})
    );
    Ok(())
}
