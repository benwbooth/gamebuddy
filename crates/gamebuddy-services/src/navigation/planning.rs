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
        .map(|m| m.title.clone());
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

/// Keys and items that places near the player need (see
/// PlanGraph::nearby_requirements), with the walkthrough's own words.
pub fn nearby_requirements(store: &Store, guide: &Guide) -> Result<Value> {
    let atlas = super::atlas(store, guide)?;
    let model = crate::outcomes::prepared(store, guide)?
        .map(|p| p.model.clone())
        .unwrap_or_default();
    let graph = PlanGraph::build(guide, &atlas, &model);
    let progress = store.progress(guide)?;
    let area = player_area(store, guide, &graph)?;
    let text = |o: &gamebuddy_core::planner::Objective| {
        o.steps
            .iter()
            .filter_map(|id| guide.step(id))
            .map(|s| s.instructions.as_str())
            .filter(|t| {
                let t = t.to_lowercase();
                t.contains("key") || t.contains("requir") || t.contains("locked")
            })
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .take(400)
            .collect::<String>()
    };
    Ok(json!(graph
        .nearby_requirements(guide, &progress, area.as_deref())
        .into_iter()
        .map(|o| {
            let walkthrough = text(o);
            json!({"objective":o.title,"area":o.area,"optional":o.optional,"requires":o.requires,"walkthrough":walkthrough,"key_needed":key_lookup(store, guide, o, &walkthrough)})
        })
        .collect::<Vec<_>>()))
}

/// Which key a place needs when the walkthrough only says "the required
/// key": looked up once with Google Search (through Gemini) in the
/// background and remembered, so the warning names it before the player
/// reaches the door.
fn key_lookup(
    store: &Store,
    guide: &Guide,
    objective: &gamebuddy_core::planner::Objective,
    walkthrough: &str,
) -> Option<String> {
    let lower = walkthrough.to_lowercase();
    let named = [
        "queen", "king", "jack", "joker", "ace", "demon", "key q", "key k", "key j",
    ]
    .iter()
    .any(|k| lower.contains(k));
    if !lower.contains("key") || named {
        return None;
    }
    let key = format!("key-lookup:{}:{}", guide.id, objective.id);
    if let Ok(Some(found)) = store.get::<Value>(&key) {
        return found["answer"].as_str().map(str::to_owned);
    }
    static RUNNING: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(vec![]);
    {
        let mut running = RUNNING.lock().ok()?;
        if running.contains(&key) {
            return None;
        }
        running.push(key.clone());
    }
    let root = store.root.clone();
    let question = format!(
        "In {} , which key (by its in-game name) is needed to enter or open this: \"{}\"? Walkthrough text: {} Answer with the key name only, or \"unknown\".",
        guide.game, objective.title, walkthrough
    );
    std::thread::spawn(move || {
        let answer = (|| -> anyhow::Result<String> {
            let api_key = crate::LiveVoice::default().api_key()?;
            let body =
                json!({"contents":[{"parts":[{"text":question}]}],"tools":[{"googleSearch":{}}]});
            let client = reqwest::blocking::Client::builder()
                .timeout(std::time::Duration::from_secs(60))
                .build()?;
            // Busy models answer 503; try the next one.
            let mut reply = Value::Null;
            for model in [
                "gemini-3.8-flash",
                "gemini-3.5-flash",
                "gemini-3.1-pro-preview",
            ] {
                reply = client
                    .post(format!("https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent?key={api_key}"))
                    .json(&body)
                    .send()?
                    .json()?;
                if reply["error"].is_null() {
                    break;
                }
            }
            let text: String = reply["candidates"][0]["content"]["parts"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|p| p["text"].as_str())
                .collect();
            anyhow::ensure!(!text.trim().is_empty(), "No answer: {}", reply["error"]);
            Ok(text.trim().chars().take(80).collect())
        })();
        if let Ok(store) = Store::open(&root) {
            match answer {
                Ok(answer) if !answer.eq_ignore_ascii_case("unknown") => {
                    let _ = store.put(&key, &json!({"answer":answer}));
                }
                Ok(_) => {}
                Err(error) => {
                    let _ = store.put("key-lookup-error", &format!("{error:#}"));
                }
            }
        }
        if let Ok(mut running) = RUNNING.lock() {
            running.retain(|k| *k != key);
        }
    });
    None
}

/// Walkthrough tips and warnings for where the player is and what comes
/// next, optional tasks included: grinding spots, shortcuts, items to
/// leave alone. The plan follows the main line only, so without these the
/// live voice never hears about, say, a cursed pendant in an optional tower.
/// Objectives in or next to the player's area, plus the next `ahead` on
/// the route, by index into the plan graph's objectives.
fn nearby(store: &Store, guide: &Guide, ahead: usize) -> Result<(PlanGraph, Vec<usize>)> {
    let atlas = super::atlas(store, guide)?;
    let model = crate::outcomes::prepared(store, guide)?
        .map(|p| p.model.clone())
        .unwrap_or_default();
    let graph = PlanGraph::build(guide, &atlas, &model);
    let progress = store.progress(guide)?;
    let area = player_area(store, guide, &graph)?;
    let near: std::collections::BTreeSet<&str> = area
        .as_deref()
        .into_iter()
        .chain(
            graph
                .crossings
                .iter()
                .filter_map(|c| match area.as_deref() {
                    Some(a) if c.from == a => Some(c.to.as_str()),
                    Some(a) if c.to == a => Some(c.from.as_str()),
                    _ => None,
                }),
        )
        .collect();
    let mark = progress
        .current_step
        .as_deref()
        .and_then(|c| {
            graph
                .objectives
                .iter()
                .position(|o| o.steps.iter().any(|s| s == c))
        })
        .unwrap_or(0);
    let picked = graph
        .objectives
        .iter()
        .enumerate()
        .filter(|(i, o)| {
            o.area.as_deref().is_some_and(|a| near.contains(a)) || (*i >= mark && *i < mark + ahead)
        })
        .map(|(i, _)| i)
        .collect();
    Ok((graph, picked))
}

/// Data cards for things named by the objectives around the player, for
/// GameBuddy's own reference widget.
pub fn area_cards(store: &Store, guide: &Guide) -> Result<Vec<Value>> {
    let (graph, picked) = nearby(store, guide, 3)?;
    let mut text = String::new();
    for o in picked.iter().map(|&i| &graph.objectives[i]) {
        text.push_str(&o.title);
        text.push(' ');
        text.push_str(&o.summary);
        for step in o.steps.iter().filter_map(|id| guide.step(id)) {
            text.push(' ');
            text.push_str(&step.instructions);
        }
    }
    crate::game_data::mentioned(store, guide, &text)
}

pub fn area_notes(store: &Store, guide: &Guide) -> Result<Value> {
    let (graph, picked) = nearby(store, guide, 10)?;
    const TIP: [&str; 18] = [
        "avoid",
        "don't",
        "do not",
        "skip",
        "grind",
        "farm",
        "level",
        "gold",
        "money",
        "curse",
        "reduce",
        "recommend",
        "safe",
        "shortcut",
        "warning",
        "beware",
        "instead",
        "optional",
    ];
    let tip = |text: &str| {
        let lower = text.to_lowercase();
        TIP.iter().any(|w| lower.contains(w))
    };
    let mut notes: Vec<Value> = picked
        .iter()
        .map(|&i| &graph.objectives[i])
        .filter_map(|o| {
            let mut lines: Vec<&str> = vec![];
            if tip(&o.summary) {
                lines.push(&o.summary);
            }
            for step in o.steps.iter().filter_map(|id| guide.step(id)) {
                if tip(&step.instructions) && lines.len() < 4 {
                    lines.push(&step.instructions);
                }
            }
            (!lines.is_empty()).then(|| {
                json!({"task":o.title,"area":o.area,"optional":o.optional,
                    "notes":lines.join(" ").chars().take(700).collect::<String>()})
            })
        })
        .take(12)
        .collect();
    // Data cards (prices, stats, tips) for anything those notes name.
    let named = crate::game_data::mentioned(store, guide, &serde_json::to_string(&notes)?)?;
    if !named.is_empty() {
        notes.push(json!({"task":"Data cards for things mentioned above","cards":named}));
    }
    Ok(json!(notes))
}
