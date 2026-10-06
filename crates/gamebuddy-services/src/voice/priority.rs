//! Foreground voice reserves interactive capacity. The short lease expires if
//! a session disconnects, panics, or stops polling, so background work recovers.
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};
static LEASES: OnceLock<Mutex<HashMap<PathBuf, Instant>>> = OnceLock::new();
pub fn update(root: &Path, active: bool) {
    let mut leases = LEASES.get_or_init(Default::default).lock().unwrap();
    let now = Instant::now();
    leases.retain(|_, until| *until > now);
    if active {
        leases.insert(root.into(), now + Duration::from_secs(2));
    } else {
        leases.remove(root);
    }
}
pub fn active(root: &Path) -> bool {
    LEASES.get().is_some_and(|l| {
        l.lock()
            .unwrap()
            .get(root)
            .is_some_and(|until| *until > Instant::now())
    })
}
