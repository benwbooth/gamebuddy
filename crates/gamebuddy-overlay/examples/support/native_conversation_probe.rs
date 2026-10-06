//! Recorded subscription speech-to-speech, with a delegated screenshot action.
use super::{play_player, wait};
use anyhow::{Context, Result, ensure};
use gamebuddy_services::{Store, screen};
use std::{
    thread,
    time::{Duration, Instant},
};

fn quiet(log: &str) -> Result<()> {
    wait("native conversation returns to listening", || {
        Ok(std::fs::read_to_string(log)?
            .lines()
            .rev()
            .find(|line| line.contains("GAMEBUDDY_REALTIME_STATUS "))
            .is_some_and(|line| line.ends_with("Live voice · speak naturally")))
    })
}

fn audio(log: &str, before: usize) -> Result<()> {
    wait("direct native reply is audible", || {
        Ok(std::fs::read_to_string(log)?[before..].contains("GAMEBUDDY_VOICE_PLAYBACK started"))
    })
}

pub fn run(store: &Store, log: &str) -> Result<()> {
    let before = std::fs::read_to_string(log)?.len();
    play_player(&std::env::var("GAMEBUDDY_TEST_SOCIAL_WAV")?)?;
    let ended = screen::now_ms();
    audio(log, before)?;
    let text = std::fs::read_to_string(log)?;
    let played = text[before..]
        .lines()
        .find_map(|line| {
            line.split_once("GAMEBUDDY_VOICE_PLAYBACK started at_ms=")
                .and_then(|(_, time)| time.parse::<u64>().ok())
        })
        .context("native playback onset")?;
    quiet(log)?;
    let text = std::fs::read_to_string(log)?;
    ensure!(
        !text[before..].contains("GAMEBUDDY_VOICE_DISPATCH "),
        "Casual conversation used a text backend"
    );
    ensure!(
        !text[before..].contains("\"type\":\"speech-ready\""),
        "Casual reply was supplied as text to speech"
    );
    println!(
        "GAMEBUDDY_NATIVE_SOCIAL_PASS first_audio_relative_to_request_end_ms={}",
        played as i64 - ended as i64
    );
    thread::sleep(Duration::from_secs(2));

    let before = std::fs::read_to_string(log)?.len();
    play_player(&std::env::var("GAMEBUDDY_TEST_STORY_WAV")?)?;
    audio(log, before)?;
    thread::sleep(Duration::from_secs(4));
    ensure!(
        !std::fs::read_to_string(log)?[before..].contains("GAMEBUDDY_VOICE_DISPATCH "),
        "Story used the text backend"
    );
    let interrupted = std::fs::read_to_string(log)?.len();
    play_player(&std::env::var("GAMEBUDDY_TEST_INTERRUPT_WAV")?)?;
    let end = Instant::now();
    wait("native voice delegates the screenshot request", || {
        Ok(std::fs::read_to_string(log)?[interrupted..].contains("GAMEBUDDY_VOICE_DISPATCH "))
    })?;
    // Acknowledging the voice while its backend works must not restart tools.
    play_player(&std::env::var("GAMEBUDDY_TEST_BACKCHANNEL_WAV")?)?;
    wait("delegated screenshot request updates its widget", || {
        let id: String = store.get("active_guide")?.context("guide")?;
        Ok(
            gamebuddy_services::harness::widgets(store, &store.guide(&id)?)?
                .iter()
                .any(|w| {
                    w.title.eq_ignore_ascii_case("Interruption Works")
                        && serde_json::to_string(&w.content)
                            .unwrap_or_default()
                            .to_lowercase()
                            .contains("green")
                }),
        )
    })?;
    quiet(log)?;
    let text = std::fs::read_to_string(log)?;
    ensure!(
        text[interrupted..]
            .matches("GAMEBUDDY_VOICE_DISPATCH ")
            .count()
            == 1,
        "A backchannel restarted the delegated request"
    );
    ensure!(
        !text[interrupted..].contains("silver lantern glows"),
        "Interrupted story resumed over the game answer"
    );
    println!(
        "GAMEBUDDY_NATIVE_DELEGATION_PASS elapsed_seconds={:.1} one_backend_request=true",
        end.elapsed().as_secs_f32()
    );
    thread::sleep(Duration::from_secs(2));

    let before = std::fs::read_to_string(log)?.len();
    play_player(&std::env::var("GAMEBUDDY_TEST_STORY_WAV")?)?;
    audio(log, before)?;
    thread::sleep(Duration::from_secs(3));
    play_player(&std::env::var("GAMEBUDDY_TEST_STOP_WAV")?)?;
    let stopped = Instant::now();
    quiet(log)?;
    ensure!(
        stopped.elapsed() < Duration::from_secs(5),
        "Native stop was too slow"
    );
    println!(
        "GAMEBUDDY_NATIVE_STOP_PASS elapsed_ms={}",
        stopped.elapsed().as_millis()
    );
    Ok(())
}
