//! Real native clarification plus a follow-up while a real tool is pending.
use super::{play_player, wait};
use anyhow::{Context, Result, ensure};
use gamebuddy_services::Store;
use std::{
    process::Command,
    thread,
    time::{Duration, Instant},
};

fn quiet(log: &str) -> Result<()> {
    wait("native follow-up finishes speaking", || {
        Ok(std::fs::read_to_string(log)?
            .lines()
            .rev()
            .find(|line| line.contains("GAMEBUDDY_REALTIME_STATUS "))
            .is_some_and(|line| line.ends_with("Live voice · speak naturally")))
    })
}

pub fn run(store: &Store, log: &str) -> Result<()> {
    let before = std::fs::read_to_string(log)?.len();
    ensure!(
        Command::new("node")
            .arg("scripts/seed-voice-followup.mjs")
            .status()?
            .success(),
        "Could not seed the verified coaching cue"
    );
    wait("green-passage cue becomes audible", || {
        Ok(std::fs::read_to_string(log)?[before..].contains("GAMEBUDDY_VOICE_PLAYBACK started"))
    })?;
    // Let the verified number reach the player, then ask the reported question.
    quiet(log)?;
    let before = std::fs::read_to_string(log)?.len();
    play_player(&std::env::var("GAMEBUDDY_TEST_EXPERIENCE_WAV")?)?;
    let end = gamebuddy_services::screen::now_ms();
    wait("experience clarification becomes audible", || {
        Ok(std::fs::read_to_string(log)?[before..].contains("GAMEBUDDY_VOICE_PLAYBACK started"))
    })?;
    quiet(log)?;
    let text = std::fs::read_to_string(log)?;
    let tail = &text[before..];
    ensure!(
        !tail.contains("GAMEBUDDY_VOICE_DISPATCH "),
        "Restating the verified target started another reasoning request"
    );
    let played = tail
        .lines()
        .find_map(|line| {
            line.split_once("GAMEBUDDY_VOICE_PLAYBACK started at_ms=")
                .and_then(|(_, s)| s.parse::<u64>().ok())
        })
        .context("clarification playback")?;
    ensure!(
        tail.contains("twenty-two hundred")
            || tail.contains("twenty two hundred")
            || tail.contains("two thousand two hundred")
            || tail.contains("2,200")
            || tail.contains("2200"),
        "Clarification lost the experience target"
    );
    println!(
        "GAMEBUDDY_NATIVE_CLARIFICATION_PASS latency_ms={} no_backend=true start_ms={} end_ms={}",
        played as i64 - end as i64,
        end,
        gamebuddy_services::screen::now_ms()
    );
    thread::sleep(Duration::from_secs(2));

    let before = std::fs::read_to_string(log)?.len();
    play_player(&std::env::var("GAMEBUDDY_TEST_VOICE_WAV")?)?;
    let gate = store
        .root
        .parent()
        .context("fixture root")?
        .join("tool-delay.jsonl");
    wait("real tool response enters the controlled delay", || {
        Ok(std::fs::read_to_string(&gate)
            .unwrap_or_default()
            .contains("\"held\""))
    })?;
    play_player(&std::env::var("GAMEBUDDY_TEST_FOLLOWUP_WAV")?)?;
    let end = Instant::now();
    let guide = store.guide(&store.get::<String>("active_guide")?.context("guide")?)?;
    wait("both original and follow-up widgets exist", || {
        let widgets = gamebuddy_services::harness::widgets(store, &guide)?;
        Ok([("Passage Check", "green"), ("Followup Works", "red")]
            .iter()
            .all(|(title, color)| {
                widgets.iter().any(|w| {
                    w.title
                        .to_lowercase()
                        .replace('-', "")
                        .starts_with(&title.to_lowercase())
                        && serde_json::to_string(&w.content)
                            .unwrap_or_default()
                            .to_lowercase()
                            .contains(color)
                })
            }))
    })?;
    wait("continued backend delivers its answer", || {
        let text = std::fs::read_to_string(log)?;
        let tail = &text[before..];
        Ok(
            tail.contains("GAMEBUDDY_VOICE_CONTEXT_ACCEPTED success=true")
                && tail.contains("GAMEBUDDY_VOICE_PLAYBACK started"),
        )
    })?;
    quiet(log)?;
    let text = std::fs::read_to_string(log)?;
    let tail = &text[before..];
    ensure!(
        tail.matches("GAMEBUDDY_VOICE_DISPATCH ").count() == 1,
        "Follow-up restarted the backend"
    );
    ensure!(
        !tail.contains("GAMEBUDDY_VOICE_BACKEND_INTERRUPT"),
        "Follow-up cancelled work already in progress"
    );
    ensure!(
        tail.matches("GAMEBUDDY_VOICE_TOOL name=show_widget success=true")
            .count()
            == 2,
        "Follow-up replayed or omitted a widget action"
    );
    println!(
        "GAMEBUDDY_NATIVE_CONTINUATION_PASS elapsed_seconds={:.1} one_backend_turn=true two_distinct_widgets=true",
        end.elapsed().as_secs_f32()
    );
    Ok(())
}
