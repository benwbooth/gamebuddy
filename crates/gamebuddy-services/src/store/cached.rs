//! Large prepared reference indexes change rarely. Live capture writes must not
//! force tens of megabytes of JSON to be parsed again for every location update.
use super::*;
use std::{
    any::{Any, TypeId},
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock},
    time::Instant,
};

pub(super) fn initialize(connection: &Connection) -> Result<()> {
    connection.execute_batch("CREATE TABLE IF NOT EXISTS document_versions(key TEXT PRIMARY KEY, version INTEGER NOT NULL);
        CREATE TRIGGER IF NOT EXISTS document_cache_insert AFTER INSERT ON documents BEGIN
            INSERT INTO document_versions(key,version) VALUES(NEW.key,1) ON CONFLICT(key) DO UPDATE SET version=version+1;
        END;
        CREATE TRIGGER IF NOT EXISTS document_cache_update AFTER UPDATE ON documents BEGIN
            INSERT INTO document_versions(key,version) VALUES(NEW.key,1) ON CONFLICT(key) DO UPDATE SET version=version+1;
        END;
        CREATE TRIGGER IF NOT EXISTS document_cache_delete AFTER DELETE ON documents BEGIN
            INSERT INTO document_versions(key,version) VALUES(OLD.key,1) ON CONFLICT(key) DO UPDATE SET version=version+1;
        END;")?;
    Ok(())
}

struct Entry {
    version: i64,
    value: Arc<dyn Any + Send + Sync>,
    used: Instant,
}
type CacheKey = (PathBuf, String, TypeId);
static CACHE: OnceLock<Mutex<HashMap<CacheKey, Entry>>> = OnceLock::new();

impl Store {
    /// Shared, bounded cache for explicitly opted-in large immutable documents.
    /// Database triggers invalidate writes from any connection, including CAS,
    /// external tools, deletion/reinsertion, and other GameBuddy processes.
    pub fn get_cached<T: DeserializeOwned + Send + Sync + 'static>(
        &self,
        key: &str,
    ) -> Result<Option<Arc<T>>> {
        let version: Option<i64> = self.connection.query_row(
            "SELECT COALESCE(v.version,0) FROM documents d LEFT JOIN document_versions v ON d.key=v.key WHERE d.key=?1",
            [key], |row| row.get(0)).optional()?;
        let Some(version) = version else {
            return Ok(None);
        };
        let cache = CACHE.get_or_init(Default::default);
        let cache_key = (self.root.clone(), key.to_owned(), TypeId::of::<T>());
        {
            let mut cache = cache
                .lock()
                .map_err(|_| anyhow::anyhow!("Reference cache lock poisoned"))?;
            if let Some(entry) = cache.get_mut(&cache_key).filter(|e| e.version == version) {
                entry.used = Instant::now();
                return Ok(entry.value.clone().downcast::<T>().ok());
            }
        }
        // Read data and its generation from the same SQLite snapshot; a writer
        // racing the earlier version lookup cannot associate new data with it.
        let row: Option<(String, i64)> = self.connection.query_row(
            "SELECT d.body,COALESCE(v.version,0) FROM documents d LEFT JOIN document_versions v ON d.key=v.key WHERE d.key=?1",
            [key], |row| Ok((row.get(0)?, row.get(1)?))).optional()?;
        let Some((body, version)) = row else {
            return Ok(None);
        };
        let value = Arc::new(serde_json::from_str::<T>(&body)?);
        let mut cache = cache
            .lock()
            .map_err(|_| anyhow::anyhow!("Reference cache lock poisoned"))?;
        if cache.len() >= 4
            && !cache.contains_key(&cache_key)
            && let Some(oldest) = cache
                .iter()
                .min_by_key(|(_, e)| e.used)
                .map(|(k, _)| k.clone())
        {
            cache.remove(&oldest);
        }
        cache.insert(
            cache_key,
            Entry {
                version,
                value: value.clone(),
                used: Instant::now(),
            },
        );
        Ok(Some(value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cached_reference_survives_capture_writes_but_refreshes_external_changes() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let first = Store::open(dir.path())?;
        let second = Store::open(dir.path())?;
        first.put("index", &vec![1u32, 2])?;
        let old = first.get_cached::<Vec<u32>>("index")?.unwrap();
        second.put("live-screen", &"new capture")?;
        assert!(Arc::ptr_eq(&old, &second.get_cached("index")?.unwrap()));
        second
            .connection
            .execute("UPDATE documents SET body='[3,4]' WHERE key='index'", [])?;
        let new = first.get_cached::<Vec<u32>>("index")?.unwrap();
        assert_eq!(*new, [3, 4]);
        assert!(!Arc::ptr_eq(&old, &new));
        second
            .connection
            .execute("DELETE FROM documents WHERE key='index'", [])?;
        assert!(first.get_cached::<Vec<u32>>("index")?.is_none());
        second.put_if_absent("index", &vec![5u32])?;
        assert_eq!(*first.get_cached::<Vec<u32>>("index")?.unwrap(), [5]);
        Ok(())
    }
}
