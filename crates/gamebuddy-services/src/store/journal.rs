use super::*;
use crate::journal::{Entry, Note, Run};
use serde_json::{Value, json};

pub(super) fn initialize(connection: &Connection) -> Result<()> {
    connection.execute_batch("CREATE TABLE IF NOT EXISTS journal_runs(id TEXT PRIMARY KEY, guide_id TEXT NOT NULL, title TEXT NOT NULL, started_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP);
        CREATE TABLE IF NOT EXISTS journal_entries(id INTEGER PRIMARY KEY AUTOINCREMENT,run_id TEXT NOT NULL,source_key TEXT NOT NULL,at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,title TEXT NOT NULL,body TEXT NOT NULL,category TEXT NOT NULL,evidence_kind TEXT NOT NULL,step_ids TEXT NOT NULL,images TEXT NOT NULL,pinned INTEGER NOT NULL DEFAULT 0,UNIQUE(run_id,source_key));
        CREATE INDEX IF NOT EXISTS events_by_guide ON events(guide_id,id);
        CREATE INDEX IF NOT EXISTS journal_by_run ON journal_entries(run_id,id DESC);
        CREATE INDEX IF NOT EXISTS journal_runs_by_guide ON journal_runs(guide_id);")?;
    Ok(())
}

impl Store {
    pub fn journal_active(&self, guide: &Guide) -> Result<String> {
        let key = format!("journal-active:{}", guide.id);
        if let Some(id) = self.get::<String>(&key)? {
            return Ok(id);
        }
        // Deterministic initial identity prevents duplicate first runs when
        // multiple background readers initialize at the same time.
        let id = format!("{}-initial", guide.id);
        self.connection.execute(
            "INSERT OR IGNORE INTO journal_runs(id,guide_id,title) VALUES(?1,?2,'Current run')",
            params![id, guide.id],
        )?;
        self.put_if_absent(&key, &id)?;
        self.get(&key)?.context("Journal run unavailable")
    }

    pub fn journal_start(&self, guide: &Guide, title: &str) -> Result<String> {
        // Finish backfill into the previous run before establishing a boundary.
        let target: i64 = self.connection.query_row(
            "SELECT coalesce(max(id),0) FROM events WHERE guide_id=?1",
            [&guide.id],
            |r| r.get(0),
        )?;
        while self
            .get::<i64>(&format!("journal-cursor:{}", guide.id))?
            .unwrap_or(0)
            < target
        {
            self.journal_sync(guide)?;
        }
        let tx = self.connection.unchecked_transaction()?;
        let id = uuid::Uuid::new_v4().to_string();
        tx.execute(
            "INSERT INTO journal_runs(id,guide_id,title) VALUES(?1,?2,?3)",
            params![id, guide.id, title],
        )?;
        self.put(&format!("journal-active:{}", guide.id), &id)?;
        let last: i64 = tx.query_row(
            "SELECT coalesce(max(id),0) FROM events WHERE guide_id=?1",
            [&guide.id],
            |r| r.get(0),
        )?;
        self.put(&format!("journal-cursor:{}", guide.id), &last)?;
        tx.commit()?;
        Ok(id)
    }

    pub fn journal_runs(&self, guide: &Guide) -> Result<Vec<Run>> {
        let mut stmt = self.connection.prepare(
            "SELECT id,title,started_at FROM journal_runs WHERE guide_id=?1 ORDER BY rowid DESC",
        )?;
        Ok(stmt
            .query_map([&guide.id], |r| {
                Ok(Run {
                    id: r.get(0)?,
                    title: r.get(1)?,
                    started_at: r.get(2)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?)
    }

    pub fn journal_count(&self, run: &str) -> Result<i64> {
        Ok(self.connection.query_row(
            "SELECT count(*) FROM journal_entries WHERE run_id=?1",
            [run],
            |r| r.get(0),
        )?)
    }

    pub fn journal_entries(
        &self,
        run: &str,
        query: &str,
        before: Option<i64>,
        pinned: bool,
        limit: i64,
    ) -> Result<Vec<Entry>> {
        let mut stmt=self.connection.prepare("SELECT id,run_id,at,title,body,category,evidence_kind,step_ids,images,pinned FROM journal_entries WHERE run_id=?1 AND (?2 IS NULL OR id<?2) AND (NOT ?3 OR pinned=1) AND (?4='' OR instr(lower(title||' '||body),lower(?4))>0) ORDER BY id DESC LIMIT ?5")?;
        let rows = stmt.query_map(
            params![run, before, pinned, query, limit.clamp(1, 30)],
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, String>(6)?,
                    r.get::<_, String>(7)?,
                    r.get::<_, String>(8)?,
                    r.get::<_, bool>(9)?,
                ))
            },
        )?;
        rows.map(|r| {
            let (id, run_id, at, title, body, category, evidence_kind, steps, images, pinned) = r?;
            Ok(Entry {
                id,
                run_id,
                at,
                title,
                body,
                category,
                evidence_kind,
                step_ids: serde_json::from_str(&steps)?,
                pinned,
                image_count: serde_json::from_str::<Vec<Value>>(&images)?.len(),
            })
        })
        .collect()
    }

    pub fn journal_note(
        &self,
        guide: &Guide,
        note: &Note,
        origin: &str,
        images: &[Value],
    ) -> Result<i64> {
        let tx = self.connection.unchecked_transaction()?;
        ensure!(
            self.guide(&guide.id)?.revision == guide.revision,
            "The guide changed before the journal note was saved"
        );
        let run = self.journal_active(guide)?;
        tx.execute("INSERT INTO journal_entries(run_id,source_key,title,body,category,evidence_kind,step_ids,images,pinned) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",params![run,uuid::Uuid::new_v4().to_string(),note.title,note.body,serde_json::to_value(&note.category)?.as_str().unwrap_or("note"),origin,serde_json::to_string(&note.step_ids)?,serde_json::to_string(images)?,note.pinned])?;
        let id = tx.last_insert_rowid();
        tx.commit()?;
        Ok(id)
    }

    pub fn journal_pin(&self, guide: &Guide, id: i64, pinned: bool) -> Result<()> {
        ensure!(self.connection.execute("UPDATE journal_entries SET pinned=?3 WHERE id=?1 AND run_id IN(SELECT id FROM journal_runs WHERE guide_id=?2)",params![id,guide.id,pinned])?==1,"Journal entry is not in this game's runs");
        Ok(())
    }

    pub fn journal_image(&self, guide: &Guide, id: i64, index: usize) -> Result<Value> {
        let images:String=self.connection.query_row("SELECT images FROM journal_entries WHERE id=?1 AND run_id IN(SELECT id FROM journal_runs WHERE guide_id=?2)",params![id,guide.id],|r|r.get(0))?;
        serde_json::from_str::<Vec<Value>>(&images)?
            .get(index)
            .cloned()
            .context("Journal image unavailable")
    }

    /// Consume existing progress events exactly once. No screenshot is added to
    /// a historical event unless it was retained with that event at the time.
    pub fn journal_sync(&self, guide: &Guide) -> Result<()> {
        let cursor_key = format!("journal-cursor:{}", guide.id);
        let cursor = self.get::<i64>(&cursor_key)?.unwrap_or(0);
        let latest: i64 = self.connection.query_row(
            "SELECT coalesce(max(id),0) FROM events WHERE guide_id=?1",
            [&guide.id],
            |r| r.get(0),
        )?;
        if cursor == latest {
            self.journal_active(guide)?;
            return Ok(());
        }
        let tx = rusqlite::Transaction::new_unchecked(
            &self.connection,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let cursor = self.get::<i64>(&cursor_key)?.unwrap_or(0);
        let run = self.journal_active(guide)?;
        let mut stmt=tx.prepare("SELECT id,at,kind,body FROM events WHERE guide_id=?1 AND id>?2 AND id<=?3 ORDER BY id LIMIT 500")?;
        let rows = stmt.query_map(params![guide.id, cursor, latest], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?;
        let mut consumed = cursor;
        for row in rows {
            let (id, at, kind, body) = row?;
            consumed = id;
            if !matches!(
                kind.as_str(),
                "observed_completion"
                    | "player_report"
                    | "reader_checkbox"
                    | "world_report"
                    | "player_location"
                    | "plan_recovery"
            ) {
                continue;
            }
            let value: Value = serde_json::from_str(&body)?;
            let step = value["step_id"].as_str().and_then(|id| guide.step(id));
            let (title, body, category) = match kind.as_str() {
                "observed_completion" => (
                    step.map(|s| s.title.clone())
                        .unwrap_or_else(|| "Recorded milestone".into()),
                    value["evidence"]["evidence"]
                        .as_str()
                        .unwrap_or("Observed across game screenshots.")
                        .to_owned(),
                    "milestone",
                ),
                "player_report" => (
                    step.map(|s| s.title.clone())
                        .unwrap_or_else(|| "Progress reported".into()),
                    value["report"]
                        .as_str()
                        .unwrap_or("Player reported this action complete.")
                        .to_owned(),
                    "milestone",
                ),
                "reader_checkbox" => (
                    step.map(|s| s.title.clone())
                        .unwrap_or_else(|| "Progress updated".into()),
                    if value["checked"] == true {
                        "Marked complete in the guide."
                    } else {
                        "Completion unchecked in the guide; this corrects the earlier record."
                    }
                    .into(),
                    "milestone",
                ),
                "world_report" => (
                    "Run information updated".into(),
                    value["report"].as_str().unwrap_or_default().to_owned(),
                    "note",
                ),
                "player_location" => (
                    "Location reported".into(),
                    value["report"].as_str().unwrap_or_default().to_owned(),
                    "location",
                ),
                _ => (
                    "Walkthrough updated".into(),
                    value["summary"].as_str().unwrap_or_default().to_owned(),
                    "plan",
                ),
            };
            let mut images:Vec<Value>=value["images"].as_array().into_iter().flatten().map(|v|if v.is_string(){json!({"path":v,"captured_at_ms":value["observed_at_ms"],"session":value["session"]})}else{v.clone()}).collect();
            if let Some(image) = value.get("journal_image").filter(|v| v.is_object()) {
                images.push(image.clone());
            }
            let steps: Vec<_> = value["step_id"].as_str().into_iter().collect();
            tx.execute("INSERT OR IGNORE INTO journal_entries(run_id,source_key,at,title,body,category,evidence_kind,step_ids,images) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",params![run,format!("event:{id}"),at,title,body,category,kind,serde_json::to_string(&steps)?,serde_json::to_string(&images)?])?;
        }
        drop(stmt);
        self.put(&cursor_key, &consumed)?;
        tx.commit()?;
        Ok(())
    }
}
