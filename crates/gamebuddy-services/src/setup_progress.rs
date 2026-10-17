//! Player-visible record of first-launch walkthrough preparation. Each stage
//! keeps its own status, counts, and latest detail; a bounded event feed keeps
//! everything the pipeline reported, including retries and failures. The live
//! report stays in memory for cheap UI polling and is persisted at transitions.
use crate::{Store, screen::now_ms};
use serde::Serialize;
use std::{
    collections::{HashMap, VecDeque},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, SystemTime},
};

const MAX_EVENTS: usize = 80;
pub const STAGES: [(&str, &str); 8] = [
    ("source", "Choose a walkthrough"),
    ("download", "Download video and captions"),
    ("references", "Sample visual references"),
    ("spatial", "Map the game world"),
    ("sections", "Write walkthrough sections"),
    ("charts", "Find charts and data cards"),
    ("atlas", "Prepare location atlas"),
    ("maps", "Register map images"),
];

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Stage {
    pub id: String,
    pub label: String,
    /// pending, running, done, skipped, attention, or failed.
    pub status: String,
    pub detail: String,
    pub done: Option<u64>,
    pub total: Option<u64>,
    pub started_at_ms: Option<u64>,
    pub finished_at_ms: Option<u64>,
}
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Event {
    pub at_ms: u64,
    pub stage: String,
    /// info, warning, or error.
    pub level: String,
    pub text: String,
}
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Report {
    pub game: String,
    /// running, ready, partial, or failed.
    pub status: String,
    pub started_at_ms: u64,
    pub updated_at_ms: u64,
    pub stages: Vec<Stage>,
    pub events: VecDeque<Event>,
    pub error: Option<String>,
}

static LIVE: OnceLock<Mutex<HashMap<String, Report>>> = OnceLock::new();
fn live_key(root: &Path, game: &str) -> String {
    format!("{}\n{}", root.display(), game.trim().to_lowercase())
}
pub fn store_key(game: &str) -> String {
    format!("game-preparation:{}", game.trim().to_lowercase())
}
/// The latest report for this data directory and game, if one ran this session.
pub fn snapshot(root: &Path, game: &str) -> Option<Report> {
    LIVE.get_or_init(Default::default)
        .lock()
        .unwrap()
        .get(&live_key(root, game))
        .cloned()
}

fn level(text: &str) -> &'static str {
    let lower = text.to_lowercase();
    if [
        "failed",
        "error",
        "could not",
        "unavailable",
        "retrying",
        "needs attention",
        "can be retried",
    ]
    .iter()
    .any(|w| lower.contains(w))
    {
        "warning"
    } else {
        "info"
    }
}

#[derive(Clone)]
pub struct Tracker {
    root: PathBuf,
    key: String,
    report: Arc<Mutex<Report>>,
}
impl Tracker {
    pub fn start(store: &Store, game: &str) -> Self {
        let now = now_ms();
        let report = Report {
            game: game.into(),
            status: "running".into(),
            started_at_ms: now,
            updated_at_ms: now,
            stages: STAGES
                .iter()
                .map(|(id, label)| Stage {
                    id: (*id).into(),
                    label: (*label).into(),
                    status: "pending".into(),
                    detail: String::new(),
                    done: None,
                    total: None,
                    started_at_ms: None,
                    finished_at_ms: None,
                })
                .collect(),
            events: VecDeque::new(),
            error: None,
        };
        let tracker = Self {
            root: store.root.clone(),
            key: live_key(&store.root, game),
            report: Arc::new(Mutex::new(report)),
        };
        tracker.update(true, |_| {});
        tracker
    }
    pub fn report(&self) -> Report {
        self.report.lock().unwrap().clone()
    }
    fn update(&self, persist: bool, change: impl FnOnce(&mut Report)) {
        let report = {
            let mut report = self.report.lock().unwrap();
            change(&mut report);
            report.updated_at_ms = now_ms();
            report.clone()
        };
        if persist && let Ok(store) = Store::open(&self.root) {
            let _ = store.put(&store_key(&report.game), &report);
        }
        LIVE.get_or_init(Default::default)
            .lock()
            .unwrap()
            .insert(self.key.clone(), report);
    }
    fn stage(&self, id: &str, persist: bool, change: impl FnOnce(&mut Stage, u64)) {
        self.update(persist, |r| {
            let now = now_ms();
            if let Some(stage) = r.stages.iter_mut().find(|s| s.id == id) {
                change(stage, now);
            }
        });
    }
    /// The stage that receives untagged pipeline messages.
    fn current(&self) -> String {
        let report = self.report.lock().unwrap();
        report
            .stages
            .iter()
            .rev()
            .find(|s| s.status == "running" && s.id != "spatial")
            .or_else(|| report.stages.iter().find(|s| s.status == "running"))
            .map(|s| s.id.clone())
            .unwrap_or_default()
    }
    pub fn begin(&self, id: &str, detail: impl Into<String>) {
        let detail = detail.into();
        self.stage(id, true, |s, now| {
            s.status = "running".into();
            s.started_at_ms.get_or_insert(now);
            s.finished_at_ms = None;
            if !detail.is_empty() {
                s.detail = detail;
            }
        });
    }
    pub fn finish(&self, id: &str, detail: impl Into<String>) {
        self.close(id, "done", detail.into());
    }
    pub fn skip(&self, id: &str, why: impl Into<String>) {
        self.close(id, "skipped", why.into());
    }
    /// The walkthrough remains usable; this stage needs a retry or a source.
    pub fn attention(&self, id: &str, why: impl Into<String>) {
        let why = why.into();
        self.log(id, "warning", &why);
        self.close(id, "attention", why);
    }
    fn close(&self, id: &str, status: &str, detail: String) {
        self.stage(id, true, |s, now| {
            s.status = status.into();
            s.started_at_ms.get_or_insert(now);
            s.finished_at_ms = Some(now);
            if !detail.is_empty() {
                s.detail = detail;
            }
            if status == "done" && s.done.is_none() {
                s.done = s.total;
            }
        });
    }
    pub fn count(&self, id: &str, done: u64, total: Option<u64>) {
        self.stage(id, false, |s, _| {
            s.done = Some(done);
            s.total = total;
        });
    }
    pub fn detail(&self, id: &str, detail: impl Into<String>) {
        let detail = detail.into();
        self.stage(id, false, |s, _| s.detail = detail);
    }
    fn log(&self, stage: &str, level: &str, text: &str) {
        let (stage, level, text) = (stage.to_owned(), level.to_owned(), text.to_owned());
        self.update(level != "info", |r| {
            // Callers often report a failure and then mark the stage with it.
            if r.events
                .back()
                .is_some_and(|e| e.stage == stage && e.text == text)
            {
                return;
            }
            r.events.push_back(Event {
                at_ms: now_ms(),
                stage,
                level,
                text,
            });
            while r.events.len() > MAX_EVENTS {
                r.events.pop_front();
            }
        });
    }
    /// Record a pipeline message against the stage currently doing the work.
    pub fn message(&self, text: &str) {
        self.message_for(&self.current(), text);
    }
    pub fn message_for(&self, stage: &str, text: &str) {
        if text.trim().is_empty() {
            return;
        }
        self.log(stage, level(text), text);
        self.detail(stage, text);
    }
    pub fn complete(&self, result: &anyhow::Result<()>) {
        let error = result.as_ref().err().map(|e| format!("{e:#}"));
        if let Some(error) = &error {
            let stage = self.current();
            self.log(&stage, "error", error);
            self.stage(&stage, false, |s, now| {
                s.status = "failed".into();
                s.finished_at_ms = Some(now);
                s.detail = error.clone();
            });
        }
        self.update(true, |r| {
            let attention = r.stages.iter().any(|s| s.status == "attention");
            r.status = if error.is_some() {
                "failed"
            } else if attention {
                "partial"
            } else {
                "ready"
            }
            .into();
            r.error = error;
            // Anything never started belongs to a branch this launch didn't need.
            for stage in r.stages.iter_mut().filter(|s| s.status == "pending") {
                if r.status != "failed" {
                    stage.status = "skipped".into();
                    if stage.detail.is_empty() {
                        stage.detail = "Not needed for this walkthrough".into();
                    }
                }
            }
        });
    }
    /// Poll a long external command's output on disk until the guard drops.
    pub fn watch(&self, every: Duration, sample: impl Fn(&Tracker) + Send + 'static) -> WatchGuard {
        let stop = Arc::new(AtomicBool::new(false));
        let (flag, tracker) = (stop.clone(), self.clone());
        std::thread::spawn(move || {
            while !flag.load(Ordering::Relaxed) {
                sample(&tracker);
                std::thread::sleep(every);
            }
        });
        WatchGuard(stop)
    }
}
pub struct WatchGuard(Arc<AtomicBool>);
impl Drop for WatchGuard {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

/// Files under `dir` written since `since`, recursively: (count, bytes).
pub fn written_since(dir: &Path, since: SystemTime, extension: Option<&str>) -> (u64, u64) {
    let mut totals = (0, 0);
    let Ok(entries) = std::fs::read_dir(dir) else {
        return totals;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if metadata.is_dir() {
            let (n, b) = written_since(&path, since, extension);
            totals = (totals.0 + n, totals.1 + b);
        } else if metadata.modified().is_ok_and(|m| m >= since)
            && extension.is_none_or(|e| path.extension().is_some_and(|x| x == e))
        {
            totals = (totals.0 + 1, totals.1 + metadata.len());
        }
    }
    totals
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stages_events_and_outcome_are_visible_to_the_ui() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let tracker = Tracker::start(&store, "Faxanadu");
        tracker.begin("source", "Finding walkthroughs");
        tracker.message("Asking your reasoning model to assess the walkthrough candidates…");
        tracker.finish("source", "Selected a full walkthrough");
        tracker.begin("spatial", "");
        tracker.begin("sections", "");
        tracker.count("sections", 2, Some(5));
        tracker.message("Retrying section after validation/provider failure: bad JSON");
        tracker.message_for("spatial", "Verifying revisits: 3/9 views");
        tracker.finish("spatial", "");
        tracker.attention("atlas", "Location atlas preparation can be retried");
        tracker.complete(&Ok(()));
        let report = snapshot(dir.path(), "faxanadu").unwrap();
        assert_eq!(report.status, "partial");
        let stage = |id: &str| report.stages.iter().find(|s| s.id == id).unwrap().clone();
        assert_eq!(stage("sections").done, Some(2));
        assert!(stage("sections").detail.starts_with("Retrying"));
        assert_eq!(stage("spatial").detail, "Verifying revisits: 3/9 views");
        assert_eq!(stage("download").status, "skipped");
        assert_eq!(stage("atlas").status, "attention");
        assert!(
            report
                .events
                .iter()
                .any(|e| e.stage == "sections" && e.level == "warning")
        );
        let saved: serde_json::Value = store.get(&store_key("Faxanadu")).unwrap().unwrap();
        assert_eq!(saved["status"], "partial");
    }
    #[test]
    fn failure_marks_the_running_stage_and_keeps_pending_stages() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let tracker = Tracker::start(&store, "Game");
        tracker.begin("source", "");
        tracker.complete(&Err(anyhow::anyhow!("No suitable walkthrough candidates")));
        let report = tracker.report();
        assert_eq!(report.status, "failed");
        assert_eq!(report.stages[0].status, "failed");
        assert_eq!(report.stages[1].status, "pending");
        assert_eq!(report.events.back().unwrap().level, "error");
    }
}
