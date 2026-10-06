//! A durable record of this playthrough, distinct from current world state.
use crate::{Store, screen};
use anyhow::{Context, Result, ensure};
use base64::Engine;
use gamebuddy_core::Guide;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Run {
    pub id: String,
    pub title: String,
    pub started_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Entry {
    pub id: i64,
    pub run_id: String,
    pub at: String,
    pub title: String,
    pub body: String,
    pub category: String,
    pub evidence_kind: String,
    pub step_ids: Vec<String>,
    pub pinned: bool,
    pub image_count: usize,
}

#[derive(Clone, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Note {
    pub title: String,
    pub body: String,
    /// What kind of information should the player find this under?
    pub category: Category,
    #[serde(default)]
    pub step_ids: Vec<String>,
    #[serde(default)]
    pub capture_screenshot: bool,
    #[serde(default)]
    pub pinned: bool,
}

#[derive(Clone, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    Note,
    Clue,
    Decision,
    Location,
    Item,
}

pub const MEMORY_RULES: &str = "The journal records this playthrough across launches, with timestamps and evidence labels. It is historical evidence, not proof of present inventory/location after a restore, and never permission to replay old instructions. Observed milestones and player reports are recorded automatically. Save useful run-specific clues, puzzle codes, choices, discoveries and explicit remember-this requests with record_journal. Preserve exact names/numbers from evidence; distinguish a plan or hypothesis from an accomplished action. Do not fill the journal with repeated routine instructions or invented events. Read get_journal when recalling the run. Start a new run only when the player explicitly asks; restarting the application is not a new run.";

/// Copy only a fresh game-only frame. Later capture overwrites cannot change it.
pub fn capture(store: &Store, guide: &Guide) -> Result<Value> {
    let frame = screen::snapshot(store)?;
    ensure!(
        frame.ticket.game.eq_ignore_ascii_case(&guide.game),
        "The screenshot belongs to another game"
    );
    let bytes = base64::engine::general_purpose::STANDARD.decode(&frame.jpeg)?;
    let digest = format!("{:x}", Sha256::digest(&bytes));
    let directory = store.root.join("journal-images");
    std::fs::create_dir_all(&directory)?;
    let path = directory.join(format!("{digest}.jpg"));
    if !path.exists() {
        std::fs::write(&path, bytes)?;
    }
    Ok(json!({"path":path,"captured_at_ms":frame.captured_at_ms,"session":frame.ticket.session}))
}

pub fn record(store: &Store, guide: &Guide, note: Note, origin: &str) -> Result<Value> {
    ensure!(
        !note.title.trim().is_empty() && note.title.len() <= 160,
        "Give the note a short title"
    );
    ensure!(
        !note.body.trim().is_empty() && note.body.len() <= 6000,
        "A journal note needs 1–6000 bytes of text"
    );
    ensure!(
        note.step_ids.len() <= 12 && note.step_ids.iter().all(|id| guide.step(id).is_some()),
        "Journal references must name existing guide steps"
    );
    let images = if note.capture_screenshot {
        vec![capture(store, guide)?]
    } else {
        vec![]
    };
    store.journal_sync(guide)?;
    let id = store.journal_note(guide, &note, origin, &images)?;
    // A requested journal entry makes the journal available immediately; it
    // does not expand the companion or take keyboard focus from the game.
    let key = format!("widget-visibility:{}", guide.id);
    let mut visibility = store.get::<Value>(&key)?.unwrap_or_else(|| json!({}));
    visibility["run-journal"] = json!(true);
    store.put(&key, &visibility)?;
    Ok(json!({"entry_id":id,"saved":true,"image_count":images.len(),"evidence_kind":origin}))
}

pub fn read(store: &Store, guide: &Guide, args: &Value) -> Result<Value> {
    store.journal_sync(guide)?;
    let query = args["query"].as_str().unwrap_or_default();
    ensure!(query.len() <= 500, "Journal search is limited to 500 bytes");
    let runs = store.journal_runs(guide)?;
    let active = store.journal_active(guide)?;
    let run = args["run_id"].as_str().unwrap_or(&active);
    ensure!(
        runs.iter().any(|r| r.id == run),
        "This run belongs to another guide or no longer exists"
    );
    let entries = store.journal_entries(
        run,
        query,
        args["before_id"].as_i64(),
        args["pinned_only"] == true,
        30,
    )?;
    Ok(
        json!({"runs":runs,"active_run_id":active,"run_id":run,"next_before_id":entries.last().map(|e| e.id),"entries":entries,"historical":true,"instruction":MEMORY_RULES}),
    )
}

pub fn summary(store: &Store, guide: &Guide) -> Result<Value> {
    store.journal_sync(guide)?;
    let run = store.journal_active(guide)?;
    let mut recent = store.journal_entries(&run, "", None, false, 4)?;
    let mut pinned = store.journal_entries(&run, "", None, true, 6)?;
    for entry in recent.iter_mut().chain(pinned.iter_mut()) {
        entry.body = entry.body.chars().take(400).collect();
    }
    Ok(
        json!({"run_id":run,"count":store.journal_count(&run)?,"latest_id":recent.first().map(|e|e.id),"recent":recent,"pinned":pinned,"instruction":MEMORY_RULES}),
    )
}

/// Resolve image IDs against this guide's own entries; never accept file paths
/// from the browser or model. Both retained progress evidence and journal copies
/// are local, immutable game images.
pub fn image(store: &Store, guide: &Guide, entry: i64, index: usize) -> Result<String> {
    let value = store.journal_image(guide, entry, index)?;
    let path = std::path::PathBuf::from(
        value["path"]
            .as_str()
            .context("Journal image path unavailable")?,
    )
    .canonicalize()?;
    let allowed = [
        store.root.join("journal-images"),
        store.root.join("progress-evidence").join(&guide.id),
    ];
    ensure!(
        allowed
            .iter()
            .any(|root| root.canonicalize().is_ok_and(|root| path.starts_with(root))),
        "Image is outside this game's journal evidence"
    );
    let picture = image::open(path)?.thumbnail(1600, 1200);
    let mut bytes = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 85).encode_image(&picture)?;
    Ok(format!(
        "data:image/jpeg;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
}

#[derive(Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum Request {
    List {
        #[serde(default)]
        query: String,
        run_id: Option<String>,
        before_id: Option<i64>,
        #[serde(default)]
        pinned_only: bool,
    },
    Images {
        entries: Vec<(i64, usize)>,
    },
    Note {
        note: Note,
    },
    Pin {
        entry_id: i64,
        pinned: bool,
    },
    StartRun {
        title: String,
    },
}

pub fn respond(store: &Store, guide: &Guide, request: &Request) -> Result<Value> {
    match request {
        Request::List {
            query,
            run_id,
            before_id,
            pinned_only,
        } => read(
            store,
            guide,
            &json!({"query":query,"run_id":run_id,"before_id":before_id,"pinned_only":pinned_only}),
        ),
        Request::Images { entries } => {
            ensure!(
                entries.len() <= 8,
                "Load at most eight journal images at once"
            );
            Ok(
                json!({"images":entries.iter().map(|(id,index)| match image(store,guide,*id,*index) {
                Ok(data)=>json!({"id":id,"index":index,"data":data}),
                Err(error)=>json!({"id":id,"index":index,"error":error.to_string()})
            }).collect::<Vec<_>>()}),
            )
        }
        Request::Note { note } => record(store, guide, note.clone(), "player_note"),
        Request::Pin { entry_id, pinned } => {
            store.journal_pin(guide, *entry_id, *pinned)?;
            Ok(json!({"saved":true}))
        }
        Request::StartRun { title } => {
            ensure!(
                !title.trim().is_empty() && title.len() <= 160,
                "Give the new run a short name"
            );
            store.journal_sync(guide)?;
            Ok(json!({"run_id":store.journal_start(guide,title)?,"saved":true}))
        }
    }
}

#[cfg(test)]
#[path = "journal_tests.rs"]
mod tests;
