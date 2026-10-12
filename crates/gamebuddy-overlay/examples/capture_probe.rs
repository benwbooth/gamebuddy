//! Read-only diagnostics for a game launched in a private Gamescope session.
#[cfg(target_os = "linux")]
mod support;
#[cfg(target_os = "linux")]
fn main() -> anyhow::Result<()> {
    use anyhow::{Context, ensure};
    use gamebuddy_overlay::x11::{atom, capture_game, cardinal};
    use std::{
        path::PathBuf,
        time::{Duration, Instant},
    };
    use x11rb::{connection::Connection, protocol::xproto::*, wrapper::ConnectionExt as _};
    let _screenshot_lock = support::screenshot_lock()?;
    let mut args = std::env::args().skip(1);
    let pid = args.next().context("PID required")?.parse()?;
    let directory = PathBuf::from(args.next().context("output directory required")?);
    std::fs::create_dir_all(&directory)?;
    let (conn, screen) = x11rb::connect(None)?;
    let root = conn.setup().roots[screen].root;
    let overlay = atom(&conn, "STEAM_OVERLAY")?;
    let registered = gamebuddy_overlay::x11::client_windows(&conn, root)?
        .into_iter()
        .filter(|w| cardinal(&conn, *w, overlay).unwrap_or(0) != 0)
        .collect::<Vec<_>>();
    ensure!(
        registered.len() == 1,
        "Expected exactly one GameBuddy overlay in private display"
    );
    let game = capture_game(pid, &directory.join("game-only.png"))?;
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
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if std::fs::metadata("/tmp/gamescope.png")
            .and_then(|m| m.modified())
            .ok()
            != previous
            && image::open("/tmp/gamescope.png").is_ok()
        {
            break;
        }
        ensure!(Instant::now() < deadline, "Gamescope screenshot timed out");
        std::thread::sleep(Duration::from_millis(50));
    }
    std::fs::copy("/tmp/gamescope.png", directory.join("composited.png"))?;
    println!(
        "GAMEBUDDY_CAPTURE_PASS game_window={game} overlay_window={}",
        registered[0]
    );
    Ok(())
}
#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("Linux only");
}
