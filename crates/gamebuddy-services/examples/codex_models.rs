//! List the Codex models this account can use, or with "check" send one
//! tiny request with the saved reasoning settings of a data directory.
fn main() -> anyhow::Result<()> {
    use gamebuddy_services::ReasoningProvider;
    let mut args = std::env::args().skip(1);
    if let (Some(command), Some(dir)) = (args.next(), args.next())
        && command == "check"
    {
        let store = gamebuddy_services::Store::open(dir)?;
        let settings: gamebuddy_services::Settings = store.get("settings")?.unwrap_or_default();
        settings.reasoning.validate()?;
        let started = std::time::Instant::now();
        let reply = gamebuddy_services::HttpProvider::new(settings.reasoning.clone())?.complete(
            gamebuddy_services::ModelRequest {
                system: "Answer in one short sentence.".into(),
                prompt: "In Faxanadu (NES), which key opens the Tower of Suffer?".into(),
                images: vec![],
                json: false,
            },
        )?;
        println!("{} ms: {reply}", started.elapsed().as_millis());
        return Ok(());
    }
    let config = gamebuddy_services::ProviderConfig {
        kind: gamebuddy_services::ProviderKind::Codex,
        endpoint: String::new(),
        model: String::new(),
        api_key_env: String::new(),
        reasoning_effort: String::new(),
    };
    println!(
        "{}",
        gamebuddy_services::provider_catalog::discover(&config)?
    );
    Ok(())
}
