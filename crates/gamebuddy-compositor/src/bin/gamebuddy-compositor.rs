//! Standalone runner: `gamebuddy-compositor [--fullscreen] [--size WxH]
//! [--ready-file PATH] [-- COMMAND ARGS...]`.

use std::{ffi::OsString, path::PathBuf};

use anyhow::{Context, Result, bail};
use gamebuddy_compositor::Config;

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let mut config = Config::default();
    let mut args = std::env::args_os().skip(1);
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
            Some("--") => {
                config.command = args.by_ref().collect::<Vec<OsString>>();
            }
            _ => bail!("Unknown argument {}", arg.to_string_lossy()),
        }
    }
    gamebuddy_compositor::run(config)
}
