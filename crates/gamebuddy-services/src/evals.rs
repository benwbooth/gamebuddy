//! The replay set and scorecard: labelled moments from real play, scored
//! automatically so every change to location, planning or voice can be
//! judged by a number instead of a feeling.
//!
//! Cases live in `<data>/evals/<guide id>/cases.json` with their frames
//! beside them. They start from moments the player told GameBuddy where they
//! were (with the frame captured at that moment); more can be added by hand.
use crate::{Store, spatial};
use anyhow::{Context, Result};
use gamebuddy_core::Guide;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Case {
    pub id: String,
    /// Frame file name inside the cases directory.
    pub image: String,
    pub at_ms: u64,
    #[serde(default)]
    pub session: String,
    /// The atlas place the player was in.
    pub expected_place: Option<String>,
    /// Human words that established the label.
    #[serde(default)]
    pub evidence: String,
    /// Where the label came from (player report, hand label).
    pub source: String,
}

pub fn dir(store: &Store, guide: &Guide) -> PathBuf {
    store.root.join("evals").join(&guide.id)
}

pub fn load(store: &Store, guide: &Guide) -> Result<Vec<Case>> {
    let path = dir(store, guide).join("cases.json");
    Ok(serde_json::from_slice(
        &std::fs::read(&path).with_context(|| format!("No replay set at {}", path.display()))?,
    )?)
}

/// Add every location the player reported with a captured frame. Existing
/// cases (and any hand edits to them) are kept.
pub fn bootstrap(store: &Store, guide: &Guide) -> Result<usize> {
    let out = dir(store, guide);
    std::fs::create_dir_all(&out)?;
    let mut cases = load(store, guide).unwrap_or_default();
    for case in cases.iter_mut().filter(|c| c.source == "player report" && c.evidence.starts_with("Screen:")) {
        case.source = "screen report".into();
    }
    let before = cases.len();
    for (id, body) in store.events_of(guide, "player_location")? {
        let value: Value = serde_json::from_str(&body)?;
        let Some(path) = value["journal_image"]["path"].as_str().map(Path::new) else { continue };
        let case_id = format!("report-{id}");
        if !path.is_file() || cases.iter().any(|c| c.id == case_id) {
            continue;
        }
        let image = format!("{case_id}.jpg");
        std::fs::copy(path, out.join(&image))?;
        cases.push(Case {
            id: case_id,
            image,
            at_ms: value["journal_image"]["captured_at_ms"].as_u64().unwrap_or(0),
            session: value["journal_image"]["session"].as_str().unwrap_or_default().into(),
            expected_place: value["location"]["place_id"].as_str().map(str::to_owned),
            evidence: value["report"].as_str().unwrap_or_default().into(),
            // "Screen: …" reports were GameBuddy's own reading of the screen,
            // not the player's words, so they are not ground truth.
            source: if value["report"].as_str().is_some_and(|r| r.starts_with("Screen:")) {
                "screen report".into()
            } else {
                "player report".into()
            },
        });
    }
    std::fs::write(out.join("cases.json"), serde_json::to_vec_pretty(&cases)?)?;
    Ok(cases.len() - before)
}

/// Score visual location on every case: the frame alone, matched against the
/// walkthrough's reference views, resolved to an atlas place.
pub fn score_location(store: &Store, guide: &Guide) -> Result<Value> {
    let cases = load(store, guide)?;
    let atlas = crate::navigation::atlas(store, guide)?;
    let world = spatial::load(store, guide)?.context("Prepare the world first")?;
    let mut engine = spatial::Engine::for_world(store, &world, "evaluation")?;
    let base = dir(store, guide);
    // Cases from one session share a TV bezel/shader: find the lit screen
    // per session and crop to it, as the live path does.
    let rectify = std::env::var_os("GAMEBUDDY_EVAL_RAW").is_none();
    let mut bounds: std::collections::HashMap<String, Option<gamebuddy_core::Crop>> = Default::default();
    if rectify {
        let mut sessions: std::collections::HashMap<String, Vec<image::DynamicImage>> = Default::default();
        for case in &cases {
            if let Ok(img) = image::open(base.join(&case.image)) {
                let key = format!("{}x{}", img.width(), img.height());
                sessions.entry(key).or_default().push(img);
            }
        }
        for (key, frames) in sessions {
            bounds.insert(key, crate::screen::screen_bounds(&frames));
        }
    }
    let (mut right, mut wrong, mut abstained, mut unlabelled) = (0, 0, 0, 0);
    let mut same_area = 0;
    // Places on the same map (a town and its shops) share an area.
    let area = |id: &str| {
        atlas.places.iter().find(|p| p.id == id).map(|p| {
            p.map_id.clone().unwrap_or_else(|| p.label.split_whitespace().next().unwrap_or_default().to_lowercase())
        })
    };
    let mut seconds = vec![];
    let mut misses = vec![];
    for case in &cases {
        let trusted = case.source != "screen report";
        let Some(expected) = case.expected_place.as_deref().filter(|p| trusted && atlas.places.iter().any(|x| x.id == *p)) else {
            unlabelled += 1;
            continue;
        };
        let started = std::time::Instant::now();
        // Each case stands alone: a fresh tracker, as on arrival, shown the
        // frame a few times as if the player stood there for two seconds
        // (the tracker wants agreeing frames before it commits).
        let mut tracker = gamebuddy_core::spatial::Tracker::default();
        let mut frame = image::open(base.join(&case.image))?;
        if let Some(Some(crop)) = bounds.get(&format!("{}x{}", frame.width(), frame.height())) {
            frame = crop.apply(&frame)?;
        }
        let mut result = Value::Null;
        for k in 0..4u64 {
            result = spatial::locate(
                &world,
                guide,
                &frame,
                &mut tracker,
                spatial::FrameContext { session: "evaluation", epoch: 0, at_ms: case.at_ms.max(1) + k * 600 },
                &mut engine,
            )?;
            if result["tracker"]["room_id"].is_string() {
                break;
            }
        }
        seconds.push(started.elapsed().as_secs_f64());
        let steps: Vec<&str> = result["step_ids"]
            .as_array()
            .into_iter()
            .flatten()
            .chain(result["related_step_ids"].as_array().into_iter().flatten())
            .filter_map(Value::as_str)
            .collect();
        let predicted = steps.iter().find_map(|s| atlas.place_for_step(guide, s)).map(|p| p.id.as_str());
        let label = |id: Option<&str>| {
            id.and_then(|id| atlas.places.iter().find(|p| p.id == id)).map(|p| p.label.clone())
        };
        match predicted {
            Some(p) if p == expected => right += 1,
            Some(p) => {
                wrong += 1;
                if area(p).is_some() && area(p) == area(expected) {
                    same_area += 1;
                }
                misses.push(json!({"case": case.id, "expected": label(Some(expected)), "predicted": label(Some(p)), "evidence": case.evidence}));
            }
            None => {
                abstained += 1;
                misses.push(json!({"case": case.id, "expected": label(Some(expected)), "predicted": null, "evidence": case.evidence}));
            }
        }
    }
    let scored = right + wrong + abstained;
    seconds.sort_by(f64::total_cmp);
    Ok(json!({
        "cases": cases.len(),
        "scored": scored,
        "unlabelled": unlabelled,
        "right": right,
        "wrong": wrong,
        "abstained": abstained,
        "accuracy": if scored > 0 { right as f64 / scored as f64 } else { 0. },
        "area_accuracy": if scored > 0 { (right + same_area) as f64 / scored as f64 } else { 0. },
        "median_seconds": seconds.get(seconds.len() / 2),
        "rectified": rectify,
        "misses": misses,
    }))
}

/// Latency and reliability across the most recent recorded sessions.
pub fn score_sessions(root: &Path, limit: usize) -> Value {
    let reports: Vec<Value> = crate::flight::sessions(root)
        .into_iter()
        .take(limit)
        .filter_map(|dir| crate::flight::report(&dir).ok())
        .collect();
    let pick = |path: &[&str]| -> Vec<f64> {
        reports
            .iter()
            .filter_map(|r| {
                let mut v = r;
                for key in path {
                    v = &v[*key];
                }
                v.as_f64()
            })
            .collect()
    };
    json!({
        "sessions": reports.len(),
        "reply_start_median_s": pick(&["voice", "reply_start_s", "median"]),
        "delegated_answer_median_s": pick(&["voice", "delegated_answer_s", "median"]),
        "delegated_answer_max_s": pick(&["voice", "delegated_answer_s", "max"]),
        "rate_limited_calls": pick(&["rate_limited_calls"]),
        "minutes": pick(&["minutes"]),
    })
}

/// The whole scorecard: location per guide with a replay set, and latency
/// from recent sessions.
pub fn scorecard(store: &Store) -> Result<Value> {
    let mut guides = vec![];
    for guide in store.guides()? {
        if !dir(store, &guide).join("cases.json").is_file() {
            continue;
        }
        let location = score_location(store, &guide).unwrap_or_else(|e| json!({"error": format!("{e:#}")}));
        // The vision model costs a call per case; opt in.
        let vision = std::env::var_os("GAMEBUDDY_EVAL_VISION")
            .map(|_| score_location_vision(store, &guide).unwrap_or_else(|e| json!({"error": format!("{e:#}")})));
        guides.push(json!({"game": guide.game, "guide": guide.id, "location": location, "location_vision": vision}));
    }
    Ok(json!({"guides": guides, "latency": score_sessions(&store.root, 10)}))
}

/// Score the vision model reading the frame directly: shown the screen and
/// the guide's places, which one is it? One model call per case.
pub fn score_location_vision(store: &Store, guide: &Guide) -> Result<Value> {
    let cases = load(store, guide)?;
    let atlas = crate::navigation::atlas(store, guide)?;
    let settings = store.get::<crate::Settings>("settings")?.unwrap_or_default();
    let provider = crate::HttpProvider::new(settings.vision.clone())?;
    let base = dir(store, guide);
    let catalog: Vec<Value> = atlas
        .places
        .iter()
        .map(|p| json!({"id": p.id, "label": p.label, "landmarks": p.landmarks.chars().take(120).collect::<String>()}))
        .collect();
    let area = |id: &str| {
        atlas.places.iter().find(|p| p.id == id).map(|p| {
            p.map_id.clone().unwrap_or_else(|| p.label.split_whitespace().next().unwrap_or_default().to_lowercase())
        })
    };
    let (mut right, mut same_area, mut wrong) = (0, 0, 0);
    let mut seconds = vec![];
    let mut misses = vec![];
    for case in cases.iter().filter(|c| c.source != "screen report") {
        let Some(expected) = case.expected_place.as_deref().filter(|p| atlas.places.iter().any(|x| x.id == *p)) else { continue };
        let started = std::time::Instant::now();
        let answer = crate::ReasoningProvider::complete(
            &provider,
            crate::ModelRequest {
                system: "You identify where a player is in a video game from a screenshot. The screenshot may show a TV bezel or CRT effect; judge the game picture. Return JSON only.".into(),
                prompt: format!(
                    "Game: {}. Which of these places does the screenshot show? Places: {}. Reply {{\"place_id\": one id from the list, \"confidence\": 0 to 1}}.",
                    guide.game,
                    serde_json::to_string(&catalog)?
                ),
                images: vec![crate::image_base64(base.join(&case.image))?],
                json: true,
            },
        );
        seconds.push(started.elapsed().as_secs_f64());
        let failure = answer.as_ref().err().map(|e| format!("{e:#}").chars().take(160).collect::<String>());
        let predicted = answer
            .ok()
            .and_then(|t| crate::parse_json::<Value>(&t).ok())
            .and_then(|v| v["place_id"].as_str().map(str::to_owned));
        let label = |id: &str| atlas.places.iter().find(|p| p.id == id).map(|p| p.label.clone());
        match predicted.as_deref() {
            Some(p) if p == expected => right += 1,
            Some(p) => {
                wrong += 1;
                if area(p).is_some() && area(p) == area(expected) {
                    same_area += 1;
                }
                misses.push(json!({"case": case.id, "expected": label(expected), "predicted": label(p), "evidence": case.evidence}));
            }
            None => {
                wrong += 1;
                misses.push(json!({"case": case.id, "expected": label(expected), "predicted": null, "error": failure}));
            }
        }
    }
    let scored = right + wrong;
    seconds.sort_by(f64::total_cmp);
    Ok(json!({
        "model": format!("{:?} {}", settings.vision.kind, settings.vision.model),
        "scored": scored,
        "right": right,
        "accuracy": if scored > 0 { right as f64 / scored as f64 } else { 0. },
        "area_accuracy": if scored > 0 { (right + same_area) as f64 / scored as f64 } else { 0. },
        "median_seconds": seconds.get(seconds.len() / 2),
        "misses": misses,
    }))
}
