//! Which X11 window a click reaches inside the private compositor. Run as the
//! compositor's command: a fullscreen game with a fullscreen overlay above it.
#[cfg(target_os = "linux")]
fn main() -> anyhow::Result<()> {
    use anyhow::ensure;
    use std::time::{Duration, Instant};
    use x11rb::{
        connection::Connection,
        protocol::{Event, xproto::*, xtest::ConnectionExt as _},
        wrapper::ConnectionExt as _,
    };
    let (conn, screen) = x11rb::connect(None)?;
    let (root, depth, visual, width, height) = {
        let s = &conn.setup().roots[screen];
        (
            s.root,
            s.root_depth,
            s.root_visual,
            s.width_in_pixels,
            s.height_in_pixels,
        )
    };
    let overlay_atom = gamebuddy_overlay::x11::atom(&conn, "STEAM_OVERLAY")?;
    let focus_atom = gamebuddy_overlay::x11::atom(&conn, "STEAM_INPUT_FOCUS")?;
    ensure!(
        conn.get_property(
            false,
            root,
            gamebuddy_overlay::x11::atom(&conn, "GAMESCOPE_XWAYLAND_SERVER_ID")?,
            AtomEnum::CARDINAL,
            0,
            1
        )?
        .reply()?
        .value_len
            == 1,
        "The click probe only runs inside the private compositor"
    );
    let window = |name: &str| -> anyhow::Result<Window> {
        let id = conn.generate_id()?;
        conn.create_window(
            depth,
            id,
            root,
            0,
            0,
            width,
            height,
            0,
            WindowClass::INPUT_OUTPUT,
            visual,
            &CreateWindowAux::new()
                .background_pixel(0)
                .event_mask(EventMask::BUTTON_PRESS),
        )?
        .check()?;
        conn.change_property8(
            PropMode::REPLACE,
            id,
            AtomEnum::WM_NAME,
            AtomEnum::STRING,
            name.as_bytes(),
        )?;
        Ok(id)
    };
    let game = window("game")?;
    conn.map_window(game)?;
    conn.flush()?;
    std::thread::sleep(Duration::from_millis(500));
    let overlay = window("overlay")?;
    conn.change_property32(
        PropMode::REPLACE,
        overlay,
        overlay_atom,
        AtomEnum::CARDINAL,
        &[1],
    )?;
    conn.change_property32(
        PropMode::REPLACE,
        overlay,
        focus_atom,
        AtomEnum::CARDINAL,
        &[0],
    )?;
    conn.map_window(overlay)?;
    conn.flush()?;
    std::thread::sleep(Duration::from_millis(500));

    let click = |mode: u32| -> anyhow::Result<&'static str> {
        conn.change_property32(
            PropMode::REPLACE,
            overlay,
            focus_atom,
            AtomEnum::CARDINAL,
            &[mode],
        )?;
        conn.flush()?;
        std::thread::sleep(Duration::from_millis(200));
        let (x, y) = ((width / 2) as i16, (height / 2) as i16);
        conn.xtest_fake_input(MOTION_NOTIFY_EVENT, 0, 0, root, x, y, 0)?;
        conn.flush()?;
        std::thread::sleep(Duration::from_millis(100));
        conn.xtest_fake_input(BUTTON_PRESS_EVENT, 1, 0, root, 0, 0, 0)?;
        conn.xtest_fake_input(BUTTON_RELEASE_EVENT, 1, 0, root, 0, 0, 0)?;
        conn.flush()?;
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(1) {
            match conn.poll_for_event()? {
                Some(Event::ButtonPress(event)) if event.event == game => return Ok("game"),
                Some(Event::ButtonPress(event)) if event.event == overlay => return Ok("overlay"),
                Some(_) => {}
                None => std::thread::sleep(Duration::from_millis(10)),
            }
        }
        Ok("nobody")
    };
    let mut pass = true;
    for (mode, expected) in [(0, "game"), (2, "overlay"), (1, "overlay"), (0, "game")] {
        let reached = click(mode)?;
        println!("CLICK_PROBE mode={mode} reached={reached} expected={expected}");
        pass &= reached == expected;
    }
    println!("CLICK_PROBE {}", if pass { "PASS" } else { "FAIL" });
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn main() {}
