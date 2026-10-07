//! Shared location updates from conversation and continuous visual evidence.
use super::{atlas, context, tracker_key};
use crate::{Store, screen};
use anyhow::{Result, ensure};
use gamebuddy_core::{Guide, PlaceTracker, SyncEstimate};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct LocationReport {
    #[serde(default)]
    pub place_id: String,
    /// Exact area name stated by the player when no known place matches. Do
    /// not substitute a named shop/interior for a town-level report.
    pub place_label: Option<String>,
    /// Only supply a task when the player states it and the atlas binds it to
    /// this place. Omit for an arrival/place report; that never guesses a task.
    pub step_id: Option<String>,
    pub player_report: String,
    pub expected_revision: u64,
    pub epoch: u64,
}

#[derive(Serialize, Deserialize)]
struct ReportedView {
    place_id: String,
    frame: screen::MovementFrame,
}

fn reported_view_key(guide: &Guide) -> String {
    format!("reported-location-view:{}", guide.id)
}

/// Similar-looking interiors must not immediately overrule a player correction.
/// The report yields to visual tracking after an actual change in the scene.
pub(super) fn reported_view_unchanged(
    store: &Store,
    guide: &Guide,
    tracker: &PlaceTracker,
) -> Result<bool> {
    if tracker.status != "reported" {
        return Ok(false);
    }
    let key = reported_view_key(guide);
    let Some((Some(saved), version)) = store.get_versioned::<Option<ReportedView>>(&key)? else {
        return Ok(false);
    };
    if tracker.place_id.as_deref() != Some(&saved.place_id)
        || tracker.session_id != saved.frame.ticket.session
    {
        return Ok(false);
    }
    let Some(current) = screen::movement_frame(store, screen::now_ms())? else {
        return Ok(true);
    };
    let unchanged = saved.frame.ticket == current.ticket
        && screen::same_scene(&saved.frame.frame_signature, &current.frame_signature);
    if !unchanged {
        store.replace_version(&key, &version, &Option::<ReportedView>::None)?;
    }
    Ok(unchanged)
}

pub fn report_location(store: &Store, guide: &Guide, mut report: LocationReport) -> Result<Value> {
    ensure!(
        report.expected_revision == guide.revision,
        "Outline changed; read the current context"
    );
    ensure!(
        !report.player_report.trim().is_empty() && report.player_report.len() <= 1000,
        "Include the player's own short location statement"
    );
    let atlas = atlas(store, guide)?;
    let new_place = if report.place_id.is_empty() {
        let label = report.place_label.as_deref().unwrap_or_default().trim();
        ensure!(
            !label.is_empty() && label.len() <= 120,
            "Supply the player's exact area name in place_label when its place ID is unknown"
        );
        ensure!(
            report
                .player_report
                .to_lowercase()
                .contains(&label.to_lowercase()),
            "Use an area name actually stated in player_report"
        );
        ensure!(
            report.step_id.is_none(),
            "An unmapped area report cannot establish a task"
        );
        if let Some(place) = atlas
            .places
            .iter()
            .find(|p| p.label.eq_ignore_ascii_case(label))
        {
            report.place_id = place.id.clone();
            None
        } else {
            let place = gamebuddy_core::Place {
                id: format!("reported-{}", uuid::Uuid::new_v4()),
                label: label.into(),
                map_id: None,
                landmarks: "Player-reported area; no specific interior or coordinates established."
                    .into(),
            };
            report.place_id = place.id.clone();
            Some(place)
        }
    } else {
        None
    };
    ensure!(
        new_place.is_some() || atlas.places.iter().any(|p| p.id == report.place_id),
        "Unknown place; read get_atlas first"
    );
    let progress = store.progress(guide)?;
    let valid = |id: &str| {
        atlas
            .place_for_step(guide, id)
            .is_some_and(|p| p.id == report.place_id)
    };
    if let Some(id) = report.step_id.as_deref() {
        ensure!(
            valid(id),
            "Task is not bound to the reported place. Omit step_id to record the place alone, or choose a task explicitly bound to this place in get_atlas."
        );
    }
    let mut tracker = PlaceTracker::for_session(
        screen::status(store)?["session_id"]
            .as_str()
            .unwrap_or_default()
            .into(),
    );
    if let Some(step) = report.step_id.as_deref() {
        tracker.report(
            &report.place_id,
            step,
            progress.epoch,
            screen::now_ms(),
            &report.player_report,
        );
    } else {
        tracker.report_place(
            &report.place_id,
            progress.epoch,
            screen::now_ms(),
            &report.player_report,
        );
    }
    let progress = store.report_location(
        guide,
        report.epoch,
        tracker,
        &report.player_report,
        new_place,
    )?;
    if let Some(frame) = screen::movement_frame(store, screen::now_ms())? {
        store.put(
            &reported_view_key(guide),
            &ReportedView {
                place_id: report.place_id,
                frame,
            },
        )?;
    }
    Ok(json!({"progress":progress,"evidence_kind":"player_report",
        "navigation":context(store, guide, &progress, &SyncEstimate::default())?}))
}

/// Keep both the overlay and MCP on the same tracker. A screenshot observer can
/// propose only a known source-backed place, and needs distinct repeated frames.
pub fn observe_place(
    store: &Store,
    guide: &Guide,
    snapshot: &screen::Snapshot,
    location: Option<&screen::SceneLocation>,
    expected_epoch: u64,
) -> Result<()> {
    let location = location.filter(|l| {
        l.confidence.is_finite() && l.confidence >= 0.85 && !l.evidence.trim().is_empty()
    });
    ensure!(
        screen::ticket(store, &snapshot.ticket.session)? == snapshot.ticket,
        "Capture changed"
    );
    ensure!(
        screen::now_ms().saturating_sub(snapshot.captured_at_ms) <= screen::SCENE_FRESH_MS,
        "Location observation is stale"
    );
    ensure!(
        store.guide(&guide.id)?.revision == guide.revision,
        "Outline changed"
    );
    let atlas = atlas(store, guide)?;
    let progress = store.progress(guide)?;
    ensure!(
        progress.epoch == expected_epoch,
        "Newer player report exists"
    );
    let step = location.and_then(|location| {
        let place_id = &location.place_id;
        progress
            .current_step
            .as_deref()
            .filter(|id| {
                atlas
                    .place_for_step(guide, id)
                    .is_some_and(|p| p.id == *place_id)
            })
            .or_else(|| {
                atlas
                    .bindings
                    .iter()
                    .find(|b| {
                        b.place_id == *place_id
                            && guide
                                .step(&b.step_id)
                                .is_some_and(|s| guide.step_enabled(s))
                    })
                    .map(|b| b.step_id.as_str())
            })
    });
    let key = format!("scene-{}", tracker_key(guide));
    let before = store.get_versioned::<PlaceTracker>(&key)?;
    let mut tracker = before.as_ref().map(|(t, _)| t.clone()).unwrap_or_default();
    if tracker.session_id != snapshot.ticket.session {
        tracker = PlaceTracker::for_session(snapshot.ticket.session.clone());
    }
    let estimate = SyncEstimate {
        epoch: progress.epoch,
        observed_at_ms: snapshot.captured_at_ms,
        // A single scene description is a candidate, even if the model claims
        // high confidence. PlaceTracker requires two separate observations.
        candidates: step
            .map(|step| gamebuddy_core::Candidate {
                step_id: step.into(),
                reference_id: String::new(),
                source_id: String::new(),
                reference_ms: 0,
                image_path: String::new(),
                visual_score: 0.9,
                rank_score: 0.9,
            })
            .into_iter()
            .collect(),
        ..Default::default()
    };
    tracker.observe(guide, &atlas, &estimate, progress.epoch, screen::now_ms());
    if tracker.status == "estimated"
        && tracker.located_at_ms == snapshot.captured_at_ms
        && let Some(location) = location
    {
        tracker.detail = format!("Visual observation: {}", location.evidence);
    }
    if let Some((_, version)) = before {
        store.replace_version(&key, &version, &tracker)?;
    } else {
        store.put_if_absent(&key, &tracker)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::call_tool;
    use gamebuddy_core::{Atlas, MapAsset, Place, PlaceBinding, Progress};

    #[test]
    fn an_unknown_area_report_does_not_guess_an_interior_or_task() -> Result<()> {
        let (_dir, store, guide) = fixture();
        let result = call_tool(
            &store,
            "report_location",
            json!({"place_label":"North Town", "player_report":"I am in North Town", "expected_revision":guide.revision,"epoch":0}),
        )?;
        assert_eq!(
            result["navigation"]["map"]["current_place"]["label"],
            "North Town"
        );
        assert!(result["navigation"]["location"]["reference_step_id"].is_null());
        assert!(result["navigation"]["guidance"]["warning"].is_null());
        assert!(store.progress(&guide)?.current_step.is_none());
        let count = atlas(&store, &guide)?.places.len();
        call_tool(
            &store,
            "report_location",
            json!({"place_label":"North Town", "player_report":"Still in North Town", "expected_revision":guide.revision,"epoch":1}),
        )?;
        assert_eq!(atlas(&store, &guide)?.places.len(), count);
        assert!(call_tool(&store, "report_location", json!({"place_label":"Invented Room", "player_report":"I am in town", "expected_revision":guide.revision,"epoch":2})).is_err());
        assert_eq!(atlas(&store, &guide)?.places.len(), count);
        Ok(())
    }

    #[test]
    fn player_correction_survives_repeated_matches_until_the_view_changes() -> Result<()> {
        let (dir, store, guide) = fixture();
        let ticket = screen::start(&store, &guide.game, true)?;
        let path = dir.path().join("frame.png");
        image::RgbImage::from_pixel(96, 64, image::Rgb([200, 20, 30])).save(&path)?;
        screen::publish(&store, &ticket, &path, "fixture")?;
        report(&store, &guide, "church", "step-1", 0)?;
        let progress = store.progress(&guide)?;
        let now = screen::now_ms();
        let estimate = |offset| SyncEstimate {
            epoch: progress.epoch,
            observed_at_ms: now + offset,
            located_step: Some("step-2".into()),
            status: "located".into(),
            ..Default::default()
        };
        for offset in 1..=3 {
            let nav = context(&store, &guide, &progress, &estimate(offset))?;
            assert_eq!(nav["location"]["status"], "reported");
            assert_eq!(nav["map"]["current_place"]["id"], "church");
        }
        image::RgbImage::from_pixel(96, 64, image::Rgb([10, 200, 240])).save(&path)?;
        screen::publish(&store, &ticket, &path, "fixture")?;
        context(&store, &guide, &progress, &estimate(4))?;
        let nav = context(&store, &guide, &progress, &estimate(5))?;
        assert_eq!(nav["map"]["current_place"]["id"], "shop");
        assert_eq!(nav["location"]["status"], "estimated");
        Ok(())
    }

    fn fixture() -> (tempfile::TempDir, Store, Guide) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let guide = gamebuddy_core::import_markdown(
            "Test",
            "Route",
            "# Church\nTalk to the guru\n# Shop\nBuy the key\n# Forest\nEnter the forest",
        )
        .unwrap();
        store.save_guide(&guide).unwrap();
        store.put("active_guide", &guide.id).unwrap();
        let mut atlas = Atlas {
            guide_id: guide.id.clone(),
            guide_revision: guide.revision,
            status: "ready".into(),
            ..Default::default()
        };
        for (i, name) in ["church", "shop", "forest"].iter().enumerate() {
            atlas.maps.push(MapAsset {
                id: name.to_string(),
                title: name.to_string(),
                url: format!("https://example.org/{name}.png"),
                source_url: "https://example.org/maps".into(),
                credit: "Fixture".into(),
                enabled: true,
            });
            atlas.places.push(Place {
                id: name.to_string(),
                label: name.to_string(),
                map_id: Some(name.to_string()),
                landmarks: name.to_string(),
            });
            atlas.bindings.push(PlaceBinding {
                step_id: guide.steps[i].id.clone(),
                place_id: name.to_string(),
                evidence: "Fixture source".into(),
            });
        }
        store.put(&format!("atlas:{}", guide.id), &atlas).unwrap();
        (dir, store, guide)
    }
    fn report(store: &Store, guide: &Guide, place: &str, step: &str, epoch: u64) -> Result<Value> {
        call_tool(
            store,
            "report_location",
            json!({"place_id":place,"step_id":step,"player_report":format!("I am at the {place}"),"expected_revision":guide.revision,"epoch":epoch}),
        )
    }
    #[test]
    fn arrival_report_replaces_old_advice_without_guessing_an_earlier_task() {
        let (_dir, store, guide) = fixture();
        screen::start(&store, &guide.game, true).unwrap();
        let progress = Progress {
            current_step: Some("step-3".into()),
            ..Default::default()
        };
        store.save_progress(&guide, &progress, "fixture").unwrap();
        // Old advice from before route enforcement must still expire on arrival.
        let session = screen::status(&store).unwrap()["session_id"]
            .as_str()
            .unwrap()
            .to_owned();
        store.put(&format!("next-action:{}", guide.id), &json!({"revision":guide.revision,"epoch":0,"session":session,"updated_at_ms":screen::now_ms(),"step_id":"step-1","instructions":"Walk left to the church."})).unwrap();
        let result = call_tool(&store,"report_location",json!({"expected_revision":guide.revision,"epoch":0,"place_id":"church","player_report":"I am already at the church."})).unwrap();
        assert_eq!(result["navigation"]["location"]["place_id"], "church");
        assert_eq!(result["navigation"]["location"]["status"], "reported");
        assert!(result["navigation"]["location"]["reference_step_id"].is_null());
        assert!(result["navigation"]["recommendation"].is_null());
        assert_eq!(result["navigation"]["guidance"]["status"], "plan");
        assert!(result["navigation"]["direction"].is_null());
        assert_eq!(result["progress"]["current_step"], "step-3");
        assert!(
            store
                .progress(&guide)
                .unwrap()
                .reported_completed
                .is_empty()
        );
    }
    #[test]
    fn a_current_player_location_can_support_advice_past_an_old_route_goal() {
        let (_dir, store, mut guide) = fixture();
        guide.goals = vec![gamebuddy_core::PlayerGoal {
            id: "old-route".into(),
            description: "Visit the forest".into(),
            step_ids: vec!["step-3".into()],
        }];
        store.save_guide(&guide).unwrap();
        screen::start(&store, &guide.game, true).unwrap();
        report(&store, &guide, "shop", "step-2", 0).unwrap();
        let progress = store.progress(&guide).unwrap();
        let result=call_tool(&store,"set_next_action",json!({"expected_revision":guide.revision,"epoch":progress.epoch,"step_id":"step-2","instructions":"Buy the key from the shopkeeper."})).unwrap();
        assert_eq!(result["navigation"]["guidance"]["now"]["id"], "step-2");
        assert!(store.progress(&guide).unwrap().completed.is_empty());
        let search = call_tool(
            &store,
            "search_guide",
            json!({"query":"I am in the shop. What should I do?"}),
        )
        .unwrap();
        assert_eq!(search["places_by_step"]["step-2"]["id"], "shop");
    }
    #[test]
    fn player_location_drives_map_and_cursor_survives_uncertainty_and_completion() {
        let (_dir, store, guide) = fixture();
        screen::start(&store, &guide.game, true).unwrap();
        let result = report(&store, &guide, "church", "step-1", 0).unwrap();
        assert_eq!(result["navigation"]["location"]["status"], "reported");
        assert_eq!(result["navigation"]["map"]["selected_map_id"], "church");
        assert_eq!(result["navigation"]["cursor"]["id"], "step-1");
        assert!(store.progress(&guide).unwrap().completed.is_empty());
        assert!(
            store
                .progress(&guide)
                .unwrap()
                .reported_completed
                .is_empty()
        );
        let mut tracker: PlaceTracker = store.get(&tracker_key(&guide)).unwrap().unwrap();
        tracker.located_at_ms = 1;
        tracker.observed_at_ms = 1;
        store.put(&tracker_key(&guide), &tracker).unwrap();
        let state = call_tool(&store, "get_game_context", json!({})).unwrap();
        assert_eq!(
            state["navigation"]["location"]["status"], "reported",
            "A report does not expire like a screenshot"
        );
        let completed = store
            .report_complete(&guide, 1, "step-1", "I spoke to the guru")
            .unwrap();
        let state = context(&store, &guide, &completed, &SyncEstimate::default()).unwrap();
        assert_eq!(state["cursor"]["id"], "step-2");
        assert_eq!(
            state["map"]["current_place"]["id"], "church",
            "Completing a task must not erase the room"
        );
        let moved = report(&store, &guide, "forest", "step-3", 2).unwrap();
        assert_eq!(moved["navigation"]["map"]["selected_map_id"], "forest");
        assert_eq!(moved["navigation"]["cursor"]["id"], "step-3");
        assert!(
            store
                .save_progress(&guide, &Progress::default(), "old_frame")
                .is_err()
        );
        assert!(report(&store, &guide, "church", "step-1", 2).is_err());
        assert!(report(&store, &guide, "church", "step-3", 3).is_err());
        assert!(report(&store, &guide, "invented", "step-3", 3).is_err());
        let reset = Progress {
            epoch: 4,
            ..Default::default()
        };
        store.save_progress(&guide, &reset, "reload").unwrap();
        assert!(context(&store,&guide,&reset,&SyncEstimate::default()).unwrap()["map"]["current_place"].is_null());
        report(&store, &guide, "church", "step-1", 4).unwrap();
        screen::start(&store, &guide.game, true).unwrap();
        assert!(call_tool(&store,"get_game_context",json!({})).unwrap()["navigation"]["map"]["current_place"].is_null(),"A new game session cannot reuse yesterday's location as current");
    }
    #[test]
    fn visual_places_need_distinct_consecutive_frames_and_cannot_overwrite_newer_reports() {
        let (dir, store, guide) = fixture();
        let ticket = screen::start(&store, &guide.game, true).unwrap();
        let path = dir.path().join("game.png");
        image::RgbImage::new(20, 20).save(&path).unwrap();
        screen::publish(&store, &ticket, &path, "game_window").unwrap();
        let first = screen::snapshot(&store).unwrap();
        let location = screen::SceneLocation {
            place_id: "shop".into(),
            confidence: 0.95,
            evidence: "Visible key counter".into(),
        };
        observe_place(&store, &guide, &first, Some(&location), 0).unwrap();
        observe_place(&store, &guide, &first, Some(&location), 0).unwrap();
        assert!(
            context(
                &store,
                &guide,
                &Progress::default(),
                &SyncEstimate::default()
            )
            .unwrap()["map"]["current_place"]
                .is_null()
        );
        let next = |delta| screen::Snapshot {
            captured_at_ms: first.captured_at_ms + delta,
            ..first.clone()
        };
        observe_place(&store, &guide, &next(1), None, 0).unwrap();
        observe_place(&store, &guide, &next(2), Some(&location), 0).unwrap();
        assert!(
            context(
                &store,
                &guide,
                &Progress::default(),
                &SyncEstimate::default()
            )
            .unwrap()["map"]["current_place"]
                .is_null()
        );
        observe_place(&store, &guide, &next(3), Some(&location), 0).unwrap();
        let state = context(
            &store,
            &guide,
            &Progress::default(),
            &SyncEstimate::default(),
        )
        .unwrap();
        assert_eq!(state["map"]["current_place"]["id"], "shop");
        assert_eq!(state["cursor"]["id"], "step-2");
        report(&store, &guide, "forest", "step-3", 0).unwrap();
        assert!(observe_place(&store, &guide, &next(4), Some(&location), 0).is_err());
        assert_eq!(
            call_tool(&store, "get_game_context", json!({})).unwrap()["navigation"]["map"]["current_place"]
                ["id"],
            "forest"
        );
        screen::set_enabled(&store, &ticket.session, &guide.game, false).unwrap();
        assert!(observe_place(&store, &guide, &next(5), Some(&location), 1).is_err());
    }
    #[test]
    fn legacy_tracker_documents_migrate_and_stale_cas_cannot_erase_a_report() {
        let (_dir, store, guide) = fixture();
        let key = tracker_key(&guide);
        store
            .put(&key, &json!({"epoch":0,"observed_at_ms":0}))
            .unwrap();
        context(
            &store,
            &guide,
            &Progress::default(),
            &SyncEstimate::default(),
        )
        .unwrap();
        let (old, version) = store.get_versioned::<PlaceTracker>(&key).unwrap().unwrap();
        report(&store, &guide, "shop", "step-2", 0).unwrap();
        assert!(!store.replace_version(&key, &version, &old).unwrap());
        assert_eq!(
            store
                .get::<PlaceTracker>(&key)
                .unwrap()
                .unwrap()
                .place_id
                .as_deref(),
            Some("shop")
        );
    }
    #[test]
    fn map_regions_reject_unseen_maps_invalid_bounds_and_unsupported_precision() {
        let (_dir, store, guide) = fixture();
        let atlas = atlas(&store, &guide).unwrap();
        let mut marker = super::super::placement::Marker {
            place_id: "church".into(),
            map_id: "church".into(),
            region: gamebuddy_core::Crop {
                x: 0.1,
                y: 0.2,
                width: 0.2,
                height: 0.3,
            },
            evidence: "Labelled church entrance".into(),
            confidence: 0.9,
        };
        marker.validate(&atlas).unwrap();
        marker.region.x = 0.9;
        assert!(marker.validate(&atlas).is_err());
        marker.region.x = 0.1;
        marker.map_id = "invented".into();
        assert!(marker.validate(&atlas).is_err());
        marker.map_id = "church".into();
        marker.confidence = 0.3;
        assert!(marker.validate(&atlas).is_err());
    }
}
