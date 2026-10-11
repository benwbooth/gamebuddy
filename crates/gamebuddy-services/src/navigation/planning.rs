//! The live plan: where the player is, what they have, and the dependency
//! plan from there (see gamebuddy_core::planner).
use crate::Store;
use anyhow::Result;
use gamebuddy_core::{Guide, PlaceTracker, planner::PlanGraph};
use serde_json::{Value, json};

/// The player's area, most direct evidence first: what they said, being
/// back in an earlier area, the screen's place match, the current step.
pub fn player_area(store: &Store, guide: &Guide, graph: &PlanGraph) -> Result<Option<String>> {
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
    let step = progress
        .current_step
        .as_deref()
        .and_then(|s| graph.area_of_step(s))
        .map(str::to_owned);
    Ok(reported.or(off).or(seen).or(step))
}

/// The plan as the live voice reads it.
pub fn context(store: &Store, guide: &Guide) -> Result<Value> {
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
            .map(|f| f.description.clone())
            .unwrap_or_else(|| fact.replace('_', " "))
    };
    let has = |fact: &str| progress.world.facts.get(fact).is_some_and(|f| f.value);
    let steps: Vec<String> = plan
        .items
        .iter()
        .map(|item| match item {
            gamebuddy_core::planner::PlanItem::Get {
                need, title, area, ..
            } => format!(
                "Get first: {} ({}{})",
                describe(need),
                title,
                area.as_deref()
                    .map(|a| format!(", {a}"))
                    .unwrap_or_default()
            ),
            gamebuddy_core::planner::PlanItem::Travel { from, to, requires } => format!(
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
        .collect();
    Ok(json!({
        "player_area": plan.area,
        "steps": steps,
        "items": plan.items,
        "missing_with_no_known_source": plan.unresolved.iter().map(|f| describe(f)).collect::<Vec<_>>(),
        "after_that": plan.upcoming,
        "instruction": "The plan from where the player is now and what they have: get what is missing first, travel, then the objective. Give its first item; say why (what it is needed for)."
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
                "Needed first ({}){}.",
                describe(need),
                target
                    .as_ref()
                    .map(|(t, _)| format!(" before \"{t}\""))
                    .unwrap_or_default()
            ),
        )),
        Some(PlanItem::Travel { to, .. }) => {
            // Lead with the objective's first step; say the area to head for.
            plan.items.iter().find_map(|i| match i {
                PlanItem::Do {
                    first_step: Some(step),
                    area,
                    title,
                    ..
                } => Some((
                    step.clone(),
                    format!(
                        "From {}, head toward {} for \"{title}\".",
                        plan.area.as_deref().unwrap_or("here"),
                        area.as_deref().unwrap_or(to)
                    ),
                )),
                _ => None,
            })
        }
        _ => None,
    })
}
