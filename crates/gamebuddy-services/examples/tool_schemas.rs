//! Print GameBuddy's agent tools and live-voice setup as JSON, for wiring and
//! testing external reasoners. With a data directory, also print the exact
//! Gemini Live session the app would open for that profile.
fn main() -> anyhow::Result<()> {
    use gamebuddy_services::voice::prompts;
    let session = match std::env::args().nth(1) {
        Some(dir) => {
            let store = gamebuddy_services::Store::open(dir)?;
            prompts::gemini_session(&store, "Faxanadu", "{}")?
        }
        None => serde_json::Value::Null,
    };
    println!(
        "{}",
        serde_json::json!({
            "tools": gamebuddy_services::harness::tool_definitions(),
            "reasoner": prompts::REASONER,
            "gemini_tools": prompts::gemini_tools(),
            "gemini_prompt": prompts::gemini_live(),
            "gemini_session": session,
        })
    );
    Ok(())
}
