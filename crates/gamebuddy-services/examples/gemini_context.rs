//! Print the compact live-voice form of a get_game_context result read from
//! stdin, for replaying live sessions outside the app.
fn main() -> anyhow::Result<()> {
    let full: serde_json::Value = serde_json::from_reader(std::io::stdin())?;
    let store = std::env::args()
        .nth(1)
        .map(gamebuddy_services::Store::open)
        .transpose()?;
    let guide = match &store {
        Some(store) => store
            .get::<String>("active_guide")?
            .and_then(|id| store.guide(&id).ok()),
        None => None,
    };
    println!(
        "{}",
        gamebuddy_services::voice::prompts::gemini_context(
            &full,
            store.as_ref().zip(guide.as_ref())
        )
    );
    Ok(())
}
