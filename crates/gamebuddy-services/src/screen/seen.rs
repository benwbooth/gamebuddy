//! Keep the exact cropped pixels used by each reader, not a newer capture.
use super::{Snapshot, Ticket, now_ms};
use crate::Store;
use anyhow::Result;
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

#[derive(Serialize, Deserialize)]
struct Seen {
    ticket: Ticket,
    captured_at_ms: u64,
    shared_at_ms: u64,
    file: String,
}
impl Seen {
    fn image_id(&self, reader: &str) -> String {
        format!(
            "seen:{reader}:{}:{}:{}",
            self.ticket.session, self.ticket.epoch, self.file
        )
    }
}

pub fn record(store: &Store, snapshot: &Snapshot, reader: &str) -> Result<()> {
    static WRITES: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = WRITES.lock().unwrap_or_else(|error| error.into_inner());
    anyhow::ensure!(
        matches!(reader, "assistant" | "observer" | "live"),
        "Unknown screen reader"
    );
    let key = format!("seen-screen:{reader}");
    let previous: Option<Seen> = store.get(&key)?;
    if previous
        .as_ref()
        .is_some_and(|p| p.ticket == snapshot.ticket && p.captured_at_ms > snapshot.captured_at_ms)
    {
        return Ok(());
    }
    let bytes = STANDARD.decode(&snapshot.jpeg)?;
    let file = format!("{reader}-{:x}.jpg", Sha256::digest(&bytes));
    let directory = store.root.join("seen-screens");
    std::fs::create_dir_all(&directory)?;
    let path = directory.join(&file);
    if !path.exists() {
        use std::io::Write;
        let mut temporary = tempfile::NamedTempFile::new_in(&directory)?;
        temporary.write_all(&bytes)?;
        temporary.persist(&path)?;
    }
    store.put(
        &key,
        &Seen {
            ticket: snapshot.ticket.clone(),
            captured_at_ms: snapshot.captured_at_ms,
            shared_at_ms: now_ms(),
            file: file.clone(),
        },
    )?;
    if let Some(previous) = previous.filter(|p| p.file != file) {
        // Only remove files in our private namespace, never a stored arbitrary path.
        if !previous.file.contains(['/', '\\']) && previous.file.starts_with(reader) {
            let _ = std::fs::remove_file(directory.join(previous.file));
        }
    }
    Ok(())
}

pub fn context(store: &Store, screen: &Value) -> Result<Value> {
    let mut result = json!({"status":screen["status"]});
    for reader in ["assistant", "observer", "live"] {
        let Some(seen) = store.get::<Seen>(&format!("seen-screen:{reader}"))? else {
            continue;
        };
        // Pause, recrop, restart and game changes revoke old pictures just as
        // they revoke visual evidence. Never silently display another session.
        if screen["session_id"] != seen.ticket.session
            || screen["capture_epoch"] != seen.ticket.epoch
            || screen["game"] != seen.ticket.game
        {
            continue;
        }
        result[reader] = json!({"image_id":seen.image_id(reader),"captured_at_ms":seen.captured_at_ms,"shared_at_ms":seen.shared_at_ms});
    }
    Ok(result)
}

/// The WebEngine page cannot read arbitrary local files. Serve only a current,
/// exact image identity through the existing native image bridge.
pub fn image(store: &Store, id: &str) -> Result<String> {
    let current = context(store, &super::status(store)?)?;
    for reader in ["assistant", "observer", "live"] {
        if current[reader]["image_id"] == id {
            let seen: Seen = store
                .get(&format!("seen-screen:{reader}"))?
                .ok_or_else(|| anyhow::anyhow!("Screen unavailable"))?;
            anyhow::ensure!(
                seen.image_id(reader) == id && !seen.file.contains(['/', '\\']),
                "Screen changed"
            );
            return Ok(format!(
                "data:image/jpeg;base64,{}",
                STANDARD.encode(std::fs::read(
                    store.root.join("seen-screens").join(seen.file)
                )?)
            ));
        }
    }
    anyhow::bail!("This shared screenshot is no longer current")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preview_preserves_consumed_pixels_and_revokes_old_capture_tickets() -> Result<()> {
        let root = tempfile::tempdir()?;
        let store = Store::open(root.path())?;
        let ticket = super::super::start(&store, "Game", true)?;
        let path = root.path().join("frame.png");
        image::RgbImage::from_pixel(60, 40, image::Rgb([200, 30, 70])).save(&path)?;
        super::super::publish(&store, &ticket, &path, "test")?;
        let first = super::super::snapshot(&store)?;
        record(&store, &first, "assistant")?;
        image::RgbImage::from_pixel(60, 40, image::Rgb([20, 210, 30])).save(&path)?;
        super::super::publish(&store, &ticket, &path, "test")?;
        let current = context(&store, &super::super::status(&store)?)?;
        let id = current["assistant"]["image_id"].as_str().unwrap();
        assert_eq!(
            image(&store, id)?,
            format!("data:image/jpeg;base64,{}", first.jpeg)
        );
        super::super::start(&store, "Other game", true)?;
        assert!(context(&store, &super::super::status(&store)?)?["assistant"].is_null());
        assert!(image(&store, id).is_err());
        assert!(image(&store, "../../gamebuddy.sqlite").is_err());
        Ok(())
    }
}
