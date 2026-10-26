//! Renew an appropriate movement cue from fresh captures, independently of
//! how long ago the player asked for directions or the matcher last finished.
use super::advice::{Recommendation, supports_action};
use crate::{Store, screen};
use anyhow::Result;
use gamebuddy_core::{
    Atlas, Guidance, Guide, PlaceTracker, Progress,
    navigation::{
        direction::{DirectionCue, ScreenDirection, bearing, movement},
        estimate::LocationEstimate,
    },
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub(super) struct View<'a> {
    pub guidance: &'a Guidance,
    pub recommendation: Option<&'a Recommendation>,
    pub spatial: &'a Value,
    pub atlas: &'a Atlas,
    pub location: &'a PlaceTracker,
    pub now: u64,
    pub estimate: &'a LocationEstimate,
    /// Where the current step's place sits on a map: `(map_id, region)`.
    pub target_region: Option<(String, [f32; 4])>,
    pub target_label: Option<String>,
    /// Width over height of that map, for angles in pixel space.
    pub map_aspect: f32,
}

/// The direction from the fused position toward the target's spot on the same
/// map. Geometry decides; words never choose left or right when this is known.
fn geometric(view: &View<'_>) -> Option<(ScreenDirection, u64)> {
    let position = view.estimate.position.as_ref()?;
    let (map_id, region) = view.target_region.as_ref()?;
    if &position.map_id != map_id
        || !view.estimate.is_known()
        || view.now.saturating_sub(view.estimate.located_at_ms) >= screen::FRESH_MS
    {
        return None;
    }
    // Inside the target's own area there is nothing left to point at.
    if position.x >= region[0]
        && position.x <= region[0] + region[2]
        && position.y >= region[1]
        && position.y <= region[1] + region[3]
    {
        return None;
    }
    let center = [region[0] + region[2] / 2., region[1] + region[3] / 2.];
    bearing([position.x, position.y], center, view.map_aspect)
        .map(|d| (d, view.estimate.located_at_ms))
}

#[derive(Serialize, Deserialize)]
struct Continuity {
    revision: u64,
    epoch: u64,
    step_id: String,
    instructions: String,
    direction: ScreenDirection,
    frame: screen::MovementFrame,
}

pub(super) fn cue(
    store: &Store,
    guide: &Guide,
    progress: &Progress,
    view: View<'_>,
) -> Result<Option<DirectionCue>> {
    let Some(action) = view
        .guidance
        .now
        .as_ref()
        .filter(|_| ["ready", "unlocated"].contains(&view.guidance.status.as_str()))
    else {
        return Ok(None);
    };
    if let Some((direction, located_at_ms)) = geometric(&view) {
        return Ok(Some(DirectionCue {
            direction,
            step_id: action.id.clone(),
            label: action.title.clone(),
            instruction: action.instructions.clone(),
            valid_until_ms: located_at_ms.saturating_add(screen::FRESH_MS),
            source: "geometry",
            target: view.target_label.clone(),
        }));
    }
    let explicit = view
        .recommendation
        .filter(|r| r.step_id == action.id)
        .and_then(|r| r.movement.as_ref());
    let parsed = movement(&action.instructions);
    let Some(direction) = explicit.map(|m| m.direction).or(parsed) else {
        // A conversation, purchase, conditional route or new action without a
        // supported direction cannot inherit the previous movement arrow.
        return Ok(None);
    };
    let Some(frame) = screen::movement_frame(store, view.now)? else {
        return Ok(None);
    };
    if explicit.is_some_and(|m| m.ticket != frame.ticket) {
        return Ok(None);
    }
    let key = format!("movement-cue:{}", guide.id);
    let previous = store.get_versioned::<Continuity>(&key)?;
    let matching = previous.as_ref().map(|(c, _)| c).filter(|c| {
        c.revision == guide.revision
            && c.epoch == progress.epoch
            && c.step_id == action.id
            && c.instructions == action.instructions
            && c.direction == direction
    });
    let continued = matching.is_some_and(|c| screen::movement_continues(&c.frame, &frame));
    let original_view = explicit.is_some_and(|m| {
        screen::movement_continues(
            &screen::MovementFrame {
                ticket: m.ticket.clone(),
                captured_at_ms: m.captured_at_ms,
                frame_signature: m.frame_signature.clone(),
            },
            &frame,
        )
    });
    let matched_at = view.spatial["observation"]["tracker"]["observed_at_ms"]
        .as_u64()
        .unwrap_or_default();
    // A source-supported route can continue after a larger camera transition.
    // Natural wording with an explicit local cue (e.g. Up into this doorway)
    // needs its actual view; a similar room alone must not revive that cue.
    let source_route = parsed == Some(direction)
        && matched_at > 0
        && matched_at <= frame.captured_at_ms
        && view.now.saturating_sub(matched_at) < screen::FRESH_MS
        && supports_action(guide, &action.id, view.spatial);
    let located_route = explicit.is_none()
        && gamebuddy_core::navigation::direction::cue(
            view.guidance,
            view.atlas,
            view.location,
            view.now.max(frame.captured_at_ms),
        )
        .is_some();
    if !continued && !original_view && !source_route && !located_route {
        return Ok(None);
    }
    if matching.is_none_or(|c| c.frame != frame) {
        let next = Continuity {
            revision: guide.revision,
            epoch: progress.epoch,
            step_id: action.id.clone(),
            instructions: action.instructions.clone(),
            direction,
            frame: frame.clone(),
        };
        // Multiple UI/voice readers can observe different captures. An older
        // reader must not overwrite a newer reader's continuity anchor.
        if let Some((_, version)) = previous {
            let _ = store.replace_version(&key, &version, &next)?;
        } else {
            let _ = store.put_if_absent(&key, &next)?;
        }
        // A lost CAS will use the winning anchor on the next refresh. This
        // reader can still display the cue supported by its own fresh capture.
    }
    Ok(Some(DirectionCue {
        direction,
        step_id: action.id.clone(),
        label: action.title.clone(),
        instruction: action.instructions.clone(),
        valid_until_ms: frame.captured_at_ms.saturating_add(screen::FRESH_MS),
        source: if explicit.is_some() {
            "advice"
        } else {
            "instructions"
        },
        target: view.target_label.clone(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::navigation::advice::ObservedMovement;
    use serde_json::json;

    // Deliberately high-contrast scenery: a small camera translation fails the
    // unshifted comparison but agrees after translation. This tests continuity,
    // not game recognition (the recorded-gameplay replay covers that path).
    fn scenery(shift: i32) -> Vec<u8> {
        (0..18)
            .flat_map(|y| {
                (0..24).flat_map(move |x| {
                    let x = x + shift;
                    [
                        (x * 67 + y * 31).rem_euclid(256) as u8,
                        (x * 23 + y * 53).rem_euclid(256) as u8,
                        (x * 41 + y * 17).rem_euclid(256) as u8,
                    ]
                })
            })
            .collect()
    }

    fn capture(store: &Store, at: u64, signature: Vec<u8>) -> Result<()> {
        let mut frame: Value = store.get("live-screen")?.unwrap();
        frame["captured_at_ms"] = json!(at);
        frame["frame_signature"] = json!(signature);
        store.put("live-screen", &frame)
    }

    struct Fixture {
        _dir: tempfile::TempDir,
        store: Store,
        guide: Guide,
        guidance: Guidance,
        recommendation: Recommendation,
        atlas: Atlas,
        location: PlaceTracker,
        spatial: Value,
        progress: Progress,
    }
    impl Fixture {
        fn new(instructions: &str, direction: ScreenDirection) -> Result<Self> {
            let dir = tempfile::tempdir()?;
            let store = Store::open(dir.path())?;
            let guide = gamebuddy_core::import_markdown(
                "Game",
                "Route",
                &format!("# Gate\n{instructions}\n# Guard\nTalk to the guard."),
            )?;
            store.save_guide(&guide)?;
            let ticket = screen::start(&store, "Game", true)?;
            let path = dir.path().join("frame.png");
            image::RgbImage::new(64, 64).save(&path)?;
            screen::publish(&store, &ticket, &path, "test")?;
            capture(&store, 100_000, scenery(0))?;
            let recommendation = Recommendation {
                revision: guide.revision,
                epoch: 0,
                session: ticket.session.clone(),
                updated_at_ms: 100_000,
                step_id: "step-1".into(),
                instructions: instructions.into(),
                location_evidence: None,
                movement: Some(ObservedMovement {
                    direction,
                    ticket,
                    captured_at_ms: 100_000,
                    frame_signature: scenery(0),
                }),
            };
            let guidance = Guidance {
                status: "ready".into(),
                now: Some(guide.steps[0].clone()),
                ..Default::default()
            };
            Ok(Self {
                _dir: dir,
                store,
                guide,
                guidance,
                recommendation,
                atlas: Atlas::default(),
                location: PlaceTracker::default(),
                spatial: Value::Null,
                progress: Progress::default(),
            })
        }
        fn cue(&self, now: u64) -> Result<Option<DirectionCue>> {
            cue(
                &self.store,
                &self.guide,
                &self.progress,
                View {
                    guidance: &self.guidance,
                    recommendation: Some(&self.recommendation),
                    spatial: &self.spatial,
                    atlas: &self.atlas,
                    location: &self.location,
                    now,
                    estimate: &LocationEstimate::default(),
                    target_region: None,
                    target_label: None,
                    map_aspect: 1.0,
                },
            )
        }
    }

    #[test]
    fn fresh_captures_keep_the_arrow_through_a_minute_of_scroll_and_matcher_gaps() -> Result<()> {
        let mut f = Fixture::new("Walk right to the gate.", ScreenDirection::Right)?;
        for i in 0..=30 {
            let at = 100_000 + i * 2_000;
            capture(&f.store, at, scenery(i as i32 * 2))?;
            let cue = f.cue(at)?.expect("Current movement must remain visible");
            assert_eq!(cue.direction, ScreenDirection::Right);
            assert_eq!(cue.valid_until_ms, at + screen::FRESH_MS);
        }
        capture(&f.store, 160_010, scenery(60))?;
        assert!(
            f.cue(160_000)?.is_some(),
            "Capture advancing during a UI refresh must not blink the arrow"
        );
        assert!(
            f.cue(175_010)?.is_none(),
            "Actual capture expiry still hides it"
        );
        capture(&f.store, 180_000, scenery(60))?;
        assert!(
            f.cue(180_000)?.is_some(),
            "Fresh unchanged view renews the cue"
        );
        capture(&f.store, 182_000, vec![0; 24 * 18 * 3])?;
        assert!(f.cue(182_000)?.is_none(), "A new scene must not inherit it");
        capture(&f.store, 184_000, scenery(60))?;
        assert!(f.cue(184_000)?.is_some());
        f.guidance.now = Some(f.guide.steps[1].clone());
        assert!(
            f.cue(184_000)?.is_none(),
            "Conversation must clear movement"
        );
        assert!(f.store.progress(&f.guide)?.completed.is_empty());
        Ok(())
    }

    #[test]
    fn fresh_source_can_recover_a_route_but_not_a_local_doorway_command() -> Result<()> {
        let mut f = Fixture::new("Walk right to the gate.", ScreenDirection::Right)?;
        capture(&f.store, 180_000, vec![0; 24 * 18 * 3])?;
        f.spatial = json!({"observation":{"tracker":{"status":"tracked","observed_at_ms":180_000},"related_step_ids":["step-1"]}});
        assert!(
            f.cue(180_000)?.is_some(),
            "A still-supported route should recover"
        );
        f.spatial = Value::Null;
        capture(&f.store, 200_000, vec![0; 24 * 18 * 3])?;
        assert!(
            f.cue(200_000)?.is_some(),
            "Source matcher need not repeat an idle frame"
        );
        f.progress.epoch += 1;
        assert!(
            f.cue(200_000)?.is_none(),
            "New progress cannot reuse old continuity"
        );
        f.progress.epoch -= 1;
        f.guide.revision += 1;
        assert!(
            f.cue(200_000)?.is_none(),
            "Revised guides cannot reuse old continuity"
        );

        let mut f = Fixture::new("Enter the door beside you.", ScreenDirection::Up)?;
        capture(&f.store, 180_000, vec![0; 24 * 18 * 3])?;
        f.spatial = json!({"observation":{"tracker":{"status":"tracked","observed_at_ms":180_000},"related_step_ids":["step-1"]}});
        assert!(
            f.cue(180_000)?.is_none(),
            "A doorway arrow needs its actual view"
        );
        Ok(())
    }

    #[test]
    fn source_and_report_based_arrows_renew_without_an_explicit_direction() -> Result<()> {
        let mut f = Fixture::new("Walk right to the gate.", ScreenDirection::Right)?;
        f.recommendation.movement = None;
        f.atlas.bindings.push(gamebuddy_core::PlaceBinding {
            step_id: "step-1".into(),
            place_id: "town".into(),
            evidence: "Start at town gate".into(),
        });
        f.location.status = "reported".into();
        f.location.place_id = Some("town".into());
        f.location.located_at_ms = 100_000;
        assert!(f.cue(100_000)?.is_some());
        capture(&f.store, 180_000, scenery(0))?;
        assert!(
            f.cue(180_000)?.is_some(),
            "The location report's age is not an arrow deadline"
        );
        let mut frame: Value = f.store.get("live-screen")?.unwrap();
        frame["enabled"] = json!(false);
        frame["ticket"]["epoch"] = json!(1);
        f.store.put("live-screen", &frame)?;
        assert!(f.cue(180_000)?.is_none());
        frame["enabled"] = json!(true);
        frame["ticket"]["epoch"] = json!(2);
        frame["jpeg"] = json!("");
        f.store.put("live-screen", &frame)?;
        assert!(f.cue(180_000)?.is_none());
        Ok(())
    }
}
