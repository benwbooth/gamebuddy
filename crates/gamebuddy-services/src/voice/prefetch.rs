//! The answer to "what now?", worked out before the player asks.
//!
//! Whenever the objective or the place changes, the reasoning model answers
//! "where should I go and what do I need?" in the background, looking at the
//! same context and screenshot a live question would get. When the player
//! then asks, the answer is spoken at once instead of after a 3 to 30 second
//! model call, as long as nothing has changed since: same plan state, same
//! objective, same located place, and the answer is under two minutes old.
use crate::{ModelRequest, ReasoningProvider, Store, screen};
use anyhow::{Context, Result};
use gamebuddy_core::Guide;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::atomic::{AtomicBool, Ordering};

/// Older answers are not trusted even if nothing seems to have changed.
const MAX_AGE_MS: u64 = 120_000;
/// Do not recompute more often than this, however often state flickers.
const MIN_GAP_MS: u64 = 20_000;

#[derive(Serialize, Deserialize, Clone)]
struct Prefetched {
    state: String,
    signature: Vec<u8>,
    text: String,
    step_id: Option<String>,
    at_ms: u64,
}

fn key(guide: &Guide) -> String {
    format!("voice-prefetch:{}", guide.id)
}

/// What the answer depends on: plan epoch, current objective, located place.
fn state(store: &Store, guide: &Guide) -> Result<(String, Option<String>)> {
    let progress = store.progress(guide)?;
    let nav = crate::navigation::context(store, guide, &progress, &Default::default())?;
    let step = nav["guidance"]["now"]["id"].as_str().map(str::to_owned);
    let place = nav["location"]["place_id"].as_str().unwrap_or_default();
    Ok((
        format!(
            "{}|{}|{}",
            progress.epoch,
            step.as_deref().unwrap_or_default(),
            place
        ),
        step,
    ))
}

/// Work out the answer now if the stored one no longer fits. Returns true
/// when a new answer was stored.
pub fn refresh(store: &Store, guide: &Guide) -> Result<bool> {
    if claude_login(store) {
        return Ok(false);
    }
    // On this computer's GPU the voice answers live and fast; this big
    // background request only slowed the game (and overflows small models).
    if store
        .get::<crate::Settings>("settings")?
        .is_some_and(|s| crate::providers::on_this_computer(&s.reasoning))
    {
        return Ok(false);
    }
    let (current, step_id) = state(store, guide)?;
    let now = screen::now_ms();
    if let Some(saved) = store.get::<Prefetched>(&key(guide))?
        && (saved.state == current && now.saturating_sub(saved.at_ms) < MAX_AGE_MS / 2
            || now.saturating_sub(saved.at_ms) < MIN_GAP_MS)
    {
        return Ok(false);
    }
    let Some(frame) = screen::movement_frame(store, now)? else {
        return Ok(false);
    };
    let question = "Where should I go now, and is there anything I need to get first?";
    let evidence = crate::harness::context_for_request(store, question)?;
    let settings = store
        .get::<crate::Settings>("settings")?
        .unwrap_or_default();
    let provider = crate::HttpProvider::new(settings.reasoning)?;
    let reply = provider.complete(ModelRequest {
        system: format!(
            "{}\nAnswer from the supplied game context and screenshot only; no tools are available in this turn. Reply with JSON {{\"answer\": the spoken reply}}.",
            super::prompts::REASONER
        ),
        prompt: format!(
            "Game context: {}\nThe player asks: {question}",
            serde_json::to_string(&evidence.value)?
        ),
        images: evidence.images.clone(),
        json: true,
    })?;
    let answer = crate::parse_json::<Value>(&reply)?
        .get("answer")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|a| !a.is_empty())
        .context("No answer")?
        .to_owned();
    // State may have moved while the model worked; store against what it saw.
    store.put(
        &key(guide),
        &Prefetched {
            state: current,
            signature: frame.frame_signature,
            text: answer,
            step_id,
            at_ms: now,
        },
    )?;
    crate::flight::event("prefetch", json!({"guide": guide.id}));
    Ok(true)
}

/// Refresh in the background, one at a time.
fn claude_login(store: &Store) -> bool {
    store
        .get::<crate::Settings>("settings")
        .ok()
        .flatten()
        .is_some_and(|s| s.reasoning.kind == crate::ProviderKind::ClaudeCode)
}

pub fn schedule(root: std::path::PathBuf, guide: Guide) {
    static RUNNING: AtomicBool = AtomicBool::new(false);
    if RUNNING.swap(true, Ordering::AcqRel) {
        return;
    }
    std::thread::spawn(move || {
        if let Err(error) = Store::open(root).and_then(|store| refresh(&store, &guide)) {
            eprintln!("GameBuddy prefetch: {error:#}");
        }
        RUNNING.store(false, Ordering::Release);
    });
}

/// The prepared answer for a plain "what now?" question, if it still fits.
pub fn reply(
    store: &Store,
    game: &str,
    request: &str,
) -> Result<Option<super::prepared::Response>> {
    // Claude sees the screen when asked, so a prepared answer could only be
    // older than what it would say.
    if claude_login(store) {
        return Ok(None);
    }
    if super::prepared::intent(request) != Some("next") {
        return Ok(None);
    }
    let Some(id) = store.get::<String>("active_guide")? else {
        return Ok(None);
    };
    let guide = store.guide(&id)?;
    if !guide.game.eq_ignore_ascii_case(game) {
        return Ok(None);
    }
    let Some(saved) = store.get::<Prefetched>(&key(&guide))? else {
        return Ok(None);
    };
    let now = screen::now_ms();
    // A live capture must still be running: the answer was made from it.
    if screen::movement_frame(store, now)?.is_none() {
        return Ok(None);
    }
    let (current, _) = state(store, &guide)?;
    let fits = saved.state == current && now.saturating_sub(saved.at_ms) < MAX_AGE_MS;
    Ok(fits.then(|| super::prepared::Response {
        text: saved.text,
        kind: "prefetched".into(),
        step_id: saved.step_id,
    }))
}
