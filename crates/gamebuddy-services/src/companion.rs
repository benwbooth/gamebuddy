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

const SYSTEM: &str = "You are GameBuddy's companion: an expert at this game, sitting beside the player and watching them play the whole time, like a friend who has beaten it many times. You also answer the player's questions; they hear your answers spoken. Your folder holds: walkthrough.md (the walkthrough, every task numbered in order with its detailed actions and its time in the walkthrough video); narration.txt (the walkthrough video's narrator, word for word with times); video/ (frames from the walkthrough video); frames/ (recent frames of the player's own game); exits.md (which exit in which room leads where); notes.md (your notes). Read notes.md and exits.md first when you start. To see any part of the walkthrough video, run ./video-frames START END STEP, for example ./video-frames 17:30 17:45 0.5, which writes frames into video/ and prints their names; then Read them. When the walkthrough does not say which door, exit or object, find the moment in the video (task times, narration times) and look at frames half a second apart just before the new room appears: where the video's cursor or character is in the very last frame before the change is the choice, not where it passed earlier; a pointing-hand cursor clicks with its fingertip, not where the hand's body sits, so count the dots or doors and say which one the fingertip touches (for example the second of three on the right). exits.md is written only from what you see the player do: when a screen shows them in a new room right after they picked an exit, record room, exit, destination. Never write a guess there. In notes.md, start every fact with [seen] (you saw it on the player's screen, the walkthrough or narrator states it, or the video shows it) or [guess]; turn a guess into seen or delete it once you know; keep it under 60 short lines, rewriting rather than adding a line per screen. Record what the player carries, rooms and what they hold, what they have done and solved, deaths and causes, anything the player told you (their word wins), the walkthrough task they are on and your plan. Work out ahead what the current task needs on screen (which door, exit, object or menu choice) and note it. Tell the player only seen facts as facts; for anything else, find out first with your files and tools, or say plainly that you are not sure. Exact commands come only from the walkthrough, the narrator, the video or the player, never from a menu word's name or from memory, because what a command does differs from game to game. Each message shows the screen and what happened since your last look. Reply to each with JSON only, no other text: {\"happening\": \"one plain sentence on what is going on\", \"place\": \"the room or area, a few words, the same name as in your notes for the same room; empty on title screens and menus\", \"task\": 0, \"say\": \"\", \"urgency\": \"none\"}. task is the number in walkthrough.md of the first task the player has not done, judged by their progress (what they carry, what they did), not the room they stand in; 0 if unclear. Unless the player asked something, leave say empty unless a friend who knows the game would speak up now: a danger or time limit, a mistake about to cost them, a death (what caused it and how to avoid it), signs they are stuck or going in circles, or something useful they are missing. Then say is one or two short spoken sentences telling them exactly what to do on this screen, naming on-screen options rather than buttons, and urgency is \"danger\" if waiting would cost them, otherwise \"tip\". Most of the time say nothing; never repeat yourself, and stay quiet on title screens and intro text that is still playing.";

/// After this many looks the session starts over from its notes, so the
/// conversation (a picture each look) never grows past what fits.
const TURNS_PER_SESSION: usize = 30;
const FRAMES_KEPT: usize = 20;

struct Job {
    guide: Guide,
    snapshot: screen::Snapshot,
    /// A question the quick answer could not settle, and where the
    /// companion's spoken answer goes.
    question: Option<(String, mpsc::Sender<String>)>,
}

struct Running {
    sender: mpsc::SyncSender<Job>,
    busy: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

static RUNNING: OnceLock<Mutex<HashMap<(PathBuf, String), Running>>> = OnceLock::new();

/// Whether the companion places the player in the walkthrough (the Claude
/// login), so other guesses at where they are stand down.
pub fn in_charge(store: &Store) -> bool {
    store
        .get::<Settings>("settings")
        .ok()
        .flatten()
        .is_some_and(|s| s.reasoning.kind == crate::ProviderKind::ClaudeCode)
}

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
    let entry = entry(&mut running, store, guide, settings);
    if entry.busy.load(Ordering::Acquire) {
        return false;
    }
    entry.busy.store(true, Ordering::Release);
    let sent = entry
        .sender
        .try_send(Job {
            guide: guide.clone(),
            snapshot: snapshot.clone(),
            question: None,
        })
        .is_ok();
    if !sent {
        entry.busy.store(false, Ordering::Release);
    }
    sent
}

fn entry<'a>(
    running: &'a mut HashMap<(PathBuf, String), Running>,
    store: &Store,
    guide: &Guide,
    settings: &Settings,
) -> &'a Running {
    running
        .entry((store.root.clone(), guide.id.clone()))
        .or_insert_with(|| {
            let (sender, receiver) = mpsc::sync_channel::<Job>(1);
            let busy = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let (root, config, flag) =
                (store.root.clone(), settings.reasoning.clone(), busy.clone());
            std::thread::spawn(move || run(root, config, receiver, flag));
            Running { sender, busy }
        })
}

/// Put a question to the companion, which can search the walkthrough, the
/// narrator and the video, and wait for its spoken answer. It waits for a
/// look already under way to finish first.
pub fn ask(
    store: &Store,
    guide: &Guide,
    settings: &Settings,
    question: &str,
    within: std::time::Duration,
) -> Option<String> {
    use std::sync::atomic::Ordering;
    let started = std::time::Instant::now();
    let snapshot = screen::snapshot(store).ok()?;
    let (answer, reply) = mpsc::channel();
    loop {
        {
            let mut running = RUNNING.get_or_init(Default::default).lock().unwrap();
            let entry = entry(&mut running, store, guide, settings);
            if !entry.busy.swap(true, Ordering::AcqRel) {
                let job = Job {
                    guide: guide.clone(),
                    snapshot: snapshot.clone(),
                    question: Some((question.to_owned(), answer.clone())),
                };
                if entry.sender.try_send(job).is_err() {
                    entry.busy.store(false, Ordering::Release);
                    return None;
                }
                break;
            }
        }
        if started.elapsed() > within {
            return None;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    reply
        .recv_timeout(within.saturating_sub(started.elapsed()))
        .ok()
        .filter(|a| !a.trim().is_empty())
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
            if let Some((question, _)) = &job.question {
                let text = &mut message["message"]["content"][0]["text"];
                *text = json!(format!(
                    "{}\n\nThe player just asked you: \"{question}\". Find the answer in walkthrough.md, narration.txt and video/ as needed, write it in your notes, and put your spoken answer to them in say (urgency tip).",
                    text.as_str().unwrap_or_default()
                ));
            }
            let reply = turn(session, message)?;
            since_ms = job.snapshot.captured_at_ms;
            let Some((_, answer)) = &job.question else {
                return watch::apply(&store, &job.guide, &job.snapshot, &reply);
            };
            // The answer is spoken by the voice that asked, not the narrator.
            let start = reply.find('{').unwrap_or(0);
            let end = reply.rfind('}').map_or(reply.len(), |at| at + 1);
            let mut look: Value = serde_json::from_str(reply.get(start..end).unwrap_or("{}"))?;
            let _ = answer.send(look["say"].as_str().unwrap_or_default().to_owned());
            look["say"] = json!("");
            watch::apply(&store, &job.guide, &job.snapshot, &look.to_string())
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
    // The companion's way into the walkthrough video, any moment it needs.
    if let Some(media) = guide.sources.iter().find_map(|s| s.media_path.clone()) {
        let script = folder.join("video-frames");
        std::fs::write(&script, video_script(&media))?;
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))?;
    }
    let exits = folder.join("exits.md");
    if !exits.is_file() {
        std::fs::write(exits, "# Exits seen\n\nroom | exit picked | led to\n")?;
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
            "Read,Grep,Glob,Edit,Write,Bash",
            "--allowedTools",
            "Read,Grep,Glob,Edit,Write,Bash(./video-frames:*)",
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

/// `./video-frames START END STEP`: frames of the walkthrough video between
/// two times (m:ss or seconds), STEP seconds apart, at most 60, into video/.
fn video_script(media: &str) -> String {
    format!(
        r#"#!/usr/bin/env bash
set -eu
seconds() {{ echo "$1" | awk -F: '{{ if (NF == 2) print $1 * 60 + $2; else print $1 }}'; }}
start=$(seconds "$1"); end=$(seconds "$2"); step="${{3:-0.5}}"
times=$(awk -v s="$start" -v e="$end" -v d="$step" 'BEGIN {{ if (d <= 0 || (e - s) / d > 60) exit 1; for (t = s; t <= e + 0.001; t += d) printf "%.1f\n", t }}') || {{ echo "At most 60 frames: use a shorter range or a longer step" >&2; exit 1; }}
mkdir -p video
for t in $times; do
  name="video/at-$(awk -v t="$t" 'BEGIN {{ printf "%dm%04.1f", int(t / 60), t - int(t / 60) * 60 }}').jpg"
  ffmpeg -nostdin -loglevel error -ss "$t" -i {media:?} -frames:v 1 -q:v 4 -y "$name"
  echo "$name"
done
"#
    )
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
        // Every half second from shortly before the task starts until just after
        // (its start time is only roughly where the step begins), where
        // the click that led here happens (two seconds apart missed it, and
        // a cursor passing over another exit looked like the choice), then
        // every two seconds.
        let mut ms = start.saturating_sub(16_000);
        while ms <= end {
            let name = format!(
                "task{}-{}m{:04.1}.jpg",
                offset + 1,
                ms / 60_000,
                (ms % 60_000) as f64 / 1000.0
            );
            let _ = Command::new("ffmpeg")
                .args(["-nostdin", "-loglevel", "error", "-ss"])
                .arg(format!("{:.1}", ms as f64 / 1000.0))
                .arg("-i")
                .arg(&media)
                .args(["-frames:v", "1", "-q:v", "4", "-y"])
                .arg(directory.join(name))
                .stdin(Stdio::null())
                .status();
            ms += if ms < start + 6_000 { 500 } else { 2_000 };
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
    let mut content = vec![
        json!({"type":"text","text":text}),
        json!({"type":"image","source":{"type":"base64","media_type":"image/jpeg","data":job.snapshot.jpeg}}),
    ];
    // The clean frame can be seconds old on a still screen: the window as it
    // is this moment comes after it.
    if let Some(live) = screen::live_jpeg() {
        content.push(json!({"type":"text","text":"And the screen this moment, as the player sees it (with any display filter):"}));
        content.push(json!({"type":"image","source":{"type":"base64","media_type":"image/jpeg","data":live}}));
    }
    Ok(json!({"type":"user","message":{"role":"user","content":content}}))
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
