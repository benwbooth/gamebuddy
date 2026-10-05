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
            // Speech recognition varies the title. The newly requested
            // screenshot content is the behavior under test.
            .any(|w| {
                matches!(&w.content, gamebuddy_core::WidgetContent::Text { body }
                if body.to_lowercase().contains("red") && body.to_lowercase().contains("green"))
            }))
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
        Ok(text[before..]
            .split_once("test-answer-released")
            .is_some_and(|(_, final_output)| {
                final_output
                    .lines()
                    .any(|l| l.contains("\"type\":\"caption\"") && final_answer(l))
            }))
    })?;
    let end = gamebuddy_services::screen::now_ms();
    quiet()?;
    ensure!(capturing()?, "Progress updates closed the microphone");
    ensure!(
        store
            .conversation_messages(&guide.game, "", None, 10)?
            .iter()
            .any(|m| m.role == "assistant" && final_answer(&m.text)),
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
    if std::env::var_os("GAMEBUDDY_TEST_NOISE").is_some() {
        ensure!(
            text[before..].contains("test-unrecognized-noise"),
            "Noise injection did not start"
        );
        ensure!(
            text[before..].matches("GAMEBUDDY_VOICE_DISPATCH ").count() == 1,
            "Background noise cancelled or repeated the recognized request"
        );
        ensure!(
            !text[before..].contains("request-error"),
            "Noise caused a request error"
        );
        println!("GAMEBUDDY_UNRECOGNIZED_NOISE_PASS");
    }
    println!("GAMEBUDDY_WORK_PROGRESS_PASS start_ms={start} end_ms={end}");
    Ok(())
}

fn final_answer(text: &str) -> bool {
    let text = text.to_lowercase();
    // The model may answer with the requested screen description instead of
    // the requested fixed phrase. Both must arrive after the held answer.
    text.contains("answer is ready") || (text.contains("red") && text.contains("green"))
}
