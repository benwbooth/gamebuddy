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

fn pactl(args: &[&str]) -> Result<String> {
    let output = Command::new("pactl").args(args).output()?;
    ensure!(
        output.status.success(),
        "pactl failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(String::from_utf8(output.stdout)?)
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
            .any(|s| s["source"].as_u64() == Some(source_index) && s["corked"] == false))
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
    let guide: String = store.get("active_guide")?.context("active guide")?;
    wait("coach answer persisted", || {
        let history = store.get::<serde_json::Value>(&format!("conversation:{guide}"))?;
        Ok(history.is_some_and(|h| h.to_string().contains("Voice integration test complete")))
    })?;
    ensure!(
        cardinal(conn, overlay, focus)? == 0,
        "Reply stole game focus"
    );
    screenshot("voice-answer.png")?;
    wait("listening resumes after the reply", &capturing)?;
    pactl(&["set-source-mute", &source, "1"])?;
    wait("final mute closes capture", || Ok(!capturing()?))?;
    ensure!(
        std::fs::read_dir(Path::new(&data).join("voice"))?
            .next()
            .is_none(),
        "Processed voice recording leaked"
    );
    println!(
        "GAMEBUDDY_VOICE_TEST_PASS unmute/capture, passive focus, silence, mute/close, real VAD/Whisper guide command, delayed provider question/reply, resume, ephemeral audio"
    );
    Ok(())
}

fn realtime(
    conn: &RustConnection,
    game: u32,
    overlay: u32,
    focus: u32,
    screenshot: impl Fn(&str) -> Result<()>,
) -> Result<()> {
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
            .any(|s| s["source"].as_u64() == Some(source_index) && s["corked"] == false))
    };
    let store = gamebuddy_services::Store::open(std::env::var("GAMEBUDDY_TEST_DATA")?)?;
    wait("automatic spoken introduction", || {
        let text = std::fs::read_to_string(&log)?;
        Ok(text.contains("GAMEBUDDY_NARRATION")
            && text.contains("GAMEBUDDY_REALTIME_TRANSCRIPT role=assistant"))
    })?;
    ensure!(
        !capturing()?,
        "Introduction opened a microphone without activation"
    );
    ensure!(
        cardinal(conn, overlay, focus)? == 0,
        "Introduction stole game focus"
    );
    thread::sleep(Duration::from_secs(18)); // let the introduction finish before the request
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
    pactl(&["set-source-mute", &source, "0"])?;
    wait("subscription WebRTC connects", || {
        Ok(std::fs::read_to_string(&log)?.contains("Live voice · speak naturally"))
    })?;
    ensure!(
        capturing()?,
        "WebRTC did not capture the selected virtual microphone"
    );
    ensure!(
        cardinal(conn, overlay, focus)? == 0,
        "Live voice stole game focus"
    );
    let wave = std::env::var("GAMEBUDDY_TEST_VOICE_WAV")?;
    ensure!(
        Command::new("paplay")
            .args(["--device", &sink, &wave])
            .status()?
            .success(),
        "Synthetic speech playback failed"
    );
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
        ensure!(
            std::fs::read_to_string(&log)?
                .contains("GAMEBUDDY_VOICE_TOOL name=get_game_screen success=true images=1"),
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
    thread::sleep(Duration::from_secs(10)); // include the complete streamed reply
    pactl(&["set-source-mute", &source, "1"])?;
    wait("mute leaves guidance available", || {
        Ok(std::fs::read_to_string(&log)?
            .contains("GAMEBUDDY_REALTIME_STATUS Guide voice ready · mic off"))
    })?;
    wait("mute closes the WebRTC microphone", || Ok(!capturing()?))?;
    if std::env::var_os("GAMEBUDDY_TEST_SCREEN").is_some() {
        let before = std::fs::read_to_string(&log)?
            .matches("GAMEBUDDY_NARRATION")
            .count();
        draw_game_over(conn, game)?;
        wait(
            "automatic game-over observation and spoken feedback",
            || {
                let state = gamebuddy_services::screen::status(&store)?;
                Ok(state["scene"]["event"] == "death"
                    && state["scene"]["fresh"] == true
                    && std::fs::read_to_string(&log)?
                        .matches("GAMEBUDDY_NARRATION")
                        .count()
                        > before)
            },
        )?;
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
        "GAMEBUDDY_REALTIME_TEST_PASS automatic introduction without capture, subscription WebRTC, synthetic microphone, validated widget tool, passive focus, mute closes capture and preserves guidance"
    );
    Ok(())
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
