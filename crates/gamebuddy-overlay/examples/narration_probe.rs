//! Native automatic speech fixture. Uses only its private store and Gamescope.
#[cfg(target_os = "linux")]
fn main() -> anyhow::Result<()> {
    use anyhow::{Context, ensure};
    use gamebuddy_services::{Settings, Store, screen};
    use serde_json::json;
    use std::{
        thread,
        time::{Duration, Instant},
    };
    use x11rb::{connection::Connection, protocol::xproto::*, wrapper::ConnectionExt as _};
    let mut args = std::env::args().skip(1);
    let mode = args.next().context("--prepare or --run")?;
    let root = args.next().context("private data directory")?;
    ensure!(root.starts_with("/tmp/"), "Private test store required");
    let store = Store::open(root)?;
    if mode == "--prepare" {
        let guide = gamebuddy_core::import_markdown(
            "Narration fixture",
            "Automatic coaching",
            "# Red door\nWalk to the red door. Climb the ladder.\n# Blue platform\nJump onto the blue platform.\n# Green door\nCross the bridge and enter the green door.",
        )?;
        store.save_guide(&guide)?;
        store.put("active_guide", &guide.id)?;
        let mut settings = Settings {
            speech: gamebuddy_services::SpeechBackend::CodexRealtime,
            ..Default::default()
        };
        settings.reasoning.kind = gamebuddy_services::ProviderKind::Codex;
        settings.reasoning.model.clear();
        settings.reasoning.endpoint.clear();
        store.put("settings", &settings)?;
        return Ok(());
    }
    let (conn, index) = x11rb::connect(None)?;
    let display = &conn.setup().roots[index];
    ensure!(
        conn.get_property(
            false,
            display.root,
            gamebuddy_overlay::x11::atom(&conn, "GAMESCOPE_XWAYLAND_SERVER_ID")?,
            AtomEnum::CARDINAL,
            0,
            1
        )?
        .reply()?
        .value_len
            == 1,
        "Isolated Gamescope required"
    );
    let window = conn.generate_id()?;
    conn.create_window(
        display.root_depth,
        window,
        display.root,
        0,
        0,
        1280,
        720,
        0,
        WindowClass::INPUT_OUTPUT,
        display.root_visual,
        &CreateWindowAux::new().background_pixel(0x142850),
    )?
    .check()?;
    conn.change_property8(
        PropMode::REPLACE,
        window,
        AtomEnum::WM_NAME,
        AtomEnum::STRING,
        b"Automatic narration fixture",
    )?;
    conn.change_property32(
        PropMode::REPLACE,
        window,
        gamebuddy_overlay::x11::atom(&conn, "_NET_WM_PID")?,
        AtomEnum::CARDINAL,
        &[std::process::id()],
    )?;
    conn.map_window(window)?.check()?;
    conn.flush()?;
    let guide = store.guide(&store.get::<String>("active_guide")?.context("guide")?)?;
    let start = Instant::now();
    let mut phase = 0;
    let mut changed_during_speech = false;
    while start.elapsed() < Duration::from_secs(46) {
        if !changed_during_speech
            && store
                .get::<Vec<serde_json::Value>>("voice-diagnostics")?
                .unwrap_or_default()
                .iter()
                .rev()
                .find(|event| event["kind"] == "browser/output-active")
                .is_some_and(|event| event["value"] == true)
        {
            // Ordinary gameplay changes the picture and bookkeeping while
            // Buddy is talking. Neither should cut off the native voice.
            conn.change_window_attributes(
                window,
                &ChangeWindowAttributesAux::new().background_pixel(0x805014),
            )?
            .check()?;
            conn.clear_area(false, window, 0, 0, 0, 0)?.check()?;
            conn.flush()?;
            let mut progress = store.progress(&guide)?;
            progress.epoch += 1;
            // This private fixture needs a single bookkeeping update, without
            // a read-to-write transaction racing the capture/diagnostic writers.
            store.put(
                &format!("progress:{}:{}", guide.id, guide.revision),
                &progress,
            )?;
            changed_during_speech = true;
            println!("NARRATION_FIXTURE_CHANGED_DURING_SPEECH");
        }
        if let Ok(snapshot) = screen::snapshot(&store) {
            let scene = serde_json::from_value(
                json!({"captured_at_ms":snapshot.captured_at_ms,"summary":"A solid blue native test window.","visible_text":[],"event":"none","confidence":1.0,"feedback":"","uncertainties":[]}),
            )?;
            let _ = screen::save_observation(&store, &snapshot, &Ok(scene));
            let elapsed = start.elapsed().as_secs();
            if (phase == 0 && elapsed >= 14) || (phase == 1 && elapsed >= 25) {
                let progress = store.progress(&guide)?;
                let step = if phase == 0 { "step-1" } else { "step-2" };
                store.report_complete(
                    &guide,
                    progress.epoch,
                    step,
                    "Explicit synthetic test completion",
                )?;
                println!(
                    "NARRATION_FIXTURE_STEP {}",
                    json!({"at_ms":screen::now_ms(),"completed":step})
                );
                phase += 1;
            }
            if phase == 2 && elapsed >= 34 {
                // Synthetic approved coaching isolates the complete delivery
                // path; the activity replay separately exercises real vision.
                let observation = gamebuddy_services::activity::prepare(&store, &guide, &snapshot)?;
                let review = serde_json::from_value(json!({"completions":[],"coaching":{
                    "kind":"strategy","step_id":"step-3","text":"Take a running start before the jump.",
                    "evidence":"Explicit synthetic strategy cue for the native audio integration fixture.",
                    "confidence":1.0,"screen_direction":null}}))?;
                gamebuddy_services::activity::save_review(&store, &observation, &review)?;
                println!("NARRATION_FIXTURE_STRATEGY at_ms={}", screen::now_ms());
                phase += 1;
            }
        }
        thread::sleep(Duration::from_millis(200));
    }
    ensure!(phase == 3, "No live capture in fixture");
    ensure!(
        changed_during_speech,
        "No audible speech for the continuity check"
    );
    println!("NARRATION_FIXTURE_FINISHED");
    Ok(())
}
#[cfg(not(target_os = "linux"))]
fn main() {
    panic!("Linux Gamescope fixture");
}
