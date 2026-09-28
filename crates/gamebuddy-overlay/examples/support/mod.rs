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
