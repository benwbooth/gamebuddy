//! GameBuddy's nested compositor: one private Xwayland server shown in a
//! window on the host desktop. Gamescope 3.16.29 is the behavioral reference
//! (see docs/compositor-plan.md).

mod handlers;
mod host;
mod host_backend;
mod input;
mod render;
mod state;
mod winit;
mod xprops;

use std::{ffi::OsString, path::PathBuf, process::Command};

use anyhow::{Context, Result};
use smithay::reexports::{calloop::EventLoop, wayland_server::Display};

pub use state::State;

#[derive(Clone, Debug, Default)]
pub struct Config {
    /// Cover the host output instead of opening a regular window.
    pub fullscreen: bool,
    /// Initial window size when not fullscreen.
    pub size: Option<(i32, i32)>,
    /// Program and arguments started once Xwayland is ready, with `DISPLAY` set.
    pub command: Vec<OsString>,
    /// Written with the Xwayland display (for example `:3`) once it is ready.
    pub ready_file: Option<PathBuf>,
}

impl Config {
    /// `[--fullscreen] [--size WxH] [--ready-file PATH] [-- COMMAND ARGS...]`
    pub fn from_args(args: impl IntoIterator<Item = OsString>) -> Result<Self> {
        let mut config = Config::default();
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            match arg.to_str() {
                Some("--fullscreen") => config.fullscreen = true,
                Some("--size") => {
                    let value = args.next().context("--size needs WIDTHxHEIGHT")?;
                    let value = value.to_string_lossy();
                    let (w, h) = value.split_once('x').context("--size needs WIDTHxHEIGHT")?;
                    config.size = Some((w.parse()?, h.parse()?));
                }
                Some("--ready-file") => {
                    config.ready_file = Some(PathBuf::from(
                        args.next().context("--ready-file needs a path")?,
                    ));
                }
                Some("--") => config.command = args.by_ref().collect(),
                _ => anyhow::bail!("Unknown compositor argument {}", arg.to_string_lossy()),
            }
        }
        Ok(config)
    }
}

/// Log to stderr, honouring `RUST_LOG`.
pub fn init_logging() {
    let _ = tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()),
        )
        .try_init();
}

pub fn run(config: Config) -> Result<()> {
    let mut event_loop: EventLoop<'static, State> =
        EventLoop::try_new().context("Creating the compositor event loop")?;
    let display: Display<State> = Display::new().context("Creating the Wayland display")?;
    let mut state = State::new(&mut event_loop, display, config)?;
    if host_backend::available() {
        host_backend::init(&mut event_loop, &mut state)?;
    } else {
        winit::init(&mut event_loop, &mut state)?;
    }
    state.start_xwayland()?;
    event_loop
        .run(None, &mut state, |state| state.after_dispatch())
        .context("Running the compositor")?;
    Ok(())
}

pub(crate) fn spawn_command(command: &[OsString], display: &str) -> Result<()> {
    let Some((program, args)) = command.split_first() else {
        return Ok(());
    };
    Command::new(program)
        .args(args)
        .env("DISPLAY", display)
        // GameBuddy's session supervisor checks it runs inside our compositor.
        .env("GAMEBUDDY_COMPOSITOR", "1")
        .env_remove("WAYLAND_DISPLAY")
        .spawn()
        .with_context(|| format!("Starting {}", program.to_string_lossy()))?;
    Ok(())
}
