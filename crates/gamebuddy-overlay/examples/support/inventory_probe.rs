//! Replay a supplied real inventory screen through native microphone and speech.
use super::{play_player, wait};
use anyhow::{Context, Result, ensure};

pub fn run(log: &str, quiet: impl Fn() -> Result<()>) -> Result<()> {
    let before = std::fs::read_to_string(log)?.len();
    play_player(&std::env::var("GAMEBUDDY_TEST_INVENTORY_WAV")?)?;
    let end = gamebuddy_services::screen::now_ms();
    wait("inventory question gets its verified spoken answer", || {
        let text = std::fs::read_to_string(log)?;
        let tail = text[before..].to_lowercase();
        Ok(tail.lines().any(|line| {
            line.contains("\"type\":\"caption\"") && line.contains("potion") && line.contains("key")
        }))
    })?;
    quiet()?;
    let text = std::fs::read_to_string(log)?;
    let tail = &text[before..];
    ensure!(
        tail.contains("images=1"),
        "Inventory image was not supplied"
    );
    ensure!(
        tail.matches("GAMEBUDDY_VOICE_DISPATCH ").count() == 1,
        "Inventory question restarted the backend"
    );
    ensure!(
        !tail.contains("name=edit_plan") && !tail.contains("name=get_outline"),
        "Inventory answer detoured into outline repair"
    );
    let caption = tail
        .lines()
        .find(|line| {
            line.contains("\"type\":\"caption\"") && line.to_lowercase().contains("potion")
        })
        .context("inventory answer caption")?;
    let caption_at = tail.find(caption).context("caption position")?;
    let onset = tail[..caption_at]
        .lines()
        .rev()
        .find_map(|line| {
            line.split_once("GAMEBUDDY_VOICE_PLAYBACK started at_ms=")
                .and_then(|(_, s)| s.parse::<u64>().ok())
        })
        .context("audible inventory reply")?;
    println!(
        "GAMEBUDDY_NATIVE_INVENTORY_PASS latency_ms={} one_backend_turn=true no_outline_repair=true",
        onset.saturating_sub(end)
    );
    Ok(())
}
