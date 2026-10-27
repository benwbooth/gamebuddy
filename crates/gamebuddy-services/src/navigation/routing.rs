//! Where the player is heading right now (a travel goal) and the way there,
//! from the walkthrough's own moves (`gamebuddy_core::navigation::routing`).
//!
//! A goal is set when the player names a place ("how do I get to Forepaw?")
//! or GameBuddy tells them to go somewhere ("head back to Forepaw"), and is
//! cleared on arrival. The arrow and the voice both follow its route.
use crate::Store;
use anyhow::Result;
use gamebuddy_core::{
    Atlas, Guide,
    navigation::{direction::ScreenDirection, routing},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Goal {
    pub place_id: String,
    pub label: String,
    pub set_at_ms: u64,
}

fn key(guide: &Guide) -> String {
    format!("travel-goal:{}", guide.id)
}

pub fn goal(store: &Store, guide: &Guide) -> Result<Option<Goal>> {
    store.get(&key(guide))
}

pub fn clear(store: &Store, guide: &Guide) -> Result<()> {
    store.put(&key(guide), &Option::<Goal>::None)
}

fn words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() > 1)
        .map(str::to_owned)
        .collect()
}

/// The place a sentence names, if any. A place counts only when the first
/// word of its label (usually the town or area: "Forepaw", "Eolis") appears;
/// among those, the one sharing most other words wins ("buy Wing Boots in
/// Forepaw" picks the Forepaw equipment shop over the Forepaw key shop when
/// the walkthrough sells them there).
pub fn named_place(atlas: &Atlas, text: &str) -> Option<String> {
    let said = words(text);
    let common = [
        "the", "of", "and", "room", "chamber", "interior", "exterior", "shop", "door",
    ];
    atlas
        .places
        .iter()
        .filter_map(|place| {
            let label = words(&place.label);
            let head = label.first()?;
            if common.contains(&head.as_str()) || !said.contains(head) {
                return None;
            }
            let shared = label.iter().filter(|w| said.contains(w)).count();
            Some((
                shared,
                std::cmp::Reverse(place.label.len()),
                place.id.clone(),
            ))
        })
        .max()
        .map(|(_, _, id)| id)
}

/// Remember where the player is heading. Returns the goal when it changed.
pub fn set_from_text(
    store: &Store,
    guide: &Guide,
    atlas: &Atlas,
    text: &str,
    current: Option<&str>,
) -> Result<Option<Goal>> {
    let Some(place_id) = named_place(atlas, text) else {
        return Ok(None);
    };
    if current == Some(place_id.as_str()) {
        return Ok(None);
    }
    let label = atlas
        .places
        .iter()
        .find(|p| p.id == place_id)
        .map(|p| p.label.clone())
        .unwrap_or_default();
    let goal = Goal {
        place_id,
        label,
        set_at_ms: crate::screen::now_ms(),
    };
    store.put(&key(guide), &Some(goal.clone()))?;
    crate::flight::event("travel_goal", json!({"place": goal.label}));
    Ok(Some(goal))
}

pub fn direction_word(direction: ScreenDirection) -> &'static str {
    use ScreenDirection::*;
    match direction {
        Left => "left",
        Right => "right",
        Up => "up",
        Down => "down",
        UpLeft => "up and left",
        UpRight => "up and right",
        DownLeft => "down and left",
        DownRight => "down and right",
    }
}

/// The area maps to cross, from the map titles themselves: a map called
/// "Forepaw To Mascon" joins the Forepaw and Mascon maps. Gives the big
/// picture ("back through Mascon, then the Forepaw-to-Mascon passage") that
/// the walkthrough's room-by-room moves do not.
pub fn area_path(atlas: &Atlas, from_map: &str, to_map: &str) -> Option<Vec<String>> {
    use std::collections::{HashMap, VecDeque};
    let title = |id: &str| {
        atlas
            .maps
            .iter()
            .find(|m| m.id == id)
            .map(|m| m.title.trim().to_owned())
    };
    let (start, goal) = (title(from_map)?, title(to_map)?);
    let titles: Vec<String> = atlas
        .maps
        .iter()
        .map(|m| m.title.trim().to_owned())
        .collect();
    let ends = |t: &str| -> Option<(String, String)> {
        let (a, b) = t.split_once(" To ")?;
        Some((a.trim().to_owned(), b.trim().to_owned()))
    };
    let neighbours = |t: &str| -> Vec<String> {
        let mut out = vec![];
        if let Some((a, b)) = ends(t) {
            out.extend(titles.iter().filter(|x| **x == a || **x == b).cloned());
        }
        out.extend(
            titles
                .iter()
                .filter(|x| ends(x).is_some_and(|(a, b)| a == t || b == t))
                .cloned(),
        );
        out
    };
    let mut previous: HashMap<String, String> = HashMap::new();
    let mut queue = VecDeque::from([start.clone()]);
    while let Some(here) = queue.pop_front() {
        if here == goal {
            break;
        }
        for next in neighbours(&here) {
            if next != start && !previous.contains_key(&next) {
                previous.insert(next.clone(), here.clone());
                queue.push_back(next);
            }
        }
    }
    let mut path = vec![goal.clone()];
    let mut at = goal;
    while at != start {
        at = previous.get(&at)?.clone();
        path.push(at.clone());
    }
    path.reverse();
    Some(path)
}

/// The way from `current` to the goal: each place to pass, which way to head
/// for it, and the walkthrough's own instruction for that move.
pub fn route_view(guide: &Guide, atlas: &Atlas, current: &str, goal: &Goal) -> Option<Value> {
    let hops = routing::route(guide, atlas, current, &goal.place_id).unwrap_or_default();
    // A place's own map, else the area map named after its town (the
    // Forepaw key shop has no map of its own, but the "Forepaw" map does).
    let map_of = |id: &str| {
        let place = atlas.places.iter().find(|p| p.id == id)?;
        place.map_id.clone().or_else(|| {
            let head = words(&place.label).into_iter().next()?;
            atlas
                .maps
                .iter()
                .find(|m| m.title.trim().eq_ignore_ascii_case(&head))
                .map(|m| m.id.clone())
        })
    };
    let areas = map_of(current)
        .zip(map_of(&goal.place_id))
        .and_then(|(a, b)| area_path(atlas, &a, &b));
    if hops.is_empty() && areas.is_none() {
        return None;
    }
    let label = |id: &str| {
        atlas
            .places
            .iter()
            .find(|p| p.id == id)
            .map(|p| p.label.clone())
            .unwrap_or_else(|| id.to_owned())
    };
    Some(json!({
        "from": label(current),
        "to": goal.label,
        "areas_to_cross": areas,
        "hops": hops.iter().take(12).map(|h| json!({
            "to": label(&h.to),
            "head": h.direction.map(direction_word),
            "going_back_over_walkthrough_path": h.backwards,
            "walkthrough_instruction": guide.step(&h.step_id).map(|s| s.instructions.chars().take(220).collect::<String>()),
        })).collect::<Vec<_>>(),
        "more_hops": hops.len().saturating_sub(12),
        "instruction": "areas_to_cross is the order of area maps from where the player is to the destination (the first is where they are). hops are the rooms the walkthrough passes on the way, with a heading when the walkthrough states one. Guide the player area by area, naming the next area to reach; give a left/right/up/down only when a hop's head gives it. Never say they are already at the destination unless the route is empty.",
    }))
}

/// The first hop's direction toward the goal, for the arrow.
pub fn first_direction(
    guide: &Guide,
    atlas: &Atlas,
    current: &str,
    goal: &Goal,
) -> Option<(ScreenDirection, String)> {
    let hops = routing::route(guide, atlas, current, &goal.place_id)?;
    // Only the hop the player is on now: a heading further down the route
    // says nothing about which way to go from here.
    hops.first().and_then(|h| {
        h.direction.map(|d| {
            let label = atlas
                .places
                .iter()
                .find(|p| p.id == h.to)
                .map(|p| p.label.clone())
                .unwrap_or_default();
            (d, label)
        })
    })
}

/// Where each area map's exits are, learned from the player's own crossings:
/// map id -> neighbouring map id -> point on the map, in fractions.
type Exits = std::collections::BTreeMap<String, std::collections::BTreeMap<String, [f32; 2]>>;

fn exits_key(guide: &Guide) -> String {
    format!("map-exits:{}", guide.id)
}

pub fn learn_exit(
    store: &Store,
    guide: &Guide,
    from_map: &str,
    to_map: &str,
    at: [f32; 2],
) -> Result<()> {
    let mut exits: Exits = store.get(&exits_key(guide))?.unwrap_or_default();
    let known = exits.entry(from_map.into()).or_default();
    if known.get(to_map) != Some(&at) {
        known.insert(to_map.into(), at);
        store.put(&exits_key(guide), &exits)?;
        crate::flight::event(
            "map_exit",
            json!({"from": from_map, "to": to_map, "at": at}),
        );
    }
    Ok(())
}

/// Which way to head on the current area map to reach the next one: toward
/// the exit the player has used before, else by how the maps are drawn
/// ("A To B" passages run left to right, from A to B).
pub fn area_heading(
    store: &Store,
    guide: &Guide,
    atlas: &Atlas,
    map_id: &str,
    position: Option<[f32; 2]>,
    next_title: &str,
) -> Option<ScreenDirection> {
    let next = atlas.maps.iter().find(|m| m.title.trim() == next_title)?;
    let here = atlas.maps.iter().find(|m| m.id == map_id)?;
    let exits: Exits = store
        .get(&exits_key(guide))
        .ok()
        .flatten()
        .unwrap_or_default();
    let aspect = image::image_dimensions(super::cached_map_path(store, here))
        .map_or(1.0, |(w, h)| w as f32 / h.max(1) as f32);
    if let (Some(position), Some(exit)) =
        (position, exits.get(map_id).and_then(|e| e.get(&next.id)))
    {
        if let Some(direction) =
            gamebuddy_core::navigation::direction::bearing(position, *exit, aspect)
        {
            return Some(direction);
        }
    }
    // The drawing convention only holds for maps laid out sideways.
    if aspect < 1.3 {
        return None;
    }
    let ends = |t: &str| {
        t.split_once(" To ")
            .map(|(a, b)| (a.trim().to_owned(), b.trim().to_owned()))
    };
    let here_title = here.title.trim();
    if let Some((a, b)) = ends(here_title) {
        if next_title == a {
            return Some(ScreenDirection::Left);
        }
        if next_title == b {
            return Some(ScreenDirection::Right);
        }
    }
    if let Some((a, b)) = ends(next_title) {
        if here_title == a {
            return Some(ScreenDirection::Right);
        }
        if here_title == b {
            return Some(ScreenDirection::Left);
        }
    }
    None
}
