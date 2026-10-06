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
    let mut unchanged = Instant::now();
    let mut previous = String::new();
    wait("native follow-up finishes speaking", || {
        let text = std::fs::read_to_string(log)?;
        let activity = text
            .lines()
            .rev()
            .find(|line| {
                line.contains("GAMEBUDDY_VOICE_PLAYBACK started")
                    || line.contains("\"type\":\"caption\"")
            })
            .unwrap_or_default();
        if activity != previous {
            previous = activity.to_owned();
            unchanged = Instant::now();
        }
        Ok(unchanged.elapsed() >= Duration::from_secs(2)
            && text
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
    ensure!(played >= end, "Clarification overlapped the preceding cue");
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
    ensure!(
        std::fs::read_to_string(log)?
            .lines()
            .rev()
            .find(|line| line.contains("GAMEBUDDY_COMPANION_THINKING "))
            .is_some_and(|line| line.ends_with("true")),
        "Compact thinking indicator is missing during backend work"
    );
    let presence_before = std::fs::read_to_string(log)?.len();
    let presence_start = gamebuddy_services::screen::now_ms();
    play_player(&std::env::var("GAMEBUDDY_TEST_PRESENCE_WAV")?)?;
    let presence_end = gamebuddy_services::screen::now_ms();
    wait(
        "hello is answered while the original tool is still pending",
        || {
            let text = std::fs::read_to_string(log)?;
            let tail = text[presence_before..].to_lowercase();
            let captions = text[presence_before..]
                .lines()
                .filter_map(|line| {
                    let (_, json) = line.split_once("GAMEBUDDY_VOICE_EVENT ")?;
                    let event: serde_json::Value = serde_json::from_str(json).ok()?;
                    (event["type"] == "caption")
                        .then(|| event["value"].as_str().unwrap_or_default().to_lowercase())
                })
                .collect::<Vec<_>>()
                .join(" ");
            Ok(tail.contains("gamebuddy_voice_playback started")
                && ["here", "hear you", "listening", "with you"]
                    .iter()
                    .any(|word| captions.contains(word)))
        },
    )?;
    let text = std::fs::read_to_string(log)?;
    let onset = text[presence_before..]
        .lines()
        .find_map(|line| {
            line.split_once("GAMEBUDDY_VOICE_PLAYBACK started at_ms=")
                .and_then(|(_, s)| s.parse::<u64>().ok())
        })
        .context("presence playback")?;
    ensure!(
        onset >= presence_start && onset.saturating_sub(presence_end) < 4000,
        "Hello waited behind the pending tool"
    );
    ensure!(
        !std::fs::read_to_string(&gate)?.contains("\"released\""),
        "Presence was only answered after the tool completed"
    );
    println!(
        "GAMEBUDDY_NATIVE_PRESENCE_PASS latency_ms={} original_tool_pending=true",
        onset as i64 - presence_end as i64
    );
    thread::sleep(Duration::from_secs(2));
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
    ensure!(
        text.lines()
            .rev()
            .find(|line| line.contains("GAMEBUDDY_COMPANION_THINKING "))
            .is_some_and(|line| line.ends_with("false")),
        "Compact thinking indicator did not clear after the reply"
    );
    println!(
        "GAMEBUDDY_NATIVE_CONTINUATION_PASS elapsed_seconds={:.1} one_backend_turn=true two_distinct_widgets=true",
        end.elapsed().as_secs_f32()
    );
    Ok(())
}
