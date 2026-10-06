//! One persisted recommendation shared by conversation, the outline and Now card.
use crate::{Store, screen};
use anyhow::{Result, ensure};
use gamebuddy_core::navigation::direction::{ScreenDirection, movement};
use gamebuddy_core::{Guide, PlaceTracker, Progress, SyncEstimate};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NextAction {
    pub expected_revision: u64,
    pub epoch: u64,
    pub step_id: String,
    /// One immediate action. For movement, include the same supported direction
    /// in screen_direction. Never guess a direction or add it to non-movement.
    pub instructions: String,
    /// Immediate screen-relative movement supported by the latest screenshot
    /// and source route, even when instructions use natural wording such as
    /// "Enter the door beside you". Use null for dialogue, purchases, uncertain
    /// routes or a direction inferred only from map orientation.
    #[serde(default)]
    pub screen_direction: Option<ScreenDirection>,
}
#[derive(Serialize, Deserialize)]
pub struct Recommendation {
    pub revision: u64,
    pub epoch: u64,
    pub session: String,
    pub updated_at_ms: u64,
    pub step_id: String,
    pub instructions: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location_evidence: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub movement: Option<ObservedMovement>,
}

#[derive(Serialize, Deserialize)]
pub struct ObservedMovement {
    pub direction: ScreenDirection,
    pub ticket: screen::Ticket,
    pub captured_at_ms: u64,
    pub frame_signature: Vec<u8>,
}

pub(crate) fn public(recommendation: &Recommendation) -> Value {
    let mut value = json!(recommendation);
    if let Some(movement) = value["movement"].as_object_mut() {
        // Capture signatures are local freshness checks, not useful model input.
        movement.remove("frame_signature");
        movement.remove("ticket");
    }
    value
}

// Walkthrough ordering is not proof that the player is still behind a gate.
// A current, accepted source match can support advice at the present location
// without claiming that every earlier battle, purchase or side task was done.
pub(crate) fn supports_action(guide: &Guide, step_id: &str, spatial: &Value) -> bool {
    let observation = &spatial["observation"];
    if observation["tracker"]["status"] != "tracked" {
        return false;
    }
    let mut at = Some(step_id);
    while let Some(id) = at {
        if observation["related_step_ids"]
            .as_array()
            .is_some_and(|ids| ids.iter().any(|v| v.as_str() == Some(id)))
        {
            return true;
        }
        at = guide.step(id).and_then(|s| s.parent_id.as_deref());
    }
    false
}

pub(crate) fn apply_location_advice(
    guide: &Guide,
    recommendation: &Recommendation,
    guidance: &mut gamebuddy_core::Guidance,
) {
    if recommendation.location_evidence.is_some()
        && guidance
            .now
            .as_ref()
            .is_none_or(|step| step.id != recommendation.step_id)
        && let Some(step) = guide.step(&recommendation.step_id)
    {
        guidance.now = Some(step.clone());
        guidance.target = Some(step.clone());
        guidance.next.clear();
        guidance.status = "ready".into();
        guidance.reason = "Advice for the currently observed area. Earlier walkthrough prerequisites remain unverified; they do not send you back to an area you have already left.".into();
    }
}

/// The accepted action is a route cursor, even when an older temporary goal
/// still exists. Resolving its world requirements always precedes continuation.
pub(crate) fn route_guidance(
    guide: &Guide,
    progress: &Progress,
    location: &PlaceTracker,
    accepted: Option<&str>,
    view: &gamebuddy_core::outcomes::PlanningView,
) -> gamebuddy_core::Guidance {
    match accepted {
        Some(id) => gamebuddy_core::guidance_for_action(guide, progress, location, id, view),
        None => gamebuddy_core::guidance_with_view(guide, progress, location, None, view),
    }
}

/// Scenery may establish arrival in a different area, but cannot distinguish
/// two visits or skip tasks within the same room. Explicit task reports can.
pub(crate) fn can_reanchor(
    guide: &Guide,
    progress: &Progress,
    atlas: &gamebuddy_core::Atlas,
    location: &PlaceTracker,
    expected: Option<&str>,
    candidate: &str,
    view: &gamebuddy_core::outcomes::PlanningView,
) -> bool {
    if location.status == "reported" && location.reference_step_id.as_deref() == Some(candidate) {
        return true;
    }
    if progress.current_step.is_none() {
        return true;
    }
    let different_place = expected
        .and_then(|id| atlas.place_for_step(guide, id))
        .zip(atlas.place_for_step(guide, candidate))
        .is_some_and(|(a, b)| a.id != b.id);
    if !different_place {
        return false;
    }
    let route = gamebuddy_core::route::authored_route(
        guide,
        progress,
        view,
        expected.unwrap(),
        guide.steps.len(),
    );
    if route.step_ids.iter().any(|id| id == candidate) {
        let place = atlas.place_for_step(guide, candidate).unwrap();
        // A newly identified room still doesn't select its later return visit.
        return route
            .step_ids
            .iter()
            .find(|id| {
                atlas
                    .place_for_step(guide, id)
                    .is_some_and(|p| p.id == place.id)
            })
            .is_some_and(|id| id == candidate);
    }
    true
}

fn key(guide: &Guide) -> String {
    format!("next-action:{}", guide.id)
}

/// A just-completed recommendation is still the route position. Its wording
/// must expire, but dropping its position revives an unrelated old checklist.
pub(crate) fn completed_cursor(
    store: &Store,
    guide: &Guide,
    progress: &Progress,
) -> Result<Option<String>> {
    Ok(store
        .get::<Recommendation>(&key(guide))?
        .filter(|r| {
            r.revision == guide.revision
                && r.epoch.saturating_add(1) == progress.epoch
                && gamebuddy_core::step_complete(guide, progress, &r.step_id)
        })
        .map(|r| r.step_id))
}

pub(crate) fn after_completion(
    store: &Store,
    guide: &Guide,
    progress: &mut Progress,
    completed: &str,
) -> Result<()> {
    let session = screen::status(store)?["session_id"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    if let Some(mut action) = store.get::<Recommendation>(&key(guide))?.filter(|r| {
        r.revision == guide.revision && r.epoch == progress.epoch && r.session == session
    }) {
        if gamebuddy_core::step_complete(guide, progress, &action.step_id) {
            progress.current_step = Some(action.step_id);
        } else {
            // Reporting an earlier task does not throw away current advice.
            action.epoch += 1;
            store.put(&key(guide), &action)?;
        }
    } else if progress
        .current_step
        .as_deref()
        .is_none_or(|id| gamebuddy_core::step_complete(guide, progress, id))
    {
        progress.current_step = Some(completed.into());
    }
    Ok(())
}

pub(crate) fn retain_route(
    store: &Store,
    guide: &Guide,
    progress: &mut Progress,
    keep_recommendation: bool,
) -> Result<()> {
    let session = screen::status(store)?["session_id"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    if let Some(mut action) = current(store, guide, progress, &session)? {
        progress.current_step = Some(action.step_id.clone());
        if keep_recommendation {
            action.epoch += 1;
            action.movement = None;
            store.put(&key(guide), &action)?;
        }
    }
    Ok(())
}
pub fn current(
    store: &Store,
    guide: &Guide,
    progress: &Progress,
    session: &str,
) -> Result<Option<Recommendation>> {
    Ok(store.get::<Recommendation>(&key(guide))?.filter(|r| {
        r.revision == guide.revision
            && r.epoch == progress.epoch
            && r.session == session
            && guide
                .step(&r.step_id)
                .is_some_and(|s| guide.step_enabled(s))
            && !gamebuddy_core::step_complete(guide, progress, &r.step_id)
    }))
}
pub fn publish(store: &Store, guide: &Guide, action: NextAction) -> Result<Value> {
    ensure!(
        action.expected_revision == guide.revision,
        "Outline changed; read get_game_context"
    );
    ensure!(
        !action.instructions.trim().is_empty() && action.instructions.len() <= 1600,
        "Give a short next instruction"
    );
    ensure!(
        gamebuddy_core::player_outline(guide)
            .iter()
            .any(|s| s.id == action.step_id)
            && !guide
                .steps
                .iter()
                .any(|s| s.parent_id.as_deref() == Some(&action.step_id)),
        "Choose an enabled player action, not a chapter"
    );
    let progress = store.progress(guide)?;
    ensure!(
        progress.epoch == action.epoch
            && !gamebuddy_core::step_complete(guide, &progress, &action.step_id),
        "Progress changed or task already complete. Current progress epoch is {}; read get_game_context before retrying.",
        progress.epoch
    );
    let session = screen::status(store)?["session_id"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let accepted = current(store, guide, &progress, &session)?;
    let existing = accepted.as_ref().filter(|r| r.step_id == action.step_id);
    // Refining the accepted current objective does not re-open old gates when
    // the player scrolls beyond the frame that originally supported it.
    let prepared = crate::outcomes::prepared(store, guide)?;
    let fallback = gamebuddy_core::outcomes::OutcomeModel::default();
    let model = prepared.as_ref().map_or(&fallback, |p| &p.model);
    let view = model.view(guide, &progress);
    ensure!(
        !view.goal_complete,
        "The selected outcome is already satisfied. Ask about optional goals before publishing more work."
    );
    ensure!(
        !view.skipped.contains_key(&action.step_id),
        "This action is not needed for the current goal: {}. Use the updated guidance or ask whether the player wants its optional rewards.",
        view.skipped
            .get(&action.step_id)
            .map(String::as_str)
            .unwrap_or_default()
    );
    ensure!(
        !view.blocked.contains_key(&action.step_id),
        "This action has unsatisfied world requirements. Resolve the missing fact before giving directions."
    );
    ensure!(
        view.requirements
            .get(&action.step_id)
            .is_none_or(Vec::is_empty),
        "This action needs a source-established outcome first: {}. Being at its location does not establish possession. Resolve these requirements before giving this instruction.",
        serde_json::to_string(&view.requirements.get(&action.step_id))?
    );
    let guidance = gamebuddy_core::guidance_for_action(
        guide,
        &progress,
        &PlaceTracker::default(),
        &action.step_id,
        &view,
    );
    let spatial = crate::spatial::context(store, guide)?;
    let reported = store
        .get::<PlaceTracker>(&super::tracker_key(guide))?
        .filter(|tracker| {
            tracker.status == "reported"
                && tracker.session_id == session
                && tracker.epoch == progress.epoch
        });
    let reported_place = if let Some(tracker) = &reported {
        super::atlas(store, guide)?
            .place_for_step(guide, &action.step_id)
            .filter(|p| Some(&p.id) == tracker.place_id.as_ref())
            .cloned()
    } else {
        None
    };
    let location_supported = supports_action(guide, &action.step_id, &spatial);
    let tracker = reported.clone().unwrap_or_default();
    let completed = completed_cursor(store, guide, &progress)?;
    let mut route = route_guidance(
        guide,
        &progress,
        &tracker,
        accepted
            .as_ref()
            .map(|r| r.step_id.as_str())
            .or(completed.as_deref()),
        &view,
    );
    if let Some(previous) = &accepted
        && view.requirements_met(&previous.step_id)
        && !view.skipped.contains_key(&previous.step_id)
    {
        apply_location_advice(guide, previous, &mut route);
    }
    let expected = route.now.as_ref().map(|s| s.id.as_str());
    let mut route_progress = progress.clone();
    if let Some(accepted) = &accepted {
        route_progress.current_step = Some(accepted.step_id.clone());
    }
    let reanchor = (reported_place.is_some() || location_supported)
        && can_reanchor(
            guide,
            &route_progress,
            &super::atlas(store, guide)?,
            &tracker,
            expected,
            &action.step_id,
            &view,
        );
    let branch_choice =
        route.status == "route_choice" && route.next.iter().any(|s| s.id == action.step_id);
    ensure!(
        expected == Some(action.step_id.as_str())
            || reanchor
            || existing.is_some()
            || branch_choice,
        "Follow the authored walkthrough route. The next action is {} ({}); choosing {} would leave that route unfinished. A matching room does not establish the visit or complete intervening pickups. Use confirmed progress, an explicit current task report, or a source-supported conditional bypass before changing the route.",
        route
            .now
            .as_ref()
            .map(|s| s.title.as_str())
            .unwrap_or("unresolved"),
        expected.unwrap_or("no supported next step"),
        action.step_id
    );
    ensure!(
        location_supported
            || reported_place.is_some()
            || existing.is_some()
            || guidance
                .now
                .as_ref()
                .is_some_and(|s| s.id == action.step_id),
        "This action has unconfirmed prerequisites and no current source match supporting its location: {}. Do not infer the player's location from this checklist. Inspect the current screen and location_actions; choose an action supported there. Record explicit completion reports, and ask about a prerequisite only when it is actually needed for the current action. Current progress epoch is {}.",
        serde_json::to_string(&guidance.requirements)?,
        progress.epoch
    );
    let movement = action
        .screen_direction
        .map(|direction| -> Result<_> {
            use base64::Engine;
            let frame = screen::snapshot(store)?;
            let image = image::load_from_memory(
                &base64::engine::general_purpose::STANDARD.decode(&frame.jpeg)?,
            )?;
            if let Some(parsed) = movement(&action.instructions) {
                ensure!(
                    parsed == direction,
                    "Screen direction contradicts the movement instruction"
                );
            }
            Ok(ObservedMovement {
                direction,
                ticket: frame.ticket,
                captured_at_ms: frame.captured_at_ms,
                frame_signature: screen::frame_signature(&image),
            })
        })
        .transpose()?;
    let recommendation = Recommendation {
        revision: guide.revision,
        epoch: progress.epoch,
        session,
        updated_at_ms: screen::now_ms(),
        step_id: action.step_id,
        instructions: action.instructions.trim().into(),
        movement,
        location_evidence: reported_place.map(|place|json!({"kind":"player_report_not_completion","place_id":place.id,"reported_at_ms":reported.as_ref().map(|t|t.observed_at_ms)})).or_else(||location_supported
            .then(|| {
                json!({
                    "kind":"current_source_match_not_completion",
                    "room_id":spatial["observation"]["tracker"]["room_id"],
                    "observed_at_ms":spatial["observation"]["tracker"]["observed_at_ms"],
                    "related_step_ids":spatial["observation"]["related_step_ids"]
                })
            })
            .or_else(|| existing.and_then(|r| r.location_evidence.clone()))),
    };
    store.put_for_progress(guide, progress.epoch, &key(guide), &recommendation)?;
    Ok(
        json!({"recommendation":public(&recommendation),"navigation":super::context(store, guide, &progress, &SyncEstimate::default())?,"completion_changed":false}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn arrival_selects_the_first_unfinished_visit_to_that_room() -> Result<()> {
        use gamebuddy_core::{Atlas, Place, PlaceBinding};
        let dir = tempfile::tempdir()?;
        let store = Store::open(dir.path())?;
        let guide = gamebuddy_core::import_markdown(
            "Game",
            "Route",
            "# Gate\nGo through.\n# First visit\nReceive the seal.\n# Forest\nCross the forest.\n# Return visit\nUnlock the chamber again.",
        )?;
        store.save_guide(&guide)?;
        let atlas = Atlas {
            guide_id: guide.id.clone(),
            guide_revision: guide.revision,
            places: ["gate", "chamber", "forest"]
                .iter()
                .map(|id| Place {
                    id: (*id).into(),
                    label: (*id).into(),
                    map_id: None,
                    landmarks: (*id).into(),
                })
                .collect(),
            bindings: ["gate", "chamber", "forest", "chamber"]
                .iter()
                .zip(&guide.steps)
                .map(|(place, step)| PlaceBinding {
                    step_id: step.id.clone(),
                    place_id: (*place).into(),
                    evidence: "Source room identity".into(),
                })
                .collect(),
            ..Default::default()
        };
        super::super::save(&store, &guide, &atlas)?;
        store.save_progress(
            &guide,
            &Progress {
                current_step: Some("step-1".into()),
                ..Default::default()
            },
            "fixture",
        )?;
        let report = super::super::report_location(
            &store,
            &guide,
            super::super::LocationReport {
                place_id: "chamber".into(),
                step_id: None,
                player_report: "I am in the chamber now.".into(),
                expected_revision: guide.revision,
                epoch: 0,
            },
        )?;
        let epoch = report["progress"]["epoch"].as_u64().unwrap();
        let action = |id: &str| NextAction {
            expected_revision: guide.revision,
            epoch,
            step_id: id.into(),
            instructions: "Follow this source action.".into(),
            screen_direction: None,
        };
        assert!(
            publish(&store, &guide, action("step-4")).is_err(),
            "Room identity must not select the return visit"
        );
        assert!(
            publish(&store, &guide, action("step-2")).is_ok(),
            "Arrival can bypass old travel without inventing completion"
        );
        assert!(store.progress(&guide)?.reported_completed.is_empty());
        Ok(())
    }

    #[test]
    fn planning_updates_keep_the_accepted_route_instead_of_reviving_an_old_goal() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let store = Store::open(dir.path())?;
        let mut guide = gamebuddy_core::import_markdown(
            "Game",
            "Route",
            "# Old goal\nCheck the old gate.\n# Current task\nReceive the seal.\n# Next task\nLeave the room.",
        )?;
        guide.goals.push(gamebuddy_core::PlayerGoal {
            id: "old".into(),
            description: "Check the old gate".into(),
            step_ids: vec!["step-1".into()],
        });
        store.save_guide(&guide)?;
        store.put(
            &key(&guide),
            &Recommendation {
                revision: guide.revision,
                epoch: 0,
                session: String::new(),
                updated_at_ms: 1,
                step_id: "step-2".into(),
                instructions: "Receive the seal.".into(),
                location_evidence: None,
                movement: None,
            },
        )?;
        let result = crate::outcomes::report(
            &store,
            &guide,
            crate::outcomes::Report {
                expected_revision: guide.revision,
                epoch: 0,
                player_report: "Keep optional extras off.".into(),
                facts: vec![],
                include_optional: Some(false),
                goal: None,
                clear_goal: false,
            },
        )?;
        assert_eq!(result["guidance"]["now"]["id"], "step-2");
        assert_eq!(result["authored_route"]["steps"][1]["id"], "step-3");
        assert_eq!(result["progress"]["current_step"], "step-2");
        assert!(store.progress(&guide)?.completed.is_empty());
        assert_eq!(store.guide(&guide.id)?.goals, guide.goals);
        let progress = store.report_complete(&guide, 1, "step-2", "I received the seal")?;
        publish(
            &store,
            &guide,
            NextAction {
                expected_revision: guide.revision,
                epoch: progress.epoch,
                step_id: "step-3".into(),
                instructions: "Leave the room.".into(),
                screen_direction: None,
            },
        )?;
        assert!(!gamebuddy_core::step_complete(&guide, &progress, "step-1"));
        Ok(())
    }
    #[test]
    fn authored_order_protects_a_pickup_without_an_outcome_rule_even_in_the_same_room() -> Result<()>
    {
        use gamebuddy_core::{Atlas, Place, PlaceBinding};
        let dir = tempfile::tempdir()?;
        let store = Store::open(dir.path())?;
        let guide = gamebuddy_core::import_markdown(
            "Game",
            "Walkthrough",
            "# Restore spring\nSpeak to the keeper.\n# Receive seal\nSpeak again for the seal before leaving.\n# Leave\nReturn to town.\n# Later visit\nUnlock this room again.",
        )?;
        store.save_guide(&guide)?;
        let atlas = Atlas {
            guide_id: guide.id.clone(),
            guide_revision: guide.revision,
            places: vec![Place {
                id: "chamber".into(),
                label: "Chamber".into(),
                map_id: None,
                landmarks: "Keeper".into(),
            }],
            bindings: guide
                .steps
                .iter()
                .map(|s| PlaceBinding {
                    step_id: s.id.clone(),
                    place_id: "chamber".into(),
                    evidence: "Same room at different times in the source".into(),
                })
                .collect(),
            ..Default::default()
        };
        super::super::save(&store, &guide, &atlas)?;
        let progress = Progress {
            current_step: Some("step-1".into()),
            reported_completed: std::collections::HashSet::from(["step-1".into()]),
            ..Default::default()
        };
        store.save_progress(&guide, &progress, "fixture")?;
        let mut place = PlaceTracker::default();
        place.report_place("chamber", 0, screen::now_ms(), "I am in the keeper's room");
        store.put(&super::super::tracker_key(&guide), &place)?;
        let action = |id: &str, epoch| NextAction {
            expected_revision: guide.revision,
            epoch,
            step_id: id.into(),
            instructions: "Continue with this action.".into(),
            screen_direction: None,
        };
        for id in ["step-3", "step-4"] {
            assert!(
                publish(&store, &guide, action(id, 0))
                    .unwrap_err()
                    .to_string()
                    .contains("authored walkthrough route")
            );
        }
        let picked = publish(&store, &guide, action("step-2", 0))?;
        assert_eq!(
            picked["navigation"]["authored_route"]["steps"][1]["id"],
            "step-3"
        );
        let progress = store.report_complete(&guide, 0, "step-2", "I received the seal")?;
        assert!(publish(&store, &guide, action("step-3", progress.epoch)).is_ok());
        assert!(!gamebuddy_core::step_complete(&guide, &progress, "step-4"));
        Ok(())
    }

    #[test]
    fn an_explicit_current_task_can_reanchor_without_fabricating_past_completion() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let store = Store::open(dir.path())?;
        let guide = gamebuddy_core::import_markdown(
            "Game",
            "Walkthrough",
            "# Old task\nSpeak to the keeper.\n# Current task\nLeave through the arch.",
        )?;
        store.save_guide(&guide)?;
        // report_location atomically stores this explicit task cursor.
        let progress = Progress {
            current_step: Some("step-2".into()),
            ..Default::default()
        };
        store.save_progress(&guide, &progress, "player_location_fixture")?;
        publish(
            &store,
            &guide,
            NextAction {
                expected_revision: guide.revision,
                epoch: 0,
                step_id: "step-2".into(),
                instructions: "Leave through the arch.".into(),
                screen_direction: None,
            },
        )?;
        assert!(store.progress(&guide)?.reported_completed.is_empty());
        assert!(store.progress(&guide)?.completed.is_empty());
        Ok(())
    }
    #[test]
    fn current_area_advice_cannot_bypass_a_missing_item_or_departure_requirement() -> Result<()> {
        use gamebuddy_core::outcomes::{KnownFact, OutcomeModel};
        let dir = tempfile::tempdir()?;
        let store = Store::open(dir.path())?;
        let guide = gamebuddy_core::import_markdown(
            "Game",
            "Route",
            "# Speak again\nReceive the amber seal before leaving. It opens the later shrine; returning consumes another key.\n# Leave the room\nTake the exit.\n# Open the shrine\nPress against the stone.",
        )?;
        store.save_guide(&guide)?;
        let model: OutcomeModel = serde_json::from_value(json!({
            "facts":[{"id":"has_seal","description":"Carrying the amber seal","possession_names":["amber seal"],"evidence_step_ids":["step-1"]}],
            "actions":[
                {"step_id":"step-1","requires":{"op":"always"},"effects":[{"id":"has_seal","value":true}],"unnecessary_when":{"op":"fact","id":"has_seal","value":true},"reason":"Separate reward conversation","evidence_step_ids":["step-1"]},
                {"step_id":"step-2","requires":{"op":"fact","id":"has_seal","value":true},"reason":"The seal opens the later shrine; returning consumes another key.","evidence_step_ids":["step-1","step-2"]},
                {"step_id":"step-3","requires":{"op":"fact","id":"has_seal","value":true},"reason":"The shrine requires the seal.","evidence_step_ids":["step-1","step-3"]}
            ]
        }))?;
        store.put(
            &format!("outcomes:{}", guide.id),
            &crate::outcomes::Prepared {
                guide_revision: guide.revision,
                reviewed_scopes: vec![],
                model: model.clone(),
                provider_model: "fixture".into(),
                elapsed_ms: 0,
            },
        )?;
        let ticket = screen::start(&store, &guide.game, true)?;
        let picture = dir.path().join("frame.png");
        image::RgbImage::from_pixel(40, 40, image::Rgb([90, 50, 100])).save(&picture)?;
        screen::publish(&store, &ticket, &picture, "test")?;
        for id in ["step-2", "step-3"] {
            let mut progress = Progress {
                current_step: Some(id.into()),
                ..Default::default()
            };
            store.save_progress(&guide, &progress, "fixture")?;
            store.put(
                &key(&guide),
                &Recommendation {
                    revision: guide.revision,
                    epoch: 0,
                    session: ticket.session.clone(),
                    updated_at_ms: screen::now_ms(),
                    step_id: id.into(),
                    instructions: "Old advice to leave or push the stone".into(),
                    location_evidence: Some(json!({"kind":"fixture"})),
                    movement: None,
                },
            )?;
            let action = || NextAction {
                expected_revision: guide.revision,
                epoch: 0,
                step_id: id.into(),
                instructions: "Proceed now.".into(),
                screen_direction: None,
            };
            assert!(
                publish(&store, &guide, action())
                    .unwrap_err()
                    .to_string()
                    .contains("source-established outcome")
            );
            let nav = super::super::context(&store, &guide, &progress, &Default::default())?;
            assert_ne!(
                nav["guidance"]["now"]["id"], id,
                "Old accepted advice bypassed a newly prepared requirement"
            );
            let guidance = gamebuddy_core::Guidance {
                now: Some(guide.steps[0].clone()),
                ..Default::default()
            };
            let warning = super::super::hazards::readiness(
                &guide,
                &progress,
                &guidance,
                &model,
                &model.view(&guide, &progress),
            )
            .expect("Warn before departure");
            assert!(warning.text.contains("another key"));
            progress.world.facts.insert(
                "has_seal".into(),
                KnownFact {
                    value: true,
                    evidence: "I have the amber seal".into(),
                    source: "player_report".into(),
                    step_id: None,
                    at_ms: screen::now_ms(),
                },
            );
            store.save_progress(&guide, &progress, "fixture")?;
            assert!(
                publish(&store, &guide, action()).is_ok(),
                "Possession should satisfy both gates without a pickup checkbox"
            );
            assert!(
                super::super::hazards::readiness(
                    &guide,
                    &progress,
                    &guidance,
                    &model,
                    &model.view(&guide, &progress)
                )
                .is_none()
            );
            assert!(store.progress(&guide)?.completed.is_empty());
        }
        Ok(())
    }
    #[test]
    fn refining_current_area_advice_preserves_its_route_until_progress_changes() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let store = Store::open(dir.path())?;
        let mut guide = gamebuddy_core::import_markdown(
            "Game",
            "Route",
            "# Old gate\nUse the key.\n# Cross the chamber\nWalk right.",
        )?;
        guide.steps[1].prerequisites = vec!["step-1".into()];
        store.save_guide(&guide)?;
        // Represents earlier accepted, source-backed advice in this session.
        store.put(
            &key(&guide),
            &Recommendation {
                revision: guide.revision,
                epoch: 0,
                session: String::new(),
                updated_at_ms: 1,
                step_id: "step-2".into(),
                instructions: "Walk right.".into(),
                movement: None,
                location_evidence: Some(
                    json!({"kind":"current_source_match_not_completion","room_id":"earlier-view"}),
                ),
            },
        )?;
        let action = |epoch| NextAction {
            expected_revision: guide.revision,
            epoch,
            step_id: "step-2".into(),
            instructions: "Climb the ladder ahead.".into(),
            screen_direction: None,
        };
        let result = publish(&store, &guide, action(0))?;
        assert_eq!(result["navigation"]["guidance"]["now"]["id"], "step-2");
        assert!(store.progress(&guide)?.completed.is_empty());
        let reset = Progress {
            epoch: 1,
            ..Default::default()
        };
        store.save_progress(&guide, &reset, "reset")?;
        assert!(publish(&store, &guide, action(1)).is_err());
        Ok(())
    }
    #[test]
    fn reporting_a_past_task_preserves_the_current_unfinished_recommendation() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let store = Store::open(dir.path())?;
        let guide = gamebuddy_core::import_markdown(
            "Game",
            "Route",
            "# Old enemy\nJump past it.\n# Treatment\nPay for healing.",
        )?;
        store.save_guide(&guide)?;
        store.save_progress(
            &guide,
            &Progress {
                current_step: Some("step-2".into()),
                ..Default::default()
            },
            "explicit_task_fixture",
        )?;
        publish(
            &store,
            &guide,
            NextAction {
                expected_revision: guide.revision,
                epoch: 0,
                step_id: "step-2".into(),
                instructions: "Accept treatment.".into(),
                screen_direction: None,
            },
        )?;
        let progress = store.report_complete(&guide, 0, "step-1", "I passed that enemy earlier")?;
        assert_eq!(progress.epoch, 1);
        assert_eq!(
            current(&store, &guide, &progress, "")?.unwrap().step_id,
            "step-2"
        );
        assert!(!gamebuddy_core::step_complete(&guide, &progress, "step-2"));
        Ok(())
    }
    #[test]
    fn completing_current_advice_advances_from_it_instead_of_an_old_cursor() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let store = Store::open(dir.path())?;
        let guide = gamebuddy_core::import_markdown(
            "Game",
            "Route",
            "# Old enemy\nJump past it.\n# Treatment\nPay for healing.\n# Leave hospital\nReturn to the street.",
        )?;
        store.save_guide(&guide)?;
        store.save_progress(
            &guide,
            &Progress {
                current_step: Some("step-2".into()),
                ..Default::default()
            },
            "fixture",
        )?;
        publish(
            &store,
            &guide,
            NextAction {
                expected_revision: guide.revision,
                epoch: 0,
                step_id: "step-2".into(),
                instructions: "Accept treatment.".into(),
                screen_direction: None,
            },
        )?;
        // A legacy stored cursor may still precede the current accepted action.
        store.save_progress(
            &guide,
            &Progress {
                current_step: Some("step-1".into()),
                ..Default::default()
            },
            "legacy_cursor_fixture",
        )?;
        let mut progress = store.report_complete(&guide, 0, "step-2", "I got the treatment")?;
        let context = |progress: &Progress| {
            super::super::context(&store, &guide, progress, &SyncEstimate::default())
        };
        assert_eq!(progress.current_step.as_deref(), Some("step-2"));
        assert_eq!(context(&progress)?["guidance"]["now"]["id"], "step-3");
        assert!(!gamebuddy_core::step_complete(&guide, &progress, "step-1"));
        // Saved progress written by the previous app version also recovers its
        // just-completed route anchor without resurrecting that instruction.
        progress.current_step = Some("step-1".into());
        store.save_progress(&guide, &progress, "old_version_fixture")?;
        assert_eq!(context(&progress)?["guidance"]["now"]["id"], "step-3");
        assert!(context(&progress)?["recommendation"].is_null());
        progress.epoch += 1;
        progress.reported_completed.clear();
        store.save_progress(&guide, &progress, "reset_fixture")?;
        assert!(completed_cursor(&store, &guide, &progress)?.is_none());
        Ok(())
    }
    #[test]
    fn explicit_movement_renews_without_a_named_place_but_stops_on_a_changed_view() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let store = Store::open(dir.path())?;
        let guide =
            gamebuddy_core::import_markdown("Game", "Route", "# Visit church\nEnter the door.")?;
        store.save_guide(&guide)?;
        let ticket = screen::start(&store, "Game", true)?;
        let path = dir.path().join("screen.png");
        image::RgbImage::from_pixel(96, 72, image::Rgb([30, 160, 40])).save(&path)?;
        screen::publish(&store, &ticket, &path, "test")?;
        let action = |instructions: &str, screen_direction| NextAction {
            expected_revision: guide.revision,
            epoch: 0,
            step_id: "step-1".into(),
            instructions: instructions.into(),
            screen_direction,
        };
        assert!(
            publish(
                &store,
                &guide,
                action("Walk right to the door.", Some(ScreenDirection::Left))
            )
            .is_err()
        );
        let result = publish(
            &store,
            &guide,
            action("Enter the door beside you.", Some(ScreenDirection::Up)),
        )?;
        assert!(result["navigation"]["location"]["place_id"].is_null());
        assert_eq!(result["navigation"]["direction"]["direction"], "up");
        assert!(
            result["recommendation"]["movement"]
                .get("frame_signature")
                .is_none()
        );
        let progress = store.progress(&guide)?;
        let context = || super::super::context(&store, &guide, &progress, &SyncEstimate::default());
        let mut saved: Recommendation = store.get(&key(&guide))?.unwrap();
        saved.movement.as_mut().unwrap().captured_at_ms -= screen::FRESH_MS;
        store.put(&key(&guide), &saved)?;
        assert_eq!(
            context()?["direction"]["direction"],
            "up",
            "fresh matching captures keep the current direction beyond the original advice deadline"
        );
        publish(
            &store,
            &guide,
            action("Walk right to the door.", Some(ScreenDirection::Right)),
        )?;
        image::RgbImage::from_pixel(96, 72, image::Rgb([180, 20, 10])).save(&path)?;
        screen::publish(&store, &ticket, &path, "changed_scene")?;
        assert!(context()?["direction"].is_null());
        publish(
            &store,
            &guide,
            action("Enter the door beside you.", Some(ScreenDirection::Up)),
        )?;
        screen::set_enabled(&store, &ticket.session, "Game", false)?;
        assert!(context()?["direction"].is_null());
        assert!(
            publish(
                &store,
                &guide,
                action("Enter the door beside you.", Some(ScreenDirection::Up))
            )
            .is_err()
        );
        screen::start(&store, "Game", true)?;
        assert!(context()?["direction"].is_null());
        assert!(progress.completed.is_empty());
        assert!(progress.reported_completed.is_empty());
        Ok(())
    }
    #[test]
    fn direction_tracks_shared_advice_and_hides_without_live_capture() {
        use gamebuddy_core::{Atlas, Place, PlaceBinding};
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let guide = gamebuddy_core::import_markdown(
            "Game",
            "Route",
            "# Leave town\nWalk right to the gate.",
        )
        .unwrap();
        store.save_guide(&guide).unwrap();
        let atlas = Atlas {
            guide_id: guide.id.clone(),
            guide_revision: guide.revision,
            places: vec![Place {
                id: "town".into(),
                label: "Town".into(),
                map_id: None,
                landmarks: "Gate".into(),
            }],
            bindings: vec![PlaceBinding {
                step_id: "step-1".into(),
                place_id: "town".into(),
                evidence: "Route starts at the gate".into(),
            }],
            ..Default::default()
        };
        super::super::save(&store, &guide, &atlas).unwrap();
        let ticket = screen::start(&store, "Game", true).unwrap();
        let frame = dir.path().join("frame.png");
        image::RgbImage::new(32, 32).save(&frame).unwrap();
        screen::publish(&store, &ticket, &frame, "test").unwrap();
        let mut location = PlaceTracker::for_session(ticket.session.clone());
        location.report("town", "step-1", 0, screen::now_ms(), "I am at the gate");
        store
            .put(&super::super::tracker_key(&guide), &location)
            .unwrap();
        let progress = store.progress(&guide).unwrap();
        let before =
            super::super::context(&store, &guide, &progress, &SyncEstimate::default()).unwrap();
        assert_eq!(before["direction"]["direction"], "right");
        let result = publish(
            &store,
            &guide,
            NextAction {
                expected_revision: guide.revision,
                epoch: progress.epoch,
                step_id: "step-1".into(),
                instructions: "Walk left to the other door.".into(),
                screen_direction: None,
            },
        )
        .unwrap();
        assert_eq!(result["navigation"]["direction"]["direction"], "left");
        assert_eq!(
            result["navigation"]["direction"]["step_id"],
            result["navigation"]["guidance"]["now"]["id"]
        );
        assert_eq!(
            serde_json::to_value(store.progress(&guide).unwrap()).unwrap(),
            serde_json::to_value(&progress).unwrap()
        );
        screen::set_enabled(&store, &ticket.session, "Game", false).unwrap();
        let paused =
            super::super::context(&store, &guide, &progress, &SyncEstimate::default()).unwrap();
        assert!(paused["direction"].is_null());
        screen::start(&store, "Game", true).unwrap();
        let replaced =
            super::super::context(&store, &guide, &progress, &SyncEstimate::default()).unwrap();
        assert!(replaced["direction"].is_null());
    }
    #[test]
    fn advice_rejects_stale_edits_and_unconfirmed_prerequisites() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let mut guide = gamebuddy_core::import_markdown(
            "Game",
            "Guide",
            "# Get key\nBuy it\n# Open door\nUse key",
        )
        .unwrap();
        guide.steps[1].prerequisites = vec!["step-1".into()];
        store.save_guide(&guide).unwrap();
        let action = |revision, epoch, step: &str| NextAction {
            expected_revision: revision,
            epoch,
            step_id: step.into(),
            instructions: "Take the next action.".into(),
            screen_direction: None,
        };
        assert!(publish(&store, &guide, action(guide.revision, 0, "step-2")).is_err());
        assert!(publish(&store, &guide, action(guide.revision + 1, 0, "step-1")).is_err());
        assert!(publish(&store, &guide, action(guide.revision, 1, "step-1")).is_err());
        assert!(store.get::<Recommendation>(&key(&guide)).unwrap().is_none());
        let stale_guide = guide.clone();
        guide.revision += 1;
        store.save_guide(&guide).unwrap();
        assert!(
            publish(
                &store,
                &stale_guide,
                action(stale_guide.revision, 0, "step-1")
            )
            .is_err()
        );
        assert!(store.get::<Recommendation>(&key(&guide)).unwrap().is_none());
    }
    #[test]
    fn advice_updates_shared_outline_without_claiming_progress_and_expires_on_reset() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let guide = gamebuddy_core::import_markdown(
            "Game",
            "Guide",
            "# Old advice\nWait here\n# New action\nTalk to the king",
        )
        .unwrap();
        store.save_guide(&guide).unwrap();
        store
            .save_progress(
                &guide,
                &Progress {
                    current_step: Some("step-2".into()),
                    ..Default::default()
                },
                "explicit_task_fixture",
            )
            .unwrap();
        let progress = store.progress(&guide).unwrap();
        let action = NextAction {
            expected_revision: guide.revision,
            epoch: progress.epoch,
            step_id: "step-2".into(),
            instructions: "Enter the castle and speak to the king.".into(),
            screen_direction: None,
        };
        let result = publish(&store, &guide, action).unwrap();
        assert_eq!(result["navigation"]["guidance"]["now"]["id"], "step-2");
        assert_eq!(
            result["navigation"]["cursor"]["instructions"],
            "Enter the castle and speak to the king."
        );
        assert_eq!(
            serde_json::to_value(store.progress(&guide).unwrap()).unwrap(),
            serde_json::to_value(&progress).unwrap()
        );
        assert!(
            current(&store, &guide, &progress, "new-session")
                .unwrap()
                .is_none()
        );
        let mut changed = progress;
        changed.epoch += 1;
        assert!(current(&store, &guide, &changed, "").unwrap().is_none());
    }
}
