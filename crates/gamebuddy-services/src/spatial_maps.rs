//! Geometric registration against actual 2D atlas pixels, not map-name guesses.
use crate::{
    Store, navigation,
    spatial::{Dimension, Engine, World},
};
use anyhow::{Context, Result, ensure};
use gamebuddy_core::{
    Guide,
    spatial::{self, Features},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashSet;

#[derive(Clone, Serialize, Deserialize)]
struct Tile {
    map_id: String,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    map_width: u32,
    map_height: u32,
    features: Features,
}
#[derive(Serialize, Deserialize)]
struct Index {
    #[serde(default)]
    complete: bool,
    encoder: String,
    tiles: Vec<Tile>,
    maps: Vec<gamebuddy_core::MapAsset>,
}

pub fn prepare(store: &Store, guide: &Guide, notify: &dyn Fn(String)) -> Result<usize> {
    let result = prepare_inner(store, guide, notify);
    match &result {
        Ok(count) => store.put(
            &format!("world-map-status:{}", guide.id),
            &json!({"status":"ready","tiles":count}),
        )?,
        Err(error) => store.put(
            &format!("world-map-status:{}", guide.id),
            &json!({"status":"needs_attention","error":error.to_string()}),
        )?,
    }
    result
}
fn prepare_inner(store: &Store, guide: &Guide, notify: &dyn Fn(String)) -> Result<usize> {
    use fs2::FileExt;
    ensure!(
        crate::spatial::profile(store, guide)?.dimension == Dimension::TwoD,
        "Atlas pixel registration is for 2D games; 3D uses visual places"
    );
    let atlas = navigation::atlas(store, guide)?;
    let maps: Vec<_> = atlas
        .maps
        .into_iter()
        .filter(|m| m.enabled && navigation::cached_map_path(store, m).is_file())
        .collect();
    ensure!(!maps.is_empty(), "No cached 2D atlas maps yet");
    let directory = store.root.join("worlds").join(&guide.id);
    std::fs::create_dir_all(&directory)?;
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(directory.join("maps.lock"))?;
    lock.try_lock_exclusive()
        .context("Atlas image indexing is already running")?;
    let mut engine = Engine::learned(store, Dimension::TwoD, "maps")?;
    let key = format!("spatial-maps:{}", guide.id);
    let mut index = store
        .get::<Index>(&key)?
        .filter(|i| i.encoder == engine.encoder && i.maps == maps)
        .unwrap_or(Index {
            complete: false,
            encoder: engine.encoder.clone(),
            tiles: vec![],
            maps: maps.clone(),
        });
    let compacted = index.tiles.iter().any(|t| !t.features.points.is_empty());
    for tile in &mut index.tiles {
        tile.features.points.clear();
        tile.features.histogram.clear();
    }
    if index.complete {
        if compacted {
            store.put(&key, &index)?;
        }
        return Ok(index.tiles.len());
    }
    let mut seen: HashSet<_> = index
        .tiles
        .iter()
        .map(|t| (t.map_id.clone(), t.x, t.y, t.width, t.height))
        .collect();
    let file = tempfile::NamedTempFile::with_suffix(".png")?;
    for map in maps {
        let before = index.tiles.len();
        notify(format!("Indexing map landmarks: {}", map.title));
        store.put(
            &format!("world-map-status:{}", guide.id),
            &json!({"status":"building","current_map":map.title,"tiles":index.tiles.len()}),
        )?;
        let image = image::open(navigation::cached_map_path(store, &map))?;
        for wanted_height in [192, 384, 768] {
            let h = wanted_height.min(image.height());
            let w = (h * 4 / 3).min(image.width());
            let mut xs: Vec<_> = (0..=image.width() - w)
                .step_by((w / 2).max(1) as usize)
                .collect();
            xs.push(image.width() - w);
            let mut ys: Vec<_> = (0..=image.height() - h)
                .step_by((h / 2).max(1) as usize)
                .collect();
            ys.push(image.height() - h);
            for y in ys {
                for &x in &xs {
                    if !seen.insert((map.id.clone(), x, y, w, h)) {
                        continue;
                    }
                    ensure!(
                        index.tiles.len() < 30000,
                        "Atlas exceeds 30000 image tiles; disable unrelated maps"
                    );
                    let color = image
                        .crop_imm(x, y, w, h)
                        .resize_exact(
                            spatial::WIDTH,
                            spatial::HEIGHT,
                            image::imageops::FilterType::Triangle,
                        )
                        .to_rgb8();
                    let gray = spatial::normalize(&image::DynamicImage::ImageRgb8(color.clone()));
                    if spatial::extract(&gray).points.len() < 12 {
                        continue;
                    }
                    color.save(file.path())?;
                    let features = engine.extract(&gray, file.path())?;
                    index.tiles.push(Tile {
                        map_id: map.id.clone(),
                        x,
                        y,
                        width: w,
                        height: h,
                        map_width: image.width(),
                        map_height: image.height(),
                        features,
                    });
                }
            }
        }
        if index.tiles.len() != before {
            store.put(&key, &index)?;
        }
    }
    index.complete = true;
    store.put(&key, &index)?;
    Ok(index.tiles.len())
}

pub fn locate(
    store: &Store,
    guide: &Guide,
    world: &World,
    image: &image::DynamicImage,
) -> Result<Option<Value>> {
    if world.profile.dimension != Dimension::TwoD {
        return Ok(None);
    }
    let Some(index) = store.get_cached::<Index>(&format!("spatial-maps:{}", guide.id))? else {
        return Ok(None);
    };
    let mut engine = Engine::learned(store, Dimension::TwoD, "live")?;
    ensure!(
        index.encoder == engine.encoder,
        "Map feature model changed; prepare map pixels again"
    );
    let atlas = navigation::atlas(store, guide)?;
    let color = world
        .profile
        .background
        .apply(image)?
        .resize_exact(
            spatial::WIDTH,
            spatial::HEIGHT,
            image::imageops::FilterType::Triangle,
        )
        .to_rgb8();
    let gray = spatial::normalize(&image::DynamicImage::ImageRgb8(color.clone()));
    let file = tempfile::NamedTempFile::with_suffix(".png")?;
    color.save(file.path())?;
    let query = engine.extract(&gray, file.path())?;
    let mut ranked: Vec<_> = index
        .tiles
        .iter()
        .filter(|t| atlas.maps.iter().any(|m| m.id == t.map_id && m.enabled))
        .map(|t| (t, spatial::retrieval(&query, &t.features)))
        .collect();
    ranked.sort_by(|a, b| b.1.total_cmp(&a.1));
    let mut matches = vec![];
    for (tile, _) in ranked.into_iter().take(12) {
        if let Some(r) = engine.register(&query, &tile.features)?
            && r.inliers >= 32
            && r.coverage >= 0.3
        {
            let point = |x: f32, y: f32| {
                let p = r.transform.point(x, y);
                [
                    (tile.x as f32 + p[0] * tile.width as f32 / spatial::WIDTH as f32)
                        / tile.map_width as f32,
                    (tile.y as f32 + p[1] * tile.height as f32 / spatial::HEIGHT as f32)
                        / tile.map_height as f32,
                ]
            };
            let center = point(128., 96.);
            let polygon =
                [[0., 0.], [256., 0.], [256., 192.], [0., 192.]].map(|p| point(p[0], p[1]));
            // Reject extrapolated/off-map matches before clipping. Clipping an
            // invalid transform can otherwise look like a confident edge marker.
            if valid_projection(center, &polygon) {
                matches.push((tile, r, center, polygon));
            }
        }
    }
    matches.sort_by(|a, b| b.1.score.total_cmp(&a.1.score));
    let Some((tile, r, center, polygon)) = matches.first() else {
        return Ok(None);
    };
    // Repeated corridors within the same image are competing positions too.
    if matches.iter().skip(1).any(|(other, score, c, _)| {
        r.score - score.score < 0.07
            && (other.map_id != tile.map_id
                || ((c[0] - center[0]) * tile.map_width as f32).abs() > 64.
                || ((c[1] - center[1]) * tile.map_height as f32).abs() > 64.)
    }) {
        return Ok(None);
    }
    let minx = polygon
        .iter()
        .map(|p| p[0])
        .fold(1f32, f32::min)
        .clamp(0., 1.);
    let maxx = polygon
        .iter()
        .map(|p| p[0])
        .fold(0f32, f32::max)
        .clamp(0., 1.);
    let miny = polygon
        .iter()
        .map(|p| p[1])
        .fold(1f32, f32::min)
        .clamp(0., 1.);
    let maxy = polygon
        .iter()
        .map(|p| p[1])
        .fold(0f32, f32::max)
        .clamp(0., 1.);
    if maxx <= minx || maxy <= miny {
        return Ok(None);
    }
    let map = atlas.maps.iter().find(|m| m.id == tile.map_id).unwrap();
    Ok(Some(
        json!({"map_id":tile.map_id,"polygon":polygon,"region":{"x":minx,"y":miny,"width":maxx-minx,"height":maxy-miny},"precision":"camera_footprint","inliers":r.inliers,"residual":r.residual,"evidence":format!("{} local landmarks agree geometrically with atlas pixels; mean residual {:.2} px. This marks the visible camera area, not the player's exact position.",r.inliers,r.residual),"source_url":map.source_url,"credit":map.credit}),
    ))
}

fn valid_projection(center: [f32; 2], polygon: &[[f32; 2]; 4]) -> bool {
    if center
        .iter()
        .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
        || polygon.iter().flatten().any(|v| !v.is_finite())
    {
        return false;
    }
    let bounds = |axis| {
        let lo = polygon
            .iter()
            .map(|p| p[axis])
            .fold(f32::INFINITY, f32::min);
        let hi = polygon
            .iter()
            .map(|p| p[axis])
            .fold(f32::NEG_INFINITY, f32::max);
        (lo, hi)
    };
    let (x0, x1) = bounds(0);
    let (y0, y1) = bounds(1);
    let full = (x1 - x0) * (y1 - y0);
    let visible = (x1.min(1.) - x0.max(0.)).max(0.) * (y1.min(1.) - y0.max(0.)).max(0.);
    full > 0. && visible / full >= 0.7
}

#[derive(Serialize, Deserialize)]
struct MapTrack {
    ticket: crate::screen::Ticket,
    revision: u64,
    epoch: u64,
    at: u64,
    pending: Option<Value>,
    votes: u8,
    #[serde(default)]
    confirmed: Option<Confirmed>,
    #[serde(default)]
    profile: Option<crate::spatial::Profile>,
}
#[derive(Clone, Serialize, Deserialize)]
struct Confirmed {
    marker: Value,
    at: u64,
    signature: Vec<u8>,
}
impl Confirmed {
    fn unchanged(&self, at: u64, signature: &[u8]) -> bool {
        at.saturating_sub(self.at) <= 3_000 && crate::screen::same_scene(&self.signature, signature)
    }
    fn output(&self, at: u64) -> Value {
        let mut marker = self.marker.clone();
        marker["observed_at_ms"] = json!(self.at);
        marker["tracking_status"] = json!(if at == self.at { "matched" } else { "held" });
        marker
    }
}
fn nearby(a: &Value, b: &Value) -> bool {
    if a["map_id"] != b["map_id"] {
        return false;
    }
    let region = |v: &Value| -> Option<[f64; 4]> {
        let r = &v["region"];
        Some([
            r["x"].as_f64()?,
            r["y"].as_f64()?,
            r["width"].as_f64()?,
            r["height"].as_f64()?,
        ])
    };
    let (Some(a), Some(b)) = (region(a), region(b)) else {
        return false;
    };
    (0..2).all(|i| {
        let span = a[i + 2].max(b[i + 2]);
        span > 0.
            && a[i + 2].min(b[i + 2]) / span >= 0.5
            && ((a[i] + a[i + 2] / 2.) - (b[i] + b[i + 2] / 2.)).abs() <= span * 0.75
    })
}
/// A single plausible transform cannot teleport the overlay to another map or
/// a distant repeated corridor. Require agreement from distinct captures.
pub(crate) fn stabilize(
    store: &Store,
    guide: &Guide,
    snapshot: &crate::screen::Snapshot,
    epoch: u64,
    signature: &[u8],
    marker: Option<Value>,
) -> Result<Option<Value>> {
    let key = format!("map-track:{}", guide.id);
    let profile = crate::spatial::profile(store, guide)?;
    let prior = store.get_versioned::<MapTrack>(&key)?;
    let previous = prior.as_ref().map(|(p, _)| p).filter(|p| {
        p.ticket == snapshot.ticket
            && p.revision == guide.revision
            && p.epoch == epoch
            && p.profile.as_ref() == Some(&profile)
    });
    let at = snapshot.captured_at_ms;
    ensure!(previous.is_none_or(|p| p.at <= at), "Older map observation");
    if let Some(previous) = previous.filter(|p| p.at == at) {
        // Duplicate work is not another vote, nor a failed observation.
        return Ok(previous.confirmed.as_ref().map(|c| c.output(at)));
    }
    let unchanged = previous
        .and_then(|p| p.confirmed.as_ref())
        .filter(|c| c.unchanged(at, signature));
    let votes = if let (Some(previous), Some(marker)) = (previous, marker.as_ref()) {
        if at.saturating_sub(previous.at) <= 6_000
            && previous.pending.as_ref().is_some_and(|p| nearby(p, marker))
        {
            previous.votes.saturating_add(1)
        } else if unchanged.is_some_and(|c| nearby(&c.marker, marker)) {
            2
        } else {
            1
        }
    } else {
        u8::from(marker.is_some())
    };
    let confirmed = match marker.as_ref() {
        Some(marker) if votes >= 2 => Some(Confirmed {
            marker: marker.clone(),
            at,
            signature: signature.to_vec(),
        }),
        // Bridge a short failed registration only on unchanged pixels. Never
        // renew its timestamp, hold a contradictory match or survive a reset.
        None => unchanged.cloned(),
        _ => None,
    };
    let output = confirmed.as_ref().map(|c| c.output(at));
    let next = MapTrack {
        ticket: snapshot.ticket.clone(),
        revision: guide.revision,
        epoch,
        at: snapshot.captured_at_ms,
        pending: marker.clone(),
        votes,
        confirmed,
        profile: Some(profile),
    };
    let saved = if let Some((_, version)) = prior {
        store.replace_version(&key, &version, &next)?
    } else {
        store.put_if_absent(&key, &next)?
    };
    ensure!(saved, "Newer map observation exists");
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn stabilize(
        store: &Store,
        guide: &Guide,
        frame: &crate::screen::Snapshot,
        epoch: u64,
        marker: Option<Value>,
    ) -> Result<Option<Value>> {
        super::stabilize(store, guide, frame, epoch, &[100; 24 * 18 * 3], marker)
    }
    #[test]
    fn projection_rejects_edge_clamping_but_allows_small_camera_overscan() {
        assert!(valid_projection(
            [0.5, 0.45],
            &[[0.2, -0.1], [0.8, -0.1], [0.8, 1.0], [0.2, 1.0]]
        ));
        assert!(!valid_projection(
            [-0.1, 0.5],
            &[[-0.3, 0.], [0.1, 0.], [0.1, 1.], [-0.3, 1.]]
        ));
        assert!(!valid_projection(
            [0.1, 0.5],
            &[[-0.8, 0.], [1., 0.], [1., 1.], [-0.8, 1.]]
        ));
        assert!(!valid_projection([f32::NAN, 0.5], &[[0., 0.]; 4]));
    }
    #[test]
    fn map_jumps_require_distinct_consistent_frames_and_revoke_on_reset() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let guide = gamebuddy_core::import_markdown("Game", "Guide", "# A").unwrap();
        let mut frame = crate::screen::Snapshot {
            ticket: crate::screen::start(&store, "Game", true).unwrap(),
            captured_at_ms: 1000,
            jpeg: String::new(),
            metadata: json!({}),
        };
        let marker =
            |x| Some(json!({"map_id":"map","region":{"x":x,"y":0.1,"width":0.1,"height":0.2}}));
        assert!(
            stabilize(&store, &guide, &frame, 0, marker(0.1))
                .unwrap()
                .is_none()
        );
        assert!(
            stabilize(&store, &guide, &frame, 0, marker(0.1))
                .unwrap()
                .is_none()
        );
        frame.captured_at_ms = 4000;
        assert!(
            stabilize(&store, &guide, &frame, 0, marker(0.12))
                .unwrap()
                .is_some()
        );
        assert!(
            stabilize(&store, &guide, &frame, 0, marker(0.12))
                .unwrap()
                .is_some(),
            "Reprocessing a confirmed capture must not erase its marker"
        );
        frame.captured_at_ms = 7000;
        assert!(
            stabilize(&store, &guide, &frame, 0, marker(0.85))
                .unwrap()
                .is_none()
        );
        frame.captured_at_ms = 10000;
        assert!(
            stabilize(&store, &guide, &frame, 0, marker(0.86))
                .unwrap()
                .is_some()
        );
        frame.captured_at_ms = 13000;
        assert!(
            stabilize(&store, &guide, &frame, 1, marker(0.86))
                .unwrap()
                .is_none()
        );
        frame.captured_at_ms = 16000;
        assert!(
            stabilize(&store, &guide, &frame, 1, None)
                .unwrap()
                .is_none()
        );
        frame.captured_at_ms = 19000;
        assert!(
            stabilize(&store, &guide, &frame, 1, marker(0.86))
                .unwrap()
                .is_none()
        );
    }
    #[test]
    fn brief_misses_hold_only_unchanged_confirmed_pixels_without_refreshing_age() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let guide = gamebuddy_core::import_markdown("Game", "Guide", "# A").unwrap();
        let mut frame = crate::screen::Snapshot {
            ticket: crate::screen::start(&store, "Game", true).unwrap(),
            captured_at_ms: 1_000,
            jpeg: String::new(),
            metadata: json!({}),
        };
        let marker =
            Some(json!({"map_id":"map","region":{"x":0.1,"y":0.1,"width":0.2,"height":0.3}}));
        assert!(
            stabilize(&store, &guide, &frame, 0, marker.clone())
                .unwrap()
                .is_none()
        );
        frame.captured_at_ms = 2_000;
        let confirmed = stabilize(&store, &guide, &frame, 0, marker.clone())
            .unwrap()
            .unwrap();
        assert_eq!(confirmed["tracking_status"], "matched");
        for at in [3_000, 4_000, 5_000] {
            frame.captured_at_ms = at;
            let held = stabilize(&store, &guide, &frame, 0, None).unwrap().unwrap();
            assert_eq!(held["observed_at_ms"], 2_000);
            assert_eq!(held["tracking_status"], "held");
        }
        frame.captured_at_ms = 5_001;
        assert!(
            stabilize(&store, &guide, &frame, 0, None)
                .unwrap()
                .is_none()
        );
        frame.captured_at_ms = 6_000;
        assert!(
            stabilize(&store, &guide, &frame, 0, marker.clone())
                .unwrap()
                .is_none()
        );
        frame.captured_at_ms = 7_000;
        assert!(
            stabilize(&store, &guide, &frame, 0, marker.clone())
                .unwrap()
                .is_some()
        );
        frame.captured_at_ms = 8_000;
        assert!(
            stabilize(&store, &guide, &frame, 0, None)
                .unwrap()
                .is_some()
        );
        frame.captured_at_ms = 9_000;
        assert_eq!(
            stabilize(&store, &guide, &frame, 0, marker)
                .unwrap()
                .unwrap()["tracking_status"],
            "matched"
        );
        frame.captured_at_ms = 10_000;
        assert!(
            super::stabilize(&store, &guide, &frame, 0, &[0; 24 * 18 * 3], None)
                .unwrap()
                .is_none(),
            "Changed room pixels must revoke the old area immediately"
        );
        frame.captured_at_ms = 8_000;
        assert!(stabilize(&store, &guide, &frame, 0, None).is_err());
    }
}
