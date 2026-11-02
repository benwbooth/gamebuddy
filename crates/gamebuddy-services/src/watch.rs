//! Watching the game the way someone beside the player does. A small local
//! model copies each new screen's text into a short log, which it does well
//! and quickly; a strong model that sees the picture says what that means
//! and speaks up when a friend who knows the game would. Spoken answers read
//! the same log, so they know what just happened, not only what is on screen.
use crate::{HttpProvider, ModelRequest, Settings, Store, providers::on_this_computer, screen};
use anyhow::Result;
use gamebuddy_core::Guide;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{Mutex, OnceLock},
};

pub const LOOK: &str = "You are an expert at this game, sitting beside the player and watching them play. You see the screen as it is now, and a log of what happened. Reply with JSON only: {\"happening\": \"one plain sentence on what is going on in the game\", \"place\": \"the room or area the player is in, in a few words; empty on title screens, menus, or if unclear\", \"say\": \"\", \"urgency\": \"none\"}. Leave say empty unless a friend who knows this game well would speak up now: a danger or time limit the player may not have noticed, a mistake about to cost them, a death (what killed them and how to avoid it), a sign they are stuck or repeating themselves, or something useful they are missing. Then say is one or two short spoken sentences telling them exactly what to do on this screen, naming the on-screen option rather than buttons, and urgency is \"danger\" if waiting would cost them, otherwise \"tip\". Exact commands come only from the walkthrough steps you are given: what a menu word does differs from game to game, so never work one out from its name or from memory. If the steps do not make the command clear, name the goal and say you are not sure of the command. Most of the time say nothing. Never describe what they can already see, never repeat what you already told them, and stay quiet on title screens, menus before the game starts, and intro text that is still playing.";

const LOG_LIMIT: usize = 30;
/// How far back the log counts as what just happened.
const LOG_MS: u64 = 300_000;
const READ_MS: u64 = 1_500;
/// A screen that looks unchanged is still read this often: scrolling text
/// is too small a change to count as a new scene.
const REREAD_MS: u64 = 5_000;
const LOOK_MS: u64 = 15_000;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Entry {
    pub at_ms: u64,
    /// Words on screen, copied by the local reader.
    pub text: String,
    /// The reader's one-line guess at the scene, kept for setups with no
    /// stronger model to look.
    pub scene: String,
    /// The strong model's understanding of what is going on.
    pub happening: String,
    /// What GameBuddy told the player unprompted.
    pub said: String,
}

#[derive(Deserialize)]
#[serde(default)]
struct Look {
    happening: String,
    place: String,
    say: String,
    urgency: String,
}

impl Default for Look {
    fn default() -> Self {
        Self {
            happening: String::new(),
            place: String::new(),
            say: String::new(),
            urgency: "none".into(),
        }
    }
}

#[derive(Default)]
struct Job {
    running: bool,
    read_at: u64,
    looked_at: u64,
    scene: Option<Vec<u8>>,
    captions_tried: bool,
}

static JOBS: OnceLock<Mutex<HashMap<PathBuf, Job>>> = OnceLock::new();

fn key(guide: &Guide) -> String {
    format!("watch-log:{}", guide.id)
}

fn place_key(guide: &Guide) -> String {
    format!("watch-place:{}", guide.id)
}

/// Where the watcher last saw the player, for games with no map to place
/// them on.
pub fn place(store: &Store, guide: &Guide) -> Option<String> {
    let seen: Value = store.get(&place_key(guide)).ok()??;
    (screen::now_ms().saturating_sub(seen["at_ms"].as_u64()?) < LOG_MS)
        .then(|| seen["place"].as_str().map(str::to_owned))?
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Room {
    pub id: String,
    pub name: String,
}

/// The rooms this player has walked through and how they connect: a map
/// drawn as they go, so it holds only what they saw and spoils nothing.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Rooms {
    pub rooms: Vec<Room>,
    pub links: Vec<[String; 2]>,
    pub current: String,
    /// The play session of `current`. Loading a save jumps between rooms,
    /// so a new session's first room connects to nothing.
    pub session: String,
}

fn rooms_key(guide: &Guide) -> String {
    format!("watch-rooms:{}", guide.id)
}

pub fn rooms(store: &Store, guide: &Guide) -> Rooms {
    store
        .get(&rooms_key(guide))
        .ok()
        .flatten()
        .unwrap_or_default()
}

fn room_image_path(store: &Store, guide: &Guide, room: &str) -> PathBuf {
    store
        .root
        .join("watch-rooms")
        .join(&guide.id)
        .join(format!("{room}.jpg"))
}

fn visit(store: &Store, guide: &Guide, name: &str, snapshot: &screen::Snapshot) -> Result<()> {
    use base64::Engine;
    let mut map = rooms(store, guide);
    let words = normalized(name);
    let room = match map.rooms.iter().find(|r| normalized(&r.name) == words) {
        Some(room) => room.id.clone(),
        None => {
            let id = format!("r{}", map.rooms.len());
            let path = room_image_path(store, guide, &id);
            std::fs::create_dir_all(path.parent().unwrap_or(&store.root))?;
            std::fs::write(
                path,
                base64::engine::general_purpose::STANDARD.decode(&snapshot.jpeg)?,
            )?;
            map.rooms.push(Room {
                id: id.clone(),
                name: name.to_owned(),
            });
            id
        }
    };
    let (from, to) = (map.current.clone(), room.clone());
    if !from.is_empty()
        && map.session == snapshot.ticket.session
        && from != to
        && !map
            .links
            .iter()
            .any(|l| (l[0] == from && l[1] == to) || (l[0] == to && l[1] == from))
    {
        map.links.push([from, to]);
    }
    map.current = room;
    map.session = snapshot.ticket.session.clone();
    store.put(&rooms_key(guide), &map)
}

/// A room's picture for the widget. Only rooms in the map can be asked for.
pub fn room_image(store: &Store, guide: &Guide, room: &str) -> Result<String> {
    use base64::Engine;
    anyhow::ensure!(
        rooms(store, guide).rooms.iter().any(|r| r.id == room),
        "Unknown room"
    );
    Ok(format!(
        "data:image/jpeg;base64,{}",
        base64::engine::general_purpose::STANDARD
            .encode(std::fs::read(room_image_path(store, guide, room))?)
    ))
}

fn normalized(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Add what the reader saw. True when it is news: new words, or a new scene
/// with no words. Text that is still typing out replaces its shorter self.
fn record(log: &mut Vec<Entry>, entry: Entry, scene_changed: bool) -> bool {
    let words = normalized(&entry.text);
    let last = log.iter_mut().rev().find(|e| !e.text.is_empty());
    if let Some(last) = last.filter(|_| !words.is_empty()) {
        let seen = normalized(&last.text);
        if seen == words || seen.contains(&words) {
            return false;
        }
        if words.starts_with(&seen) {
            last.text = entry.text;
            last.at_ms = entry.at_ms;
            return true;
        }
    } else if words.is_empty() && !scene_changed {
        return false;
    }
    log.push(entry);
    let excess = log.len().saturating_sub(LOG_LIMIT);
    log.drain(..excess);
    true
}

/// What has just happened, oldest first, as lines for a model to read.
pub fn lines(store: &Store, guide: &Guide) -> Vec<String> {
    let log: Vec<Entry> = store.get(&key(guide)).ok().flatten().unwrap_or_default();
    let now = screen::now_ms();
    let understood = log.iter().any(|e| !e.happening.is_empty());
    log.iter()
        .filter(|e| now.saturating_sub(e.at_ms) < LOG_MS)
        .flat_map(|e| {
            let ago = now.saturating_sub(e.at_ms) / 1000;
            let scene = (!understood).then_some(&e.scene);
            [
                (&e.text, "the screen said"),
                (&e.happening, "going on"),
                (&e.said, "you told the player"),
            ]
            .into_iter()
            .chain(scene.map(|s| (s, "on screen")))
            .filter(|(line, _)| !line.trim().is_empty())
            .map(|(line, what)| format!("{ago} s ago, {what}: {}", line.trim()))
            .collect::<Vec<_>>()
        })
        .collect()
}

/// The walkthrough steps about this moment, wherever the plan thinks the
/// player is: matched on what the watcher last saw and what was just said.
pub fn steps_here(store: &Store, guide: &Guide, question: &str) -> Vec<String> {
    let log: Vec<Entry> = store.get(&key(guide)).ok().flatten().unwrap_or_default();
    let recent = log
        .iter()
        .rev()
        .take(4)
        .flat_map(|e| [e.happening.as_str(), e.text.as_str()])
        .collect::<Vec<_>>()
        .join(" ");
    let words = |text: &str| -> HashSet<String> {
        text.to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| w.len() > 3)
            .map(str::to_owned)
            .collect()
    };
    let asked = words(&format!("{recent} {question}"));
    let mut scored: Vec<_> = guide
        .steps
        .iter()
        .filter(|s| !s.instructions.starts_with("Reference reviewed"))
        .filter(|s| !s.instructions.starts_with("Chapter supplied"))
        .map(|s| {
            let title = words(&s.title).intersection(&asked).count();
            let body = words(&s.instructions).intersection(&asked).count();
            (title * 3 + body, s)
        })
        .filter(|(score, _)| *score >= 3)
        .collect();
    scored.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
    let narration = scored
        .first()
        .and_then(|(_, step)| narrated(store, guide, step));
    scored
        .into_iter()
        .take(4)
        .map(|(_, s)| {
            let body: String = s.instructions.chars().take(420).collect();
            format!("{}: {body}", s.title)
        })
        .chain(narration)
        .collect()
}

/// What the walkthrough's narrator says over this step, word for word.
fn narrated(store: &Store, guide: &Guide, step: &gamebuddy_core::Step) -> Option<String> {
    let path = crate::media::captions_of(store, guide)?;
    let anchor = step.anchors.first()?;
    let start = anchor.start_ms?.saturating_sub(15_000);
    let end = anchor.end_ms.unwrap_or(start + 60_000) + 15_000;
    let cues = crate::captions::excerpt(&path, start, end).ok()?;
    let words: Vec<_> = cues.iter().map(|c| c.text.as_str()).collect();
    let words: String = words.join(" ").chars().take(1500).collect();
    (!words.is_empty()).then(|| format!("The narrator, word for word: {words}"))
}

/// Called with each new frame. Reading and looking run behind it.
pub fn schedule(store: &Store, guide: &Guide, snapshot: &screen::Snapshot) {
    let mut jobs = JOBS.get_or_init(Default::default).lock().unwrap();
    let job = jobs.entry(store.root.clone()).or_default();
    let since = snapshot.captured_at_ms.saturating_sub(job.read_at);
    if job.running || since < READ_MS {
        return;
    }
    let Ok((signature, changed)) = screen::scene_change(store, job.scene.as_deref()) else {
        return;
    };
    if !changed && since < REREAD_MS {
        return;
    }
    if !job.captions_tried {
        job.captions_tried = true;
        let (root, guide) = (store.root.clone(), guide.clone());
        std::thread::spawn(move || {
            let recovered =
                Store::open(&root).and_then(|store| crate::media::recover_captions(&store, &guide));
            if let Err(error) = recovered {
                eprintln!("Walkthrough captions: {error:#}");
            }
        });
    }
    job.running = true;
    job.read_at = snapshot.captured_at_ms;
    job.scene = Some(signature);
    let may_look = snapshot.captured_at_ms.saturating_sub(job.looked_at) >= LOOK_MS;
    let root = store.root.clone();
    let guide = guide.clone();
    let snapshot = snapshot.clone();
    std::thread::spawn(move || {
        let looked = Store::open(&root)
            .and_then(|store| watch(&store, &guide, &snapshot, changed, may_look));
        if let Err(error) = &looked {
            eprintln!("Watching: {error:#}");
        }
        if let Some(job) = JOBS.get().unwrap().lock().unwrap().get_mut(&root) {
            job.running = false;
            if matches!(looked, Ok(true)) {
                job.looked_at = snapshot.captured_at_ms;
            }
        }
    });
}

/// One pass over one frame. True when the strong model looked.
pub fn watch(
    store: &Store,
    guide: &Guide,
    snapshot: &screen::Snapshot,
    changed: bool,
    may_look: bool,
) -> Result<bool> {
    let settings: Settings = store.get("settings")?.unwrap_or_default();
    let mut log: Vec<Entry> = store.get(&key(guide))?.unwrap_or_default();
    let mut entry = Entry {
        at_ms: snapshot.captured_at_ms,
        ..Default::default()
    };
    if on_this_computer(&settings.vision) {
        let reader = HttpProvider::new(settings.vision.clone())?;
        // A small model sometimes runs on past its reply's end. One unread
        // frame costs nothing, and the strong model can still look.
        match crate::brain::read_screen(&reader, std::slice::from_ref(&snapshot.jpeg)) {
            Ok(reading) => {
                entry.text = reading.text;
                entry.scene = reading.scene;
            }
            Err(error) => eprintln!("Reading the screen: {error:#}"),
        }
    }
    let news = record(&mut log, entry, changed);
    store.put(&key(guide), &log)?;
    // The last seen screen widget shows what the watcher last looked at.
    screen::seen::record(store, snapshot, "observer")?;
    if !news || !may_look || on_this_computer(&settings.reasoning) {
        return Ok(false);
    }
    look(store, guide, &settings, snapshot)?;
    Ok(true)
}

fn look(
    store: &Store,
    guide: &Guide,
    settings: &Settings,
    snapshot: &screen::Snapshot,
) -> Result<()> {
    let provider = HttpProvider::new(settings.reasoning.clone())?;
    let plan = crate::voice::talk::situation_lines(store, guide).unwrap_or_default();
    let reply = provider.complete_streaming(
        ModelRequest {
            system: LOOK.into(),
            prompt: format!(
                "Game: {}\n\nRooms seen so far (use the same name again for the same room): {}\n\nWhat the guide knows:\n{}\n\nWalkthrough steps for this moment:\n{}\n\nWhat just happened, oldest first:\n{}\n\nThe image is the screen now.",
                guide.game,
                rooms(store, guide).rooms.iter().map(|r| r.name.as_str()).collect::<Vec<_>>().join(", "),
                plan.join("\n"),
                steps_here(store, guide, "").join("\n"),
                lines(store, guide).join("\n"),
            ),
            images: vec![snapshot.jpeg.clone()],
            json: false,
        },
        &mut |_| {},
    )?;
    let start = reply.find('{').unwrap_or(0);
    let end = reply.rfind('}').map_or(reply.len(), |at| at + 1);
    let look: Look = serde_json::from_str(reply.get(start..end).unwrap_or_default())?;
    let say = if look.urgency == "none" {
        ""
    } else {
        look.say.trim()
    };
    let mut log: Vec<Entry> = store.get(&key(guide))?.unwrap_or_default();
    log.push(Entry {
        at_ms: screen::now_ms(),
        happening: look.happening.clone(),
        said: say.to_owned(),
        ..Default::default()
    });
    let excess = log.len().saturating_sub(LOG_LIMIT);
    log.drain(..excess);
    store.put(&key(guide), &log)?;
    if !look.place.trim().is_empty() {
        visit(store, guide, look.place.trim(), snapshot)?;
        store.put(
            &place_key(guide),
            &json!({"place": look.place.trim(), "at_ms": screen::now_ms()}),
        )?;
    }
    crate::flight::event(
        "watch",
        json!({"happening": look.happening, "place": look.place, "say": say, "urgency": look.urgency}),
    );
    if !say.is_empty() {
        offer(store, guide, snapshot, say, &look.urgency)?;
    }
    Ok(())
}

/// Hand the remark to the narrator, which decides when it can be said. It
/// is dropped if the game moved on while the model thought.
fn offer(
    store: &Store,
    guide: &Guide,
    seen: &screen::Snapshot,
    text: &str,
    urgency: &str,
) -> Result<()> {
    use base64::Engine;
    let signature = |snapshot: &screen::Snapshot| -> Result<Vec<u8>> {
        Ok(screen::frame_signature(&image::load_from_memory(
            &base64::engine::general_purpose::STANDARD.decode(&snapshot.jpeg)?,
        )?))
    };
    let latest = screen::snapshot(store)?;
    let current = signature(&latest)?;
    if !screen::same_scene(&signature(seen)?, &current) {
        return Ok(());
    }
    let progress = store.progress(guide)?;
    let cue: Value = json!({
        "kind": if urgency == "danger" { "danger" } else { "strategy" },
        "step_id": "",
        "text": text,
        "evidence": "Seen on screen by the watcher.",
        "confidence": 1.0,
        "screen_direction": null,
        "id": uuid::Uuid::new_v4().to_string(),
        "revision": guide.revision,
        "epoch": progress.epoch,
        "ticket": latest.ticket,
        "captured_at_ms": latest.captured_at_ms,
        "frame_signature": current,
    });
    store.put_for_progress(
        guide,
        progress.epoch,
        &format!("coaching:{}", guide.id),
        &cue,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seen(text: &str) -> Entry {
        Entry {
            text: text.into(),
            ..Default::default()
        }
    }

    #[test]
    fn typing_text_replaces_itself_and_repeats_are_not_news() {
        let mut log = vec![];
        assert!(record(&mut log, seen("LOOK OUT!! THE LAST"), true));
        assert!(record(&mut log, seen("LOOK OUT!! THE LAST THING"), false));
        assert_eq!(log.len(), 1);
        assert!(!record(&mut log, seen("look out!! the last thing"), true));
        assert!(!record(&mut log, seen(""), false));
        assert!(record(&mut log, seen("THE SMELL OF GASOLINE"), true));
        assert_eq!(log.len(), 2);
    }
}
