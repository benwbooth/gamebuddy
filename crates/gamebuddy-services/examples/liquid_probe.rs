//! Check that the LFM3 reasoning server starts for the "liquid" setup:
//! liquid_probe <data dir with that setup saved>
fn main() -> anyhow::Result<()> {
    let store = gamebuddy_services::Store::open(std::env::args().nth(1).unwrap_or_default())?;
    gamebuddy_services::liquid::ensure(&store);
    if std::env::var_os("LIQUID_TEXT").is_some() {
        gamebuddy_services::liquid::ready_for(&gamebuddy_services::ProviderConfig {
            kind: gamebuddy_services::ProviderKind::OpenAiCompatible,
            endpoint: gamebuddy_services::providers::LIQUID_TEXT_ENDPOINT.into(),
            model: gamebuddy_services::providers::LIQUID_TEXT_MODEL.into(),
            api_key_env: String::new(),
            reasoning_effort: String::new(),
        });
    }
    for i in 0..60 {
        if gamebuddy_services::liquid::answering(11440) {
            println!("answering after {:.1} s", i as f64 * 0.5);
            // LIQUID_HOLD=<seconds>: keep the servers up for replays.
            if let Ok(hold) = std::env::var("LIQUID_HOLD") {
                std::thread::sleep(std::time::Duration::from_secs(hold.parse()?));
            }
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    anyhow::bail!("not answering after 30 s")
}
