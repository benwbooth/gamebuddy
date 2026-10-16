//! Export the credential-free session setup for a Gemini Live protocol probe.
fn main() -> anyhow::Result<()> {
    let live = gamebuddy_services::LiveVoice {
        model: "gemini-3.8-live-extended-thinking".into(),
        ..Default::default()
    };
    let instructions = gamebuddy_services::voice::prompts::gemini_thinking();
    println!(
        "{}",
        gamebuddy_services::voice::prompts::gemini_setup(&live, &instructions)?
    );
    Ok(())
}
