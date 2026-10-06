//! Advice is not completion. Keep its source, capture and plan scope with it.
use super::*;
use gamebuddy_core::navigation::direction::ScreenDirection;

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    NextAction,
    Strategy,
    Danger,
    Recovery,
}

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Coaching {
    pub kind: Kind,
    pub step_id: String,
    pub text: String,
    pub evidence: String,
    pub confidence: f32,
    pub screen_direction: Option<ScreenDirection>,
}

pub fn save_review(store: &Store, observation: &Observation, review: &Review) -> Result<usize> {
    let completed = save(store, observation, &review.completions)?;
    let Some(coaching) = &review.coaching else {
        return Ok(completed);
    };
    let latest = observation
        .frames
        .last()
        .ok_or_else(|| anyhow::anyhow!("No coaching screenshot"))?;
    let guide = &observation.guide;
    let progress = store.progress(guide)?;
    let newer_advice = store
        .get::<Value>(&format!("next-action:{}", guide.id))?
        .is_some_and(|r| {
            r["updated_at_ms"]
                .as_u64()
                .is_some_and(|at| at > latest.captured_at_ms)
        });
    // Completion remains valid even if the player left the scene before its
    // review finished. Spoken tactics must still fit the current screen.
    if !coaching.confidence.is_finite()
        || !(0.8..=1.).contains(&coaching.confidence)
        || !(8..=600).contains(&coaching.text.trim().len())
        || !(12..=1200).contains(&coaching.evidence.trim().len())
        || !observation.actions.iter().any(|s| s.id == coaching.step_id)
        || gamebuddy_core::step_complete(guide, &progress, &coaching.step_id)
        || progress.epoch != observation.progress.epoch + u64::from(completed > 0)
        || store.get::<String>("active_guide")?.as_deref() != Some(&guide.id)
        || store.guide(&guide.id)?.revision != guide.revision
        || screen::now_ms().saturating_sub(latest.captured_at_ms)
            > if matches!(coaching.kind, Kind::Danger | Kind::Recovery) {
                3_000
            } else {
                30_000
            }
        || !screen::still_current(store, latest)?
        || newer_advice
    {
        return Ok(completed);
    }
    if crate::outcomes::prepared(store, guide)?.is_some_and(|p| {
        p.model
            .view(guide, &progress)
            .skipped
            .contains_key(&coaching.step_id)
    }) {
        return Ok(completed);
    }
    if matches!(coaching.kind, Kind::NextAction) {
        navigation::advice::publish(
            store,
            guide,
            navigation::advice::NextAction {
                expected_revision: guide.revision,
                epoch: progress.epoch,
                step_id: coaching.step_id.clone(),
                instructions: coaching.text.clone(),
                screen_direction: coaching.screen_direction,
            },
        )?;
    }
    use base64::Engine;
    let signature = screen::frame_signature(&image::load_from_memory(
        &base64::engine::general_purpose::STANDARD.decode(&latest.jpeg)?,
    )?);
    let mut value = json!(coaching);
    let fields = value.as_object_mut().unwrap();
    fields.insert("id".into(), json!(uuid::Uuid::new_v4().to_string()));
    fields.insert("revision".into(), json!(guide.revision));
    fields.insert("epoch".into(), json!(progress.epoch));
    fields.insert("ticket".into(), json!(latest.ticket));
    fields.insert("captured_at_ms".into(), json!(latest.captured_at_ms));
    fields.insert("frame_signature".into(), json!(signature));
    store.put_for_progress(
        guide,
        progress.epoch,
        &format!("coaching:{}", guide.id),
        &value,
    )?;
    Ok(completed)
}

pub fn current_coaching(
    store: &Store,
    guide: &Guide,
    progress: &Progress,
) -> Result<Option<Value>> {
    let Some(mut cue) = store.get::<Value>(&format!("coaching:{}", guide.id))? else {
        return Ok(None);
    };
    let capture = screen::status(store)?;
    if capture["status"] != "live"
        || cue["revision"] != guide.revision
        || cue["epoch"] != progress.epoch
        || cue["ticket"]["session"] != capture["session_id"]
        || cue["ticket"]["epoch"] != capture["capture_epoch"]
        || cue["captured_at_ms"].as_u64().is_none_or(|at| {
            screen::now_ms().saturating_sub(at)
                > if matches!(cue["kind"].as_str(), Some("danger" | "recovery")) {
                    3_000
                } else {
                    30_000
                }
        })
        || cue["step_id"]
            .as_str()
            .is_none_or(|id| gamebuddy_core::step_complete(guide, progress, id))
    {
        return Ok(None);
    }
    if crate::outcomes::prepared(store, guide)?.is_some_and(|p| {
        p.model
            .view(guide, progress)
            .skipped
            .contains_key(cue["step_id"].as_str().unwrap_or_default())
    }) {
        return Ok(None);
    }
    let signature: Vec<u8> = serde_json::from_value(cue["frame_signature"].clone())?;
    if !screen::matches_latest(
        store,
        cue["ticket"]["session"].as_str().unwrap_or_default(),
        cue["ticket"]["epoch"].as_u64().unwrap_or_default(),
        cue["captured_at_ms"].as_u64().unwrap_or_default(),
        &signature,
    )? {
        return Ok(None);
    }
    cue.as_object_mut().unwrap().remove("frame_signature");
    cue.as_object_mut().unwrap().remove("ticket");
    Ok(Some(cue))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn current_strategy_reaches_navigation_without_completing_steps() -> Result<()> {
        let (_dir, store, observation, _) = super::super::tests::fixture()?;
        let review = Review {
            completions: vec![],
            coaching: Some(Coaching {
                kind: Kind::Strategy,
                step_id: "step-1".into(),
                text: "Accept treatment before heading outside.".into(),
                evidence: "The source recommends treatment and the health meter is low.".into(),
                confidence: 0.98,
                screen_direction: None,
            }),
        };
        assert_eq!(save_review(&store, &observation, &review)?, 0);
        let progress = store.progress(&observation.guide)?;
        assert!(progress.completed.is_empty());
        let nav = navigation::context(&store, &observation.guide, &progress, &Default::default())?;
        assert_eq!(
            nav["coaching"]["text"],
            review.coaching.as_ref().unwrap().text
        );
        assert!(nav["coaching"].get("frame_signature").is_none());
        screen::set_enabled(&store, &observation.frames[1].ticket.session, "Game", false)?;
        assert!(current_coaching(&store, &observation.guide, &progress)?.is_none());
        Ok(())
    }
    #[test]
    fn changed_scene_or_progress_cannot_publish_late_coaching() -> Result<()> {
        for change in ["progress", "scene", "session"] {
            let (dir, store, observation, _) = super::super::tests::fixture()?;
            let review = Review {
                completions: vec![],
                coaching: Some(Coaching {
                    kind: Kind::NextAction,
                    step_id: "step-1".into(),
                    text: "Accept treatment before heading outside.".into(),
                    evidence: "The source recommends treatment and the health meter is low.".into(),
                    confidence: 0.98,
                    screen_direction: None,
                }),
            };
            match change {
                "progress" => {
                    let mut p = observation.progress.clone();
                    p.epoch += 1;
                    store.save_progress(&observation.guide, &p, "reset")?;
                }
                "scene" => {
                    let path = dir.path().join("changed.png");
                    image::RgbImage::from_pixel(64, 64, image::Rgb([240, 10, 10])).save(&path)?;
                    screen::publish(&store, &observation.frames[1].ticket, &path, "changed")?;
                }
                _ => {
                    screen::start(&store, "Game", true)?;
                }
            }
            assert_eq!(save_review(&store, &observation, &review)?, 0);
            assert!(
                store
                    .get::<Value>(&format!("coaching:{}", observation.guide.id))?
                    .is_none()
            );
            assert!(
                store
                    .get::<Value>(&format!("next-action:{}", observation.guide.id))?
                    .is_none()
            );
        }
        Ok(())
    }
    #[test]
    fn next_action_coaching_updates_now_and_the_arrow_together() -> Result<()> {
        let (_dir, store, observation, _) = super::super::tests::fixture()?;
        let review = Review {
            completions: vec![],
            coaching: Some(Coaching {
                kind: Kind::NextAction,
                step_id: "step-1".into(),
                text: "Walk right to the attendant.".into(),
                evidence: "The attendant is to the right and the source calls for treatment."
                    .into(),
                confidence: 0.98,
                screen_direction: Some(ScreenDirection::Right),
            }),
        };
        save_review(&store, &observation, &review)?;
        let nav = navigation::context(
            &store,
            &observation.guide,
            &store.progress(&observation.guide)?,
            &Default::default(),
        )?;
        assert_eq!(
            nav["guidance"]["now"]["instructions"],
            review.coaching.unwrap().text
        );
        assert_eq!(nav["direction"]["direction"], "right");
        assert!(store.progress(&observation.guide)?.completed.is_empty());
        Ok(())
    }
}
