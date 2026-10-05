//! Regression using a private copy of a save and a later-area screenshot.
use super::{play_player, wait};
use anyhow::{Context, Result, ensure};
use gamebuddy_services::Store;
use serde_json::Value;
use std::{
    process::Command,
    thread,
    time::{Duration, Instant},
};

pub fn run(
    store: &Store,
    log: &str,
    quiet: impl Fn() -> Result<()>,
    capturing: impl Fn() -> Result<bool>,
) -> Result<()> {
    let id: String = store.get("active_guide")?.context("Guide")?;
    let guide = store.guide(&id)?;
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let context = gamebuddy_services::spatial::context(store, &guide)?;
        if context["observation"]["tracker"]["status"] == "tracked" {
            break;
        }
        ensure!(
            Instant::now() < deadline,
            "Later-area source match never settled"
        );
        thread::sleep(Duration::from_millis(200));
    }
    let key = format!("next-action:{id}");
    let started = gamebuddy_services::screen::now_ms();
    play_player(&std::env::var("GAMEBUDDY_TEST_PROGRESS_WAV")?)?;
    wait("current-area advice replaces obsolete gate advice", || {
        let advice: Value = store.get(&key)?.unwrap_or_default();
        Ok(
            advice["updated_at_ms"].as_u64().unwrap_or_default() >= started
                && advice["step_id"]
                    .as_str()
                    .is_some_and(|s| !s.starts_with("chapter-3.part-1.1"))
                && advice["location_evidence"].is_object(),
        )
    })?;
    quiet()?;
    println!(
        "GAMEBUDDY_CURRENT_AREA_ADVICE_PASS {}",
        store.get::<Value>(&key)?.unwrap_or_default()
    );
    play_player(&std::env::var("GAMEBUDDY_TEST_COMPLETION_WAV")?)?;
    wait("explicit completion report is saved", || {
        let progress = store.progress(&guide)?;
        Ok(
            gamebuddy_core::step_complete(&guide, &progress, "chapter-3.part-1.1.2")
                && gamebuddy_core::step_complete(&guide, &progress, "chapter-3.part-1.1.3"),
        )
    })?;
    quiet()?;
    println!("GAMEBUDDY_COMPLETION_REPORT_PASS");
    let before = std::fs::read_to_string(log)?.len();
    ensure!(
        Command::new("node")
            .arg("scripts/stall-voice-once.mjs")
            .status()?
            .success(),
        "Fault injection failed"
    );
    play_player(&std::env::var("GAMEBUDDY_TEST_RECOVERY_WAV")?)?;
    wait(
        "timed-out speech reconnects and plays completed answer",
        || {
            let text = std::fs::read_to_string(log)?;
            let tail = &text[before..];
            Ok(tail.contains("Reconnecting voice")
                && tail.contains("GAMEBUDDY_VOICE_PLAYBACK started"))
        },
    )?;
    quiet()?;
    ensure!(capturing()?, "Recovery left microphone unavailable");
    let events: Vec<Value> = store
        .get("voice-diagnostics")?
        .context("Persistent diagnostics")?;
    ensure!(
        events.iter().any(|e| e["kind"] == "browser/cue-timeout"),
        "Timeout was not persisted"
    );
    println!("GAMEBUDDY_SPEECH_RECOVERY_PASS actual reconnect, playback and active microphone");
    Ok(())
}
