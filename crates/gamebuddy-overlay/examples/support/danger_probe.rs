//! Native spoken warning from a source route, followed by a routine question.
use super::{play_player, wait};
use anyhow::{Context, Result, ensure};
use gamebuddy_services::{Store, harness, screen};
use serde_json::json;
use std::time::{Duration, Instant};

fn quiet(log: &str) -> Result<()> {
    let mut last = String::new();
    let mut changed = Instant::now();
    wait("danger cue finishes", || {
        let text = std::fs::read_to_string(log)?;
        let event = text
            .lines()
            .rev()
            .find(|l| {
                l.contains("GAMEBUDDY_VOICE_PLAYBACK started") || l.contains("\"type\":\"caption\"")
            })
            .unwrap_or_default();
        if event != last {
            last = event.into();
            changed = Instant::now();
        }
        Ok(changed.elapsed() >= Duration::from_secs(3)
            && text
                .lines()
                .rev()
                .find(|l| l.contains("GAMEBUDDY_REALTIME_STATUS "))
                .is_some_and(|l| l.ends_with("Live voice · speak naturally")))
    })
}

pub fn run(store: &Store, log: &str) -> Result<()> {
    ensure!(
        store
            .root
            .to_string_lossy()
            .starts_with("/tmp/gamebuddy-realtime-test-"),
        "Private fixture required"
    );
    let id: String = store.get("active_guide")?.context("Fixture guide")?;
    let original = store.guide(&id)?;
    let mut guide = gamebuddy_core::import_markdown(
        &original.game,
        "Imminent danger fixture",
        "# Open the tower door\nUse the key at the tower door.\n# Defeat the guardian\nFight inside, then collect the reward.\n## Fight the guardian\nAttack its exposed head after it lands. Dodge fireballs; your shield does not stop magic.\n## Claim the reward\nCollect the reward.\n# Speak to the elder\nTalk to the elder.",
    )?;
    guide.id = original.id;
    guide.revision = original.revision + 1;
    store.save_guide(&guide)?;
    store.put("active_guide", &guide.id)?;
    let before = std::fs::read_to_string(log)?.len();
    let started = screen::now_ms();
    harness::call_tool(
        store,
        "set_next_action",
        json!({"expected_revision":guide.revision,"epoch":0,
        "step_id":guide.steps[0].id,"instructions":guide.steps[0].instructions,"screen_direction":null}),
    )?;
    wait("source danger becomes audible before entering", || {
        let text = std::fs::read_to_string(log)?;
        let tail = text[before..].to_lowercase();
        Ok(tail.contains("gamebuddy_voice_playback started") && tail.contains("guardian"))
    })?;
    quiet(log)?;
    let text = std::fs::read_to_string(log)?;
    let tail = &text[before..];
    ensure!(
        !tail.contains("GAMEBUDDY_VOICE_DISPATCH "),
        "Danger waited for a reasoning request"
    );
    let at = tail
        .lines()
        .find_map(|l| {
            l.split_once("GAMEBUDDY_VOICE_PLAYBACK started at_ms=")
                .and_then(|(_, at)| at.parse::<u64>().ok())
        })
        .context("Danger playback time")?;
    ensure!(
        tail.to_lowercase().contains("shield"),
        "Warning lost its protection limit"
    );
    ensure!(
        tail.to_lowercase().contains("head"),
        "Warning lost the boss tactic"
    );
    println!(
        "GAMEBUDDY_ROUTE_DANGER_PASS latency_ms={} no_backend=true start_ms={} end_ms={}",
        at.saturating_sub(started),
        started,
        screen::now_ms()
    );
    let before = text.len();
    play_player(&std::env::var("GAMEBUDDY_TEST_ROUTINE_WAV")?)?;
    let end = screen::now_ms();
    wait("Now what receives routine advice", || {
        Ok(std::fs::read_to_string(log)?[before..].contains("GAMEBUDDY_VOICE_PLAYBACK started"))
    })?;
    quiet(log)?;
    let text = std::fs::read_to_string(log)?;
    let tail = &text[before..];
    ensure!(
        !tail.contains("GAMEBUDDY_VOICE_DISPATCH "),
        "Routine question started full reasoning"
    );
    let at = tail
        .lines()
        .find_map(|l| {
            l.split_once("GAMEBUDDY_VOICE_PLAYBACK started at_ms=")
                .and_then(|(_, at)| at.parse::<u64>().ok())
        })
        .context("Routine playback time")?;
    ensure!(
        tail.to_lowercase().contains("key"),
        "Routine answer did not give the door instruction"
    );
    ensure!(
        store.progress(&guide)?.completed.is_empty(),
        "Warning inferred completion"
    );
    println!(
        "GAMEBUDDY_ROUTE_ROUTINE_PASS latency_ms={} no_backend=true",
        at.saturating_sub(end)
    );
    Ok(())
}
