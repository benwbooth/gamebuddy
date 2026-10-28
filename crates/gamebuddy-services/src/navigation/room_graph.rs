//! How the screens of each area map connect, learned from the walkthrough
//! video placed on the maps frame by frame: the run's own path. Areas used to
//! be chained by title only, so the best GameBuddy could say inside an area
//! was a guess from how the map is drawn ("left toward Forepaw", into a
//! wall). Routing over the screens the run actually crossed gives "go right
//! one screen, then go down".
//!
//! A sample is where the map matcher put the camera for one video frame. The
//! map is cut into screens the size of that footprint (Faxanadu's area maps
//! are a grid of whole screens). Two samples in a row on neighbouring
//! screens are a walkable link, used both ways; leaving one map for another
//! marks that screen as the way out toward the other.
use crate::Store;
use anyhow::{Context, Result, ensure};
use gamebuddy_core::{Guide, navigation::direction::ScreenDirection};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};

/// Where the camera was for one video frame, as fractions of the map image.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Sample {
    pub at_ms: u64,
    pub map_id: String,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

pub fn key(guide: &Guide) -> String {
    format!("video-trace:{}", guide.id)
}

pub fn samples(store: &Store, guide: &Guide) -> Result<Vec<Sample>> {
    Ok(store
        .get_cached::<Vec<Sample>>(&key(guide))?
        .map(|s| s.as_ref().clone())
        .unwrap_or_default())
}

/// Place the walkthrough video from `start_ms` to `end_ms` on the area maps,
/// one frame every `every_ms`, and keep the samples (replacing earlier ones
/// from that stretch). Returns how many frames were placed.
pub fn trace_video(
    store: &Store,
    guide: &Guide,
    start_ms: u64,
    end_ms: u64,
    every_ms: u64,
    notify: &dyn Fn(String),
    between_frames: &dyn Fn(),
) -> Result<usize> {
    ensure!(every_ms > 0 && end_ms > start_ms, "Empty stretch of video");
    let world =
        crate::spatial::load_cached(store, guide)?.context("Map matching is not prepared")?;
    let source = guide
        .sources
        .iter()
        .find(|s| s.media_path.is_some())
        .context("No walkthrough video on disk")?;
    let crop = crate::spatial::profile(store, guide)?
        .source_crops
        .get(&source.id)
        .copied()
        .unwrap_or_default();
    let mut found = vec![];
    // A minute at a time, so the frames on disk stay few.
    let mut chunk = start_ms;
    while chunk < end_ms {
        let until = (chunk + 60_000).min(end_ms);
        let frames = tempfile::tempdir()?;
        let status = std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-ss"])
            .arg(format!("{:.3}", chunk as f64 / 1000.))
            .arg("-to")
            .arg(format!("{:.3}", until as f64 / 1000.))
            .arg("-i")
            .arg(source.media_path.as_ref().unwrap())
            .arg("-vf")
            .arg(format!("fps=1000/{every_ms}"))
            .arg(frames.path().join("%06d.png"))
            .status()?;
        ensure!(
            status.success(),
            "Could not read frames from the walkthrough video"
        );
        let mut paths: Vec<_> = std::fs::read_dir(frames.path())?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .collect();
        paths.sort();
        for (i, path) in paths.iter().enumerate() {
            between_frames();
            let image = image::open(path)?;
            let (w, h) = (image.width() as f32, image.height() as f32);
            let image = image.crop_imm(
                (crop.x * w) as u32,
                (crop.y * h) as u32,
                (crop.width * w).max(1.) as u32,
                (crop.height * h).max(1.) as u32,
            );
            let Ok(Some(marker)) = crate::spatial_maps::locate(store, guide, &world, &image) else {
                continue;
            };
            let r = &marker["region"];
            let (Some(map_id), Some(x), Some(y), Some(width), Some(height)) = (
                marker["map_id"].as_str(),
                r["x"].as_f64(),
                r["y"].as_f64(),
                r["width"].as_f64(),
                r["height"].as_f64(),
            ) else {
                continue;
            };
            found.push(Sample {
                at_ms: chunk + i as u64 * every_ms,
                map_id: map_id.to_owned(),
                x: (x + width / 2.) as f32,
                y: (y + height / 2.) as f32,
                width: width as f32,
                height: height as f32,
            });
        }
        notify(format!(
            "Tracing the walkthrough's path through the maps: {} of {} s",
            until / 1000,
            end_ms / 1000
        ));
        chunk = until;
    }
    let mut all: Vec<Sample> = samples(store, guide)?
        .into_iter()
        .filter(|s| s.at_ms < start_ms || s.at_ms >= end_ms)
        .collect();
    let placed = found.len();
    all.extend(found);
    all.sort_by_key(|s| s.at_ms);
    store.put(&key(guide), &all)?;
    Ok(placed)
}

fn covered_key(guide: &Guide) -> String {
    format!("video-trace-covered:{}", guide.id)
}

/// The stretches of video already traced, in ms.
fn covered(store: &Store, guide: &Guide) -> Vec<(u64, u64)> {
    store
        .get(&covered_key(guide))
        .ok()
        .flatten()
        .unwrap_or_default()
}

/// The next stretch to trace: around where the player is in the walkthrough
/// first (that is where they will ask the way), then the rest from the
/// start. None when the whole video is done.
fn next_stretch(
    covered: &[(u64, u64)],
    around_ms: Option<u64>,
    duration_ms: u64,
) -> Option<(u64, u64)> {
    const STRETCH_MS: u64 = 300_000;
    let done = |ms: u64| covered.iter().any(|(a, b)| *a <= ms && ms < *b);
    let mut wanted: Vec<u64> = vec![];
    if let Some(at) = around_ms {
        // The player's stretch, five minutes back and ten ahead.
        let from = at.saturating_sub(STRETCH_MS);
        wanted.extend((0..3).map(|i| from + i * STRETCH_MS));
    }
    wanted.extend((0..duration_ms.div_ceil(STRETCH_MS)).map(|i| i * STRETCH_MS));
    let start = wanted
        .into_iter()
        .find(|ms| *ms < duration_ms && !done(*ms))?;
    // Stop where an earlier stretch begins, so nothing is traced twice.
    let end = covered
        .iter()
        .map(|(a, _)| *a)
        .filter(|a| *a > start)
        .chain([start + STRETCH_MS, duration_ms])
        .min()?;
    Some((start, end))
}

/// Trace the walkthrough video in the background, a few minutes at a time,
/// keeping what is done so the next launch picks up from there. The map
/// matcher shares the GPU with the game, so while a game is on screen it
/// places one frame every couple of seconds; between sessions it runs flat
/// out. A full Faxanadu run (96 minutes) takes about 25 minutes flat out.
pub fn trace_in_background(store: &Store, guide: &Guide) {
    static RUNNING: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(vec![]);
    let Some(duration_ms) = guide
        .sources
        .iter()
        .find(|s| s.media_path.is_some())
        .and_then(|s| s.duration_ms)
    else {
        return;
    };
    let key = format!("{}:{}", store.root.display(), guide.id);
    {
        let mut running = RUNNING.lock().unwrap();
        if running.contains(&key)
            || next_stretch(&covered(store, guide), None, duration_ms).is_none()
        {
            return;
        }
        running.push(key.clone());
    }
    let root = store.root.clone();
    let guide = guide.clone();
    std::thread::spawn(move || {
        let result = Store::open(root).and_then(|store| {
            let playing = || crate::screen::status(&store).is_ok_and(|s| s["status"] == "live");
            trace_remaining(&store, &guide, &|m| eprintln!("{m}"), &|| {
                if playing() {
                    std::thread::sleep(std::time::Duration::from_secs(2));
                }
            })
        });
        if let Err(error) = result {
            eprintln!("Tracing the walkthrough's path: {error:#}");
        }
        RUNNING.lock().unwrap().retain(|k| *k != key);
    });
}

/// Trace every stretch of the walkthrough video not traced yet, the
/// player's own stretch first, recording each as it is done.
pub fn trace_remaining(
    store: &Store,
    guide: &Guide,
    notify: &dyn Fn(String),
    between_frames: &dyn Fn(),
) -> Result<()> {
    let duration_ms = guide
        .sources
        .iter()
        .find(|s| s.media_path.is_some())
        .and_then(|s| s.duration_ms)
        .context("No walkthrough video on disk")?;
    loop {
        let around = store
            .progress(guide)?
            .current_step
            .and_then(|id| guide.step(&id)?.anchors.first()?.start_ms);
        let mut done = covered(store, guide);
        let Some((start, end)) = next_stretch(&done, around, duration_ms) else {
            return Ok(());
        };
        trace_video(store, guide, start, end, 1000, notify, between_frames)?;
        done.push((start, end));
        store.put(&covered_key(guide), &done)?;
    }
}

type Cell = (i32, i32);

#[derive(Default)]
struct Graph {
    /// Screens across and down, per map.
    grids: HashMap<String, (i32, i32)>,
    links: HashMap<(String, Cell), HashSet<Cell>>,
    /// (map, the map it leads to) -> the screen the run crossed over from.
    exits: HashMap<(String, String), Cell>,
}

fn cell(grid: (i32, i32), x: f32, y: f32) -> Cell {
    (
        ((x * grid.0 as f32) as i32).clamp(0, grid.0 - 1),
        ((y * grid.1 as f32) as i32).clamp(0, grid.1 - 1),
    )
}

/// Longest gap between two samples that still counts as one walk: climbing
/// a long ladder through screens the matcher cannot place takes a few
/// seconds.
const SAME_WALK_MS: u64 = 8_000;
/// Longest gap across a change of map (doors fade out and in).
const CROSSING_MS: u64 = 20_000;

fn build(samples: &[Sample]) -> Graph {
    let mut graph = Graph::default();
    // Screen size from the footprints: the median is safe from odd matches.
    let mut sizes: HashMap<&str, Vec<(f32, f32)>> = HashMap::new();
    for s in samples {
        sizes
            .entry(&s.map_id)
            .or_default()
            .push((s.width, s.height));
    }
    for (map, mut list) in sizes {
        list.sort_by(|a, b| a.0.total_cmp(&b.0));
        let w = list[list.len() / 2].0;
        list.sort_by(|a, b| a.1.total_cmp(&b.1));
        let h = list[list.len() / 2].1;
        if w > 0. && h > 0. {
            graph.grids.insert(
                map.to_owned(),
                (
                    (1. / w).round().max(1.) as i32,
                    (1. / h).round().max(1.) as i32,
                ),
            );
        }
    }
    // Runs of samples on one map; a lone sample on another map between two
    // runs is a mismatch, not a visit.
    let mut runs: Vec<Vec<&Sample>> = vec![];
    for s in samples {
        match runs.last_mut() {
            Some(run)
                if run[0].map_id == s.map_id
                    && s.at_ms - run.last().unwrap().at_ms <= CROSSING_MS =>
            {
                run.push(s)
            }
            _ => runs.push(vec![s]),
        }
    }
    runs.retain(|run| run.len() >= 2);
    // Dropping a mismatch can leave the same map on both sides of it.
    let mut merged: Vec<Vec<&Sample>> = vec![];
    for run in runs {
        match merged.last_mut() {
            Some(last)
                if last[0].map_id == run[0].map_id
                    && run[0].at_ms - last.last().unwrap().at_ms <= CROSSING_MS =>
            {
                last.extend(run)
            }
            _ => merged.push(run),
        }
    }
    let runs = merged;
    for (i, run) in runs.iter().enumerate() {
        let Some(&grid) = graph.grids.get(&run[0].map_id) else {
            continue;
        };
        let map = &run[0].map_id;
        for pair in run.windows(2) {
            let (a, b) = (
                cell(grid, pair[0].x, pair[0].y),
                cell(grid, pair[1].x, pair[1].y),
            );
            let steps = (a.0 - b.0).abs() + (a.1 - b.1).abs();
            // A straight move of up to two screens (a long ladder crosses a
            // screen the matcher cannot place); anything else is a mismatch.
            if a != b
                && pair[1].at_ms - pair[0].at_ms <= SAME_WALK_MS
                && (a.0 == b.0 || a.1 == b.1)
                && steps <= 2
            {
                graph.links.entry((map.clone(), a)).or_default().insert(b);
                graph.links.entry((map.clone(), b)).or_default().insert(a);
            }
        }
        if let Some(next) = runs.get(i + 1)
            && next[0].map_id != *map
            && next[0].at_ms - run.last().unwrap().at_ms <= CROSSING_MS
            && let Some(&next_grid) = graph.grids.get(&next[0].map_id)
        {
            let last = run.last().unwrap();
            graph.exits.insert(
                (map.clone(), next[0].map_id.clone()),
                cell(grid, last.x, last.y),
            );
            graph.exits.insert(
                (next[0].map_id.clone(), map.clone()),
                cell(next_grid, next[0].x, next[0].y),
            );
        }
    }
    graph
}

/// One straight stretch of the way: a direction and how many screens.
#[derive(Clone, Debug, PartialEq)]
pub struct Move {
    pub direction: ScreenDirection,
    pub screens: u32,
}

/// The way from where the camera is on `map_id` to the screen the run used
/// to cross into `toward_map`, as moves; empty when already there. None
/// when the run never passed this screen or never crossed that way.
pub fn way_out(
    store: &Store,
    guide: &Guide,
    map_id: &str,
    at: [f32; 2],
    toward_map: &str,
) -> Result<Option<Vec<Move>>> {
    Ok(route(
        &build(&samples(store, guide)?),
        map_id,
        at,
        toward_map,
    ))
}

fn route(graph: &Graph, map_id: &str, at: [f32; 2], toward_map: &str) -> Option<Vec<Move>> {
    let grid = *graph.grids.get(map_id)?;
    let target = *graph
        .exits
        .get(&(map_id.to_owned(), toward_map.to_owned()))?;
    let start = cell(grid, at[0], at[1]);
    let mut came: HashMap<Cell, Cell> = HashMap::new();
    let mut queue = VecDeque::from([start]);
    let mut seen = HashSet::from([start]);
    while let Some(here) = queue.pop_front() {
        if here == target {
            let mut path = vec![here];
            while let Some(&before) = came.get(path.last().unwrap()) {
                path.push(before);
            }
            path.reverse();
            return Some(moves(&path));
        }
        for &next in graph
            .links
            .get(&(map_id.to_owned(), here))
            .into_iter()
            .flatten()
        {
            if seen.insert(next) {
                came.insert(next, here);
                queue.push_back(next);
            }
        }
    }
    None
}

fn moves(path: &[Cell]) -> Vec<Move> {
    let mut out: Vec<Move> = vec![];
    for pair in path.windows(2) {
        let (dx, dy) = (pair[1].0 - pair[0].0, pair[1].1 - pair[0].1);
        let direction = match (dx.signum(), dy.signum()) {
            (1, _) => ScreenDirection::Right,
            (-1, _) => ScreenDirection::Left,
            (_, 1) => ScreenDirection::Down,
            _ => ScreenDirection::Up,
        };
        let screens = (dx.abs() + dy.abs()) as u32;
        match out.last_mut() {
            Some(last) if last.direction == direction => last.screens += screens,
            _ => out.push(Move { direction, screens }),
        }
    }
    out
}

/// The first few moves in plain words: "go right one screen, then go down
/// one screen, then go left one screen". More than three moves end with
/// "and so on", since the player asks again from wherever they get to.
pub fn words(moves: &[Move]) -> String {
    let count = |n: u32| match n {
        1 => "one screen".to_owned(),
        2 => "two screens".to_owned(),
        3 => "three screens".to_owned(),
        n => format!("{n} screens"),
    };
    let mut parts: Vec<String> = moves
        .iter()
        .take(3)
        .map(|m| {
            let verb = match m.direction {
                ScreenDirection::Right => "go right",
                ScreenDirection::Left => "go left",
                ScreenDirection::Down => "go down",
                _ => "climb up",
            };
            format!("{verb} {}", count(m.screens))
        })
        .collect();
    let mut said = match parts.len() {
        0 => String::new(),
        1 => parts.remove(0),
        _ => {
            let last = parts.pop().unwrap();
            format!("{}, then {last}", parts.join(", then "))
        }
    };
    if moves.len() > 3 {
        said.push_str(", and ask me again from there");
    }
    said
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(at_s: u64, map: &str, col: i32, row: i32) -> Sample {
        // A 7 by 6 grid of screens, as on Faxanadu's "Mascon To Victim".
        Sample {
            at_ms: at_s * 1000,
            map_id: map.into(),
            x: (col as f32 + 0.5) / 7.,
            y: (row as f32 + 0.5) / 6.,
            width: 1. / 7.,
            height: 1. / 6.,
        }
    }

    #[test]
    fn the_run_leads_back_out_the_way_it_came_in() {
        // In from Mascon at the lower left, along the bottom, up through
        // the middle, then later into the screen the player got stuck in.
        let mut samples = vec![sample(0, "mascon", 3, 0), sample(1, "mascon", 3, 0)];
        for (t, (c, r)) in [
            (0, 4),
            (0, 5),
            (1, 5),
            (2, 5),
            (3, 5),
            (4, 5),
            (4, 4),
            (5, 4),
            (5, 3),
            (6, 3),
        ]
        .into_iter()
        .enumerate()
        {
            samples.push(sample(5 + t as u64 * 2, "mvt", c, r));
        }
        samples.extend([sample(40, "mvt", 5, 3), sample(42, "mvt", 4, 3)]);
        let graph = build(&samples);
        let at = [4.5 / 7., 3.5 / 6.];
        let moves = route(&graph, "mvt", at, "mascon").unwrap();
        assert_eq!(
            words(&moves),
            "go right one screen, then go down one screen, then go left one screen, and ask me again from there"
        );
        assert_eq!(moves.len(), 6);
        // A screen the run never crossed has no known way.
        assert!(route(&graph, "mvt", [0.5 / 7., 0.5 / 6.], "mascon").is_none());
    }

    #[test]
    fn the_players_stretch_is_traced_first_and_nothing_twice() {
        let minutes = |m: u64| m * 60_000;
        assert_eq!(
            next_stretch(&[], Some(minutes(50)), minutes(96)),
            Some((minutes(45), minutes(50)))
        );
        let done = [(minutes(45), minutes(60))];
        assert_eq!(
            next_stretch(&done, Some(minutes(50)), minutes(96)),
            Some((0, minutes(5)))
        );
        assert_eq!(
            next_stretch(
                &[(0, minutes(40)), (minutes(42), minutes(96))],
                None,
                minutes(96)
            ),
            Some((minutes(40), minutes(42)))
        );
        assert_eq!(next_stretch(&[(0, minutes(96))], None, minutes(96)), None);
    }

    #[test]
    fn a_lone_match_on_another_map_is_not_a_crossing() {
        let samples = vec![
            sample(0, "a", 0, 0),
            sample(1, "a", 1, 0),
            sample(2, "b", 2, 2),
            sample(3, "a", 2, 0),
            sample(4, "a", 3, 0),
        ];
        let graph = build(&samples);
        assert!(graph.exits.is_empty());
        assert!(graph.links[&("a".to_owned(), (1, 0))].contains(&(2, 0)));
    }
}
