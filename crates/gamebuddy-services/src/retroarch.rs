//! The game picture straight from RetroArch, before its shaders.
//!
//! A captured window shows whatever the player's shader draws (a CRT's
//! curved glass, scanlines, a bezel), which the walkthrough's clean frames
//! do not. RetroArch can record the core's own output instead: with
//! `video_gpu_record` off, its recorder writes every unfiltered frame
//! (256x240 for the NES, 60 a second) to a file without touching what the
//! player sees. Asking it for screenshots instead flickered the screen, since
//! each one re-presented a frame. GameBuddy starts the recording over
//! RetroArch's command port, follows the growing file with ffmpeg, which
//! writes the newest frame as a PNG four times a second, and hands that to
//! the capture loop. Only the picture is read, never the emulator's memory.
//!
//! The launcher adds this module's config to RetroArch's appended configs
//! when GameBuddy runs; the capture loop asks for a frame and falls back to
//! the window until the recording is up.
use anyhow::{Context, Result};
use std::{
    net::UdpSocket,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::Mutex,
    time::{Duration, Instant, SystemTime},
};

/// RetroArch's command port while GameBuddy runs (not its 55355 default, so
/// it never collides with a port the player set up for something else).
pub const PORT: u16 = 55393;
/// Frames per second ffmpeg extracts from the recording.
const FRAMES_PER_SECOND: u32 = 4;
/// A recording this old is from an earlier game, not the one running now.
const STALE_RECORDING: Duration = Duration::from_secs(60);
/// Start a fresh recording file this often (a one-line RetroArch notice each
/// time) so a long evening does not leave one multi-gigabyte file.
const ROTATE_AFTER: Duration = Duration::from_secs(45 * 60);
/// Without a new frame for this long the follower is restarted.
const FOLLOWER_TIMEOUT: Duration = Duration::from_secs(20);

pub fn dir(root: &Path) -> PathBuf {
    root.join("retroarch")
}

fn recordings(root: &Path) -> PathBuf {
    dir(root).join("recordings")
}

fn latest_png(root: &Path) -> PathBuf {
    dir(root).join("latest.png")
}

/// Raw frames are off while `<data>/retroarch/off` exists: GameBuddy then
/// looks at the game window as the player sees it, and RetroArch is never
/// asked for anything. For emulators or setups where the recorder misbehaves.
pub fn disabled(root: &Path) -> bool {
    dir(root).join("off").is_file()
}

pub fn write_config(root: &Path) -> Result<PathBuf> {
    let recordings = recordings(root);
    std::fs::create_dir_all(&recordings)?;
    for old in std::fs::read_dir(&recordings)?.flatten() {
        let _ = std::fs::remove_file(old.path());
    }
    let _ = std::fs::remove_file(latest_png(root));
    let _ = std::fs::remove_dir_all(dir(root).join("frames"));
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
             record_driver = \"ffmpeg\"\n\
             video_gpu_record = \"false\"\n\
             video_post_filter_record = \"false\"\n\
             video_record_quality = \"3\"\n\
             video_record_threads = \"2\"\n\
             video_record_scale_factor = \"1\"\n\
             recording_output_directory = \"{}\"\n",
            recordings.display()
        ),
    )?;
    Ok(path)
}

#[derive(Default)]
struct Recording {
    /// The file RetroArch is writing and when it was first seen.
    file: Option<(PathBuf, Instant)>,
    follower: Option<Child>,
    last_toggle: Option<Instant>,
    /// Modification time of the last frame handed out, and when it arrived.
    served: Option<SystemTime>,
    last_frame_at: Option<Instant>,
    /// Set after a rotation stop; the next call starts the new recording.
    restarting: bool,
}

static RECORDING: Mutex<Recording> = Mutex::new(Recording {
    file: None,
    follower: None,
    last_toggle: None,
    served: None,
    last_frame_at: None,
    restarting: false,
});

fn toggle_recording(state: &mut Recording) -> Result<()> {
    let socket = UdpSocket::bind("127.0.0.1:0")?;
    socket.send_to(b"RECORDING_TOGGLE", ("127.0.0.1", PORT))?;
    state.last_toggle = Some(Instant::now());
    Ok(())
}

fn newest_recording(root: &Path) -> Option<(PathBuf, SystemTime)> {
    std::fs::read_dir(recordings(root))
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .is_some_and(|e| ["mkv", "webm", "mp4", "avi"].contains(&e.to_str().unwrap_or("")))
        })
        .filter_map(|p| {
            let modified = std::fs::metadata(&p).ok()?.modified().ok()?;
            Some((p, modified))
        })
        .max_by_key(|(_, modified)| *modified)
}

fn stop_follower(state: &mut Recording) {
    if let Some(mut child) = state.follower.take() {
        let _ = child.kill();
        let _ = child.wait();
    }
}

fn start_follower(root: &Path, file: &Path) -> Result<Child> {
    Command::new("ffmpeg")
        .args(["-nostdin", "-loglevel", "error", "-follow", "1", "-i"])
        .arg(file)
        .args([
            "-vf",
            &format!("fps={FRAMES_PER_SECOND}"),
            "-atomic_writing",
            "1",
            "-update",
            "1",
            "-y",
        ])
        .arg(latest_png(root))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("ffmpeg is needed to follow RetroArch's recording")
}

/// The newest unfiltered frame, copied to `out`. `None` while the recording
/// is not up yet (RetroArch still starting, another emulator, raw frames
/// turned off): the caller then captures the window. Once frames flow, the
/// previous frame is handed out again between new ones, so the picture
/// GameBuddy looks at never flips between the clean and the shaded source.
pub fn frame(root: &Path, out: &Path) -> Result<Option<PathBuf>> {
    if disabled(root) || !dir(root).join("raw-frames.cfg").is_file() {
        return Ok(None);
    }
    let mut state = RECORDING.lock().unwrap();
    let now = Instant::now();
    if let Some(child) = state.follower.as_mut()
        && child.try_wait()?.is_some()
    {
        state.follower = None;
    }
    // A long recording is cut into a new file; the old one is deleted.
    if let Some((file, since)) = state.file.clone()
        && now.duration_since(since) > ROTATE_AFTER
        && !state.restarting
    {
        stop_follower(&mut state);
        toggle_recording(&mut state)?;
        state.file = None;
        state.restarting = true;
        let _ = std::fs::remove_file(file);
        return Ok(state
            .served
            .map(|_| out.to_path_buf())
            .filter(|p| p.is_file()));
    }
    if state.file.is_none() {
        let fresh = newest_recording(root).filter(|(_, modified)| {
            SystemTime::now()
                .duration_since(*modified)
                .is_ok_and(|age| age < STALE_RECORDING)
        });
        match fresh {
            Some((file, _)) => {
                state.file = Some((file, now));
                state.restarting = false;
            }
            None => {
                // Ask once, then give RetroArch time: a second toggle would
                // stop the recording that the first one started.
                if state
                    .last_toggle
                    .is_none_or(|t| now.duration_since(t) > Duration::from_secs(8))
                {
                    toggle_recording(&mut state)?;
                }
                return Ok(None);
            }
        }
    }
    if state.follower.is_none()
        && let Some((file, _)) = state.file.clone()
    {
        state.follower = Some(start_follower(root, &file)?);
        state.last_frame_at = Some(now);
    }
    let latest = latest_png(root);
    let modified = std::fs::metadata(&latest)
        .ok()
        .and_then(|m| m.modified().ok());
    if let Some(modified) = modified
        && state.served != Some(modified)
    {
        // ffmpeg writes atomically; a copy keeps `out` stable while it runs.
        std::fs::copy(&latest, out)?;
        state.served = Some(modified);
        state.last_frame_at = Some(now);
        return Ok(Some(out.to_path_buf()));
    }
    if state
        .last_frame_at
        .is_some_and(|t| now.duration_since(t) > FOLLOWER_TIMEOUT)
    {
        // The recording stopped (RetroArch was paused for long, or the
        // player toggled it): start over from whatever is current.
        stop_follower(&mut state);
        state.file = None;
        state.last_toggle = None;
        state.last_frame_at = None;
        return Ok(None);
    }
    Ok(state
        .served
        .map(|_| out.to_path_buf())
        .filter(|p| p.is_file()))
}
