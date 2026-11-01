//! Gather every source of the player's location into one fused estimate
//! (see `gamebuddy_core::navigation::estimate`), smooth its map position
//! over time and remember it so the flight recorder sees each change.
use crate::Store;
use anyhow::Result;
use gamebuddy_core::{
    Atlas, Guide, PlaceTracker,
    navigation::estimate::{Evidence, EvidenceKind, LocationEstimate, MapPosition, fuse},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub(super) struct Inputs<'a> {
    pub atlas: &'a Atlas,
    pub guide: &'a Guide,
    pub tracker: &'a PlaceTracker,
    pub reported_view_unchanged: bool,
    pub observation: &'a Value,
    pub geometric_marker: Option<&'a Value>,
    pub held_marker: Option<&'a Value>,
    pub target_place_id: Option<&'a str>,
    pub session: &'a str,
    pub now: u64,
}

#[derive(Serialize, Deserialize)]
struct Remembered {
    session: String,
    estimate: LocationEstimate,
}

pub fn key(guide: &Guide) -> String {
    format!("location-estimate:{}", guide.id)
}

fn marker_position(marker: &Value, strength_scale: f32) -> Option<(MapPosition, f32)> {
    let map_id = marker["map_id"].as_str()?.to_owned();
    let r = &marker["region"];
    let region = [
        r["x"].as_f64()? as f32,
        r["y"].as_f64()? as f32,
        r["width"].as_f64()? as f32,
        r["height"].as_f64()? as f32,
    ];
    let inliers = marker["inliers"].as_f64().unwrap_or(20.) as f32;
    let strength = (0.5 + inliers / 60.).min(1.0) * strength_scale;
    Some((
        MapPosition {
            map_id,
            x: region[0] + region[2] / 2.,
            y: region[1] + region[3] / 2.,
            region: Some(region),
        },
        strength,
    ))
}

/// The one place drawn on a map, when the atlas has exactly one.
fn place_on_map(atlas: &Atlas, map_id: &str) -> Option<String> {
    let mut places = atlas
        .places
        .iter()
        .filter(|p| p.map_id.as_deref() == Some(map_id));
    let first = places.next()?;
    places.next().is_none().then(|| first.id.clone())
}

pub(super) fn evidence(inputs: &Inputs<'_>) -> Vec<Evidence> {
    let mut out = vec![];
    let atlas = inputs.atlas;
    let tracker = inputs.tracker;
    if tracker.status == "reported"
        && let Some(place) = &tracker.place_id
    {
        out.push(Evidence {
            kind: EvidenceKind::PlayerReport,
            place_id: Some(place.clone()),
            map_id: atlas
                .places
                .iter()
                .find(|p| &p.id == place)
                .and_then(|p| p.map_id.clone()),
            position: None,
            at_ms: tracker.located_at_ms,
            // A report holds its full weight while the scene has not changed.
            strength: if inputs.reported_view_unchanged {
                1.0
            } else {
                0.6
            },
            detail: tracker.detail.clone(),
        });
    } else if tracker.status == "estimated"
        && let Some(place) = &tracker.place_id
    {
        out.push(Evidence {
            kind: EvidenceKind::VisualRoom,
            place_id: Some(place.clone()),
            map_id: atlas
                .places
                .iter()
                .find(|p| &p.id == place)
                .and_then(|p| p.map_id.clone()),
            position: None,
            at_ms: tracker.located_at_ms,
            strength: 0.6,
            detail: tracker.detail.clone(),
        });
    }
    for (marker, scale) in inputs
        .geometric_marker
        .map(|m| (m, 1.0))
        .into_iter()
        .chain(inputs.held_marker.map(|m| (m, 0.5)))
    {
        if let Some((position, strength)) = marker_position(marker, scale) {
            out.push(Evidence {
                kind: EvidenceKind::MapRegistration,
                place_id: place_on_map(atlas, &position.map_id),
                map_id: Some(position.map_id.clone()),
                position: Some(position),
                at_ms: marker["observed_at_ms"].as_u64().unwrap_or(inputs.now),
                strength,
                detail: marker["evidence"].as_str().unwrap_or_default().to_owned(),
            });
        }
    }
    let visual = &inputs.observation["tracker"];
    if visual["session"] == inputs.session
        && let Some(at) = visual["observed_at_ms"].as_u64()
        && let Some(strength) = match visual["status"].as_str() {
            Some("tracked") => Some(0.9),
            Some("candidate") => Some(0.4),
            _ => None,
        }
    {
        let places: std::collections::BTreeSet<&str> = inputs.observation["step_ids"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .filter_map(|id| atlas.place_for_step(inputs.guide, id))
            .map(|p| p.id.as_str())
            .collect();
        if places.len() == 1 {
            let place = places.into_iter().next().unwrap();
            out.push(Evidence {
                kind: EvidenceKind::VisualRoom,
                place_id: Some(place.to_owned()),
                map_id: atlas
                    .places
                    .iter()
                    .find(|p| p.id == place)
                    .and_then(|p| p.map_id.clone()),
                position: None,
                at_ms: at,
                strength,
                detail: format!(
                    "Screen matched walkthrough frames of room {}",
                    visual["room_id"].as_str().unwrap_or("?")
                ),
            });
        }
    }
    if let Some(target) = inputs.target_place_id
        && let Some(place) = atlas.places.iter().find(|p| p.id == target)
    {
        out.push(Evidence {
            kind: EvidenceKind::PlanExpectation,
            place_id: Some(place.id.clone()),
            map_id: place.map_id.clone(),
            position: None,
            at_ms: 0,
            strength: 1.0,
            detail: format!(
                "The walkthrough's current step takes place in {}",
                place.label
            ),
        });
    }
    out
}

/// Fuse the evidence, smooth the position against the remembered estimate
/// and store the result (a change is one line in the flight recorder).
pub(super) fn update(store: &Store, inputs: &Inputs<'_>) -> Result<LocationEstimate> {
    let mut estimate = fuse(&evidence(inputs), inputs.now);
    let key = key(inputs.guide);
    let previous = store
        .get::<Remembered>(&key)?
        .filter(|r| r.session == inputs.session)
        .map(|r| r.estimate);
    // A momentary loss (a screen transition, a frame the matcher could not
    // place) should not wipe a location seen a few seconds ago: hold it as
    // "likely" for up to 20 s.
    if !estimate.is_known()
        && let Some(last) = previous
            .as_ref()
            .filter(|p| p.is_known() && inputs.now.saturating_sub(p.located_at_ms) < 20_000)
    {
        estimate = LocationEstimate {
            status: "likely".into(),
            confidence: last.confidence.min(0.6),
            ..last.clone()
        };
    }
    if let (Some(current), Some(last)) = (
        estimate.position.as_mut(),
        previous.as_ref().and_then(|p| p.position.as_ref()),
    ) && last.map_id == current.map_id
        && inputs
            .now
            .saturating_sub(previous.as_ref().map_or(0, |p| p.located_at_ms))
            < 6_000
        && (last.x - current.x).abs() < 0.15
        && (last.y - current.y).abs() < 0.15
    {
        // A small step on the same map: average with where the dot was, so
        // it glides instead of jittering between neighbouring matches.
        current.x = 0.5 * (last.x + current.x);
        current.y = 0.5 * (last.y + current.y);
    }
    // Crossing from one area map to another: where the player left the old
    // map is that map's exit toward the new one. Learned from real play, it
    // lets the arrow point at the way out when routing back.
    if let (Some(last), Some(now_map)) = (previous.as_ref(), estimate.map_id.as_ref())
        && let (Some(left_at), Some(old_map)) = (last.position.as_ref(), last.map_id.as_ref())
        && old_map != now_map
        && last.is_known()
        && estimate.is_known()
        && inputs.now.saturating_sub(last.located_at_ms) < 20_000
    {
        super::routing::learn_exit(
            store,
            inputs.guide,
            old_map,
            now_map,
            [left_at.x, left_at.y],
        )?;
    }
    let changed = previous.as_ref().is_none_or(|p| {
        p.status != estimate.status
            || p.place_id != estimate.place_id
            || p.map_id != estimate.map_id
            || p.position != estimate.position
            || (p.confidence - estimate.confidence).abs() > 0.05
    });
    // A replay holds the recorded location; its frame shows the past.
    if changed && std::env::var_os("GAMEBUDDY_PROBE_ESTIMATE").is_none() {
        store.put(
            &key,
            &Remembered {
                session: inputs.session.to_owned(),
                estimate: estimate.clone(),
            },
        )?;
    }
    Ok(estimate)
}
