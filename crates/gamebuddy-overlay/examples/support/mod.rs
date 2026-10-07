//! Serialize diagnostics using Gamescope's fixed legacy screenshot filename.
#[allow(dead_code)]
pub mod crop_probe;
#[allow(dead_code)]
pub mod navigation_probe;
#[allow(dead_code)]
pub mod voice_probe;
use std::{
    fs::{File, OpenOptions},
    os::{fd::AsRawFd, unix::fs::OpenOptionsExt},
};
/// Read the production QML control geometry, including resized panels/popups.
/// Coordinates are private Gamescope pixels at the fixture's 1x scale.
#[allow(dead_code)]
pub fn companion_center(id: &str) -> anyhow::Result<(i16, i16)> {
    let log = std::env::var("GAMEBUDDY_TEST_LOG")?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let text = std::fs::read_to_string(&log)?;
        if let Some(data) = text.lines().rev().find_map(|line| {
            line.split_once("GAMEBUDDY_COMPANION_LAYOUT ")
                .map(|(_, data)| data)
        }) && let Ok(value) = serde_json::from_str::<serde_json::Value>(data)
            && let Some(rect) = value[id].as_array()
            && rect.len() == 4
        {
            return Ok((
                (rect[0].as_f64().unwrap() + rect[2].as_f64().unwrap() / 2.).round() as i16,
                (rect[1].as_f64().unwrap() + rect[3].as_f64().unwrap() / 2.).round() as i16,
            ));
        }
        anyhow::ensure!(
            std::time::Instant::now() < deadline,
            "Companion control {id} not visible; run with --ui-trace"
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}
pub fn screenshot_lock() -> anyhow::Result<File> {
    let root = std::env::var_os("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .mode(0o600)
        .open(root.join("gamebuddy-screenshot-test.lock"))?;
    // SAFETY: the file owns this valid descriptor for the lifetime of the lock.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(file)
}
