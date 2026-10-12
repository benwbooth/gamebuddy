//! GameBuddy's nested compositor: one private Xwayland server shown in a
//! window on the host desktop. Gamescope 3.16.29 is the behavioral reference
//! (see docs/compositor-plan.md).

mod handlers;
mod input;
mod state;
mod winit;

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

pub fn run(config: Config) -> Result<()> {
    let mut event_loop: EventLoop<'static, State> =
        EventLoop::try_new().context("Creating the compositor event loop")?;
    let display: Display<State> = Display::new().context("Creating the Wayland display")?;
    let mut state = State::new(&mut event_loop, display, config)?;
    winit::init(&mut event_loop, &mut state)?;
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
        .env_remove("WAYLAND_DISPLAY")
        .spawn()
        .with_context(|| format!("Starting {}", program.to_string_lossy()))?;
    Ok(())
}
