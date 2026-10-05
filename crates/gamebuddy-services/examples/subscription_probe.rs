use gamebuddy_services::{
    HttpProvider, ModelRequest, ProviderConfig, ProviderKind, ReasoningProvider,
};
fn main() -> anyhow::Result<()> {
    let kind = match std::env::args().nth(1).as_deref() {
        Some("codex") => ProviderKind::Codex,
        Some("claude") => ProviderKind::ClaudeCode,
        _ => anyhow::bail!("Usage: subscription_probe codex|claude [image]"),
    };
    let images = std::env::args()
        .nth(2)
        .map(gamebuddy_services::image_base64)
        .transpose()?
        .into_iter()
        .collect();
    let provider = HttpProvider::new(ProviderConfig {
        kind,
        endpoint: String::new(),
        model: std::env::var("GAMEBUDDY_PROBE_MODEL").unwrap_or_default(),
        api_key_env: String::new(),
        reasoning_effort: std::env::var("GAMEBUDDY_PROBE_EFFORT").unwrap_or_default(),
    })?;
    let request = ModelRequest {
        system: "You are a game guide. Return JSON only. Use no tools.".into(),
        prompt:
            "Return {\"ready\":true,\"observation\":\"one sentence about the image, or no image\"}."
                .into(),
        images,
        json: true,
    };
    let rounds: usize = std::env::var("GAMEBUDDY_PROBE_ROUNDS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(1);
    for round in 0..rounds.clamp(1, 8) {
        let started = std::time::Instant::now();
        let answer = provider.complete(request.clone())?;
        let value: serde_json::Value = gamebuddy_services::parse_json(&answer)?;
        anyhow::ensure!(value["ready"] == true, "Unexpected provider response");
        println!(
            "round={} {} ({:.2}s)",
            round + 1,
            value,
            started.elapsed().as_secs_f32()
        );
    }
    Ok(())
}
