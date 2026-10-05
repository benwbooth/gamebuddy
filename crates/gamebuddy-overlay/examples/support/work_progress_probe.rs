//! Hold a real completed answer for 35 seconds while spoken status stays live.
use super::{play_player, wait};
use anyhow::{Context, Result, ensure};
use gamebuddy_services::Store;
use std::process::Command;

pub fn run(
    store: &Store,
    log: &str,
    quiet: impl Fn() -> Result<()>,
    capturing: impl Fn() -> Result<bool>,
) -> Result<()> {
    let id: String = store.get("active_guide")?.context("Guide")?;
    let guide = store.guide(&id)?;
    ensure!(
        Command::new("node")
            .arg("scripts/hold-voice-answer.mjs")
            .status()?
            .success(),
        "Could not install controlled answer delay"
    );
    let before = std::fs::read_to_string(log)?.len();
    play_player(&std::env::var("GAMEBUDDY_TEST_VOICE_WAV")?)?;
    let start = gamebuddy_services::screen::now_ms();
    wait("real screenshot tool creates requested widget", || {
        Ok(gamebuddy_services::harness::widgets(store, &guide)?
            .iter()
            .any(|w| w.title == "Waiting Test"))
    })?;
    wait("real completed answer reaches controlled delay", || {
        Ok(std::fs::read_to_string(log)?[before..].contains("test-answer-held"))
    })?;
    wait("repeated audible work updates", || {
        let text = std::fs::read_to_string(log)?;
        Ok(text[before..].matches("\"type\":\"work-progress\"").count() >= 3)
    })?;
    wait("final answer is spoken after status updates", || {
        let text = std::fs::read_to_string(log)?;
        Ok(text[before..].lines().any(|l| {
            l.contains("\"type\":\"caption\"") && l.to_lowercase().contains("answer is ready")
        }))
    })?;
    let end = gamebuddy_services::screen::now_ms();
    quiet()?;
    ensure!(capturing()?, "Progress updates closed the microphone");
    ensure!(
        store
            .conversation_messages(&guide.game, "answer is ready", None, 10)?
            .iter()
            .any(|m| m.role == "assistant"),
        "Status updates displaced the saved final answer"
    );
    let text = std::fs::read_to_string(log)?;
    ensure!(
        !text[before..].contains("cue-timeout"),
        "Speech timed out during progress test"
    );
    ensure!(
        text[before..]
            .matches("GAMEBUDDY_VOICE_TOOL name=show_widget success=true")
            .count()
            == 1,
        "Progress replayed a tool action"
    );
    println!("GAMEBUDDY_WORK_PROGRESS_PASS start_ms={start} end_ms={end}");
    Ok(())
}
