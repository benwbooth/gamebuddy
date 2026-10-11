//! Keep the walkthrough position on where the player actually is. The
//! spatial matcher registers each capture against frames of the walkthrough
//! video; those moments map straight to steps through their video anchors.
//! When the screen matches a moment shortly ahead of the current step, move
//! there (passing, not completing, the steps between). If it only matches
//! earlier moments (a death sent the player back, or a detour), keep progress
//! and mark them off route so advice starts from where they really are.
//! Repeated scenery is why the window is forward-only and short.
use crate::Store;
use anyhow::Result;
use gamebuddy_core::{Atlas, Guide};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// How far ahead in the walkthrough video a match may move the route.
const AHEAD_MS: u64 = 10 * 60_000;
/// A match this far behind the current step means the player went back.
const BEHIND_MS: u64 = 60_000;
/// The new position must hold this long before the route follows it.
const HOLD_MS: u64 = 3_000;
/// Hypotheses within this score of the best are equally plausible.
const SCORE_SLACK: f64 = 0.05;
/// Equally plausible moments further apart than this mean the scenery
/// repeats (town streets, shop interiors), so it says nothing about where
/// the player is in the walkthrough.
const AMBIGUOUS_SPAN_MS: u64 = 3 * 60_000;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct OffRoute {
    /// The walkthrough step whose scenery the screen matches.
    pub seen_step_id: String,
    pub seen_step_title: String,
    /// The area map title, when the map match names one.
    pub map_title: String,
    pub route_step_id: String,
    pub route_step_title: String,
    pub since_ms: u64,
    /// The game capture session it was seen in; a new session starts over.
    #[serde(default)]
    pub session: String,
}

fn capture_session(store: &Store) -> String {
    crate::screen::status(store)
        .ok()
        .and_then(|s| s["session_id"].as_str().map(str::to_owned))
        .unwrap_or_default()
}

#[derive(Default, Serialize, Deserialize)]
struct Candidate {
    key: String,
    first_at: u64,
}

fn start_ms(guide: &Guide, step: &str) -> Option<u64> {
    let own = guide
        .step(step)?
        .anchors
        .iter()
        .filter_map(|a| a.start_ms)
        .min();
    own.or_else(|| {
        guide
            .steps
            .iter()
            .filter(|s| s.parent_id.as_deref() == Some(step))
            .filter_map(|s| start_ms(guide, &s.id))
            .min()
    })
}

fn covers(step: &gamebuddy_core::Step, ms: u64) -> bool {
    step.anchors
        .iter()
        .any(|a| a.start_ms.is_some_and(|s| s <= ms) && a.end_ms.is_some_and(|e| ms < e))
}

/// The area map title from a map registration, if it names one.
pub fn map_title(store: &Store, guide: &Guide, registration: &Value) -> Option<String> {
    let id = registration["map_id"].as_str()?;
    let atlas = super::atlas(store, guide).ok()?;
    Some(title(&atlas, id))
}

fn title(atlas: &Atlas, map: &str) -> String {
    atlas
        .maps
        .iter()
        .find(|m| m.id == map)
        .map(|m| m.title.clone())
        .unwrap_or_else(|| map.to_owned())
}

/// The walkthrough's own path from the step the screen shows back to the
/// route step: what a friend would walk the player through after a death.
/// Lasting changes already made (a raised ladder, a bought shield) stay done
/// and are left out; travel steps and anything whose effect a later action
/// can undo (a key that gets used up) remain.
pub fn return_path(store: &Store, guide: &Guide, off: &OffRoute) -> Result<Vec<String>> {
    if off.seen_step_id.is_empty() {
        return Ok(vec![]);
    }
    let model = crate::outcomes::prepared(store, guide)?
        .map(|p| p.model.clone())
        .unwrap_or_default();
    let lasting = |id: &str| {
        model.actions.iter().any(|rule| {
            rule.step_id == id
                && !rule.effects.is_empty()
                && rule.invalidate_facts.is_empty()
                && rule.effects.iter().all(|effect| {
                    effect.value
                        && !model
                            .actions
                            .iter()
                            .any(|other| other.invalidate_facts.contains(&effect.id))
                })
        })
    };
    let fresh = gamebuddy_core::Progress::default();
    let view = gamebuddy_core::outcomes::OutcomeModel::default().view(guide, &fresh);
    Ok(
        gamebuddy_core::route::authored_route(guide, &fresh, &view, &off.seen_step_id, 60)
            .step_ids
            .into_iter()
            .take_while(|id| *id != off.route_step_id)
            .filter(|id| !lasting(id))
            .collect(),
    )
}

/// The player said (or the screen shows) they are back in an earlier,
/// finished area, e.g. a checkpoint after a death: guide them back from
/// there, keeping progress.
pub fn report_return(store: &Store, guide: &Guide, seen_step: &str, area: &str) -> Result<()> {
    let progress = store.progress(guide)?;
    let current = progress.current_step.clone().unwrap_or_default();
    let off = OffRoute {
        seen_step_id: seen_step.into(),
        seen_step_title: guide
            .step(seen_step)
            .map(|s| s.title.clone())
            .unwrap_or_default(),
        map_title: area.into(),
        route_step_title: guide
            .step(&current)
            .map(|s| s.title.clone())
            .unwrap_or_default(),
        route_step_id: current,
        since_ms: crate::screen::now_ms(),
        session: capture_session(store),
    };
    store.put(&format!("off-route:{}", guide.id), &Some(off))
}

pub fn off_route(store: &Store, guide: &Guide) -> Result<Option<OffRoute>> {
    let session = capture_session(store);
    Ok(store
        .get::<Option<OffRoute>>(&format!("off-route:{}", guide.id))?
        .flatten()
        .filter(|o| o.session == session))
}

/// Video moments (ms) the current screen matches, with their scores.
pub fn video_matches(observation: &Value) -> Vec<(u64, f64)> {
    observation["tracker"]["hypotheses"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|h| {
            let ms = h["reference_id"]
                .as_str()?
                .rsplit('-')
                .next()?
                .parse()
                .ok()?;
            Some((ms, h["score"].as_f64()?))
        })
        .collect()
}

/// Follow the screen's matches against the walkthrough video.
pub fn observed(
    store: &Store,
    guide: &Guide,
    epoch: u64,
    matches: &[(u64, f64)],
    map_title: Option<&str>,
    at_ms: u64,
) -> Result<Option<Value>> {
    let progress = store.progress(guide)?;
    let (Some(current), Some(best)) = (
        progress.current_step.clone(),
        matches.iter().map(|m| m.1).reduce(f64::max),
    ) else {
        return Ok(None);
    };
    if progress.epoch != epoch {
        return Ok(None);
    }
    let Some(current_ms) = start_ms(guide, &current) else {
        return Ok(None);
    };
    let plausible: Vec<u64> = matches
        .iter()
        .filter(|m| m.1 >= best - SCORE_SLACK)
        .map(|m| m.0)
        .collect();
    let off_key = format!("off-route:{}", guide.id);
    let candidate_key = format!("area-candidate:{}", guide.id);
    let current_step = guide.step(&current);
    // Still at the current step's own moment: on route.
    if plausible.iter().any(|ms| {
        current_step.is_some_and(|s| covers(s, *ms))
            || (*ms + BEHIND_MS >= current_ms && *ms <= current_ms)
    }) {
        store.put(&off_key, &Option::<OffRoute>::None)?;
        store.put(&candidate_key, &Candidate::default())?;
        return Ok(None);
    }
    let span = plausible.iter().max().unwrap_or(&0) - plausible.iter().min().unwrap_or(&0);
    if span > AMBIGUOUS_SPAN_MS {
        store.put(&candidate_key, &Candidate::default())?;
        return Ok(None);
    }
    let view = gamebuddy_core::outcomes::OutcomeModel::default().view(guide, &progress);
    let route = gamebuddy_core::route::authored_route(guide, &progress, &view, &current, 120);
    let ahead = plausible
        .iter()
        .filter(|ms| **ms > current_ms && **ms <= current_ms + AHEAD_MS)
        .min()
        .copied();
    let target = ahead.and_then(|ms| {
        route
            .step_ids
            .iter()
            .position(|id| guide.step(id).is_some_and(|s| covers(s, ms)))
    });
    let behind = plausible.iter().all(|ms| ms + BEHIND_MS < current_ms);
    let key = match (target, behind) {
        (Some(index), _) => format!("ahead:{}", route.step_ids[index]),
        (None, true) => format!("behind:{}", plausible.iter().max().unwrap_or(&0)),
        _ => return Ok(None),
    };
    let candidate: Candidate = store.get(&candidate_key)?.unwrap_or_default();
    // Hold on the same kind of position, not the exact video moment.
    let same = candidate.key == key
        || (candidate.key.starts_with("behind:") && key.starts_with("behind:"));
    if !same {
        store.put(
            &candidate_key,
            &Candidate {
                key,
                first_at: at_ms,
            },
        )?;
        return Ok(None);
    }
    if at_ms.saturating_sub(candidate.first_at) < HOLD_MS {
        return Ok(None);
    }
    if let Some(index) = target {
        let mut steps = route.step_ids;
        let passed: Vec<String> = steps.drain(..index).collect();
        let arrival = steps.swap_remove(0);
        let progress = store.observed_arrival(
            guide,
            epoch,
            &arrival,
            &passed,
            map_title.unwrap_or_default(),
        )?;
        store.put(&off_key, &Option::<OffRoute>::None)?;
        store.put(&candidate_key, &Candidate::default())?;
        return Ok(Some(
            json!({"kind":"arrival","step_id":arrival,"passed":passed,"epoch":progress.epoch}),
        ));
    }
    let seen_ms = *plausible.iter().max().unwrap_or(&0);
    let seen = guide.steps.iter().rev().find(|s| {
        covers(s, seen_ms)
            && !guide
                .steps
                .iter()
                .any(|c| c.parent_id.as_deref() == Some(&s.id))
    });
    // The route only passed that step on an inferred jump: the jump was
    // wrong (look-alike scenery), so correct it rather than treat the player
    // as having gone back.
    if let Some(seen) = seen.filter(|s| progress.passed.contains(&s.id)) {
        let from = start_ms(guide, &seen.id).unwrap_or(seen_ms);
        let unpass: Vec<String> = progress
            .passed
            .iter()
            .filter(|id| start_ms(guide, id).is_some_and(|ms| ms >= from))
            .cloned()
            .collect();
        let progress = store.observed_return(guide, epoch, &seen.id, &unpass)?;
        store.put(&off_key, &Option::<OffRoute>::None)?;
        store.put(&candidate_key, &Candidate::default())?;
        return Ok(Some(
            json!({"kind":"corrected","step_id":seen.id,"unpassed":unpass,"epoch":progress.epoch}),
        ));
    }
    let off = OffRoute {
        seen_step_id: seen.map(|s| s.id.clone()).unwrap_or_default(),
        seen_step_title: seen.map(|s| s.title.clone()).unwrap_or_default(),
        map_title: map_title.unwrap_or_default().into(),
        route_step_title: current_step.map(|s| s.title.clone()).unwrap_or_default(),
        route_step_id: current,
        since_ms: at_ms,
        session: capture_session(store),
    };
    if off_route(store, guide)?.is_some_and(|o| o.seen_step_id == off.seen_step_id) {
        return Ok(None);
    }
    store.put(&off_key, &Some(off.clone()))?;
    Ok(Some(json!({"kind":"off_route","off_route":off})))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gamebuddy_core::{Anchor, Progress};

    fn fixture() -> (tempfile::TempDir, Store, Guide) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let mut guide = gamebuddy_core::import_markdown(
            "Test",
            "Route",
            "# Town\nBuy a key\n# Road\nWalk right\n# Cave\nOpen the door\n# Cave hall\nClimb",
        )
        .unwrap();
        let source = guide.sources[0].id.clone();
        for (i, step) in guide.steps.iter_mut().enumerate() {
            let start = i as u64 * 120_000;
            step.anchors = vec![Anchor {
                source_id: source.clone(),
                start_ms: Some(start),
                end_ms: Some(start + 120_000),
                section: None,
            }];
        }
        store.save_guide(&guide).unwrap();
        store.put("active_guide", &guide.id).unwrap();
        let progress = Progress {
            current_step: Some(guide.steps[1].id.clone()),
            ..Default::default()
        };
        store.save_progress(&guide, &progress, "fixture").unwrap();
        (dir, store, guide)
    }

    #[test]
    fn the_route_follows_a_held_match_ahead_and_a_return_is_off_route() -> Result<()> {
        let (_dir, store, guide) = fixture();
        let epoch = store.progress(&guide)?.epoch;
        // On the current step's moment: nothing changes.
        assert!(observed(&store, &guide, epoch, &[(130_000, 0.8)], None, 0)?.is_none());
        // Ahead (the cave): must hold before moving.
        let cave = [(250_000, 0.8), (900_000, 0.7)];
        assert!(observed(&store, &guide, epoch, &cave, None, 1_000)?.is_none());
        let moved = observed(&store, &guide, epoch, &cave, None, 5_000)?.unwrap();
        assert_eq!(moved["step_id"], guide.steps[2].id.as_str());
        let progress = store.progress(&guide)?;
        assert!(progress.passed.contains(&guide.steps[1].id) && progress.completed.is_empty());
        // Distinctly back at the step the jump passed: the jump was wrong.
        let road = [(130_000, 0.8)];
        observed(&store, &guide, progress.epoch, &road, None, 5_200)?;
        let fixed = observed(&store, &guide, progress.epoch, &road, None, 8_500)?.unwrap();
        assert_eq!(fixed["kind"], "corrected");
        let corrected = store.progress(&guide)?;
        assert_eq!(
            corrected.current_step.as_deref(),
            Some(guide.steps[1].id.as_str())
        );
        assert!(corrected.passed.is_empty());
        // Ahead again for the rest of the test.
        observed(&store, &guide, corrected.epoch, &cave, None, 8_600)?;
        observed(&store, &guide, corrected.epoch, &cave, None, 12_000)?;
        let progress = store.progress(&guide)?;
        assert_eq!(
            progress.current_step.as_deref(),
            Some(guide.steps[2].id.as_str())
        );
        // Scenery seen at distant moments (a look-alike town) moves nothing.
        let alike = [(20_000, 0.8), (500_000, 0.78)];
        observed(&store, &guide, progress.epoch, &alike, None, 5_500)?;
        assert!(observed(&store, &guide, progress.epoch, &alike, None, 9_000)?.is_none());
        assert!(off_route(&store, &guide)?.is_none());
        // Back in town after a death: keep progress, flag off route.
        let epoch = progress.epoch;
        let town = [(20_000, 0.8)];
        assert!(observed(&store, &guide, epoch, &town, Some("Town"), 6_000)?.is_none());
        let off = observed(&store, &guide, epoch, &town, Some("Town"), 10_000)?.unwrap();
        assert_eq!(off["kind"], "off_route");
        assert_eq!(store.progress(&guide)?.current_step, progress.current_step);
        assert_eq!(off_route(&store, &guide)?.unwrap().seen_step_title, "Town");
        let path = return_path(&store, &guide, &off_route(&store, &guide)?.unwrap())?;
        assert_eq!(
            path,
            vec![guide.steps[0].id.clone(), guide.steps[1].id.clone()]
        );
        // Back at the route's moment clears it.
        observed(&store, &guide, epoch, &[(250_000, 0.8)], None, 11_000)?;
        assert!(off_route(&store, &guide)?.is_none());
        // A far-ahead look-alike never jumps the route.
        let epoch = store.progress(&guide)?.epoch;
        observed(&store, &guide, epoch, &[(2_000_000, 0.9)], None, 12_000)?;
        observed(&store, &guide, epoch, &[(2_000_000, 0.9)], None, 20_000)?;
        assert_eq!(
            store.progress(&guide)?.current_step.as_deref(),
            Some(guide.steps[2].id.as_str())
        );
        Ok(())
    }
}
