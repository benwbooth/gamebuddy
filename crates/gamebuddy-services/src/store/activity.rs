use super::*;
use crate::{
    activity::{Completion, Observation},
    screen,
};
use base64::Engine;

impl Store {
    pub(crate) fn observe_completions(
        &self,
        observation: &Observation,
        events: &[&Completion],
    ) -> Result<()> {
        let frame = observation
            .frames
            .last()
            .context("No completion screenshots")?;
        let tx = rusqlite::Transaction::new_unchecked(
            &self.connection,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        ensure!(
            self.get::<String>("active_guide")?.as_deref() == Some(&observation.guide.id)
                && self.guide(&observation.guide.id)?.revision == observation.guide.revision,
            "The active guide changed during event observation"
        );
        let ticket = screen::ticket(self, &frame.ticket.session)?;
        ensure!(
            ticket == frame.ticket
                && screen::now_ms().saturating_sub(frame.captured_at_ms) <= 45_000,
            "Completion screenshots belong to an old capture"
        );
        let mut progress = self.progress(&observation.guide)?;
        ensure!(
            progress.epoch == observation.progress.epoch,
            "Progress changed during event observation"
        );
        let mut records = vec![];
        for event in events {
            ensure!(
                !gamebuddy_core::step_complete(&observation.guide, &progress, &event.step_id),
                "Action already completed"
            );
            let mut paths = vec![];
            for index in [event.before_frame, event.after_frame] {
                let evidence = &observation.frames[index];
                let directory = self
                    .root
                    .join("progress-evidence")
                    .join(&observation.guide.id);
                std::fs::create_dir_all(&directory)?;
                let path = directory.join(format!(
                    "{}-{}.jpg",
                    evidence.ticket.session, evidence.captured_at_ms
                ));
                std::fs::write(
                    &path,
                    base64::engine::general_purpose::STANDARD.decode(&evidence.jpeg)?,
                )?;
                paths.push(path);
            }
            progress.completed.insert(event.step_id.clone());
            records.push(
                serde_json::json!({"step_id":event.step_id,"evidence":event,"images":paths,
                "session":frame.ticket.session,"observed_at_ms":frame.captured_at_ms,
                "guide_revision":observation.guide.revision,"progress_epoch":progress.epoch+1}),
            );
        }
        // An observed action is evidence of the current route position. The
        // next task follows it without claiming earlier unchecked tasks done.
        if let Some(last) = events.last() {
            progress.current_step = Some(last.step_id.clone());
        }
        // Old advice expires with this new observed route position. It must
        // not pull the cursor back to an action from the previous section.
        progress.epoch += 1;
        self.put(
            &format!(
                "progress:{}:{}",
                observation.guide.id, observation.guide.revision
            ),
            &progress,
        )?;
        for record in records {
            tx.execute(
                "INSERT INTO events(guide_id,kind,body) VALUES(?1,'observed_completion',?2)",
                params![observation.guide.id, record.to_string()],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn observed_progress(
        &self,
        guide: &Guide,
        progress: &Progress,
    ) -> Result<Vec<serde_json::Value>> {
        let mut statement = self.connection.prepare(
            "SELECT body FROM events WHERE guide_id=?1 AND kind='observed_completion' ORDER BY id DESC LIMIT 12")?;
        let rows = statement.query_map([&guide.id], |r| r.get::<_, String>(0))?;
        let mut events = vec![];
        for row in rows {
            let mut event: serde_json::Value = serde_json::from_str(&row?)?;
            if event["guide_revision"].as_u64() == Some(guide.revision)
                && event["step_id"]
                    .as_str()
                    .is_some_and(|id| progress.completed.contains(id))
            {
                event.as_object_mut().unwrap().remove("images");
                events.push(event);
            }
        }
        Ok(events)
    }
}
