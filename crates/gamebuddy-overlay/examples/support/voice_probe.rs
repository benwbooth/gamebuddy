//! Real virtual-source -> Qt PCM -> WebRTC VAD -> Whisper -> overlay command.
use anyhow::{Context, Result, ensure};
use gamebuddy_overlay::x11::cardinal;
use std::{
    path::Path,
    process::Command,
    thread,
    time::{Duration, Instant},
};
use x11rb::rust_connection::RustConnection;

#[path = "progress_probe.rs"]
mod progress_probe;
#[path = "responsiveness_probe.rs"]
mod responsiveness_probe;
#[path = "work_progress_probe.rs"]
mod work_progress_probe;

fn pactl(args: &[&str]) -> Result<String> {
    let output = Command::new("pactl").args(args).output()?;
    ensure!(
        output.status.success(),
        "pactl failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(String::from_utf8(output.stdout)?)
}
// Volume-control applications open peak-meter streams on every source. They
// carry level samples, not microphone audio, and outlive the application capture.
fn records_source(stream: &serde_json::Value, source_index: u64) -> bool {
    stream["source"].as_u64() == Some(source_index)
        && stream["corked"] == false
        && stream["resample_method"] != "peaks"
        && stream["properties"]["resample.peaks"] != "true"
}

fn wait(label: &str, mut check: impl FnMut() -> Result<bool>) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(35);
    while !check()? {
        ensure!(Instant::now() < deadline, "Timed out: {label}");
        thread::sleep(Duration::from_millis(100));
    }
    Ok(())
}
pub fn run(
    conn: &RustConnection,
    game: u32,
    overlay: u32,
    focus: u32,
    directory: &Path,
    screenshot: impl Fn(&str) -> Result<()>,
) -> Result<()> {
    if let Ok(phase) = std::env::var("GAMEBUDDY_TEST_MEMORY") {
        return memory(&phase);
    }
    if std::env::var_os("GAMEBUDDY_TEST_REALTIME").is_some() {
        return realtime(conn, game, overlay, focus, screenshot);
    }
    // All reasoning requests stay in this local fixture, including a misheard command.
    let server =
        tiny_http::Server::http("127.0.0.1:0").map_err(|e| anyhow::anyhow!(e.to_string()))?;
    let data = std::env::var("GAMEBUDDY_TEST_DATA")?;
    let store = gamebuddy_services::Store::open(&data)?;
    let mut settings = gamebuddy_services::Settings::default();
    settings.reasoning.endpoint = format!("http://{}", server.server_addr());
    settings.reasoning.model = "voice-test".into();
    store.put("settings", &settings)?;
    let source = std::env::var("GAMEBUDDY_TEST_MIC_SOURCE")?;
    ensure!(
        source.starts_with("gamebuddy_test_"),
        "Only mutate a dedicated test microphone"
    );
    let sink = std::env::var("GAMEBUDDY_TEST_MIC_SINK")?;
    ensure!(
        sink.starts_with("gamebuddy_test_"),
        "Only play into a dedicated test sink"
    );
    let source_index =
        serde_json::from_str::<serde_json::Value>(&pactl(&["-f", "json", "list", "sources"])?)?
            .as_array()
            .context("source list")?
            .iter()
            .find(|s| s["name"] == source)
            .context("test microphone missing")?["index"]
            .as_u64()
            .context("source index")?;
    let capturing = || -> Result<bool> {
        let outputs: serde_json::Value =
            serde_json::from_str(&pactl(&["-f", "json", "list", "source-outputs"])?)?;
        Ok(outputs
            .as_array()
            .context("capture list")?
            .iter()
            .any(|s| records_source(s, source_index)))
    };
    ensure!(!capturing()?, "Mic captured before an unmute gesture");
    pactl(&["set-source-mute", &source, "0"])?;
    wait("unmute starts capture", &capturing)?;
    ensure!(
        cardinal(conn, overlay, focus)? == 0,
        "Unmute stole game focus"
    );
    screenshot("voice-listening.png")?;
    thread::sleep(Duration::from_secs(2));
    ensure!(
        cardinal(conn, overlay, focus)? == 0,
        "Silence changed overlay focus"
    );
    pactl(&["set-source-mute", &source, "1"])?;
    wait("mute closes capture", || Ok(!capturing()?))?;
    screenshot("voice-muted.png")?;
    pactl(&["set-source-mute", &source, "0"])?;
    wait("second unmute restarts capture", &capturing)?;
    thread::sleep(Duration::from_millis(400));
    let wave = std::env::var("GAMEBUDDY_TEST_VOICE_WAV")?;
    let result = Command::new("paplay")
        .args(["--device", &sink, &wave])
        .status()?;
    ensure!(result.success(), "Could not feed synthetic speech");
    wait("spoken open-guide command focuses overlay", || {
        Ok(cardinal(conn, overlay, focus)? == 1)
    })?;
    thread::sleep(Duration::from_millis(600));
    screenshot("voice-guide.png")?;
    ensure!(
        image::open(directory.join("voice-guide.png"))?
            .to_rgb8()
            .get_pixel(500, 500)
            .0
            != [20, 40, 80],
        "Spoken guide command took focus without rendering the guide"
    );
    let control = std::env::var("GAMEBUDDY_CONTROL_SOCKET")?;
    gamebuddy_overlay::control::send(Path::new(&control), "release")?;
    wait("release after spoken command", || {
        Ok(cardinal(conn, overlay, focus)? == 0)
    })?;
    wait("listening resumes after transcription", &capturing)?;
    // A delayed local reasoning fixture checks the question path without network AI.
    let question = std::env::var("GAMEBUDDY_TEST_QUESTION_WAV")?;
    ensure!(
        Command::new("paplay")
            .args(["--device", &sink, &question])
            .status()?
            .success(),
        "Question playback failed"
    );
    let request = server
        .recv_timeout(Duration::from_secs(35))?
        .context("Recognized question never reached the reasoning provider")?;
    ensure!(
        !capturing()?,
        "Microphone kept capturing while the voice turn was processing"
    );
    ensure!(
        cardinal(conn, overlay, focus)? == 0,
        "Ordinary question stole focus"
    );
    thread::sleep(Duration::from_millis(500));
    let answer =
        serde_json::json!({"answer":"Voice integration test complete.","widgets":[]}).to_string();
    request.respond(tiny_http::Response::from_string(
        serde_json::json!({"message":{"content":answer}}).to_string(),
    ))?;
    let guide_id: String = store.get("active_guide")?.context("active guide")?;
    let guide = store.guide(&guide_id)?;
    wait("coach answer persisted", || {
        Ok(!store
            .conversation_messages(&guide.game, "Voice integration test complete", None, 1)?
            .is_empty())
    })?;
    ensure!(
        cardinal(conn, overlay, focus)? == 0,
        "Reply stole game focus"
    );
    screenshot("voice-answer.png")?;
    wait("listening resumes after the reply", &capturing)?;
    // Exercise the real Stop listening control with Hands-free still enabled.
    // The fixture is confined to Gamescope's private X server by its caller.
    {
        use x11rb::{
            connection::Connection,
            protocol::{xproto::*, xtest::ConnectionExt as _},
        };
        let root = conn.get_geometry(overlay)?.reply()?.root;
        gamebuddy_overlay::control::send(Path::new(&control), "focus")?;
        wait("expand voice controls", || {
            Ok(cardinal(conn, overlay, focus)? == 1)
        })?;
        thread::sleep(Duration::from_millis(400));
        screenshot("voice-hands-free.png")?;
        // Companion at (36,90); the Listen/Stop listening row is at y=300.
        conn.xtest_fake_input(MOTION_NOTIFY_EVENT, 0, 0, root, 180, 300, 0)?
            .check()?;
        conn.flush()?;
        thread::sleep(Duration::from_millis(200));
        conn.xtest_fake_input(BUTTON_PRESS_EVENT, 1, 0, root, 0, 0, 0)?
            .check()?;
        conn.xtest_fake_input(BUTTON_RELEASE_EVENT, 1, 0, root, 0, 0, 0)?
            .check()?;
        conn.flush()?;
        wait("Stop listening closes capture", || Ok(!capturing()?))?;
        thread::sleep(Duration::from_secs(2));
        ensure!(!capturing()?, "Hands-free defeated Stop listening");
        screenshot("voice-stopped.png")?;
        gamebuddy_overlay::control::send(Path::new(&control), "release")?;
        pactl(&["set-source-mute", &source, "1"])?;
        // Allow the asynchronous mute monitor to observe the gesture.
        thread::sleep(Duration::from_millis(600));
        pactl(&["set-source-mute", &source, "0"])?;
        wait("explicit unmute resumes after Stop listening", &capturing)?;
    }
    pactl(&["set-source-mute", &source, "1"])?;
    wait("final mute closes capture", || Ok(!capturing()?))?;
    ensure!(
        std::fs::read_dir(Path::new(&data).join("voice"))?
            .next()
            .is_none(),
        "Processed voice recording leaked"
    );
    println!(
        "GAMEBUDDY_VOICE_TEST_PASS muted startup, unmute/capture, passive focus, silence, mute/close, real VAD/Whisper guide command, delayed provider question/reply, resume, Stop listening stays stopped, explicit unmute resumes, ephemeral audio"
    );
    Ok(())
}

/// Two separate application launches, with actual subscription WebRTC audio.
/// The second request deliberately does not contain the phrase to be recalled.
fn memory(phase: &str) -> Result<()> {
    let sink = std::env::var("GAMEBUDDY_TEST_MIC_SINK")?;
    ensure!(
        sink.starts_with("gamebuddy_test_"),
        "Private test audio required"
    );
    let store = gamebuddy_services::Store::open(std::env::var("GAMEBUDDY_TEST_DATA")?)?;
    let log = std::env::var("GAMEBUDDY_TEST_LOG")?;
    wait("memory voice connects", || {
        Ok(std::fs::read_to_string(&log)?.contains("Live voice · speak naturally"))
    })?;
    wait("startup voice returns to waiting", || {
        // Wait for the bounded startup cue to finish (or expire) before the
        // recall question. Greeting output has its own realtime test.
        let text = std::fs::read_to_string(&log)?;
        Ok(text.rfind("GAMEBUDDY_VOICE_GATE playback-allowed false")
            > text.rfind("GAMEBUDDY_VOICE_GATE playback-allowed true"))
    })?;
    thread::sleep(Duration::from_secs(1)); // let the acoustic tail clear
    ensure!(
        Command::new("paplay")
            .args([
                "--device",
                &sink,
                &std::env::var("GAMEBUDDY_TEST_VOICE_WAV")?
            ])
            .status()?
            .success(),
        "Speech playback failed"
    );
    if phase == "save" {
        wait("user phrase committed", || {
            Ok(!store
                .conversation_messages("Faxanadu", "silver lantern", None, 10)?
                .iter()
                .filter(|m| m.role == "user")
                .collect::<Vec<_>>()
                .is_empty())
        })?;
        wait("spoken acknowledgement committed", || {
            Ok(store
                .conversation_messages("Faxanadu", "silver lantern", None, 10)?
                .iter()
                .any(|m| m.role == "assistant"))
        })?;
    } else {
        wait("recalled phrase appears in a new widget", || {
            let id: String = store.get("active_guide")?.context("guide")?;
            Ok(
                gamebuddy_services::harness::widgets(&store, &store.guide(&id)?)?
                    .iter()
                    .any(|w| {
                        w.title.eq_ignore_ascii_case("Memory Restored")
                            && serde_json::to_string(&w.content)
                                .unwrap_or_default()
                                .to_lowercase()
                                .contains("silver lantern")
                    }),
            )
        })?;
        let sessions: std::collections::HashSet<_> = store
            .conversation_messages("Faxanadu", "", None, 50)?
            .into_iter()
            .map(|m| m.session_id)
            .collect();
        ensure!(
            sessions.len() >= 2,
            "Test did not cross a fresh voice session"
        );
    }
    println!("GAMEBUDDY_MEMORY_TEST_PASS phase={phase}");
    Ok(())
}

fn realtime(
    conn: &RustConnection,
    game: u32,
    overlay: u32,
    focus: u32,
    screenshot: impl Fn(&str) -> Result<()>,
) -> Result<()> {
    let game_audio = DuckingAudio::new()?;
    let source = std::env::var("GAMEBUDDY_TEST_MIC_SOURCE")?;
    let sink = std::env::var("GAMEBUDDY_TEST_MIC_SINK")?;
    ensure!(
        source.starts_with("gamebuddy_test_") && sink.starts_with("gamebuddy_test_"),
        "Only use dedicated test devices"
    );
    let log = std::env::var("GAMEBUDDY_TEST_LOG")?;
    let sources: serde_json::Value =
        serde_json::from_str(&pactl(&["-f", "json", "list", "sources"])?)?;
    let source_index = sources
        .as_array()
        .context("sources")?
        .iter()
        .find(|s| s["name"] == source)
        .context("test source")?["index"]
        .as_u64()
        .context("source index")?;
    let capturing = || -> Result<bool> {
        let streams: serde_json::Value =
            serde_json::from_str(&pactl(&["-f", "json", "list", "source-outputs"])?)?;
        Ok(streams
            .as_array()
            .context("source outputs")?
            .iter()
            .any(|s| records_source(s, source_index)))
    };
    let store = gamebuddy_services::Store::open(std::env::var("GAMEBUDDY_TEST_DATA")?)?;
    let quiet = || -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            wait("voice returns to waiting for player speech", || {
                let text = std::fs::read_to_string(&log)?;
                Ok(
                    // A new request no longer opens playback before approval.
                    // A stop can therefore finish while it is already closed,
                    // without another playback-allowed transition to false.
                    text.lines()
                        .rev()
                        .find(|line| line.contains("GAMEBUDDY_REALTIME_STATUS "))
                        .is_some_and(|line| line.ends_with("Live voice · speak naturally")),
                )
            })?;
            let before = std::fs::read_to_string(&log)?;
            let permits = before
                .matches("GAMEBUDDY_VOICE_GATE playback-allowed true")
                .count()
                + before.matches("\"type\":\"prepared-start\"").count();
            thread::sleep(Duration::from_secs(15));
            let after = std::fs::read_to_string(&log)?;
            let next = after
                .matches("GAMEBUDDY_VOICE_GATE playback-allowed true")
                .count()
                + after.matches("\"type\":\"prepared-start\"").count();
            if next == permits {
                break;
            }
            // Startup perception can legitimately select the first gameplay cue
            // after the greeting. It is not unsolicited filler; start the quiet
            // interval after that selected cue finishes, without hiding extra audio.
            ensure!(
                after[before.len()..]
                    .matches("GAMEBUDDY_NARRATION characters=")
                    .count()
                    >= next - permits,
                "Voice started speaking again without player speech or a selected cue"
            );
            ensure!(
                Instant::now() < deadline,
                "Voice never settled after selected cues"
            );
        }
        println!("GAMEBUDDY_VOICE_WAIT_TEST_PASS 15 seconds of silence with live screen updates");
        Ok(())
    };
    wait("spoken introduction lowers game audio", || {
        // A short greeting may finish while PipeWire recreates the fixture's
        // stream. The production worker also reports a completed volume change.
        Ok(game_audio.ducked()?
            || std::fs::read_to_string(&log)?
                .contains("GAMEBUDDY_AUDIO Game audio lowered during conversation"))
    })?;
    println!("GAMEBUDDY_AUDIO_INTRO_PASS");
    wait("automatic spoken introduction", || {
        let text = std::fs::read_to_string(&log)?;
        Ok(text.contains("GAMEBUDDY_NARRATION")
            && text.contains("GAMEBUDDY_VOICE_PLAYBACK started"))
    })?;
    wait(
        "already-unmuted microphone opens automatically at launch",
        &capturing,
    )?;
    println!("GAMEBUDDY_HANDS_FREE_START_PASS no clicks or mute/unmute gestures");
    ensure!(
        cardinal(conn, overlay, focus)? == 0,
        "Introduction stole game focus"
    );
    quiet()?;
    wait("quiet play restores game audio", || {
        Ok(!game_audio.ducked()?)
    })?;
    if std::env::var_os("GAMEBUDDY_TEST_FAST_EVENT").is_some() {
        let before = std::fs::read_to_string(&log)?.len();
        play_player(&std::env::var("GAMEBUDDY_TEST_STORY_WAV")?)?;
        wait("reasoning begins before the new urgent event", || {
            Ok(std::fs::read_to_string(&log)?[before..].contains("GAMEBUDDY_VOICE_DISPATCH "))
        })?;
        let event_at = gamebuddy_services::screen::now_ms();
        draw_detector_text(conn, game)?;
        wait("local OCR confirms the changed screen", || {
            Ok(std::fs::read_to_string(&log)?[before..]
                .contains("GAMEBUDDY_FAST_EVENT event=death"))
        })?;
        wait("urgent screen event reaches playback", || {
            let text = std::fs::read_to_string(&log)?;
            Ok(text[before..].contains("The screen says game over.")
                && text[before..].contains("GAMEBUDDY_VOICE_PLAYBACK started"))
        })?;
        let text = std::fs::read_to_string(&log)?;
        let at = text[before..]
            .lines()
            .filter_map(|l| {
                l.split_once("GAMEBUDDY_VOICE_PLAYBACK started at_ms=")
                    .and_then(|(_, v)| v.parse::<u64>().ok())
            })
            .find(|at| *at >= event_at)
            .context("Urgent audio onset")?;
        println!(
            "GAMEBUDDY_FAST_EVENT_PASS displayed_ms={event_at} playback_ms={at} elapsed_ms={}",
            at - event_at
        );
        thread::sleep(Duration::from_secs(12));
        let text = std::fs::read_to_string(&log)?;
        ensure!(
            !text[before..].contains("silver lantern glows"),
            "Superseded story played after the urgent event"
        );
        ensure!(capturing()?, "Urgent interruption closed the microphone");
        screenshot("fast-game-over.png")?;
        return Ok(());
    }
    if std::env::var_os("GAMEBUDDY_TEST_ROUTINE").is_some() {
        responsiveness_probe::run(&store, &log)?;
        if std::env::var_os("GAMEBUDDY_TEST_EVENT_IMAGE").is_some() {
            let before = std::fs::read_to_string(&log)?.len();
            play_player(&std::env::var("GAMEBUDDY_TEST_ROUTINE_WAV")?)?;
            wait("instruction starts before a scene change", || {
                Ok(std::fs::read_to_string(&log)?[before..]
                    .contains("GAMEBUDDY_VOICE_PLAYBACK started"))
            })?;
            let changed = gamebuddy_services::screen::now_ms();
            draw_detector_text(conn, game)?;
            wait("changed view cancels playing advice", || {
                Ok(std::fs::read_to_string(&log)?[before..]
                    .contains("GAMEBUDDY_VOICE_SCOPE_EXPIRED"))
            })?;
            ensure!(
                gamebuddy_services::screen::now_ms() - changed < 1500,
                "Stale speech was not promptly cancelled"
            );
            ensure!(capturing()?, "Cancelling stale advice stopped capture");
            println!(
                "GAMEBUDDY_PLAYBACK_SCOPE_PASS elapsed_ms={}",
                gamebuddy_services::screen::now_ms() - changed
            );
        }
        return Ok(());
    }
    if std::env::var_os("GAMEBUDDY_TEST_WORKING").is_some() {
        work_progress_probe::run(&store, &log, quiet, capturing)?;
        let before = std::fs::read_to_string(&log)?.len();
        play_player(&std::env::var("GAMEBUDDY_TEST_PRESENCE_WAV")?)?;
        let end = gamebuddy_services::screen::now_ms();
        wait("local presence reply reaches playback", || {
            let text = std::fs::read_to_string(&log)?;
            Ok(text[before..].contains("GAMEBUDDY_VOICE_LOCAL reply=true")
                && text[before..].contains("GAMEBUDDY_VOICE_PLAYBACK started"))
        })?;
        println!(
            "GAMEBUDDY_LOCAL_REPLY_PASS speech_end_to_audio_ms={}",
            gamebuddy_services::screen::now_ms().saturating_sub(end)
        );
        quiet()?;
        play_player(&std::env::var("GAMEBUDDY_TEST_COMMAND_WAV")?)?;
        let end = gamebuddy_services::screen::now_ms();
        wait("voice opens the guide without reasoning", || {
            Ok(cardinal(conn, overlay, focus)? == 1)
        })?;
        let text = std::fs::read_to_string(&log)?;
        ensure!(
            text[before..].contains("GAMEBUDDY_VOICE_LOCAL action=7")
                && !text[before..].contains("GAMEBUDDY_VOICE_DISPATCH "),
            "Local commands went through the reasoning backend"
        );
        println!(
            "GAMEBUDDY_LOCAL_COMMAND_PASS speech_end_to_ui_ms={}",
            gamebuddy_services::screen::now_ms().saturating_sub(end)
        );
        screenshot("realtime-work-progress.png")?;
        return Ok(());
    }
    if std::env::var_os("GAMEBUDDY_TEST_PROGRESS").is_some() {
        progress_probe::run(&store, &log, quiet, capturing)?;
        screenshot("realtime-progress-recovery.png")?;
        return Ok(());
    }
    if std::env::var_os("GAMEBUDDY_TEST_SCENE").is_some() {
        let id: String = store.get("active_guide")?.context("Guide")?;
        let guide = store.guide(&id)?;
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            let state = gamebuddy_services::navigation::context(
                &store,
                &guide,
                &store.progress(&guide)?,
                &Default::default(),
            )?;
            if state["map"]["geometric_marker"].is_object() {
                break;
            }
            ensure!(
                Instant::now() < deadline,
                "Saved gate view never matched its atlas map"
            );
            thread::sleep(Duration::from_millis(200));
        }
        let key = format!("next-action:{id}");
        let mut old: serde_json::Value = store.get(&key)?.context("Prior recommendation")?;
        old["session"] = gamebuddy_services::screen::status(&store)?["session_id"].clone();
        old["instructions"] = serde_json::json!("Keep walking right along the corridor.");
        store.put(&key, &old)?;
        let before = std::fs::read_to_string(&log)?.len();
        play_player(&std::env::var("GAMEBUDDY_TEST_GATE_WAV")?)?;
        wait("gate advice selects the key interaction", || {
            let advice: serde_json::Value = store.get(&key)?.unwrap_or_default();
            Ok(advice["instructions"]
                .as_str()
                .is_some_and(|s| s.to_lowercase().contains("key")))
        })?;
        quiet()?;
        let tail = std::fs::read_to_string(&log)?[before..].to_string();
        ensure!(
            tail.contains("images=2"),
            "Gate reasoning did not receive both actual screen and map images"
        );
        ensure!(
            tail.contains("GAMEBUDDY_VOICE_PLAYBACK started"),
            "Gate answer was never played"
        );
        ensure!(
            tail.lines()
                .any(|l| l.contains("\"type\":\"caption\"") && l.to_lowercase().contains("key")),
            "Played captions did not mention the key"
        );
        screenshot("realtime-gate-context.png")?;
        println!(
            "GAMEBUDDY_GATE_VOICE_TEST_PASS actual captured gate + atlas images; old right advice replaced by key interaction; captions and audible reply"
        );
        return Ok(());
    }
    if std::env::var_os("GAMEBUDDY_TEST_SCREEN").is_some() {
        let id: String = store.get("active_guide")?.context("guide")?;
        ensure!(
            store.index(&store.guide(&id)?)?.is_none(),
            "Screen test must have no reference index"
        );
        wait(
            "automatic visual observation without a player request",
            || {
                let status = gamebuddy_services::screen::status(&store)?;
                let summary = status["scene"]["summary"]
                    .as_str()
                    .unwrap_or_default()
                    .to_lowercase();
                Ok(status["status"] == "live"
                    && status["scene"]["fresh"] == true
                    && summary.contains("red")
                    && summary.contains("green"))
            },
        )?;
        let scene = gamebuddy_services::screen::status(&store)?;
        println!("GAMEBUDDY_SCREEN_WATCH_TEST_PASS {}", scene["scene"]);
        let output = gamebuddy_services::harness::call_tool_with_images(
            &store,
            "get_game_screen",
            serde_json::json!({}),
        )?;
        ensure!(
            output.images.len() == 1 && output.value["source"] == "game_window",
            "Screen tool has no game-only image"
        );
    }
    wait("subscription WebRTC connects", || {
        Ok(std::fs::read_to_string(&log)?.contains("Live voice · speak naturally"))
    })?;
    ensure!(
        capturing()?,
        "WebRTC did not capture the selected virtual microphone"
    );
    ensure!(
        cardinal(conn, overlay, focus)? == 0,
        "Live voice stole game focus (mode={})",
        cardinal(conn, overlay, focus)?
    );
    let wave = std::env::var("GAMEBUDDY_TEST_VOICE_WAV")?;
    thread::sleep(Duration::from_secs(2));
    ensure!(
        !game_audio.ducked()?,
        "An unmuted but silent mic lowered the game"
    );
    let request = thread::spawn(move || play_player(&wave));
    wait("microphone speech lowers game audio", || {
        game_audio.ducked()
    })?;
    println!("GAMEBUDDY_AUDIO_MIC_PASS");
    request
        .join()
        .map_err(|_| anyhow::anyhow!("Player audio worker failed"))??;
    let started = Instant::now();
    wait("spoken request reaches GameBuddy widget tool", || {
        let id: String = store.get("active_guide")?.context("active guide")?;
        Ok(
            gamebuddy_services::harness::widgets(&store, &store.guide(&id)?)?
                .iter()
                .any(|w| w.id != "compositor-test"),
        )
    })?;
    println!(
        "GAMEBUDDY_REALTIME_TOOL latency_seconds={:.1}",
        started.elapsed().as_secs_f32()
    );
    if std::env::var_os("GAMEBUDDY_TEST_SCREEN").is_some() {
        let log_text = std::fs::read_to_string(&log)?;
        ensure!(
            ["get_game_context", "get_game_screen"]
                .iter()
                .any(|name| log_text.contains(&format!(
                    "GAMEBUDDY_VOICE_TOOL name={name} success=true images=1"
                )))
                || log_text.contains("GAMEBUDDY_VOICE_CONTEXT images=1"),
            "Voice never inspected native screenshot content"
        );
        let id: String = store.get("active_guide")?.unwrap();
        let widgets = gamebuddy_services::harness::widgets(&store, &store.guide(&id)?)?;
        let found = widgets.iter().find(|w| w.id != "compositor-test").unwrap();
        let description = serde_json::to_string(&found.content)?.to_lowercase();
        ensure!(
            description.contains("red") && description.contains("green"),
            "Voice did not identify visible colors: {description}"
        );
        println!("GAMEBUDDY_SCREEN_VOICE_TEST_PASS {description}");
    }
    thread::sleep(Duration::from_secs(3)); // allow the OSD's regular store poll
    screenshot("realtime-widget.png")?;
    quiet()?;
    if std::env::var_os("GAMEBUDDY_TEST_LOCATION").is_some() {
        let id: String = store.get("active_guide")?.context("guide")?;
        let guide = store.guide(&id)?;
        let old = store.progress(&guide)?;
        gamebuddy_services::harness::call_tool(
            &store,
            "set_next_action",
            serde_json::json!({
                "expected_revision":guide.revision,"epoch":old.epoch,"step_id":"step-2",
                "instructions":"Walk left toward the restoration vendors."
            }),
        )?;
        thread::sleep(Duration::from_secs(3));
        play_player(&std::env::var("GAMEBUDDY_TEST_LOCATION_WAV")?)?;
        wait(
            "arrival correction changes both location and next action",
            || {
                let nav = gamebuddy_services::navigation::context(
                    &store,
                    &guide,
                    &store.progress(&guide)?,
                    &Default::default(),
                )?;
                Ok(nav["location"]["place_id"] == "vendors"
                    && nav["location"]["status"] == "reported"
                    && nav["recommendation"]["step_id"] == "step-3"
                    && !nav["recommendation"]["instructions"]
                        .as_str()
                        .unwrap_or_default()
                        .to_lowercase()
                        .starts_with("walk left"))
            },
        )?;
        ensure!(
            store.progress(&guide)?.completed == old.completed,
            "Location report manufactured verified completion"
        );
        screenshot("realtime-location-correction.png")?;
        quiet()?;
        println!(
            "GAMEBUDDY_LOCATION_VOICE_TEST_PASS explicit arrival replaces old walk-left advice and updates shared location"
        );
    }
    if std::env::var_os("GAMEBUDDY_TEST_CAPTIONS").is_some() {
        let before = std::fs::read_to_string(&log)?.len();
        play_player(&std::env::var("GAMEBUDDY_TEST_ONSET_WAV")?)?;
        wait("live captions arrive before the answer completes", || {
            let text = std::fs::read_to_string(&log)?;
            let tail = &text[before..];
            Ok(tail.contains("GAMEBUDDY_CAPTION_UPDATE") && tail.contains("playback=true"))
        })?;
        thread::sleep(Duration::from_millis(450));
        screenshot("realtime-live-captions.png")?;
        wait(
            "onset phrase is spoken (recognition may singularize elephant)",
            || {
                Ok(store
                    .conversation_messages("Faxanadu", "purple elephant", None, 10)?
                    .iter()
                    .any(|m| m.role == "assistant"))
            },
        )?;
        quiet()?;
        println!(
            "GAMEBUDDY_CAPTIONS_TEST_PASS subtitles follow the audio playback clock; verify Purple elephants in captured audio"
        );
    }
    if std::env::var_os("GAMEBUDDY_TEST_RESPONSIVE").is_some() {
        responsive(&store, &log)?;
        quiet()?;
    }
    pactl(&["set-source-mute", &source, "1"])?;
    wait("mute leaves guidance available", || {
        Ok(std::fs::read_to_string(&log)?
            .contains("GAMEBUDDY_REALTIME_STATUS Guide voice ready · mic off"))
    })?;
    wait("mute closes the WebRTC microphone", || Ok(!capturing()?)).with_context(|| {
        let streams = pactl(&["-f", "json", "list", "source-outputs"])
            .ok().and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok());
        let remaining: Vec<_> = streams.as_ref().and_then(|s| s.as_array()).into_iter().flatten()
            .filter(|s| s["source"].as_u64() == Some(source_index))
            .map(|s| serde_json::json!({"index":s["index"],"corked":s["corked"],"application":s["properties"]["application.name"],"pid":s["properties"]["application.process.id"]})).collect();
        format!("Remaining test-microphone streams: {remaining:?}")
    })?;
    if std::env::var_os("GAMEBUDDY_TEST_SCREEN").is_some() {
        let before = std::fs::read_to_string(&log)?
            .matches("GAMEBUDDY_NARRATION")
            .count();
        draw_game_over(conn, game)?;
        // Automatic event cues have a 90-second interval and a post-conversation
        // quiet period; background observations must remain available meanwhile.
        let deadline = Instant::now() + Duration::from_secs(110);
        loop {
            let state = gamebuddy_services::screen::status(&store)?;
            if state["scene"]["event"] == "death"
                && state["scene"]["fresh"] == true
                && std::fs::read_to_string(&log)?
                    .matches("GAMEBUDDY_NARRATION")
                    .count()
                    > before
            {
                break;
            }
            ensure!(
                Instant::now() < deadline,
                "Timed out waiting for the selected game-over cue"
            );
            thread::sleep(Duration::from_millis(200));
        }
        ensure!(
            !capturing()?,
            "Visual feedback must not activate the microphone"
        );
        println!(
            "GAMEBUDDY_SCREEN_EVENT_TEST_PASS {}",
            gamebuddy_services::screen::status(&store)?["scene"]
        );
        screenshot("automatic-game-over.png")?;
        thread::sleep(Duration::from_secs(10));
    }
    println!(
        "GAMEBUDDY_REALTIME_TEST_PASS hands-free startup without clicks or mute/unmute, automatic introduction, subscription WebRTC, synthetic microphone, validated widget tool, passive focus, mute closes capture and preserves guidance"
    );
    Ok(())
}

// The two speech backends share reply-pending and actual output-meter events.
// Native playback permission alone says nothing about local prepared audio.
fn reply_start(text: &str) -> Option<usize> {
    text.rfind("\"type\":\"speech-ready\"")
        .max(text.rfind("\"type\":\"prepared-start\""))
}
fn reply_open(text: &str) -> bool {
    text.rfind("\"type\":\"reply-pending\",\"value\":true")
        > text.rfind("\"type\":\"reply-pending\",\"value\":false")
}

/// Speak into the same echo-looped virtual microphone while a long reply is
/// playing, then let a second long reply finish. No direct transport shortcuts.
fn responsive(store: &gamebuddy_services::Store, log: &str) -> Result<()> {
    let story = std::env::var("GAMEBUDDY_TEST_STORY_WAV")?;
    let interrupt = std::env::var("GAMEBUDDY_TEST_INTERRUPT_WAV")?;
    let wait_story = |before: usize| -> Result<()> {
        // A short work update is not the requested story. Wait for the final
        // answer's speech-ready event and its subsequent actual playback.
        let deadline = Instant::now() + Duration::from_secs(90);
        loop {
            let text = std::fs::read_to_string(log)?;
            let tail = &text[before..];
            if reply_start(tail).is_some_and(|ready| {
                tail.rfind("GAMEBUDDY_VOICE_PLAYBACK started")
                    .is_some_and(|at| at > ready)
            }) {
                break;
            }
            ensure!(
                Instant::now() < deadline,
                "Completed long-form reply never started"
            );
            thread::sleep(Duration::from_millis(100));
        }
        Ok(())
    };
    let before = std::fs::read_to_string(log)?.len();
    play_player(&story)?;
    let started = Instant::now();
    wait_story(before)?;
    println!(
        "GAMEBUDDY_RESPONSIVE_STORY_START latency_seconds={:.1}",
        started.elapsed().as_secs_f32()
    );
    thread::sleep(Duration::from_secs(6));
    let text = std::fs::read_to_string(log)?;
    ensure!(
        reply_open(&text),
        "The story ended before the interruption test"
    );
    let interrupt_before = std::fs::read_to_string(log)?.len();
    play_player(&interrupt)?;
    let started = Instant::now();
    wait("interruption reaches the tool backend", || {
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
    println!(
        "GAMEBUDDY_RESPONSIVE_INTERRUPT_PASS latency_seconds={:.1}",
        started.elapsed().as_secs_f32()
    );
    wait("interrupted answer finishes", || {
        let text = std::fs::read_to_string(log)?;
        let tail = &text[interrupt_before..];
        Ok(reply_start(tail).is_some() && !reply_open(tail))
    })?;
    let before = std::fs::read_to_string(log)?.len();
    let last_message = store
        .conversation_messages("Faxanadu", "", None, 1)?
        .last()
        .map(|m| m.id)
        .unwrap_or(0);
    play_player(&story)?;
    let started = Instant::now();
    wait_story(before)?;
    let spoken_started = Instant::now();
    // The request is at least 250 words; a naturally paced five-paragraph
    // response can legitimately exceed a minute and a half.
    let deadline = started + Duration::from_secs(180);
    loop {
        if store
            .conversation_messages("Faxanadu", "silver lantern glows", None, 10)?
            .iter()
            .any(|m| m.id > last_message && m.role == "assistant")
            && !reply_open(&std::fs::read_to_string(log)?)
        {
            break;
        }
        ensure!(
            Instant::now() < deadline,
            "Long reply did not finish its final words"
        );
        thread::sleep(Duration::from_millis(100));
    }
    ensure!(
        spoken_started.elapsed() >= Duration::from_secs(22),
        "Reply was too short to exercise the former 20-second cutoff"
    );
    println!(
        "GAMEBUDDY_RESPONSIVE_LONG_REPLY_PASS duration_seconds={:.1}; verify final words in the recorded output",
        spoken_started.elapsed().as_secs_f32()
    );
    wait("long reply audio drains", || {
        let text = std::fs::read_to_string(log)?;
        Ok(!reply_open(&text))
    })?;
    let before = std::fs::read_to_string(log)?.len();
    play_player(&story)?;
    wait_story(before)?;
    thread::sleep(Duration::from_secs(3));
    let prior_input = std::fs::read_to_string(log)?
        .matches("\"type\":\"input-active\",\"value\":true")
        .count();
    play_player(&std::env::var("GAMEBUDDY_TEST_STOP_WAV")?)?;
    let stopped = Instant::now();
    wait(
        "one-word interruption silences speech and returns to waiting",
        || {
            let text = std::fs::read_to_string(log)?;
            Ok(text
                .matches("\"type\":\"input-active\",\"value\":true")
                .count()
                > prior_input
                && !reply_open(&text)
                && text.rfind("GAMEBUDDY_REALTIME_STATUS Live voice · speak naturally")
                    > text.rfind("GAMEBUDDY_REALTIME_STATUS Listening…"))
        },
    )?;
    ensure!(
        stopped.elapsed() < Duration::from_secs(5),
        "Short interruption took too long"
    );
    println!(
        "GAMEBUDDY_RESPONSIVE_SHORT_STOP_PASS latency_seconds={:.1}",
        stopped.elapsed().as_secs_f32()
    );
    Ok(())
}

fn play_player(path: &str) -> Result<()> {
    use std::io::{Read, Write};
    let mut stream =
        std::os::unix::net::UnixStream::connect(std::env::var("GAMEBUDDY_TEST_PLAY_SOCKET")?)?;
    stream.set_read_timeout(Some(Duration::from_secs(45)))?;
    writeln!(stream, "{}", serde_json::to_string(path)?)?;
    let mut result = String::new();
    stream.read_to_string(&mut result)?;
    ensure!(result == "ok", "Synthetic player speech failed: {result}");
    Ok(())
}

/// A silent audio stream belonging to the fixture game, on the test's private
/// output sink. Actual attenuation is measured by audio_duck_probe separately.
struct DuckingAudio(std::process::Child);
impl DuckingAudio {
    fn new() -> Result<Self> {
        use std::io::Write;
        let sink = std::env::var("PULSE_SINK")?;
        ensure!(
            sink.starts_with("gamebuddy_test_"),
            "Private output sink required"
        );
        let mut child = Command::new("paplay")
            .args([
                "--raw",
                "--rate=48000",
                "--channels=2",
                "--format=s16le",
                "--latency-msec=50",
                "--volume=65536",
                "--device",
                &sink,
            ])
            .stdin(std::process::Stdio::piped())
            .spawn()?;
        let mut input = child.stdin.take().unwrap();
        thread::spawn(move || while input.write_all(&[0; 19200]).is_ok() {});
        Ok(Self(child))
    }
    fn ducked(&self) -> Result<bool> {
        let output = Command::new("pw-dump").output()?;
        let objects: serde_json::Value = serde_json::from_slice(&output.stdout)?;
        let node = objects
            .as_array()
            .context("PipeWire objects")?
            .iter()
            .find(|o| {
                o["info"]["props"]["media.class"] == "Stream/Output/Audio"
                    && o["info"]["props"]["application.process.id"]
                        .as_u64()
                        .or_else(|| {
                            o["info"]["props"]["application.process.id"]
                                .as_str()?
                                .parse()
                                .ok()
                        })
                        == Some(self.0.id() as u64)
            });
        let props = node.map(|n| &n["info"]["params"]["Props"][0]);
        Ok(props
            .and_then(|p| {
                Some((
                    p["channelVolumes"][0].as_f64()?,
                    p["softVolumes"][0].as_f64()?,
                ))
            })
            .is_some_and(|(base, soft)| base > 0. && soft < base * 0.6))
    }
}
impl Drop for DuckingAudio {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Change actual game pixels after the conversation, with no model text hint.
fn draw_game_over(conn: &RustConnection, window: u32) -> Result<()> {
    use x11rb::{connection::Connection, protocol::xproto::*};
    let gc = conn.generate_id()?;
    conn.create_gc(gc, window, &CreateGCAux::new().foreground(0))?
        .check()?;
    conn.poly_fill_rectangle(
        window,
        gc,
        &[Rectangle {
            x: 0,
            y: 0,
            width: 1280,
            height: 720,
        }],
    )?
    .check()?;
    conn.change_gc(gc, &ChangeGCAux::new().foreground(0xffffff))?
        .check()?;
    let mut pixels = Vec::new();
    for (i, char) in "GAME OVER".chars().enumerate() {
        let rows = match char {
            'G' => [14, 17, 16, 23, 17, 17, 14],
            'A' => [14, 17, 17, 31, 17, 17, 17],
            'M' => [17, 27, 21, 21, 17, 17, 17],
            'E' => [31, 16, 16, 30, 16, 16, 31],
            'O' => [14, 17, 17, 17, 17, 17, 14],
            'V' => [17, 17, 17, 17, 17, 10, 4],
            'R' => [30, 17, 17, 30, 20, 18, 17],
            _ => [0; 7],
        };
        for (y, row) in rows.iter().enumerate() {
            for x in 0..5 {
                if row & (1 << (4 - x)) != 0 {
                    pixels.push(Rectangle {
                        x: (315 + i * 72 + x * 12) as i16,
                        y: (310 + y * 12) as i16,
                        width: 12,
                        height: 12,
                    });
                }
            }
        }
    }
    conn.poly_fill_rectangle(window, gc, &pixels)?.check()?;
    conn.free_gc(gc)?.check()?;
    conn.flush()?;
    Ok(())
}

/// A readable text fixture for the local OCR path. The existing block-font
/// game-over fixture remains a separate general-vision fallback regression.
fn draw_detector_text(conn: &RustConnection, window: u32) -> Result<()> {
    use x11rb::{connection::Connection, protocol::xproto::*};
    let image = image::open(std::env::var("GAMEBUDDY_TEST_EVENT_IMAGE")?)?;
    let geometry = conn.get_geometry(window)?.reply()?;
    let image = image
        .resize_exact(
            geometry.width.into(),
            geometry.height.into(),
            image::imageops::FilterType::Triangle,
        )
        .to_rgb8();
    let gc = conn.generate_id()?;
    conn.create_gc(gc, window, &CreateGCAux::new())?.check()?;
    let pixels: Vec<u8> = image.pixels().flat_map(|p| [p[2], p[1], p[0], 0]).collect();
    let stride = geometry.width as usize * 4;
    for (row, bytes) in pixels.chunks(stride * 16).enumerate() {
        conn.put_image(
            ImageFormat::Z_PIXMAP,
            window,
            gc,
            geometry.width,
            (bytes.len() / stride) as u16,
            0,
            (row * 16) as i16,
            0,
            geometry.depth,
            bytes,
        )?
        .check()?;
    }
    conn.free_gc(gc)?.check()?;
    conn.flush()?;
    Ok(())
}
