//! Native input for the private Gamescope fixture; refuses desktop X servers.
#[cfg(target_os = "linux")]
fn main() -> anyhow::Result<()> {
    use anyhow::{Context, ensure};
    use x11rb::{
        connection::Connection,
        protocol::{xproto::*, xtest::ConnectionExt as _},
    };
    let (conn, screen) = x11rb::connect(None)?;
    let root = conn.setup().roots[screen].root;
    if std::env::args().nth(1).as_deref() == Some("cursor") {
        use x11rb::protocol::xfixes::ConnectionExt as _;
        conn.xfixes_query_version(5, 0)?.reply()?;
        let cursor = conn.xfixes_get_cursor_image()?.reply()?;
        println!(
            "CURSOR {}x{} hotspot={},{}",
            cursor.width, cursor.height, cursor.xhot, cursor.yhot
        );
        return Ok(());
    }
    let atom = gamebuddy_overlay::x11::atom(&conn, "GAMESCOPE_XWAYLAND_SERVER_ID")?;
    let gamescope = conn
        .get_property(false, root, atom, AtomEnum::CARDINAL, 0, 1)?
        .reply()?
        .value_len
        == 1;
    if !gamescope {
        // The outer-input regression test starts a virtual KWin and passes
        // its PID/display. Never fall back to the player's desktop display.
        let pid: u32 = std::env::var("GAMEBUDDY_TEST_KWIN_PID")?.parse()?;
        let command = std::fs::read(format!("/proc/{pid}/cmdline"))?;
        let executable = command
            .split(|b| *b == 0)
            .next()
            .context("KWin executable")?;
        ensure!(
            std::path::Path::new(std::str::from_utf8(executable)?)
                .file_name()
                .is_some_and(|name| name == "kwin_wayland")
                && command.split(|b| *b == 0).any(|arg| arg == b"--virtual")
                && String::from_utf8_lossy(&command).contains("/tmp/gamebuddy-outer-input-")
                && std::env::var("DISPLAY")? == std::env::var("GAMEBUDDY_TEST_OUTER_DISPLAY")?,
            "Native input is restricted to the isolated Gamescope/KWin fixture"
        );
    }
    let mut args = std::env::args().skip(1);
    let action = args.next().context("action")?;
    let x: i16 = args.next().context("x")?.parse()?;
    let y: i16 = args.next().context("y")?.parse()?;
    let detail: u8 = args.next().unwrap_or_else(|| "1".into()).parse()?;
    let kind = match action.as_str() {
        "move" => MOTION_NOTIFY_EVENT,
        "down" => BUTTON_PRESS_EVENT,
        "up" => BUTTON_RELEASE_EVENT,
        _ => anyhow::bail!("Unknown input action"),
    };
    conn.xtest_fake_input(
        kind,
        if kind == MOTION_NOTIFY_EVENT {
            0
        } else {
            detail
        },
        0,
        root,
        x,
        y,
        0,
    )?
    .check()?;
    conn.flush()?;
    Ok(())
}
#[cfg(not(target_os = "linux"))]
fn main() {
    panic!("Gamescope input fixture is Linux-only")
}
