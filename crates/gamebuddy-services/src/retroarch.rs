//! The game picture straight from RetroArch, before its shaders.
//!
//! A captured window shows whatever the player's shader draws (a CRT's
//! curved glass, scanlines, a bezel), which the walkthrough's clean frames
//! do not. RetroArch can save the core's own frame instead: with
//! `video_gpu_screenshot` off, its `SCREENSHOT` network command writes the
//! unfiltered picture (256x240 for the NES) in about 30 ms. RetroArch's
//! screenshot flash and popup are turned off, or every frame GameBuddy asks
//! for would flicker the player's screen. The launcher adds
//! this module's config to RetroArch's appended configs when GameBuddy runs;
//! the capture loop then asks for a frame and falls back to the window when
//! none arrives. Only the picture is read, never the emulator's memory.
use anyhow::Result;
use std::{
    net::UdpSocket,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, Instant},
};

/// RetroArch's command port while GameBuddy runs (not its 55355 default, so
/// it never collides with a port the player set up for something else).
pub const PORT: u16 = 55393;

pub fn dir(root: &Path) -> PathBuf {
    root.join("retroarch")
}

fn frames(root: &Path) -> PathBuf {
    dir(root).join("frames")
}

/// Write the RetroArch config the launcher appends, and return its path.
/// The folder must be outside /tmp: Flatpak RetroArch has a private /tmp.
/// Raw frames are off while `<data>/retroarch/off` exists: GameBuddy then
/// looks at the game window as the player sees it, and RetroArch is never
/// asked for a screenshot. For telling whether the frame requests disturb
/// the picture, and for emulators or shaders where they do.
pub fn disabled(root: &Path) -> bool {
    dir(root).join("off").is_file()
}

pub fn write_config(root: &Path) -> Result<PathBuf> {
    let frames = frames(root);
    std::fs::create_dir_all(&frames)?;
    for old in std::fs::read_dir(&frames)?.flatten() {
        let _ = std::fs::remove_file(old.path());
    }
    let path = dir(root).join("raw-frames.cfg");
    if disabled(root) {
        std::fs::write(
            &path,
            "# Written by GameBuddy. Raw frames are turned off (retroarch/off exists).\n",
        )?;
        return Ok(path);
    }
    std::fs::write(
        &path,
        format!(
            "# Written by GameBuddy for the duration of a game it watches.\n\
             network_cmd_enable = \"true\"\n\
             network_cmd_port = \"{PORT}\"\n\
             video_gpu_screenshot = \"false\"\n\
             notification_show_screenshot = \"false\"\n\
             notification_show_screenshot_flash = \"0\"\n\
             notification_show_screenshot_duration = \"0\"\n\
             screenshots_in_content_dir = \"false\"\n\
             screenshot_directory = \"{}\"\n",
            frames.display()
        ),
    )?;
    Ok(path)
}

struct Health {
    misses: u32,
    paused_until: Option<Instant>,
}

static HEALTH: Mutex<Health> = Mutex::new(Health {
    misses: 0,
    paused_until: None,
});

/// Ask RetroArch for the current unfiltered frame and move it to `out`.
/// None when RetroArch is not answering (an older launch, another emulator):
/// after three misses in a row it is left alone for a minute.
pub fn frame(root: &Path, out: &Path) -> Result<Option<PathBuf>> {
    let frames = frames(root);
    if disabled(root) || !dir(root).join("raw-frames.cfg").is_file() || !frames.is_dir() {
        return Ok(None);
    }
    {
        let health = HEALTH.lock().unwrap();
        if health.paused_until.is_some_and(|t| Instant::now() < t) {
            return Ok(None);
        }
    }
    let before: Vec<PathBuf> = pngs(&frames);
    let socket = UdpSocket::bind("127.0.0.1:0")?;
    socket.send_to(b"SCREENSHOT", ("127.0.0.1", PORT))?;
    let deadline = Instant::now() + Duration::from_millis(250);
    let fresh = loop {
        if let Some(path) = pngs(&frames).into_iter().find(|p| !before.contains(p)) {
            break Some(path);
        }
        if Instant::now() >= deadline {
            break None;
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    let mut health = HEALTH.lock().unwrap();
    let Some(path) = fresh else {
        health.misses += 1;
        if health.misses >= 3 {
            health.misses = 0;
            health.paused_until = Some(Instant::now() + Duration::from_secs(60));
        }
        return Ok(None);
    };
    health.misses = 0;
    drop(health);
    // The file appears before RetroArch finishes writing it: wait until it
    // decodes, briefly.
    let written = Instant::now() + Duration::from_millis(200);
    while image::open(&path).is_err() {
        if Instant::now() >= written {
            let _ = std::fs::remove_file(&path);
            return Ok(None);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    std::fs::rename(&path, out)?;
    for stale in before {
        let _ = std::fs::remove_file(stale);
    }
    Ok(Some(out.to_path_buf()))
}

fn pngs(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "png"))
        .collect()
}
