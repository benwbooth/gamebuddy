//! Repeated virtual-microphone requests with the actual output sink recorded.
use super::{play_player, wait};
use anyhow::{Result, ensure};
use gamebuddy_services::{Store, screen};
use std::{
    thread,
    time::{Duration, Instant},
};
pub fn run(store: &Store, log: &str) -> Result<()> {
    let id: String = store.get("active_guide")?.unwrap();
    let guide = store.guide(&id)?;
    let before_progress = serde_json::to_value(store.progress(&guide)?)?;
    for n in 0..8 {
        let before = std::fs::read_to_string(log)?.len();
        play_player(&std::env::var("GAMEBUDDY_TEST_ROUTINE_WAV")?)?;
        let end = screen::now_ms();
        wait("routine question reaches actual prepared playback", || {
            let text = std::fs::read_to_string(log)?;
            Ok(text[before..].contains("GAMEBUDDY_VOICE_PLAYBACK started"))
        })?;
        let text = std::fs::read_to_string(log)?;
        ensure!(
            !text[before..].contains("GAMEBUDDY_VOICE_DISPATCH "),
            "Routine request used full reasoning"
        );
        let at = text[before..]
            .lines()
            .find_map(|l| {
                l.split_once("GAMEBUDDY_VOICE_PLAYBACK started at_ms=")
                    .and_then(|(_, v)| v.parse::<u64>().ok())
            })
            .unwrap();
        println!(
            "GAMEBUDDY_ROUTINE_SAMPLE attempt={n} speech_end_ms={end} playback_ms={at} elapsed_ms={}",
            at.saturating_sub(end)
        );
        let deadline = Instant::now() + Duration::from_secs(25);
        loop {
            let text = std::fs::read_to_string(log)?;
            if text[before..]
                .lines()
                .rev()
                .find(|l| l.contains("GAMEBUDDY_REALTIME_STATUS "))
                .is_some_and(|l| l.ends_with("Live voice · speak naturally"))
            {
                break;
            }
            ensure!(Instant::now() < deadline, "Routine speech did not finish");
            thread::sleep(Duration::from_millis(100));
        }
        thread::sleep(Duration::from_millis(1400));
    }
    ensure!(
        serde_json::to_value(store.progress(&guide)?)? == before_progress,
        "Asking for advice changed progress"
    );
    println!(
        "GAMEBUDDY_ROUTINE_PASS eight requests, no full reasoning calls, no progress mutations"
    );
    Ok(())
}
