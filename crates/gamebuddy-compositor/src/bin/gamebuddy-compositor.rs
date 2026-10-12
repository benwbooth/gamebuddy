//! Standalone runner for testing: `gamebuddy-compositor [--fullscreen]
//! [--size WxH] [--ready-file PATH] [-- COMMAND ARGS...]`. GameBuddy itself
//! runs the compositor in-process (`gamebuddy --nested-compositor ...`).

fn main() -> anyhow::Result<()> {
    gamebuddy_compositor::init_logging();
    gamebuddy_compositor::run(gamebuddy_compositor::Config::from_args(
        std::env::args_os().skip(1),
    )?)
}
