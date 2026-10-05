//! Exercise the same discovery and connection-check endpoints as the native UI.
use gamebuddy_services::{ProviderConfig, ProviderKind, provider_catalog};

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let kind = match args.next().as_deref() {
        Some("codex") => ProviderKind::Codex,
        Some("claude") => ProviderKind::ClaudeCode,
        Some("ollama") => ProviderKind::Ollama,
        _ => anyhow::bail!("Usage: provider_settings_probe codex|claude|ollama [model [effort]]"),
    };
    let model = args.next();
    let config = ProviderConfig {
        endpoint: if kind == ProviderKind::Ollama {
            "http://127.0.0.1:11434".into()
        } else {
            String::new()
        },
        kind,
        model: model.clone().unwrap_or_default(),
        reasoning_effort: args.next().unwrap_or_default(),
        api_key_env: String::new(),
    };
    let result = if model.is_some() {
        provider_catalog::test(config)?
    } else {
        provider_catalog::discover(&config)?
    };
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}
