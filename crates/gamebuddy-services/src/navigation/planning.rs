//! The live plan: where the player is, what they have, and the dependency
//! plan from there (see gamebuddy_core::planner).
use crate::Store;
use anyhow::Result;
use gamebuddy_core::{Guide, PlaceTracker, planner::PlanGraph};
use serde_json::{Value, json};

/// The player's area, most direct evidence first: what they said, being
/// back in an earlier area, the screen's place match, the current step.
pub fn player_area(store: &Store, guide: &Guide, graph: &PlanGraph) -> Result<Option<String>> {
    Ok(locate(store, guide, graph)?.0)
}

/// The area and whether this game session has evidence for it (otherwise
/// it is only the current route step's area).
pub fn locate(store: &Store, guide: &Guide, graph: &PlanGraph) -> Result<(Option<String>, bool)> {
    let atlas = super::atlas(store, guide)?;
    let progress = store.progress(guide)?;
    // Only evidence from the current game session describes where the
    // player is now.
    let session = crate::screen::status(store)
        .ok()
        .and_then(|s| s["session_id"].as_str().map(str::to_owned))
        .unwrap_or_default();
    let place = |tracker: Option<PlaceTracker>, status: &str| {
        tracker
            .filter(|t| t.status == status && (session.is_empty() || t.session_id == session))
            .and_then(|t| t.place_id)
            .and_then(|id| gamebuddy_core::planner::place_area(&atlas, &id))
    };
    let reported = place(store.get(&super::tracker_key(guide))?, "reported");
    let off = super::follow::off_route(store, guide)?
        .map(|o| o.map_title)
        .filter(|t| !t.is_empty());
    let seen = place(
        store.get(&format!("world-place-tracker:{}", guide.id))?,
        "estimated",
    );
    // The live screen registered against an area map this session.
    let mapped = store
        .get::<Value>(&format!("world-observation:{}", guide.id))?
        .filter(|o| session.is_empty() || o["tracker"]["session"] == session.as_str())
        .filter(|o| {
            o["map_registration"]["observed_at_ms"]
                .as_u64()
                .is_some_and(|at| crate::screen::now_ms().saturating_sub(at) < 120_000)
        })
        .and_then(|o| o["map_registration"]["map_id"].as_str().map(str::to_owned))
        .and_then(|id| atlas.maps.iter().find(|m| m.id == id))
        .map(|m| m.title.clone())
        .filter(|t| !t.contains(" To "));
    let step = progress
        .current_step
        .as_deref()
        .and_then(|s| graph.area_of_step(s))
        .map(str::to_owned);
    let evidence = reported.or(off).or(mapped).or(seen);
    let confirmed = evidence.is_some();
    Ok((evidence.or(step), confirmed))
}

/// Where a death or a restart puts the player: the area of the last
/// guru visit the player finished (the walkthrough's respawn points are the
/// guru's chambers).
pub fn checkpoint(
    guide: &Guide,
    atlas: &gamebuddy_core::Atlas,
    progress: &gamebuddy_core::Progress,
) -> Option<String> {
    let current = progress
        .current_step
        .as_deref()
        .and_then(|c| guide.steps.iter().position(|s| s.id == c))
        .unwrap_or(guide.steps.len());
    guide.steps[..current]
        .iter()
        .rev()
        .filter(|s| gamebuddy_core::step_complete(guide, progress, &s.id))
        .find_map(|s| {
            let binding = atlas.bindings.iter().find(|b| b.step_id == s.id)?;
            let place = atlas.places.iter().find(|p| p.id == binding.place_id)?;
            place
                .label
                .to_lowercase()
                .contains("guru")
                .then(|| gamebuddy_core::planner::place_area(atlas, &place.id))
                .flatten()
        })
}

/// The plan as the live voice reads it.
pub fn context(store: &Store, guide: &Guide) -> Result<Value> {
    let atlas = super::atlas(store, guide)?;
    let model = crate::outcomes::prepared(store, guide)?
        .map(|p| p.model.clone())
        .unwrap_or_default();
    let graph = PlanGraph::build(guide, &atlas, &model);
    let progress = store.progress(guide)?;
    let (area, confirmed) = locate(store, guide, &graph)?;
    let plan = graph.plan(guide, &progress, area.as_deref());
    let checkpoint = checkpoint(guide, &atlas, &progress);
    let describe = |fact: &str| {
        model
            .facts
            .iter()
            .find(|f| f.id == fact)
            .map(|f| f.description.clone())
            .unwrap_or_else(|| fact.replace('_', " "))
    };
    let has = |fact: &str| progress.world.facts.get(fact).is_some_and(|f| f.value);
    let render = |plan: &gamebuddy_core::planner::Plan| -> Vec<String> {
        plan.items
            .iter()
            .map(|item| match item {
                gamebuddy_core::planner::PlanItem::Get {
                    need, title, area, ..
                } => format!(
                    "Get first: {} ({}{}). If the player already has it (they say so, or the screen showed it), record report_world_state facts [{{id: \"{need}\", value: true}}] and read the context again.",
                    describe(need),
                    title,
                    area.as_deref()
                        .map(|a| format!(", {a}"))
                        .unwrap_or_default()
                ),
                gamebuddy_core::planner::PlanItem::Travel {
                    from, to, requires, ..
                } => format!(
                    "Travel {from} -> {to}{}",
                    if requires.iter().all(|r| has(r)) {
                        String::new()
                    } else {
                        format!(
                            " (needs: {})",
                            requires
                                .iter()
                                .map(|r| describe(r))
                                .collect::<Vec<_>>()
                                .join(", ")
                        )
                    }
                ),
                gamebuddy_core::planner::PlanItem::Do { title, area, .. } => format!(
                    "Then: {title}{}",
                    area.as_deref()
                        .map(|a| format!(" ({a})"))
                        .unwrap_or_default()
                ),
            })
            .collect()
    };
    let steps = render(&plan);
    let at_checkpoint = checkpoint
        .as_deref()
        .filter(|c| !confirmed && Some(*c) != area.as_deref())
        .map(|c| render(&graph.plan(guide, &progress, Some(c))));
    Ok(json!({
        "player_area": plan.area,
        "area_confirmed": confirmed,
        "checkpoint": checkpoint,
        "plan_if_at_checkpoint": at_checkpoint,
        "steps": steps,
        "items": plan.items,
        "missing_with_no_known_source": plan.unresolved.iter().map(|f| describe(f)).collect::<Vec<_>>(),
        "after_that": plan.upcoming,
        "instruction": "The plan from where the player is now and what they have: get what is missing first, travel, then the objective. Give its first item; say why (what it is needed for). When area_confirmed is false, player_area is only the saved route's guess: a death or restart puts the player at the checkpoint, where plan_if_at_checkpoint applies."
    }))
}

/// The plan's first item when it is a detour (get something first, or
/// travel to another area): its step and a short reason to show with it.
pub fn first_detour(store: &Store, guide: &Guide) -> Result<Option<(String, String)>> {
    use gamebuddy_core::planner::PlanItem;
    let atlas = super::atlas(store, guide)?;
    let model = crate::outcomes::prepared(store, guide)?
        .map(|p| p.model.clone())
        .unwrap_or_default();
    let graph = PlanGraph::build(guide, &atlas, &model);
    let progress = store.progress(guide)?;
    let area = player_area(store, guide, &graph)?;
    let plan = graph.plan(guide, &progress, area.as_deref());
    let describe = |fact: &str| {
        model
            .facts
            .iter()
            .find(|f| f.id == fact)
            .map(|f| f.description.trim_end_matches('.').to_owned())
            .unwrap_or_else(|| fact.replace('_', " "))
    };
    let target = plan.items.iter().rev().find_map(|i| match i {
        PlanItem::Do { title, area, .. } => Some((title.clone(), area.clone())),
        _ => None,
    });
    Ok(match plan.items.first() {
        Some(PlanItem::Get {
            need,
            first_step: Some(step),
            ..
        }) => Some((
            step.clone(),
            format!(
                "Needed first ({}){}. (If the player already has it, record report_world_state fact {need}=true.)",
                describe(need),
                target
                    .as_ref()
                    .map(|(t, _)| format!(" before \"{t}\""))
                    .unwrap_or_default()
            ),
        )),
        Some(PlanItem::Travel {
            to,
            via_step: Some(step),
            ..
        }) => Some((
            step.clone(),
            format!(
                "From {}, head toward {to}{}.",
                plan.area.as_deref().unwrap_or("here"),
                target
                    .as_ref()
                    .map(|(t, area)| format!(
                        " on the way to \"{t}\"{}",
                        area.as_deref()
                            .map(|a| format!(" in {a}"))
                            .unwrap_or_default()
                    ))
                    .unwrap_or_default()
            ),
        )),
        _ => None,
    })
}

/// When the live screen has held a match against one area map for a few
/// seconds and the route cursor is in another area, report that area like a
/// player location report, once per change. This moves the cursor to the
/// area's steps (forward, or back over steps a wrong jump only passed).
pub fn sync_area(store: &Store, guide: &Guide, at_ms: u64) -> Result<()> {
    let atlas = super::atlas(store, guide)?;
    let session = crate::screen::status(store)
        .ok()
        .and_then(|s| s["session_id"].as_str().map(str::to_owned))
        .unwrap_or_default();
    let Some(area) = store
        .get::<Value>(&format!("world-observation:{}", guide.id))?
        .filter(|o| session.is_empty() || o["tracker"]["session"] == session.as_str())
        .and_then(|o| o["map_registration"]["map_id"].as_str().map(str::to_owned))
        .and_then(|id| atlas.maps.iter().find(|m| m.id == id))
        .map(|m| m.title.clone())
        .filter(|t| !t.contains(" To "))
    else {
        return Ok(());
    };
    let key = format!("area-sync:{}", guide.id);
    let mut state: Value = store.get(&key)?.unwrap_or_else(|| json!({}));
    if state["candidate"] != area.as_str() {
        store.put(
            &key,
            &json!({"candidate":area,"since":at_ms,"reported":state["reported"]}),
        )?;
        return Ok(());
    }
    if at_ms.saturating_sub(state["since"].as_u64().unwrap_or(at_ms)) < 4_000
        || state["reported"] == area.as_str()
    {
        return Ok(());
    }
    let model = crate::outcomes::prepared(store, guide)?
        .map(|p| p.model.clone())
        .unwrap_or_default();
    let graph = PlanGraph::build(guide, &atlas, &model);
    let progress = store.progress(guide)?;
    let cursor = progress
        .current_step
        .as_deref()
        .and_then(|s| graph.area_of_step(s))
        .map(str::to_owned);
    state["reported"] = json!(area);
    store.put(&key, &state)?;
    if cursor.as_deref() == Some(area.as_str()) {
        return Ok(());
    }
    super::report_location(
        store,
        guide,
        super::LocationReport {
            place_id: String::new(),
            place_label: Some(area.clone()),
            step_id: None,
            player_report: format!("Screen: the live screen matches the {area} map"),
            expected_revision: guide.revision,
            epoch: progress.epoch,
        },
    )?;
    Ok(())
}

/// The walkthrough step the plan's first item starts with: what to get, the
/// way out of this area, or the objective's next step.
pub fn next_step(store: &Store, guide: &Guide) -> Result<Option<String>> {
    use gamebuddy_core::planner::PlanItem;
    let atlas = super::atlas(store, guide)?;
    let model = crate::outcomes::prepared(store, guide)?
        .map(|p| p.model.clone())
        .unwrap_or_default();
    let graph = PlanGraph::build(guide, &atlas, &model);
    let progress = store.progress(guide)?;
    let area = player_area(store, guide, &graph)?;
    let plan = graph.plan(guide, &progress, area.as_deref());
    Ok(plan.items.iter().find_map(|item| match item {
        PlanItem::Get { first_step, .. } | PlanItem::Do { first_step, .. } => first_step.clone(),
        PlanItem::Travel { via_step, .. } => via_step.clone(),
    }))
}
