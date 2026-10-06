//! Bounded local text-event detection. This does not attempt general scene
//! reasoning, infer inventory from room appearance, or complete objectives.
use crate::{Store, screen};
use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap},
    io::{Read, Write},
    path::PathBuf,
    process::{Command, Stdio},
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Event {
    pub event: String,
    pub text: String,
    pub evidence: String,
    pub captured_at_ms: u64,
    pub ticket: screen::Ticket,
    #[serde(skip)]
    pub signature: Vec<u8>,
}
#[derive(Default)]
struct State {
    running: bool,
    pending: Option<(screen::Snapshot, Arc<dyn Fn() + Send + Sync>)>,
    candidate: Option<Event>,
    confirmed: Option<Event>,
    last_at: u64,
    unavailable: bool,
    cached: Option<(String, Option<Event>)>,
}
static STATES: OnceLock<Mutex<HashMap<PathBuf, State>>> = OnceLock::new();

fn normalized(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
        .trim_end_matches(['.', '!'])
        .to_owned()
}

/// Read Tesseract word confidence and retain line boundaries. A phrase in an
/// instruction such as "avoid the game over screen" is never an event.
pub fn event_from_tsv(tsv: &str) -> Option<(String, String, String)> {
    let mut lines: BTreeMap<(u32, u32, u32), Vec<(String, f32)>> = BTreeMap::new();
    for line in tsv.lines().skip(1) {
        let f: Vec<_> = line.splitn(12, '\t').collect();
        if f.len() != 12 || f[0] != "5" {
            continue;
        }
        let Some(conf) = f[10].parse::<f32>().ok() else {
            continue;
        };
        let id = (
            f[2].parse().unwrap_or(0),
            f[3].parse().unwrap_or(0),
            f[4].parse().unwrap_or(0),
        );
        lines.entry(id).or_default().push((f[11].into(), conf));
    }
    for words in lines.values() {
        if words.is_empty() || words.iter().any(|(_, confidence)| *confidence < 85.0) {
            continue;
        }
        let text = words
            .iter()
            .map(|(text, _)| text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let normalized = normalized(&text);
        let hp = normalized.replace([' ', ':'], "");
        if let Some((current, max)) = hp.strip_prefix("hp").and_then(|s| s.split_once('/'))
            && let (Ok(current), Ok(max)) = (current.parse::<u32>(), max.parse::<u32>())
            && max > 0
            && max <= 99999
            && current > 0
            && current <= max / 5
        {
            return Some((
                "danger".into(),
                "Your health is low. Watch your health before the next fight.".into(),
                text,
            ));
        }
        let event = match normalized.as_str() {
            "game over" | "you died" | "you are dead" => "death",
            "you win" | "victory" => "victory",
            _ => continue,
        };
        let cue = if event == "death" {
            "The screen says game over. Let's work through the next attempt."
        } else {
            "The screen shows a victory message."
        };
        return Some((event.into(), cue.into(), text));
    }
    None
}

pub fn inspect(snapshot: &screen::Snapshot) -> Result<Option<Event>> {
    let image = image::load_from_memory(&STANDARD.decode(&snapshot.jpeg)?)?;
    // Text events are generally central overlays. Preserve enough resolution
    // for bitmap fonts; no model or full-frame text generation is required.
    let signature = screen::frame_signature(&image);
    let image = image.thumbnail(640, 480).to_rgb8();
    let mut png = std::io::Cursor::new(Vec::new());
    image.write_to(&mut png, image::ImageFormat::Png)?;
    let mut process = Command::new("tesseract")
        .args(["stdin", "stdout", "--psm", "11", "tsv"])
        .env("OMP_THREAD_LIMIT", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("Local text detector needs Tesseract")?;
    let input = png.into_inner();
    let mut stdin = process.stdin.take().context("OCR input")?;
    std::thread::spawn(move || {
        let _ = stdin.write_all(&input);
    });
    let mut stdout = process.stdout.take().context("OCR output")?;
    let output = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout.read_to_end(&mut bytes).map(|_| bytes)
    });
    let deadline = Instant::now() + Duration::from_millis(700);
    while process.try_wait()?.is_none() {
        if Instant::now() >= deadline {
            let _ = process.kill();
            let _ = process.wait();
            anyhow::bail!("Local text detector exceeded its budget");
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    ensure!(process.wait()?.success(), "Local text detector failed");
    let bytes = output
        .join()
        .map_err(|_| anyhow::anyhow!("OCR reader stopped"))??;
    Ok(
        event_from_tsv(&String::from_utf8_lossy(&bytes)).map(|(event, text, evidence)| Event {
            event,
            text,
            evidence,
            captured_at_ms: snapshot.captured_at_ms,
            ticket: snapshot.ticket.clone(),
            signature,
        }),
    )
}

pub fn schedule(store: &Store, snapshot: screen::Snapshot, notify: Arc<dyn Fn() + Send + Sync>) {
    let root = store.root.clone();
    let states = STATES.get_or_init(Default::default);
    let mut all = states.lock().unwrap();
    if all.len() >= 16 && !all.contains_key(&root) {
        all.retain(|_, s| s.running);
        if all.len() >= 16 {
            return;
        }
    }
    let state = all.entry(root.clone()).or_default();
    if state.unavailable || snapshot.captured_at_ms.saturating_sub(state.last_at) < 150 {
        return;
    }
    state.pending = Some((snapshot, notify));
    if state.running {
        return;
    }
    state.running = true;
    drop(all);
    std::thread::spawn(move || {
        loop {
            let job = {
                let mut all = states.lock().unwrap();
                let state = all.get_mut(&root).unwrap();
                match state.pending.take() {
                    Some(job) => {
                        state.last_at = job.0.captured_at_ms;
                        Some(job)
                    }
                    None => {
                        state.running = false;
                        None
                    }
                }
            };
            let Some((snapshot, notify)) = job else {
                return;
            };
            // Identical pixels need no second OCR inference. They still count
            // as a distinct captured frame for persistence/confirmation.
            let digest = format!("{:x}", Sha256::digest(snapshot.jpeg.as_bytes()));
            let cached = states
                .lock()
                .unwrap()
                .get(&root)
                .and_then(|state| state.cached.as_ref())
                .filter(|(key, _)| key == &digest)
                .map(|(_, event)| event.clone());
            let result = if let Some(event) = cached {
                Ok(event.map(|mut event| {
                    event.ticket = snapshot.ticket.clone();
                    event.captured_at_ms = snapshot.captured_at_ms;
                    event
                }))
            } else {
                inspect(&snapshot)
            };
            let current = Store::open(&root)
                .ok()
                .is_some_and(|s| screen::still_current(&s, &snapshot).unwrap_or(false));
            let mut all = states.lock().unwrap();
            let state = all.get_mut(&root).unwrap();
            state.cached = result.as_ref().ok().map(|event| (digest, event.clone()));
            match result {
                Ok(Some(event))
                    if current && screen::now_ms().saturating_sub(event.captured_at_ms) < 1200 =>
                {
                    if let Some(old) = &state.candidate
                        && old.ticket == event.ticket
                        && old.event == event.event
                        && event.captured_at_ms > old.captured_at_ms
                        && event.captured_at_ms.saturating_sub(old.captured_at_ms) < 1500
                    {
                        if state.confirmed.as_ref().is_none_or(|old| {
                            old.event != event.event || old.ticket != event.ticket
                        }) && std::env::args().any(|a| a == "--voice-trace")
                        {
                            eprintln!(
                                "GAMEBUDDY_FAST_EVENT event={} captured_at_ms={} at_ms={}",
                                event.event,
                                event.captured_at_ms,
                                screen::now_ms()
                            );
                        }
                        state.confirmed = Some(event.clone());
                    }
                    state.candidate = Some(event);
                }
                Err(error)
                    if error
                        .downcast_ref::<std::io::Error>()
                        .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
                {
                    state.unavailable = true;
                    state.candidate = None;
                    state.confirmed = None;
                }
                _ => {
                    state.candidate = None;
                    state.confirmed = None;
                }
            }
            drop(all);
            notify();
        }
    });
}

pub fn context(store: &Store) -> Value {
    let event = STATES
        .get()
        .and_then(|s| s.lock().ok()?.get(&store.root)?.confirmed.clone());
    let Some(event) = event else {
        return Value::Null;
    };
    let current = screen::status(store).unwrap_or_default();
    let fresh = current["status"] == "live"
        && screen::ticket(store, &event.ticket.session).ok().as_ref() == Some(&event.ticket)
        && screen::now_ms().saturating_sub(event.captured_at_ms) < 1500
        && screen::matches_latest_view(store, &event.ticket, &event.signature, 1500)
            .unwrap_or(false);
    json!({"event":event.event,"text":event.text,"evidence":event.evidence,"captured_at_ms":event.captured_at_ms,"fresh":fresh,"source":"local_screen_text"})
}

#[cfg(test)]
mod tests {
    use super::*;
    fn tsv(text: &str, confidence: f32) -> String {
        format!(
            "header\n{}",
            text.split_whitespace()
                .enumerate()
                .map(|(i, w)| format!("5\t1\t1\t1\t1\t{}\t0\t0\t20\t20\t{confidence}\t{w}", i + 1))
                .collect::<Vec<_>>()
                .join("\n")
        )
    }
    #[test]
    fn visible_event_requires_complete_literal_text_and_word_confidence() {
        assert_eq!(event_from_tsv(&tsv("GAME OVER", 96.0)).unwrap().0, "death");
        assert!(event_from_tsv(&tsv("Avoid the game over screen", 96.0)).is_none());
        assert!(event_from_tsv(&tsv("GAME OVER", 50.0)).is_none());
        assert!(event_from_tsv(&tsv("Obtain victory by climbing", 96.0)).is_none());
        assert_eq!(
            event_from_tsv(&tsv("HP: 3 / 20", 96.0)).unwrap().0,
            "danger"
        );
        assert!(event_from_tsv(&tsv("HP 30/20", 96.0)).is_none());
        assert!(event_from_tsv(&tsv("Recover HP 3/20", 96.0)).is_none());
    }

    #[test]
    fn transient_event_expires_on_changed_pixels_pause_and_time() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let store = Store::open(dir.path())?;
        let ticket = screen::start(&store, "Fixture", true)?;
        let path = dir.path().join("screen.png");
        let image = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            96,
            64,
            image::Rgb([180, 30, 50]),
        ));
        image.save(&path)?;
        screen::publish(&store, &ticket, &path, "fixture")?;
        let event = Event {
            event: "death".into(),
            text: "Game over".into(),
            evidence: "GAME OVER".into(),
            captured_at_ms: screen::now_ms(),
            ticket,
            signature: screen::frame_signature(&image),
        };
        let states = STATES.get_or_init(Default::default);
        states.lock().unwrap().insert(
            store.root.clone(),
            State {
                confirmed: Some(event.clone()),
                ..Default::default()
            },
        );
        assert_eq!(context(&store)["fresh"], true);
        states
            .lock()
            .unwrap()
            .get_mut(&store.root)
            .unwrap()
            .confirmed
            .as_mut()
            .unwrap()
            .captured_at_ms -= 2000;
        assert_eq!(context(&store)["fresh"], false);
        states
            .lock()
            .unwrap()
            .get_mut(&store.root)
            .unwrap()
            .confirmed = Some(event.clone());
        image::RgbImage::from_pixel(96, 64, image::Rgb([10, 200, 220])).save(&path)?;
        screen::publish(&store, &event.ticket, &path, "fixture")?;
        assert_eq!(context(&store)["fresh"], false);
        screen::start(&store, "Fixture", false)?;
        assert_eq!(context(&store)["fresh"], false);
        states.lock().unwrap().remove(&store.root);
        Ok(())
    }
}
