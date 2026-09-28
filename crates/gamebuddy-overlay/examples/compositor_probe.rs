//! An isolated X11 integration fixture, launched as the "game" by GameBuddy.
//! Never connects to the user's desktop display. Outputs screenshots and checks
//! the real Qt overlay plus Gamescope input routing and XComposite capture.
#[cfg(target_os = "linux")]
mod support;
#[cfg(target_os = "linux")]
fn main() -> anyhow::Result<()> {
    use anyhow::{Context, ensure};
    use gamebuddy_overlay::x11::{atom, capture_game, cardinal};
    use std::{
        path::PathBuf,
        thread,
        time::{Duration, Instant},
    };
    use x11rb::{
        connection::Connection,
        protocol::{Event, xproto::*, xtest::ConnectionExt as _},
        wrapper::ConnectionExt as _,
    };
    if std::env::args().nth(1).as_deref() == Some("--prepare") {
        let root = std::env::args_os()
            .nth(2)
            .context("Data directory required")?;
        let store = gamebuddy_services::Store::open(root)?;
        let guide = gamebuddy_core::import_markdown(
            "Faxanadu",
            "Overlay test",
            "# Overlay fixture\nTest display only.\n## Current action\nThis is a test scene, not a real game walkthrough.\n## Next action\nInspect the collapsible outline.",
        )?;
        store.save_guide(&guide)?;
        store.put("active_guide", &guide.id)?;
        store.save_progress(
            &guide,
            &gamebuddy_core::Progress {
                current_step: Some("step-2".into()),
                ..Default::default()
            },
            "test_fixture",
        )?;
        store.put(&format!("widgets:{}:{}",guide.id,guide.revision), &serde_json::json!([
            {"id":"compositor-test","title":"Overlay test","source_step_ids":[],"content":{"kind":"text","body":"Drag and resize this HTML widget."}}
        ]))?;
        store.put(
            "widget-layout",
            &serde_json::json!({"compositor-test":{"x":600,"y":80,"width":350,"height":260}}),
        )?;
        return Ok(());
    }
    let _screenshot_lock = support::screenshot_lock()?;
    let directory = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .context("Output directory required")?,
    );
    std::fs::create_dir_all(&directory)?;
    let (conn, screen) = x11rb::connect(None)?;
    let screen = &conn.setup().roots[screen];
    let root = screen.root;
    // Lunchpail starts its child directly on the returned DISPLAY, so it need
    // not inherit Gamescope's environment. Verify the server itself before
    // generating any test input; never send events to the desktop Xwayland.
    ensure!(
        conn.get_property(
            false,
            root,
            atom(&conn, "GAMESCOPE_XWAYLAND_SERVER_ID")?,
            AtomEnum::CARDINAL,
            0,
            1
        )?
        .reply()?
        .value_len
            == 1,
        "Run only inside an isolated Gamescope test session"
    );
    let window = conn.generate_id()?;
    conn.create_window(
        screen.root_depth,
        window,
        root,
        0,
        0,
        1280,
        720,
        0,
        WindowClass::INPUT_OUTPUT,
        screen.root_visual,
        &CreateWindowAux::new()
            .background_pixel(0x142850)
            .event_mask(EventMask::EXPOSURE | EventMask::KEY_PRESS | EventMask::BUTTON_PRESS),
    )?
    .check()?;
    conn.change_property8(
        PropMode::REPLACE,
        window,
        AtomEnum::WM_NAME,
        AtomEnum::STRING,
        b"GameBuddy compositor test fixture",
    )?;
    conn.change_property32(
        PropMode::REPLACE,
        window,
        atom(&conn, "_NET_WM_PID")?,
        AtomEnum::CARDINAL,
        &[std::process::id()],
    )?;
    conn.map_window(window)?.check()?;
    conn.flush()?;
    let overlay_atom = atom(&conn, "STEAM_OVERLAY")?;
    let focus = atom(&conn, "STEAM_INPUT_FOCUS")?;
    let deadline = Instant::now() + Duration::from_secs(30);
    let overlay = loop {
        if let Some(w) = conn
            .query_tree(root)?
            .reply()?
            .children
            .into_iter()
            .find(|w| cardinal(&conn, *w, overlay_atom).unwrap_or(0) != 0)
        {
            break w;
        }
        ensure!(Instant::now() < deadline, "Qt overlay never registered");
        thread::sleep(Duration::from_millis(50));
    };
    let keymap = conn
        .get_keyboard_mapping(
            conn.setup().min_keycode,
            conn.setup().max_keycode - conn.setup().min_keycode + 1,
        )?
        .reply()?;
    let key = |symbol| -> anyhow::Result<u8> {
        Ok(keymap
            .keysyms
            .chunks(keymap.keysyms_per_keycode as usize)
            .position(|keys| keys.contains(&symbol))
            .context("key missing")? as u8
            + conn.setup().min_keycode)
    };
    let a = key(0x61)?;
    let shift = key(0xffe1)?;
    let tab = key(0xff09)?;
    let pause = || thread::sleep(Duration::from_millis(300));
    let send_key = |code, down| -> anyhow::Result<()> {
        conn.xtest_fake_input(
            if down {
                KEY_PRESS_EVENT
            } else {
                KEY_RELEASE_EVENT
            },
            code,
            0,
            root,
            0,
            0,
            0,
        )?
        .check()?;
        conn.flush()?;
        Ok(())
    };
    let toggle = || -> anyhow::Result<()> {
        send_key(shift, true)?;
        send_key(tab, true)?;
        send_key(tab, false)?;
        send_key(shift, false)?;
        pause();
        Ok(())
    };
    let count_a = || -> anyhow::Result<usize> {
        let mut count = 0;
        while let Some(event) = conn.poll_for_event()? {
            if let Event::KeyPress(e) = event
                && e.detail == a
            {
                count += 1;
            }
        }
        Ok(count)
    };
    let screenshot = |name: &str| -> anyhow::Result<()> {
        // Gamescope 3.16's X11 debug screenshot IPC includes the overlay plane.
        let previous = std::fs::metadata("/tmp/gamescope.png")
            .and_then(|m| m.modified())
            .ok();
        let property = atom(&conn, "GAMESCOPECTRL_DEBUG_REQUEST_SCREENSHOT")?;
        conn.change_property32(PropMode::REPLACE, root, property, AtomEnum::CARDINAL, &[2])?
            .check()?;
        conn.flush()?;
        let until = Instant::now() + Duration::from_secs(5);
        while std::fs::metadata("/tmp/gamescope.png")
            .and_then(|m| m.modified())
            .ok()
            == previous
            || image::open("/tmp/gamescope.png").is_err()
        {
            ensure!(
                Instant::now() < until,
                "Gamescope did not complete screenshot"
            );
            thread::sleep(Duration::from_millis(50));
        }
        std::fs::copy("/tmp/gamescope.png", directory.join(name))?;
        Ok(())
    };
    conn.xtest_fake_input(MOTION_NOTIFY_EVENT, 0, 0, root, 900, 600, 0)?
        .check()?;
    conn.flush()?;
    thread::sleep(Duration::from_secs(2));
    screenshot("initial-input.png")?;
    ensure!(
        cardinal(&conn, overlay, focus)? == 0,
        "Collapsed overlay trapped input: mode={} pointer={:?}",
        cardinal(&conn, overlay, focus)?,
        conn.query_pointer(root)?.reply()?
    );
    count_a()?;
    send_key(a, true)?;
    send_key(a, false)?;
    pause();
    ensure!(
        count_a()? == 1,
        "Game did not receive keyboard input while collapsed"
    );
    screenshot("collapsed.png")?;
    capture_game(std::process::id(), &directory.join("game-only.png"))?;
    let image = image::open(directory.join("game-only.png"))?.to_rgb8();
    ensure!(
        image.pixels().all(|p| p.0 == [20, 40, 80]),
        "Overlay contaminated game-only capture"
    );
    if std::env::var_os("GAMEBUDDY_TEST_MIC_SOURCE").is_some() {
        if std::env::var_os("GAMEBUDDY_TEST_SCREEN").is_some() {
            let gc = conn.generate_id()?;
            conn.create_gc(gc, window, &CreateGCAux::new().foreground(0xff2020))?
                .check()?;
            conn.poly_fill_rectangle(
                window,
                gc,
                &[Rectangle {
                    x: 420,
                    y: 180,
                    width: 240,
                    height: 240,
                }],
            )?
            .check()?;
            conn.change_gc(gc, &ChangeGCAux::new().foreground(0x20ee40))?
                .check()?;
            conn.poly_fill_rectangle(
                window,
                gc,
                &[Rectangle {
                    x: 820,
                    y: 200,
                    width: 130,
                    height: 300,
                }],
            )?
            .check()?;
            conn.free_gc(gc)?.check()?;
            conn.flush()?;
        }
        support::voice_probe::run(&conn, window, overlay, focus, &directory, screenshot)?;
        return Ok(());
    }
    if std::env::var_os("GAMEBUDDY_TEST_CROP").is_some() {
        support::crop_probe::run(&conn, window, overlay, focus, root, toggle, screenshot)?;
        return Ok(());
    }
    toggle()?;
    ensure!(
        cardinal(&conn, overlay, focus)? == 1,
        "Shift+Tab did not open overlay"
    );
    send_key(a, true)?;
    send_key(a, false)?;
    pause();
    ensure!(count_a()? == 0, "Keyboard leaked through expanded overlay");
    screenshot("expanded.png")?;
    capture_game(
        std::process::id(),
        &directory.join("game-only-expanded.png"),
    )?;
    ensure!(
        std::fs::read(directory.join("game-only.png"))?
            == std::fs::read(directory.join("game-only-expanded.png"))?,
        "Expanded overlay contaminated capture"
    );
    if let Some(data_root) = std::env::var_os("GAMEBUDDY_TEST_DATA") {
        let mouse = |kind, detail, x, y| -> anyhow::Result<()> {
            conn.xtest_fake_input(kind, detail, 0, root, x, y, 0)?
                .check()?;
            conn.flush()?;
            pause();
            Ok(())
        };
        // Drag directly while collapsed: no Arrange button or focus shortcut.
        toggle()?;
        mouse(MOTION_NOTIFY_EVENT, 0, 650, 104)?;
        ensure!(
            cardinal(&conn, overlay, focus)? == 2,
            "Widget hover did not acquire pointer-only focus"
        );
        send_key(a, true)?;
        send_key(a, false)?;
        pause();
        ensure!(count_a()? == 1, "Widget hover stole game keyboard input");
        mouse(BUTTON_PRESS_EVENT, 1, 0, 0)?;
        mouse(MOTION_NOTIFY_EVENT, 0, 800, 300)?;
        mouse(BUTTON_RELEASE_EVENT, 1, 0, 0)?;
        screenshot("widgets-after-drag.png")?;
        let store = gamebuddy_services::Store::open(data_root)?;
        let layout: serde_json::Value = store.get("widget-layout")?.context("layout not saved")?;
        let box_ = &layout["compositor-test"];
        ensure!(
            box_["x"].as_f64().unwrap_or(0.) > 740.,
            "HTML widget drag not saved: {layout}"
        );
        let x = box_["x"].as_f64().unwrap();
        let y = box_["y"].as_f64().unwrap();
        let w = box_["width"].as_f64().unwrap();
        let h = box_["height"].as_f64().unwrap();
        mouse(
            MOTION_NOTIFY_EVENT,
            0,
            (x + w - 8.) as i16,
            (y + h - 8.) as i16,
        )?;
        mouse(BUTTON_PRESS_EVENT, 1, 0, 0)?;
        mouse(
            MOTION_NOTIFY_EVENT,
            0,
            (x + w + 72.) as i16,
            (y + h + 62.) as i16,
        )?;
        mouse(BUTTON_RELEASE_EVENT, 1, 0, 0)?;
        let layout: serde_json::Value = store.get("widget-layout")?.unwrap();
        ensure!(
            layout["compositor-test"]["width"].as_f64().unwrap_or(0.) > w + 60.,
            "HTML widget resize not saved: {layout}"
        );
        screenshot("widgets-arranged.png")?;
        println!("GAMEBUDDY_HTML_TEST_PASS {layout}");
        // The close affordance returns the widget to the companion tray.
        let box_ = &layout["compositor-test"];
        mouse(
            MOTION_NOTIFY_EVENT,
            0,
            (box_["x"].as_f64().unwrap() + box_["width"].as_f64().unwrap() - 30.) as i16,
            (box_["y"].as_f64().unwrap() + 28.) as i16,
        )?;
        mouse(BUTTON_PRESS_EVENT, 1, 0, 0)?;
        mouse(BUTTON_RELEASE_EVENT, 1, 0, 0)?;
        let guide: String = store.get("active_guide")?.unwrap();
        let visibility: serde_json::Value = store
            .get(&format!("widget-visibility:{guide}"))?
            .context("dismiss preference missing")?;
        ensure!(
            visibility["compositor-test"] == false,
            "Dismiss did not hide the widget"
        );
        screenshot("widget-dismissed.png")?;
        toggle()?;
        // Open the production tray, then restore its fourth item (the custom widget).
        mouse(MOTION_NOTIFY_EVENT, 0, 304, 530)?;
        mouse(BUTTON_PRESS_EVENT, 1, 0, 0)?;
        mouse(BUTTON_RELEASE_EVENT, 1, 0, 0)?;
        screenshot("widget-tray.png")?;
        mouse(MOTION_NOTIFY_EVENT, 0, 210, 405)?;
        mouse(BUTTON_PRESS_EVENT, 1, 0, 0)?;
        mouse(BUTTON_RELEASE_EVENT, 1, 0, 0)?;
        let visibility: serde_json::Value =
            store.get(&format!("widget-visibility:{guide}"))?.unwrap();
        ensure!(
            visibility["compositor-test"] == true,
            "Tray did not restore the widget: {visibility}"
        );
        let restored: serde_json::Value = store.get("widget-layout")?.unwrap();
        ensure!(restored == layout, "Restoring a widget lost its layout");
        screenshot("widget-restored.png")?;
        mouse(MOTION_NOTIFY_EVENT, 0, 365, 215)?;
        mouse(BUTTON_PRESS_EVENT, 1, 0, 0)?;
        mouse(BUTTON_RELEASE_EVENT, 1, 0, 0)?;
        mouse(MOTION_NOTIFY_EVENT, 0, 140, 530)?;
        mouse(BUTTON_PRESS_EVENT, 1, 0, 0)?;
        mouse(BUTTON_RELEASE_EVENT, 1, 0, 0)?;
        screenshot("widgets-hidden.png")?;
        ensure!(
            image::open(directory.join("widgets-hidden.png"))?
                .to_rgb8()
                .get_pixel(800, 300)
                .0
                == [20, 40, 80],
            "Hide widgets left a widget visible"
        );
        mouse(BUTTON_PRESS_EVENT, 1, 0, 0)?;
        mouse(BUTTON_RELEASE_EVENT, 1, 0, 0)?;
        screenshot("widgets-shown.png")?;
        ensure!(
            image::open(directory.join("widgets-shown.png"))?
                .to_rgb8()
                .get_pixel(800, 300)
                .0
                != [20, 40, 80],
            "Show widgets did not restore the widget"
        );
        println!(
            "GAMEBUDDY_WIDGET_VISIBILITY_TEST_PASS dismiss, tray restore, layout persistence, hide/show all"
        );
        mouse(MOTION_NOTIFY_EVENT, 0, 1250, 690)?;
    }
    toggle()?;
    ensure!(
        cardinal(&conn, overlay, focus)? == 0,
        "Closing overlay did not restore game input"
    );
    send_key(a, true)?;
    send_key(a, false)?;
    pause();
    ensure!(count_a()? == 1, "Game input not restored");
    // Hover/click the real Qt companion orb in our private test display.
    conn.xtest_fake_input(MOTION_NOTIFY_EVENT, 0, 0, root, 76, 130, 0)?
        .check()?;
    conn.flush()?;
    let deadline = Instant::now() + Duration::from_secs(5);
    while cardinal(&conn, overlay, focus)? != 2 {
        ensure!(
            Instant::now() < deadline,
            "Orb hover did not acquire pointer-only focus (mode={}, cursor_visible={}, pointer={:?})",
            cardinal(&conn, overlay, focus)?,
            cardinal(
                &conn,
                root,
                atom(&conn, "GAMESCOPE_CURSOR_VISIBLE_FEEDBACK")?
            )?,
            conn.query_pointer(root)?.reply()?
        );
        thread::sleep(Duration::from_millis(50));
    }
    conn.xtest_fake_input(BUTTON_PRESS_EVENT, 1, 0, root, 0, 0, 0)?
        .check()?;
    conn.xtest_fake_input(BUTTON_RELEASE_EVENT, 1, 0, root, 0, 0, 0)?
        .check()?;
    conn.flush()?;
    pause();
    ensure!(
        cardinal(&conn, overlay, focus)? == 1,
        "Clicking orb did not expand companion"
    );
    toggle()?;
    conn.xtest_fake_input(BUTTON_PRESS_EVENT, 1, 0, root, 0, 0, 0)?
        .check()?;
    conn.flush()?;
    pause();
    conn.xtest_fake_input(MOTION_NOTIFY_EVENT, 0, 0, root, 300, 230, 0)?
        .check()?;
    conn.flush()?;
    pause();
    conn.xtest_fake_input(BUTTON_RELEASE_EVENT, 1, 0, root, 0, 0, 0)?
        .check()?;
    conn.flush()?;
    pause();
    screenshot("dragged.png")?;
    let collapsed = image::open(directory.join("collapsed.png"))?.to_rgb8();
    let dragged = image::open(directory.join("dragged.png"))?.to_rgb8();
    ensure!(
        collapsed.get_pixel(76, 130) != dragged.get_pixel(76, 130),
        "Orb did not leave old position"
    );
    ensure!(
        collapsed.get_pixel(300, 230) != dragged.get_pixel(300, 230),
        "Orb did not arrive at dragged position"
    );
    // Escape releases focus; a separate chord hides all surfaces.
    toggle()?;
    let escape = key(0xff1b)?;
    send_key(escape, true)?;
    send_key(escape, false)?;
    pause();
    ensure!(
        cardinal(&conn, overlay, focus)? != 1,
        "Escape did not release the overlay"
    );
    conn.xtest_fake_input(MOTION_NOTIFY_EVENT, 0, 0, root, 900, 650, 0)?
        .check()?;
    conn.flush()?;
    pause();
    let control = key(0xffe3)?;
    send_key(control, true)?;
    toggle()?;
    send_key(control, false)?;
    pause();
    ensure!(
        cardinal(&conn, overlay, focus)? == 0,
        "Hidden overlay retained input focus"
    );
    screenshot("hidden.png")?;
    let hidden = image::open(directory.join("hidden.png"))?.to_rgb8();
    ensure!(
        hidden.get_pixel(300, 230) == hidden.get_pixel(500, 700),
        "Hidden orb is still visible"
    );
    ensure!(
        hidden.get_pixel(800, 300) == hidden.get_pixel(500, 700),
        "Hidden widget is still visible"
    );
    send_key(control, true)?;
    toggle()?;
    send_key(control, false)?;
    pause();
    screenshot("restored.png")?;
    let restored = image::open(directory.join("restored.png"))?.to_rgb8();
    ensure!(
        restored.get_pixel(300, 230) != restored.get_pixel(500, 700),
        "Visibility shortcut did not restore the icon"
    );
    if let Some(path) = std::env::var_os("GAMEBUDDY_CONTROL_SOCKET") {
        gamebuddy_overlay::control::send(std::path::Path::new(&path), "focus")?;
        pause();
        ensure!(
            cardinal(&conn, overlay, focus)? == 1,
            "Session command did not focus overlay"
        );
        gamebuddy_overlay::control::send(std::path::Path::new(&path), "hide")?;
        pause();
        ensure!(
            cardinal(&conn, overlay, focus)? == 0,
            "Session command did not release game input"
        );
        gamebuddy_overlay::control::send(std::path::Path::new(&path), "show")?;
        pause();
        println!("GAMEBUDDY_CONTROL_TEST_PASS keyboard visibility, Escape, session commands");
    }
    println!(
        "GAMEBUDDY_COMPOSITOR_TEST_PASS overlay={overlay} game={window}: keyboard passthrough, hotkey focus, pointer expansion, drag, clean capture"
    );
    Ok(())
}
#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("Gamescope integration test requires Linux");
}
