use anyhow::{Context, Result, ensure};
use gamebuddy_core::{Guide, Progress, VisualIndex};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Serialize, de::DeserializeOwned};
use std::path::{Path, PathBuf};

mod cached;
mod conversation;

pub fn data_dir() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("GAMEBUDDY_DATA_DIR") {
        return Ok(PathBuf::from(path));
    }
    Ok(directories::BaseDirs::new()
        .context("Cannot determine application data directory")?
        .data_local_dir()
        .join("GameBuddy"))
}

pub struct Store {
    pub root: PathBuf,
    connection: Connection,
}
impl Store {
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref().to_owned();
        std::fs::create_dir_all(&root)?;
        let connection = Connection::open(root.join("gamebuddy.sqlite"))?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        connection.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE IF NOT EXISTS guides(id TEXT PRIMARY KEY, body TEXT NOT NULL); CREATE TABLE IF NOT EXISTS documents(key TEXT PRIMARY KEY, body TEXT NOT NULL); CREATE TABLE IF NOT EXISTS events(id INTEGER PRIMARY KEY, guide_id TEXT NOT NULL, at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP, kind TEXT NOT NULL, body TEXT NOT NULL);")?;
        connection.execute_batch("CREATE TABLE IF NOT EXISTS guide_revisions(guide_id TEXT NOT NULL, revision INTEGER NOT NULL, body TEXT NOT NULL, summary TEXT NOT NULL, PRIMARY KEY(guide_id,revision));")?;
        cached::initialize(&connection)?;
        conversation::initialize(&connection)?;
        Ok(Self { root, connection })
    }
    pub fn save_guide(&self, guide: &Guide) -> Result<()> {
        guide.validate()?;
        self.connection.execute("INSERT INTO guides(id,body) VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET body=excluded.body",params![guide.id,serde_json::to_string(guide)?])?;
        Ok(())
    }
    pub fn guide(&self, id: &str) -> Result<Guide> {
        let body: String =
            self.connection
                .query_row("SELECT body FROM guides WHERE id=?1", [id], |r| r.get(0))?;
        let guide: Guide = serde_json::from_str(&body)?;
        guide.validate()?;
        Ok(guide)
    }
    pub fn guide_revision(&self, id: &str, revision: u64) -> Result<Guide> {
        let body: String = self.connection.query_row(
            "SELECT body FROM guide_revisions WHERE guide_id=?1 AND revision=?2",
            params![id, i64::try_from(revision)?],
            |r| r.get(0),
        )?;
        let guide: Guide = serde_json::from_str(&body)?;
        guide.validate()?;
        Ok(guide)
    }
    pub fn revision_history(&self, id: &str) -> Result<Vec<serde_json::Value>> {
        let mut statement = self.connection.prepare(
            "SELECT revision,summary FROM guide_revisions WHERE guide_id=?1 ORDER BY revision DESC",
        )?;
        let rows = statement.query_map([id], |row| Ok(serde_json::json!({"revision":row.get::<_,i64>(0)?,"summary":row.get::<_,String>(1)?})))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
    /// Serialize concurrent editors with a SQLite write lock, then compare the
    /// expected revision. Guide, history, progress and index move together.
    pub fn revise_guide(&self, guide: &Guide, expected: u64, summary: &str) -> Result<()> {
        guide.validate()?;
        ensure!(
            guide.revision == expected + 1,
            "A plan edit must create the next revision"
        );
        let transaction = rusqlite::Transaction::new_unchecked(
            &self.connection,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let old = self.guide(&guide.id)?;
        ensure!(
            old.revision == expected,
            "Plan changed; read the latest outline before editing"
        );
        ensure!(
            old.game == guide.game,
            "An outline edit cannot change the game identity"
        );
        let unchanged = |id: &str| old.step(id).is_some() && old.step(id) == guide.step(id);
        let mut progress = self.progress(&old)?;
        progress.epoch += 1;
        progress.completed.retain(|id| unchanged(id));
        progress.reported_completed.retain(|id| unchanged(id));
        if progress.current_step.as_deref().is_some_and(|id| {
            !unchanged(id) || guide.step(id).is_none_or(|s| !guide.step_enabled(s))
        }) {
            progress.current_step = None;
        }
        transaction.execute("INSERT OR IGNORE INTO guide_revisions(guide_id,revision,body,summary) VALUES(?1,?2,?3,'Before first tracked edit')",params![old.id,i64::try_from(old.revision)?,serde_json::to_string(&old)?])?;
        transaction.execute(
            "INSERT INTO guide_revisions(guide_id,revision,body,summary) VALUES(?1,?2,?3,?4)",
            params![
                guide.id,
                i64::try_from(guide.revision)?,
                serde_json::to_string(guide)?,
                summary
            ],
        )?;
        self.save_guide(guide)?;
        self.put(
            &format!("progress:{}:{}", guide.id, guide.revision),
            &progress,
        )?;
        // Reuse fingerprints only while their source and timestamp anchors still
        // identify the same content. Changed/new sections need fresh indexing.
        if let Some(mut index) = self.index(&old)? {
            for frame in &mut index.frames {
                if old.sources.iter().find(|s| s.id == frame.source_id)
                    != guide.sources.iter().find(|s| s.id == frame.source_id)
                {
                    frame.step_ids.clear();
                    continue;
                }
                // Rebind existing images to newly compiled substeps without
                // decoding the full video again for every outline revision.
                let matching: Vec<_> = guide
                    .steps
                    .iter()
                    .filter(|s| {
                        s.anchors.iter().any(|a| {
                            a.source_id == frame.source_id
                                && a.start_ms.is_some_and(|t| t <= frame.at_ms)
                                && a.end_ms.is_some_and(|t| frame.at_ms < t)
                        })
                    })
                    .collect();
                if matching.is_empty() {
                    frame.step_ids.retain(|id| {
                        old.step(id)
                            .zip(guide.step(id))
                            .is_some_and(|(a, b)| a.anchors == b.anchors)
                    });
                } else {
                    let parents: std::collections::HashSet<_> = matching
                        .iter()
                        .filter_map(|s| s.parent_id.as_deref())
                        .collect();
                    frame.step_ids = matching
                        .iter()
                        .filter(|s| !parents.contains(s.id.as_str()))
                        .map(|s| s.id.clone())
                        .collect();
                }
            }
            index.frames.retain(|frame| !frame.step_ids.is_empty());
            index.guide_revision = guide.revision;
            self.save_index(&index)?;
        }
        transaction.execute(
            "INSERT INTO events(guide_id,kind,body) VALUES(?1,'plan_edit',?2)",
            params![
                guide.id,
                serde_json::json!({"from":expected,"to":guide.revision,"summary":summary})
                    .to_string()
            ],
        )?;
        transaction.commit()?;
        Ok(())
    }
    pub fn guides(&self) -> Result<Vec<Guide>> {
        let mut stmt = self
            .connection
            .prepare("SELECT body FROM guides ORDER BY rowid DESC")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        rows.map(|r| {
            let guide: Guide = serde_json::from_str(&r?)?;
            guide.validate()?;
            Ok(guide)
        })
        .collect()
    }
    pub fn put<T: Serialize>(&self, key: &str, value: &T) -> Result<()> {
        self.connection.execute("INSERT INTO documents(key,body) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET body=excluded.body",params![key,serde_json::to_string(value)?])?;
        Ok(())
    }
    pub fn put_if_absent<T: Serialize>(&self, key: &str, value: &T) -> Result<bool> {
        Ok(self.connection.execute(
            "INSERT INTO documents(key,body) VALUES(?1,?2) ON CONFLICT(key) DO NOTHING",
            params![key, serde_json::to_string(value)?],
        )? == 1)
    }
    /// Atomically replace a document only if its complete previous value matches.
    pub fn compare_exchange<T: Serialize>(
        &self,
        key: &str,
        expected: &T,
        value: &T,
    ) -> Result<bool> {
        Ok(self.connection.execute(
            "UPDATE documents SET body=?3 WHERE key=?1 AND body=?2",
            params![
                key,
                serde_json::to_string(expected)?,
                serde_json::to_string(value)?
            ],
        )? == 1)
    }
    /// Publish derived data only while the input guide revision is still current.
    pub fn put_for_guide<T: Serialize>(&self, guide: &Guide, key: &str, value: &T) -> Result<()> {
        let transaction = rusqlite::Transaction::new_unchecked(
            &self.connection,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        ensure!(
            self.guide(&guide.id)?.revision == guide.revision,
            "Outline changed while derived data was being built; retry preparation"
        );
        self.put(key, value)?;
        transaction.commit()?;
        Ok(())
    }
    pub(crate) fn put_for_progress<T: Serialize>(
        &self,
        guide: &Guide,
        epoch: u64,
        key: &str,
        value: &T,
    ) -> Result<()> {
        let transaction = rusqlite::Transaction::new_unchecked(
            &self.connection,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        ensure!(
            self.guide(&guide.id)?.revision == guide.revision
                && self.progress(guide)?.epoch == epoch,
            "Plan or progress changed; read the current context"
        );
        self.put(key, value)?;
        transaction.commit()?;
        Ok(())
    }
    pub fn get<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>> {
        Ok(self.get_versioned(key)?.map(|(value, _)| value))
    }
    /// Retain the original serialized value for CAS, including documents saved
    /// before new serde-default fields were added.
    pub fn get_versioned<T: DeserializeOwned>(&self, key: &str) -> Result<Option<(T, String)>> {
        let body: Option<String> = self
            .connection
            .query_row("SELECT body FROM documents WHERE key=?1", [key], |r| {
                r.get(0)
            })
            .optional()?;
        body.map(|s| Ok((serde_json::from_str(&s)?, s))).transpose()
    }
    pub fn replace_version<T: Serialize>(
        &self,
        key: &str,
        version: &str,
        value: &T,
    ) -> Result<bool> {
        Ok(self.connection.execute(
            "UPDATE documents SET body=?3 WHERE key=?1 AND body=?2",
            params![key, version, serde_json::to_string(value)?],
        )? == 1)
    }
    pub fn index(&self, guide: &Guide) -> Result<Option<VisualIndex>> {
        self.get(&format!("index:{}:{}", guide.id, guide.revision))
    }
    pub fn save_index(&self, index: &VisualIndex) -> Result<()> {
        self.put(
            &format!("index:{}:{}", index.guide_id, index.guide_revision),
            index,
        )
    }
    pub fn progress(&self, guide: &Guide) -> Result<Progress> {
        Ok(self
            .get(&format!("progress:{}:{}", guide.id, guide.revision))?
            .unwrap_or_default())
    }
    pub fn report_complete(
        &self,
        guide: &Guide,
        epoch: u64,
        step_id: &str,
        report: &str,
    ) -> Result<Progress> {
        ensure!(
            !report.trim().is_empty() && report.len() <= 1000,
            "Include the player's short explicit completion report"
        );
        ensure!(
            guide.step(step_id).is_some_and(|s| guide.step_enabled(s)),
            "Unknown or disabled task"
        );
        let transaction = rusqlite::Transaction::new_unchecked(
            &self.connection,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        ensure!(
            self.guide(&guide.id)?.revision == guide.revision,
            "Outline changed; read the current context"
        );
        let mut progress = self.progress(guide)?;
        ensure!(
            progress.epoch == epoch,
            "Game progress changed; read the current context"
        );
        progress.reported_completed.insert(step_id.into());
        if progress.current_step.is_none() {
            progress.current_step = Some(step_id.into());
        }
        let tracker_key = crate::navigation::tracker_key(guide);
        if let Some(mut tracker) = self.get::<gamebuddy_core::PlaceTracker>(&tracker_key)?
            && tracker.epoch == progress.epoch
        {
            tracker.epoch += 1;
            self.put(&tracker_key, &tracker)?;
        }
        progress.epoch += 1;
        self.put(
            &format!("progress:{}:{}", guide.id, guide.revision),
            &progress,
        )?;
        self.connection.execute(
            "INSERT INTO events(guide_id,kind,body) VALUES(?1,'player_report',?2)",
            params![
                guide.id,
                serde_json::json!({"step_id":step_id,"report":report,"progress":progress})
                    .to_string()
            ],
        )?;
        transaction.commit()?;
        Ok(progress)
    }
    /// A direct checkbox edit is atomic and invalidates older screen work.
    /// Unchecking a child must also remove inherited completion from its parents.
    pub fn check_step(
        &self,
        guide: &Guide,
        epoch: u64,
        step_id: &str,
        checked: bool,
    ) -> Result<Progress> {
        let step = guide.step(step_id).context("Unknown task")?;
        let tx = rusqlite::Transaction::new_unchecked(
            &self.connection,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        ensure!(
            self.guide(&guide.id)?.revision == guide.revision,
            "Outline changed; reload the reader"
        );
        let mut progress = self.progress(guide)?;
        ensure!(
            progress.epoch == epoch,
            "Progress changed; reload the reader and try again"
        );
        if checked {
            progress.completed.insert(step_id.into());
        } else {
            let previously_done: Vec<_> = guide
                .steps
                .iter()
                .filter(|s| {
                    !guide
                        .steps
                        .iter()
                        .any(|child| child.parent_id.as_deref() == Some(&s.id))
                        && gamebuddy_core::step_complete(guide, &progress, &s.id)
                })
                .map(|s| s.id.clone())
                .collect();
            let mut ids = std::collections::HashSet::from([step_id.to_owned()]);
            let mut parent = step.parent_id.as_deref();
            while let Some(id) = parent {
                ids.insert(id.into());
                parent = guide.step(id).and_then(|s| s.parent_id.as_deref());
            }
            for candidate in &guide.steps {
                let mut ancestor = candidate.parent_id.as_deref();
                while let Some(id) = ancestor {
                    if id == step_id {
                        ids.insert(candidate.id.clone());
                        break;
                    }
                    ancestor = guide.step(id).and_then(|s| s.parent_id.as_deref());
                }
            }
            progress.completed.retain(|id| !ids.contains(id));
            progress.reported_completed.retain(|id| !ids.contains(id));
            progress
                .completed
                .extend(previously_done.into_iter().filter(|id| !ids.contains(id)));
        }
        let key = crate::navigation::tracker_key(guide);
        if let Some(mut tracker) = self.get::<gamebuddy_core::PlaceTracker>(&key)?
            && tracker.epoch == epoch
        {
            tracker.epoch += 1;
            self.put(&key, &tracker)?;
        }
        progress.epoch += 1;
        self.put(
            &format!("progress:{}:{}", guide.id, guide.revision),
            &progress,
        )?;
        tx.execute(
            "INSERT INTO events(guide_id,kind,body) VALUES(?1,'reader_checkbox',?2)",
            params![
                guide.id,
                serde_json::json!({"step_id":step_id,"checked":checked,"progress":progress})
                    .to_string()
            ],
        )?;
        tx.commit()?;
        Ok(progress)
    }
    /// Commit a player location and its task cursor together. Incrementing the
    /// epoch rejects any screen comparison that began before this report.
    pub(crate) fn report_location(
        &self,
        guide: &Guide,
        epoch: u64,
        mut tracker: gamebuddy_core::PlaceTracker,
        report: &str,
    ) -> Result<Progress> {
        let tx = rusqlite::Transaction::new_unchecked(
            &self.connection,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        ensure!(
            self.guide(&guide.id)?.revision == guide.revision,
            "Outline changed; read the current context"
        );
        let mut progress = self.progress(guide)?;
        ensure!(
            progress.epoch == epoch,
            "Game progress changed; read the current context"
        );
        progress.current_step = tracker.reference_step_id.clone();
        progress.epoch += 1;
        tracker.epoch = progress.epoch;
        self.put(
            &format!("progress:{}:{}", guide.id, guide.revision),
            &progress,
        )?;
        self.put(&crate::navigation::tracker_key(guide), &tracker)?;
        tx.execute(
            "INSERT INTO events(guide_id,kind,body) VALUES(?1,'player_location',?2)",
            params![
                guide.id,
                serde_json::json!({"report":report,"location":tracker}).to_string()
            ],
        )?;
        tx.commit()?;
        Ok(progress)
    }
    pub fn save_progress(&self, guide: &Guide, progress: &Progress, kind: &str) -> Result<()> {
        let tx = self.connection.unchecked_transaction()?;
        ensure!(
            progress.epoch >= self.progress(guide)?.epoch,
            "Newer player progress exists; discard this stale observation"
        );
        let body = serde_json::to_string(progress)?;
        tx.execute("INSERT INTO documents(key,body) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET body=excluded.body",params![format!("progress:{}:{}",guide.id,guide.revision),body])?;
        tx.execute(
            "INSERT INTO events(guide_id,kind,body) VALUES(?1,?2,?3)",
            params![guide.id, kind, body],
        )?;
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn player_reports_survive_stale_capture_and_require_current_epoch() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path()).unwrap();
        let guide =
            gamebuddy_core::import_markdown("Game", "Guide", "# Buy\nPurchase the spell.").unwrap();
        store.save_guide(&guide).unwrap();
        let report = store
            .report_complete(&guide, 0, "step-1", "I bought the spell")
            .unwrap();
        assert_eq!(report.epoch, 1);
        assert!(report.completed.is_empty());
        assert!(report.reported_completed.contains("step-1"));
        assert!(
            store
                .save_progress(&guide, &Progress::default(), "stale_capture")
                .is_err()
        );
        assert!(
            store
                .report_complete(&guide, 0, "step-1", "Old turn")
                .is_err()
        );
        assert!(gamebuddy_core::step_complete(
            &guide,
            &store.progress(&guide).unwrap(),
            "step-1"
        ));
    }
    #[test]
    fn guide_and_progress_survive_reopen_and_revision_isolation() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let mut g =
            gamebuddy_core::import_markdown("Faxanadu", "Test", "# Start\nA source instruction.")
                .unwrap();
        store.save_guide(&g).unwrap();
        let p = Progress {
            current_step: Some("step-1".into()),
            ..Default::default()
        };
        store.save_progress(&g, &p, "user_location").unwrap();
        drop(store);
        let store = Store::open(dir.path()).unwrap();
        assert_eq!(store.guide(&g.id).unwrap(), g);
        assert_eq!(store.progress(&g).unwrap().current_step, p.current_step);
        g.revision += 1;
        assert!(store.progress(&g).unwrap().current_step.is_none());
    }
}
