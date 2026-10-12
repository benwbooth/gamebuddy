//! Native-resolution coverage and pixel-sharpness regression in a private
//! Gamescope display. --host prints the physical desktop mode without a window.
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
    use x11rb::{connection::Connection, protocol::xproto::*, wrapper::ConnectionExt as _};
    if std::env::args().nth(1).as_deref() == Some("--host") {
        let size = gamebuddy_overlay::display::native_size()?;
        println!("{} {}", size.width, size.height);
        return Ok(());
    }
    let _lock = support::screenshot_lock()?;
    let mut args = std::env::args().skip(1);
    let directory = PathBuf::from(args.next().context("artifact directory required")?);
    let width: u16 = args.next().context("expected width required")?.parse()?;
    let height: u16 = args.next().context("expected height required")?.parse()?;
    std::fs::create_dir_all(&directory)?;
    let (conn, screen) = x11rb::connect(None)?;
    let screen = &conn.setup().roots[screen];
    let root = screen.root;
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
        "Resolution probe requires a private Gamescope display"
    );
    ensure!(
        (screen.width_in_pixels, screen.height_in_pixels) == (width, height),
        "Game sees {}x{}, expected {width}x{height}",
        screen.width_in_pixels,
        screen.height_in_pixels
    );
    ensure!(
        width >= 800 && height >= 600,
        "Probe needs at least 800x600"
    );
    let window = conn.generate_id()?;
    conn.create_window(
        screen.root_depth,
        window,
        root,
        0,
        0,
        width,
        height,
        0,
        WindowClass::INPUT_OUTPUT,
        screen.root_visual,
        &CreateWindowAux::new().background_pixel(0x142850),
    )?
    .check()?;
    conn.change_property8(
        PropMode::REPLACE,
        window,
        AtomEnum::WM_NAME,
        AtomEnum::STRING,
        b"GameBuddy native resolution test",
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
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let windows = gamebuddy_overlay::x11::client_windows(&conn, root)?;
        if windows
            .iter()
            .any(|w| cardinal(&conn, *w, overlay_atom).unwrap_or(0) != 0)
        {
            break;
        }
        ensure!(Instant::now() < deadline, "Overlay did not register");
        thread::sleep(Duration::from_millis(50));
    }
    // One physical pixel per stripe exposes even fractional bilinear scaling.
    let gc = conn.generate_id()?;
    conn.create_gc(gc, window, &CreateGCAux::new())?;
    for i in 0..512i16 {
        conn.change_gc(
            gc,
            &ChangeGCAux::new().foreground(if i % 2 == 0 { 0xffffff } else { 0 }),
        )?;
        conn.poly_fill_rectangle(
            window,
            gc,
            &[Rectangle {
                x: width as i16 - 600 + i,
                y: height as i16 - 180,
                width: 1,
                height: 160,
            }],
        )?;
    }
    conn.flush()?;
    thread::sleep(Duration::from_secs(2));
    capture_game(std::process::id(), &directory.join("game-only.png"))?;
    let previous = std::fs::metadata("/tmp/gamescope.png")
        .and_then(|m| m.modified())
        .ok();
    conn.change_property32(
        PropMode::REPLACE,
        root,
        atom(&conn, "GAMESCOPECTRL_DEBUG_REQUEST_SCREENSHOT")?,
        AtomEnum::CARDINAL,
        &[2],
    )?
    .check()?;
    conn.flush()?;
    let deadline = Instant::now() + Duration::from_secs(15);
    let frame = loop {
        if std::fs::metadata("/tmp/gamescope.png")
            .and_then(|m| m.modified())
            .ok()
            != previous
            && let Ok(frame) = image::open("/tmp/gamescope.png")
        {
            break frame.to_rgb8();
        }
        ensure!(Instant::now() < deadline, "Compositor screenshot timed out");
        thread::sleep(Duration::from_millis(50));
    };
    frame.save(directory.join("composited.png"))?;
    // A compositor may round the outer Wayland surface by one physical pixel
    // at fractional desktop scale; the game's advertised mode must be exact.
    let (w, h) = frame.dimensions();
    ensure!(
        w.abs_diff(u32::from(width)) <= 1 && h.abs_diff(u32::from(height)) <= 1,
        "Output is {w}x{h}, expected native {width}x{height}"
    );
    for (x, y) in [(0, 0), (w - 1, 0), (0, h - 1), (w - 1, h - 1)] {
        ensure!(
            frame.get_pixel(x, y).0.iter().any(|c| *c > 10),
            "Black border at {x},{y}"
        );
    }
    let row: Vec<_> = (w - 590..w - 100)
        .map(|x| frame.get_pixel(x, h - 100).0)
        .collect();
    ensure!(
        row.iter()
            .all(|p| p.iter().all(|c| *c < 5) || p.iter().all(|c| *c > 250)),
        "One-pixel stripes were blurred during composition"
    );
    let transitions = row.windows(2).filter(|pair| pair[0] != pair[1]).count();
    ensure!(
        transitions >= 485,
        "Lost fine pixel detail: {transitions} transitions"
    );
    println!(
        "GAMEBUDDY_RESOLUTION_PASS render={width}x{height} output={w}x{h} sharp_transitions={transitions} corners=filled"
    );
    Ok(())
}
#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("Linux only");
}
