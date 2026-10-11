//! Print the compact live-voice form of a get_game_context result read from
//! stdin, for replaying live sessions outside the app.
fn main() -> anyhow::Result<()> {
    let full: serde_json::Value = serde_json::from_reader(std::io::stdin())?;
    println!(
        "{}",
        gamebuddy_services::voice::prompts::gemini_context(&full)
    );
    Ok(())
}
