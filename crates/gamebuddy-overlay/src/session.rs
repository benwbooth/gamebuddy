//! Start Gamescope before the game. The bridge protocol lets Lunchpail keep
//! ownership of the real emulator Child (including its controller pipes).
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    ffi::OsString,
    fs,
    os::unix::{fs::PermissionsExt, process::CommandExt},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

#[derive(Debug, Serialize, Deserialize)]
pub struct Ready {
    pub version: u32,
    pub display: String,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct Attachment {
    pub game_pid: u32,
}

pub fn atomic_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, serde_json::to_vec(value)?)?;
    fs::rename(tmp, path)?;
    Ok(())
}

pub struct OwnedChild(pub Child);
impl Drop for OwnedChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn owned_spawn(command: &mut Command) -> Result<OwnedChild> {
    // No allocation or locking after fork. Each owned child dies with its
    // supervisor, including when the launcher is abruptly terminated.
    let parent = std::process::id();
    unsafe {
        command.pre_exec(move || {
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            if libc::getppid() as u32 != parent {
                return Err(std::io::Error::other("session supervisor exited"));
            }
            Ok(())
        });
    }
    Ok(OwnedChild(command.spawn()?))
}

#[derive(Default, Debug)]
struct Options {
    bridge: Option<PathBuf>,
    inside: bool,
    launch: bool,
    windowed: bool,
    width: Option<u32>,
    height: Option<u32>,
    backend: Option<OsString>,
    ui_args: Vec<OsString>,
    game: Vec<OsString>,
    result: Option<PathBuf>,
    control: Option<PathBuf>,
}
impl Options {
    fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Self> {
        let mut opts = Self::default();
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            match arg.to_str() {
                Some("--") => {
                    opts.game.extend(args);
                    break;
                }
                Some("--session-control") => {
                    opts.control = Some(args.next().context("missing session control path")?.into())
                }
                Some("--session-result") => {
                    opts.result = Some(args.next().context("missing session result path")?.into())
                }
                Some("--launch") => opts.launch = true,
                Some("--inside-gamescope") => opts.inside = true,
                Some("--gamescope-bridge") => {
                    opts.bridge = Some(
                        args.next()
                            .context("--gamescope-bridge needs a directory")?
                            .into(),
                    )
                }
                Some("--windowed") => opts.windowed = true,
                Some("--width" | "--height") => {
                    let value: u32 = args
                        .next()
                        .context("size needs a value")?
                        .to_str()
                        .context("invalid size")?
                        .parse()?;
                    ensure!(
                        (320..=7680).contains(&value),
                        "size must be between 320 and 7680"
                    );
                    if arg == "--width" {
                        opts.width = Some(value);
                    } else {
                        opts.height = Some(value);
                    }
                }
                Some("--gamescope-backend") => {
                    let value = args.next().context("--gamescope-backend needs a value")?;
                    ensure!(
                        matches!(
                            value.to_str(),
                            Some("auto" | "wayland" | "sdl" | "headless")
                        ),
                        "unsupported Gamescope backend"
                    );
                    opts.backend = Some(value);
                }
                Some(
                    "--game" | "--game-id" | "--platform" | "--session-id" | "--data-dir"
                    | "--game-pid" | "--replay" | "--smoke-test",
                ) => {
                    opts.ui_args.push(arg);
                    opts.ui_args
                        .push(args.next().context("GameBuddy option needs a value")?);
                }
                _ => opts.ui_args.push(arg),
            }
        }
        Ok(opts)
    }
}

/// Call before creating any Qt objects. Returns None for the ordinary UI mode.
pub fn dispatch() -> Result<Option<i32>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.first().is_some_and(|a| a == "--overlay-action") {
        ensure!(
            args.len() == 3,
            "Usage: gamebuddy --overlay-action ACTION SOCKET_PATH"
        );
        crate::control::send(
            Path::new(&args[2]),
            args[1].to_str().context("Invalid action")?,
        )?;
        return Ok(Some(0));
    }
    if !args
        .iter()
        .take_while(|a| *a != "--")
        .any(|a| a == "--launch" || a == "--gamescope-bridge" || a == "--inside-gamescope")
    {
        return Ok(None);
    }
    let opts = Options::parse(args)?;
    let result_path = opts.result.clone();
    let is_inside = opts.inside;
    let result = if is_inside { inside(opts) } else { outer(opts) };
    if is_inside && let Some(path) = result_path {
        atomic_json(&path, &result.as_ref().copied().unwrap_or(1))?;
    }
    result.map(Some)
}

fn outer(opts: Options) -> Result<i32> {
    ensure!(
        std::env::var_os("GAMEBUDDY_GAMESCOPE").is_none(),
        "Already in a GameBuddy Gamescope session"
    );
    ensure!(
        opts.bridge.is_some() || (opts.launch && !opts.game.is_empty()),
        "Usage: gamebuddy --launch --game TITLE [--windowed] -- PROGRAM [ARG ...]"
    );
    let runtime = tempfile::Builder::new()
        .prefix("gamebuddy-session-")
        .tempdir()?;
    fs::set_permissions(runtime.path(), fs::Permissions::from_mode(0o700))?;
    let result_path = runtime.path().join("exit.json");
    let mut cmd = Command::new(
        std::env::var_os("GAMEBUDDY_GAMESCOPE_BIN").unwrap_or_else(|| "gamescope".into()),
    );
    cmd.args([
        "-w",
        &opts.width.unwrap_or(1280).to_string(),
        "-h",
        &opts.height.unwrap_or(720).to_string(),
        "--xwayland-count",
        "1",
    ]);
    if !opts.windowed {
        cmd.arg("--fullscreen");
    }
    if let Some(backend) = &opts.backend {
        cmd.arg("--backend").arg(backend);
    }
    cmd.arg("--")
        .arg(std::env::current_exe()?)
        .arg("--inside-gamescope")
        .arg("--session-result")
        .arg(&result_path);
    if let Some(dir) = &opts.bridge {
        cmd.arg("--gamescope-bridge").arg(dir);
    }
    cmd.arg("--session-control").arg(
        opts.bridge
            .as_ref()
            .unwrap_or(&runtime.path().to_path_buf())
            .join("control.sock"),
    );
    cmd.args(&opts.ui_args);
    if !opts.game.is_empty() {
        cmd.arg("--").args(&opts.game);
    }
    cmd.stdin(Stdio::null());
    let mut child = owned_spawn(&mut cmd).context(
        "Starting Gamescope. Install gamescope or set GAMEBUDDY_GAMESCOPE_BIN to its executable",
    )?;
    loop {
        if let Some(status) = child.0.try_wait()? {
            return Ok(fs::read(&result_path)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<i32>(&bytes).ok())
                .unwrap_or_else(|| status.code().filter(|code| *code != 0).unwrap_or(1)));
        }
        if let Some(dir) = &opts.bridge
            && (!dir.is_dir() || dir.join("stop").exists())
        {
            return Ok(0);
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn inside(opts: Options) -> Result<i32> {
    let display =
        std::env::var("DISPLAY").context("Gamescope did not provide an Xwayland display")?;
    ensure!(
        std::env::var_os("GAMESCOPE_WAYLAND_DISPLAY").is_some(),
        "--inside-gamescope must be started by Gamescope"
    );
    let mut game_child = None;
    let pid;
    if let Some(dir) = &opts.bridge {
        let meta = fs::symlink_metadata(dir).context("bridge directory is missing")?;
        ensure!(
            meta.is_dir() && meta.permissions().mode() & 0o077 == 0,
            "bridge directory must be private (mode 0700)"
        );
        atomic_json(
            &dir.join("ready.json"),
            &Ready {
                version: 1,
                display,
            },
        )?;
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            if !dir.is_dir() || dir.join("stop").exists() {
                return Ok(0);
            }
            if let Ok(bytes) = fs::read(dir.join("game.json")) {
                pid = serde_json::from_slice::<Attachment>(&bytes)?.game_pid;
                ensure!(pid > 1, "invalid game PID");
                break;
            }
            ensure!(
                Instant::now() < deadline,
                "Timed out waiting for the launcher's game PID"
            );
            thread::sleep(Duration::from_millis(25));
        }
    } else {
        let program = opts.game.first().context("No game command provided")?;
        let mut cmd = Command::new(program);
        if Path::new(program)
            .file_name()
            .is_some_and(|name| name == "flatpak")
            && opts.game.get(1).is_some_and(|arg| arg == "run")
        {
            cmd.args(["run", "--socket=x11", "--nosocket=wayland"])
                .args(&opts.game[2..]);
        } else {
            cmd.args(&opts.game[1..]);
        }
        cmd.env_remove("WAYLAND_DISPLAY")
            .env("SDL_VIDEODRIVER", "x11")
            .env("QT_QPA_PLATFORM", "xcb");
        if let Some(path) = &opts.control {
            cmd.env("GAMEBUDDY_CONTROL_SOCKET", path);
        }
        let child = owned_spawn(&mut cmd).context("Starting the game")?;
        pid = child.0.id();
        game_child = Some(child);
    }
    let mut ui = Command::new(std::env::current_exe()?);
    ui.args(&opts.ui_args)
        .args(["--game-pid", &pid.to_string()])
        .env("GAMEBUDDY_GAMESCOPE", "1")
        .env("QT_QPA_PLATFORM", "xcb")
        .env_remove("WAYLAND_DISPLAY")
        .stdin(Stdio::null());
    if let Some(path) = &opts.control {
        ui.env("GAMEBUDDY_CONTROL_SOCKET", path);
        eprintln!("GAMEBUDDY_CONTROL_SOCKET {}", path.display());
    }
    let mut overlay = owned_spawn(&mut ui).context("Starting the overlay")?;
    eprintln!(
        "GAMEBUDDY_SESSION game_pid={pid} overlay_pid={} display={}",
        overlay.0.id(),
        std::env::var("DISPLAY")?
    );
    let mut overlay_exited = false;
    loop {
        if let Some(game) = &mut game_child {
            if let Some(status) = game.0.try_wait()? {
                return Ok(status.code().unwrap_or(1));
            }
        } else if !Path::new(&format!("/proc/{pid}")).exists() {
            return Ok(0);
        }
        if let Some(dir) = &opts.bridge
            && (!dir.is_dir() || dir.join("stop").exists())
        {
            return Ok(0);
        }
        if !overlay_exited && let Some(status) = overlay.0.try_wait()? {
            eprintln!("GAMEBUDDY_OVERLAY_EXIT {status}; game remains running");
            overlay_exited = true;
        }
        thread::sleep(Duration::from_millis(100));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn command_boundary_preserves_literal_game_arguments() {
        let opts = Options::parse(
            [
                "--launch",
                "--game",
                "Faxanadu; $(bad)",
                "--windowed",
                "--",
                "emulator",
                "a b.nes",
                "--width",
                "123",
            ]
            .map(OsString::from),
        )
        .unwrap();
        assert!(opts.launch && opts.windowed);
        assert_eq!(opts.ui_args, ["--game", "Faxanadu; $(bad)"]);
        assert_eq!(opts.game, ["emulator", "a b.nes", "--width", "123"]);
        assert!(opts.width.is_none());
        let reserved_title =
            Options::parse(["--launch", "--game", "--windowed", "--", "game"].map(OsString::from))
                .unwrap();
        assert!(!reserved_title.windowed);
        assert_eq!(reserved_title.ui_args, ["--game", "--windowed"]);
    }
    #[test]
    fn rejects_invalid_compositor_options() {
        assert!(Options::parse(["--width", "0"].map(OsString::from)).is_err());
        assert!(Options::parse(["--gamescope-backend", "arbitrary"].map(OsString::from)).is_err());
    }
}
