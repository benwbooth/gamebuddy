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
    let Some(index) = store.get::<Index>(&format!("spatial-maps:{}", guide.id))? else {
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
            matches.push((tile, r, center, polygon));
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
