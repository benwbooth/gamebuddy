//! The companion: one Claude session that watches a whole game, the way a
//! friend on the couch does. It is shown each new screen as it happens,
//! keeps its own notes on the player's progress, and can read and search the
//! walkthrough, the narrator's words and recent frames in its folder. Spoken
//! answers read those notes, so they come from something that has been
//! following along instead of a fresh guess each time.
use crate::{Settings, Store, screen, watch};
use anyhow::{Context, Result};
use base64::Engine;
use gamebuddy_core::Guide;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    sync::{Mutex, OnceLock, mpsc},
};

const SYSTEM: &str = "You are GameBuddy's companion: an expert at this game, sitting beside the player and watching them play the whole time, like a friend who has beaten it many times. Your folder holds walkthrough.md (the walkthrough, every task numbered in order with its detailed actions and its time in the walkthrough video), narration.txt (the walkthrough video's narrator, word for word with times), video/ (full frames from the walkthrough video from the task before the player's to two tasks after it, named by task number and video time: when the text does not say which door, exit or object, Read the frames just before the new room or result appears and find where the video's cursor or character was, such as the exit picked on a map or the door walked through, then match it to the player's screen; the video may label rooms by name), frames/ (recent screens) and notes.md (your own notes). Read notes.md first when you start. Keep notes.md current with Edit or Write: what the player carries, rooms seen and how they connect, what they have done and solved, deaths and their causes, anything the player told you (their corrections always win), the walkthrough task they are on and your plan. Keep it under 60 short lines: rewrite entries as things change rather than adding a line per screen, and update it whenever something changes. Work out ahead what the current task needs on screen (which door, exit, object or menu choice) from the walkthrough, the narration and the video frames, and write it in your notes, so that a question about it can be answered at once. Use Grep and Read on walkthrough.md and narration.txt to check facts before telling the player anything; exact commands and solutions come only from those files or the player, never from a menu word's name or from memory, because what a command does differs from game to game. Each message shows the screen now and what happened since your last look. Reply to each with JSON only, no other text: {\"happening\": \"one plain sentence on what is going on\", \"place\": \"the room or area, a few words, the same name as in your notes for the same room; empty on title screens and menus\", \"task\": 0, \"say\": \"\", \"urgency\": \"none\"}. task is the number in walkthrough.md of the first task the player has not done, judged by their progress (what they carry, what they did), not the room they stand in; 0 if unclear. Leave say empty unless a friend who knows the game would speak up now: a danger or time limit, a mistake about to cost them, a death (what caused it and how to avoid it), signs they are stuck or going in circles, or something useful they are missing. Then say is one or two short spoken sentences telling them exactly what to do on this screen, naming on-screen options rather than buttons, and urgency is \"danger\" if waiting would cost them, otherwise \"tip\". Most of the time say nothing; never repeat yourself, and stay quiet on title screens and intro text that is still playing.";

/// After this many looks the session starts over from its notes, so the
/// conversation (a picture each look) never grows past what fits.
const TURNS_PER_SESSION: usize = 30;
const FRAMES_KEPT: usize = 20;

struct Job {
    guide: Guide,
    snapshot: screen::Snapshot,
}

struct Running {
    sender: mpsc::SyncSender<Job>,
    busy: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

static RUNNING: OnceLock<Mutex<HashMap<(PathBuf, String), Running>>> = OnceLock::new();

/// Whether the companion is still thinking about a screen.
pub fn busy(store: &Store, guide: &Guide) -> bool {
    RUNNING
        .get()
        .and_then(|running| {
            let running = running.lock().ok()?;
            let entry = running.get(&(store.root.clone(), guide.id.clone()))?;
            Some(entry.busy.load(std::sync::atomic::Ordering::Acquire))
        })
        .unwrap_or(false)
}

pub fn folder(store: &Store, guide: &Guide) -> PathBuf {
    store.root.join("companion").join(&guide.id)
}

/// The companion's notes, for spoken answers; empty before it has any.
pub fn notes(store: &Store, guide: &Guide) -> String {
    std::fs::read_to_string(folder(store, guide).join("notes.md")).unwrap_or_default()
}

/// Show the companion a new screen. It looks when free; a screen that comes
/// while it is still thinking is skipped, as the next one will show more.
/// True when it was handed over.
pub fn observe(
    store: &Store,
    guide: &Guide,
    settings: &Settings,
    snapshot: &screen::Snapshot,
) -> bool {
    use std::sync::atomic::Ordering;
    let mut running = RUNNING.get_or_init(Default::default).lock().unwrap();
    let key = (store.root.clone(), guide.id.clone());
    let entry = running.entry(key).or_insert_with(|| {
        let (sender, receiver) = mpsc::sync_channel::<Job>(1);
        let busy = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (root, config, flag) = (store.root.clone(), settings.reasoning.clone(), busy.clone());
        std::thread::spawn(move || run(root, config, receiver, flag));
        Running { sender, busy }
    });
    if entry.busy.load(Ordering::Acquire) {
        return false;
    }
    entry.busy.store(true, Ordering::Release);
    let sent = entry
        .sender
        .try_send(Job {
            guide: guide.clone(),
            snapshot: snapshot.clone(),
        })
        .is_ok();
    if !sent {
        entry.busy.store(false, Ordering::Release);
    }
    sent
}

struct Session {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    turns: usize,
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn run(
    root: PathBuf,
    config: crate::ProviderConfig,
    jobs: mpsc::Receiver<Job>,
    busy: std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    let mut session: Option<Session> = None;
    let mut since_ms = 0;
    let mut heard = None;
    let mut video = None;
    while let Ok(job) = jobs.recv() {
        let result = Store::open(&root).and_then(|store| {
            if session
                .as_ref()
                .is_none_or(|s| s.turns >= TURNS_PER_SESSION)
            {
                session = None;
                prepare(&store, &job.guide)?;
                session = Some(start(&folder(&store, &job.guide), &config)?);
            }
            let session = session.as_mut().unwrap();
            let fresh_video = match video_frames(&store, &job.guide, &mut video) {
                Ok(fresh) => fresh,
                Err(error) => {
                    eprintln!("Companion video frames: {error:#}");
                    false
                }
            };
            let mut message = message(&store, &job, since_ms, &mut heard)?;
            if fresh_video {
                let text = &mut message["message"]["content"][0]["text"];
                *text = json!(format!(
                    "video/ now holds walkthrough video frames from the task before the player's to two after it; use them for anything your notes call unknown.\n\n{}",
                    text.as_str().unwrap_or_default()
                ));
            }
            let reply = turn(session, message)?;
            since_ms = job.snapshot.captured_at_ms;
            watch::apply(&store, &job.guide, &job.snapshot, &reply)
        });
        if let Err(error) = result {
            eprintln!("Companion: {error:#}");
            crate::flight::event("companion_error", json!({"error": format!("{error:#}")}));
            session = None;
        }
        busy.store(false, std::sync::atomic::Ordering::Release);
    }
}

/// Write the files the companion reads: the walkthrough in order, the
/// narrator's words and an empty notes file the first time.
fn prepare(store: &Store, guide: &Guide) -> Result<()> {
    let folder = folder(store, guide);
    std::fs::create_dir_all(folder.join("frames"))?;
    let numbers: HashMap<&str, usize> = watch::tasks(guide)
        .iter()
        .enumerate()
        .map(|(i, step)| (step.id.as_str(), i + 1))
        .collect();
    let mut text = format!(
        "# {} walkthrough\n\nTasks are numbered in the order the walkthrough does them; actions inside a task are indented under it.\n\n",
        guide.game
    );
    for task in watch::tasks(guide) {
        let number = numbers[task.id.as_str()];
        let at = task
            .anchors
            .first()
            .and_then(|a| a.start_ms)
            .map(|ms| format!(" (video {})", clock(ms)))
            .unwrap_or_default();
        text.push_str(&format!(
            "## {number}. {}{at}\n{}\n",
            task.title, task.instructions
        ));
        for child in guide
            .steps
            .iter()
            .filter(|s| s.parent_id.as_deref() == Some(task.id.as_str()))
        {
            text.push_str(&format!("  - {}: {}\n", child.title, child.instructions));
        }
        text.push('\n');
    }
    std::fs::write(folder.join("walkthrough.md"), text)?;
    if let Some(path) = crate::media::captions_of(store, guide) {
        let cues: Vec<crate::captions::CaptionCue> = serde_json::from_slice(&std::fs::read(path)?)?;
        let lines: Vec<String> = cues
            .iter()
            .map(|c| {
                let seconds = c.start_ms / 1000;
                format!("[{}:{:02}] {}", seconds / 60, seconds % 60, c.text)
            })
            .collect();
        std::fs::write(folder.join("narration.txt"), lines.join("\n"))?;
    }
    let notes = folder.join("notes.md");
    if !notes.is_file() {
        std::fs::write(
            notes,
            format!("# {} notes\n\nNothing seen yet.\n", guide.game),
        )?;
    }
    Ok(())
}

fn start(folder: &Path, config: &crate::ProviderConfig) -> Result<Session> {
    let mut command = Command::new("claude");
    command
        .args([
            "--print",
            "--verbose",
            "--no-session-persistence",
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--tools",
            "Read,Grep,Glob,Edit,Write",
            "--allowedTools",
            "Read,Grep,Glob,Edit,Write",
            "--permission-mode",
            "acceptEdits",
            "--strict-mcp-config",
            "--mcp-config",
            "{\"mcpServers\":{}}",
            "--system-prompt",
            SYSTEM,
        ])
        .env_remove("ANTHROPIC_API_KEY")
        .env_remove("ANTHROPIC_AUTH_TOKEN")
        .current_dir(folder)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if !config.model.is_empty() {
        command.args(["--model", &config.model]);
    }
    if !matches!(config.reasoning_effort.as_str(), "" | "auto") {
        command.args(["--effort", &config.reasoning_effort]);
    }
    let mut child = command
        .spawn()
        .context("Install Claude Code and run `claude auth login` with your Claude account")?;
    let input = child.stdin.take().context("Claude stdin unavailable")?;
    let output = BufReader::new(child.stdout.take().context("Claude stdout unavailable")?);
    Ok(Session {
        child,
        input,
        output,
        turns: 0,
    })
}

fn clock(ms: u64) -> String {
    let seconds = ms / 1000;
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

/// Frames of the walkthrough video around the task the player is on and the
/// next two, every two seconds from a little before each starts: the
/// walkthrough's text often leaves out which door or object was clicked,
/// and the video shows it.
fn video_frames(store: &Store, guide: &Guide, shown: &mut Option<usize>) -> Result<bool> {
    let Some(at) = watch::last_task(store, guide) else {
        return Ok(false);
    };
    if *shown == Some(at) {
        return Ok(false);
    }
    let Some(media) = guide.sources.iter().find_map(|s| s.media_path.clone()) else {
        return Ok(false);
    };
    let directory = folder(store, guide).join("video");
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory)?;
    // From the task before: how the player got here is often the question
    // (which door led in), and it is shown at the end of the previous task.
    let first = at.saturating_sub(1);
    for (offset, task) in watch::tasks(guide).iter().enumerate().skip(first).take(4) {
        let Some(start) = task.anchors.first().and_then(|a| a.start_ms) else {
            continue;
        };
        let end = task.anchors[0]
            .end_ms
            .unwrap_or(start + 30_000)
            .min(start + 30_000);
        let mut ms = start.saturating_sub(12_000);
        while ms <= end {
            let name = format!("task{}-{}.jpg", offset + 1, clock(ms).replace(':', "m"));
            let _ = Command::new("ffmpeg")
                .args(["-nostdin", "-loglevel", "error", "-ss"])
                .arg(format!("{:.1}", ms as f64 / 1000.0))
                .arg("-i")
                .arg(&media)
                .args(["-frames:v", "1", "-q:v", "4", "-y"])
                .arg(directory.join(name))
                .stdin(Stdio::null())
                .status();
            ms += 2_000;
        }
    }
    *shown = Some(at);
    Ok(true)
}

/// What happened since the last look, what the player said, and the screen.
fn message(store: &Store, job: &Job, since_ms: u64, heard: &mut Option<i64>) -> Result<Value> {
    let folder = folder(store, &job.guide);
    let name = format!("{}.jpg", job.snapshot.captured_at_ms);
    let bytes = base64::engine::general_purpose::STANDARD.decode(&job.snapshot.jpeg)?;
    std::fs::write(folder.join("frames").join(&name), bytes)?;
    let mut frames: Vec<_> = std::fs::read_dir(folder.join("frames"))?
        .flatten()
        .map(|e| e.path())
        .collect();
    frames.sort();
    for old in frames.iter().rev().skip(FRAMES_KEPT) {
        let _ = std::fs::remove_file(old);
    }
    let log: Vec<watch::Entry> = store.get(&watch::key(&job.guide))?.unwrap_or_default();
    let new: Vec<String> = log
        .iter()
        .filter(|e| e.at_ms > since_ms && !e.text.trim().is_empty())
        .map(|e| format!("The screen said: {}", e.text.trim()))
        .collect();
    // Only what was said since the last look: old lines shown again read as
    // new mistakes to correct.
    let messages = crate::conversation::recent(store, &job.guide.game).unwrap_or_default();
    let fresh: Vec<_> = match *heard {
        Some(last) => messages.iter().filter(|m| m.id > last).collect(),
        None => messages.iter().rev().take(4).rev().collect(),
    };
    if let Some(last) = messages.iter().map(|m| m.id).max() {
        *heard = Some(last);
    }
    let said: Vec<String> = fresh
        .iter()
        .map(|m| {
            format!(
                "{}: {}",
                if m.role == "user" {
                    "Player"
                } else {
                    "GameBuddy"
                },
                m.text
            )
        })
        .collect();
    let text = format!(
        "Game: {}. New screen, saved as frames/{name}.\n\nSince your last look:\n{}\n\nSaid since your last look:\n{}\n\nUpdate notes.md if anything changed, then reply with the JSON.",
        job.guide.game,
        if new.is_empty() {
            "(no new text)".into()
        } else {
            new.join("\n")
        },
        if said.is_empty() {
            "(nothing)".into()
        } else {
            said.join("\n")
        },
    );
    Ok(json!({"type":"user","message":{"role":"user","content":[
        {"type":"text","text":text},
        {"type":"image","source":{"type":"base64","media_type":"image/jpeg","data":job.snapshot.jpeg}}
    ]}}))
}

fn turn(session: &mut Session, message: Value) -> Result<String> {
    serde_json::to_writer(&mut session.input, &message)?;
    session.input.write_all(b"\n")?;
    session.input.flush()?;
    session.turns += 1;
    let mut line = String::new();
    loop {
        line.clear();
        anyhow::ensure!(
            session.output.read_line(&mut line)? > 0,
            "The companion's Claude session ended"
        );
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if value["type"] == "result" {
            let text = value["result"].as_str().unwrap_or_default().to_owned();
            if value["is_error"] == true && crate::providers::is_quota_error(&text) {
                return Err(crate::providers::out_of_tokens("Claude", &text));
            }
            anyhow::ensure!(value["is_error"] != true, "Claude: {text}");
            return Ok(text);
        }
    }
}
