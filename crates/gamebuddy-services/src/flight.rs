//! The flight recorder: one timeline per play session, kept after the game
//! closes, for finding out what happened and for replaying it later.
//!
//! A session directory holds `timeline.jsonl` (one event per line: frames,
//! location and plan changes, model calls with their latency, voice events,
//! what was said), `frames/` (downscaled game frames when the screen
//! changes) and `app.log` (everything the app and its helpers printed, which
//! otherwise vanished with the launcher's temporary folder).
//!
//! Recording is process-wide and does nothing until `start` is called, so
//! command-line tools and probes never write sessions.
use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    fs::{File, OpenOptions},
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

/// Keep at most this much recorded history; oldest sessions go first.
const BUDGET_BYTES: u64 = 4_000_000_000;
/// A frame is kept when the screen changed, or at least this often.
const FRAME_EVERY_MS: u64 = 10_000;
const FRAME_MIN_GAP_MS: u64 = 1_000;
const FRAME_WIDTH: u32 = 480;

struct Recorder {
    dir: PathBuf,
    timeline: BufWriter<File>,
    last_frame: Option<(u64, Vec<u8>)>,
    documents: HashMap<String, u64>,
}

fn current() -> &'static Mutex<Option<Recorder>> {
    static CURRENT: OnceLock<Mutex<Option<Recorder>>> = OnceLock::new();
    CURRENT.get_or_init(|| Mutex::new(None))
}

fn now_ms() -> u64 {
    crate::screen::now_ms()
}

fn slug(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

pub fn sessions_dir(root: &Path) -> PathBuf {
    root.join("sessions")
}

/// Begin recording this process's play session. Returns its directory.
pub fn start(root: &Path, game: &str, details: Value) -> Result<PathBuf> {
    prune(&sessions_dir(root), BUDGET_BYTES);
    let stamp = chrono_stamp(now_ms());
    let dir = sessions_dir(root).join(format!("{stamp}-{}", slug(game)));
    std::fs::create_dir_all(dir.join("frames"))?;
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("timeline.jsonl"))?;
    std::fs::write(
        dir.join("session.json"),
        serde_json::to_vec_pretty(&json!({
            "game": game,
            "started_ms": now_ms(),
            "version": env!("CARGO_PKG_VERSION"),
            "details": details,
        }))?,
    )?;
    *current().lock().unwrap() = Some(Recorder {
        dir: dir.clone(),
        timeline: BufWriter::new(file),
        last_frame: None,
        documents: HashMap::new(),
    });
    event("session_start", json!({"game": game}));
    Ok(dir)
}

/// The directory of the session being recorded, if any.
pub fn session_dir() -> Option<PathBuf> {
    current().lock().ok()?.as_ref().map(|r| r.dir.clone())
}

fn write_line(recorder: &mut Recorder, line: &str) {
    let _ = recorder.timeline.write_all(line.as_bytes());
    let _ = recorder.timeline.write_all(b"\n");
    let _ = recorder.timeline.flush();
}

/// Record one event. A no-op when no session is being recorded.
pub fn event(kind: &str, value: Value) {
    let Ok(mut guard) = current().lock() else {
        return;
    };
    let Some(recorder) = guard.as_mut() else {
        return;
    };
    let line = json!({"t": now_ms(), "kind": kind, "value": value}).to_string();
    write_line(recorder, &line);
}

/// Store documents whose changes are worth a line in the timeline: where
/// GameBuddy thinks the player is, what it plans next, widget input.
const DOCUMENTS: [&str; 10] = [
    "place-tracker:",
    "location-estimate:",
    "world-tracker:",
    "next-action:",
    "off-route:",
    "player-places:",
    "area-candidate:",
    "level-maps:",
    "widget-layout-status",
    "widget-events",
];

/// Called for every store write; records the tracked documents when their
/// value changed. `body` is the stored JSON text.
pub fn document(key: &str, body: &str) {
    if !DOCUMENTS.iter().any(|p| key.starts_with(p)) {
        return;
    }
    let Ok(mut guard) = current().lock() else {
        return;
    };
    let Some(recorder) = guard.as_mut() else {
        return;
    };
    let hash = {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        body.hash(&mut h);
        h.finish()
    };
    if recorder.documents.insert(key.to_owned(), hash) == Some(hash) {
        return;
    }
    let line = format!(
        "{{\"t\":{},\"kind\":\"doc\",\"key\":{},\"value\":{}}}",
        now_ms(),
        Value::String(key.to_owned()),
        body
    );
    write_line(recorder, &line);
}

/// A tiny grayscale signature for change detection.
fn signature(image: &image::RgbImage) -> Vec<u8> {
    image::imageops::resize(
        &image::DynamicImage::ImageRgb8(image.clone()).to_luma8(),
        32,
        24,
        image::imageops::FilterType::Triangle,
    )
    .into_raw()
}

/// Keep a downscaled copy of a captured game frame when the screen changed
/// (or every ten seconds), at most once a second.
pub fn frame(at_ms: u64, image: &image::RgbImage) {
    let Ok(mut guard) = current().lock() else {
        return;
    };
    let Some(recorder) = guard.as_mut() else {
        return;
    };
    let sig = signature(image);
    if let Some((last_at, last_sig)) = &recorder.last_frame {
        let gap = at_ms.saturating_sub(*last_at);
        if gap < FRAME_MIN_GAP_MS {
            return;
        }
        let diff = sig
            .iter()
            .zip(last_sig)
            .map(|(a, b)| a.abs_diff(*b) as u32)
            .sum::<u32>() as f32
            / sig.len() as f32;
        if diff < 4. && gap < FRAME_EVERY_MS {
            return;
        }
    }
    let small = image::imageops::thumbnail(
        image,
        FRAME_WIDTH,
        (image.height() * FRAME_WIDTH / image.width().max(1)).max(1),
    );
    let name = format!("{at_ms}.jpg");
    let mut bytes = vec![];
    if image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 75)
        .encode_image(&small)
        .is_err()
        || std::fs::write(recorder.dir.join("frames").join(&name), bytes).is_err()
    {
        return;
    }
    recorder.last_frame = Some((at_ms, sig));
    let line = json!({"t": at_ms, "kind": "frame", "value": {"file": name}}).to_string();
    write_line(recorder, &line);
}

/// Copy everything written to this process's stderr (and its helpers'
/// stderr, which they inherit) into `path`, while still printing it.
#[cfg(unix)]
pub fn tee_stderr(path: &Path) -> Result<()> {
    use std::io::Read;
    use std::os::fd::FromRawFd;
    let mut log = OpenOptions::new().create(true).append(true).open(path)?;
    let mut fds = [0; 2];
    // SAFETY: plain POSIX calls on descriptors this function owns; fd 2 is
    // replaced by the pipe's write end and the original kept for echoing.
    unsafe {
        anyhow::ensure!(libc::pipe(fds.as_mut_ptr()) == 0, "pipe failed");
        let original = libc::dup(2);
        anyhow::ensure!(original >= 0, "dup failed");
        anyhow::ensure!(libc::dup2(fds[1], 2) >= 0, "dup2 failed");
        libc::close(fds[1]);
        let mut reader = File::from_raw_fd(fds[0]);
        let mut echo = File::from_raw_fd(original);
        std::thread::Builder::new()
            .name("stderr-tee".into())
            .spawn(move || {
                let mut buffer = [0u8; 8192];
                while let Ok(n) = reader.read(&mut buffer) {
                    if n == 0 {
                        break;
                    }
                    let _ = echo.write_all(&buffer[..n]);
                    let _ = log.write_all(&buffer[..n]);
                }
            })
            .context("Could not start the log copier")?;
    }
    Ok(())
}

#[cfg(not(unix))]
pub fn tee_stderr(_path: &Path) -> Result<()> {
    Ok(())
}

fn dir_size(path: &Path) -> u64 {
    std::fs::read_dir(path)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| match e.metadata() {
            Ok(m) if m.is_dir() => dir_size(&e.path()),
            Ok(m) => m.len(),
            Err(_) => 0,
        })
        .sum()
}

/// Delete the oldest sessions until the rest fit in `budget` bytes.
fn prune(sessions: &Path, budget: u64) {
    let mut all: Vec<(PathBuf, u64)> = std::fs::read_dir(sessions)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| (e.path(), dir_size(&e.path())))
        .collect();
    // Directory names start with a sortable timestamp.
    all.sort();
    let mut total: u64 = all.iter().map(|(_, s)| s).sum();
    for (path, size) in all {
        if total <= budget {
            break;
        }
        if std::fs::remove_dir_all(&path).is_ok() {
            total -= size;
        }
    }
}

/// UTC `YYYYMMDD-HHMMSS` without a date library.
fn chrono_stamp(ms: u64) -> String {
    let secs = ms / 1000;
    let (days, rem) = (secs / 86_400, secs % 86_400);
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}{m:02}{d:02}-{:02}{:02}{:02}",
        rem / 3600,
        rem / 60 % 60,
        rem % 60
    )
}

fn percentile(values: &mut [f64], q: f64) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(f64::total_cmp);
    let i = ((values.len() - 1) as f64 * q).round() as usize;
    values.get(i).copied()
}

fn spread(mut values: Vec<f64>) -> Value {
    json!({
        "count": values.len(),
        "median": percentile(&mut values, 0.5),
        "p90": percentile(&mut values, 0.9),
        "max": percentile(&mut values, 1.0),
    })
}

/// Recorded sessions, newest first.
pub fn sessions(root: &Path) -> Vec<PathBuf> {
    let mut all: Vec<PathBuf> = std::fs::read_dir(sessions_dir(root))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join("timeline.jsonl").is_file())
        .collect();
    all.sort();
    all.reverse();
    all
}

/// Every event of a recorded session, in order.
pub fn timeline(dir: &Path) -> Result<Vec<Value>> {
    let text = std::fs::read_to_string(dir.join("timeline.jsonl"))
        .with_context(|| format!("No timeline in {}", dir.display()))?;
    Ok(text
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect())
}

/// What a session looked like: how long replies took, how often location
/// and plan changed, model calls by purpose with their latency, errors.
pub fn report(dir: &Path) -> Result<Value> {
    let events = timeline(dir)?;
    let first = events.first().and_then(|e| e["t"].as_u64()).unwrap_or(0);
    let last = events.last().and_then(|e| e["t"].as_u64()).unwrap_or(0);
    let mut kinds: HashMap<String, usize> = HashMap::new();
    let mut models: HashMap<String, (Vec<f64>, usize)> = HashMap::new();
    let mut rate_limited = 0;
    let (mut reply_start, mut delegated) = (vec![], vec![]);
    let (mut heard_at, mut dispatched_at): (Option<u64>, Option<u64>) = (None, None);
    let mut places: Vec<String> = vec![];
    let (mut place_changes, mut progress_changes) = (0, 0);
    let mut said = (0, 0);
    for e in &events {
        let kind = e["kind"].as_str().unwrap_or_default();
        *kinds.entry(kind.to_owned()).or_default() += 1;
        let t = e["t"].as_u64().unwrap_or(0);
        let v = &e["value"];
        match kind {
            "model" => {
                let purpose: String = v["purpose"]
                    .as_str()
                    .unwrap_or_default()
                    .chars()
                    .take(48)
                    .collect();
                let entry = models
                    .entry(format!(
                        "{} · {}",
                        v["model"].as_str().unwrap_or("?"),
                        purpose
                    ))
                    .or_default();
                entry.0.push(v["ms"].as_f64().unwrap_or(0.) / 1000.);
                if v["error"].is_string() {
                    entry.1 += 1;
                    if v["error"]
                        .as_str()
                        .is_some_and(|s| s.contains("429") || s.contains("Too Many Requests"))
                    {
                        rate_limited += 1;
                    }
                }
            }
            "voice" => match v["kind"].as_str().unwrap_or_default() {
                // The player finished speaking; the next audio out is the reply.
                "thread/realtime/transcript/done" if v["value"]["role"] == "user" => {
                    heard_at = Some(t)
                }
                "browser/output-active" => {
                    if let Some(h) = heard_at.take() {
                        reply_start.push(t.saturating_sub(h) as f64 / 1000.);
                    }
                }
                "request_dispatched" => dispatched_at = Some(t),
                "answer_ready" => {
                    if let Some(d) = dispatched_at.take() {
                        delegated.push(t.saturating_sub(d) as f64 / 1000.);
                    }
                }
                _ => {}
            },
            "doc"
                if e["key"]
                    .as_str()
                    .is_some_and(|k| k.starts_with("place-tracker:")) =>
            {
                place_changes += 1;
                if let Some(p) = v["place_id"].as_str()
                    && !places.iter().any(|x| x == p)
                {
                    places.push(p.to_owned());
                }
            }
            "progress" => progress_changes += 1,
            "said" if v["role"] == "user" => said.0 += 1,
            "said" => said.1 += 1,
            _ => {}
        }
    }
    let log = std::fs::read_to_string(dir.join("app.log")).unwrap_or_default();
    let log_lines = |needle: &[&str]| {
        log.lines()
            .filter(|l| {
                let l = l.to_lowercase();
                needle.iter().any(|n| l.contains(n))
            })
            .count()
    };
    let mut by_model: Vec<(String, Value)> = models
        .into_iter()
        .map(|(k, (ms, errors))| {
            let mut s = spread(ms);
            s["errors"] = json!(errors);
            (k, s)
        })
        .collect();
    by_model.sort_by_key(|(_, s)| std::cmp::Reverse(s["count"].as_u64().unwrap_or(0)));
    Ok(json!({
        "session": dir.file_name().map(|n| n.to_string_lossy().into_owned()),
        "minutes": (last.saturating_sub(first)) as f64 / 60_000.,
        "events": kinds,
        "voice": {
            "reply_start_s": spread(reply_start),
            "delegated_answer_s": spread(delegated),
            "player_utterances": said.0,
            "buddy_replies": said.1,
        },
        "models": by_model.into_iter().map(|(k, s)| json!({"call": k, "seconds": s})).collect::<Vec<_>>(),
        "rate_limited_calls": rate_limited,
        "location": {"changes": place_changes, "distinct_places": places.len()},
        "plan_changes": progress_changes,
        "log": {"lines": log.lines().count(), "errors": log_lines(&["error", "panic", "failed"]), "rate_limits": log_lines(&["too many requests"])},
    }))
}
