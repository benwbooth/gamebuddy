//! One queue for every model call in the process, so background work can
//! never starve or rate-limit the conversation.
//!
//! Calls come in three priorities. Live calls (the player is waiting for an
//! answer) take any free slot and always have one held back for them.
//! Watching calls (screen reads, background coaching) use the rest when no
//! live call is waiting. Background calls (guide preparation) use at most
//! two slots, wait behind both, and pause for a minute after the provider
//! says "too many requests".
use crate::ProviderKind;
use std::{
    collections::HashMap,
    sync::{Condvar, Mutex, OnceLock},
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Priority {
    Live,
    Watching,
    Background,
}

#[derive(Default)]
struct Lane {
    in_flight: usize,
    background: usize,
    live_waiting: usize,
    cooldown_until: Option<Instant>,
    /// The account is out of quota: nothing but the player's own questions
    /// goes to this provider until then.
    quota_until: Option<Instant>,
}

struct Queue {
    lanes: Mutex<HashMap<String, Lane>>,
    changed: Condvar,
}

fn queue() -> &'static Queue {
    static QUEUE: OnceLock<Queue> = OnceLock::new();
    QUEUE.get_or_init(|| Queue {
        lanes: Mutex::new(HashMap::new()),
        changed: Condvar::new(),
    })
}

/// How many calls a provider runs at once. A local model shares one GPU.
fn slots(kind: &ProviderKind) -> usize {
    match kind {
        ProviderKind::Ollama => 1,
        ProviderKind::ClaudeCode => 3,
        _ => 4,
    }
}

const BACKGROUND_SLOTS: usize = 2;
const COOLDOWN: Duration = Duration::from_secs(60);
const QUOTA_COOLDOWN: Duration = Duration::from_secs(10 * 60);

/// A held slot; dropping it frees the slot for the next caller.
pub struct Permit {
    key: String,
    priority: Priority,
    /// How long the call waited for its slot.
    pub waited: Duration,
}

impl Drop for Permit {
    fn drop(&mut self) {
        let q = queue();
        let mut lanes = q.lanes.lock().unwrap();
        if let Some(lane) = lanes.get_mut(&self.key) {
            lane.in_flight -= 1;
            if self.priority == Priority::Background {
                lane.background -= 1;
            }
        }
        q.changed.notify_all();
    }
}

/// Wait for a slot on this provider at this priority.
pub fn acquire(kind: &ProviderKind, priority: Priority) -> Permit {
    let key = format!("{kind:?}");
    let total = slots(kind);
    let q = queue();
    let started = Instant::now();
    let mut lanes = q.lanes.lock().unwrap();
    if priority == Priority::Live {
        lanes.entry(key.clone()).or_default().live_waiting += 1;
    }
    loop {
        let lane = lanes.entry(key.clone()).or_default();
        let free = match priority {
            Priority::Live => lane.in_flight < total,
            // One slot stays free for the player's next question.
            Priority::Watching => {
                lane.in_flight + 1 < total.max(2)
                    && lane.live_waiting == 0
                    && lane.quota_until.is_none_or(|t| Instant::now() >= t)
            }
            Priority::Background => {
                lane.in_flight + 1 < total.max(2)
                    && lane.live_waiting == 0
                    && lane.background < BACKGROUND_SLOTS
                    && lane.cooldown_until.is_none_or(|t| Instant::now() >= t)
                    && lane.quota_until.is_none_or(|t| Instant::now() >= t)
            }
        };
        if free {
            lane.in_flight += 1;
            if priority == Priority::Background {
                lane.background += 1;
            }
            if priority == Priority::Live {
                lane.live_waiting -= 1;
            }
            break;
        }
        // Re-check at least every second so a cooldown can expire.
        lanes = q
            .changed
            .wait_timeout(lanes, Duration::from_secs(1))
            .unwrap()
            .0;
    }
    Permit {
        key,
        priority,
        waited: started.elapsed(),
    }
}

/// The account has no quota left: hold everything but live questions for a
/// while, instead of failing hundreds of times until the plan resets.
pub fn quota_exhausted(kind: &ProviderKind) {
    let q = queue();
    let mut lanes = q.lanes.lock().unwrap();
    lanes.entry(format!("{kind:?}")).or_default().quota_until =
        Some(Instant::now() + QUOTA_COOLDOWN);
    q.changed.notify_all();
}

/// The provider asked us to slow down: hold background work for a minute.
pub fn rate_limited(kind: &ProviderKind) {
    let q = queue();
    let mut lanes = q.lanes.lock().unwrap();
    lanes.entry(format!("{kind:?}")).or_default().cooldown_until = Some(Instant::now() + COOLDOWN);
    q.changed.notify_all();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn background_never_takes_the_last_slot_and_yields_to_live() {
        let kind = ProviderKind::Anthropic;
        let a = acquire(&kind, Priority::Background);
        let b = acquire(&kind, Priority::Background);
        let c = acquire(&kind, Priority::Watching);
        // Four slots: three used, the last is held for live.
        let live = acquire(&kind, Priority::Live);
        assert_eq!(live.waited < Duration::from_millis(500), true);
        drop((a, b, c, live));
    }
}
