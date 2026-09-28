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
    overlay: u32,
    focus: u32,
    directory: &Path,
    screenshot: impl Fn(&str) -> Result<()>,
) -> Result<()> {
    if std::env::var_os("GAMEBUDDY_TEST_REALTIME").is_some() {
        return realtime(conn, overlay, focus, screenshot);
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
                .any(|w| w.title.to_lowercase().contains("voice works")),
        )
    })?;
    println!(
        "GAMEBUDDY_REALTIME_TOOL latency_seconds={:.1}",
        started.elapsed().as_secs_f32()
    );
    thread::sleep(Duration::from_secs(3)); // allow the OSD's regular store poll
    screenshot("realtime-widget.png")?;
    thread::sleep(Duration::from_secs(10)); // include the complete streamed reply
    pactl(&["set-source-mute", &source, "1"])?;
    wait("mute ends realtime session", || {
        Ok(std::fs::read_to_string(&log)?.contains("GAMEBUDDY_REALTIME_STATUS Voice stopped"))
    })?;
    wait("mute closes the WebRTC microphone", || Ok(!capturing()?))?;
    println!(
        "GAMEBUDDY_REALTIME_TEST_PASS subscription WebRTC, synthetic microphone, validated widget tool, passive focus, mute disconnect"
    );
    Ok(())
}
