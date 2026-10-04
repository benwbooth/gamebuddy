//! The in-game reader uses the live guide, never a stale exported edition.
use crate::{
    Store,
    plan::{PlanEdit, edit_plan},
};
use anyhow::{Context, Result, ensure};
use gamebuddy_core::{Guide, PlayerGoal};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeSet;

#[derive(Default, Serialize, Deserialize)]
pub struct ReaderState {
    pub bookmarks: BTreeSet<String>,
}

#[derive(Deserialize)]
pub struct Request {
    pub request_id: u64,
    pub guide_id: String,
    pub revision: u64,
    #[serde(flatten)]
    pub action: Action,
}

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Action {
    Snapshot,
    Images {
        ids: Vec<String>,
    },
    Bookmark {
        step_id: String,
        enabled: bool,
    },
    Complete {
        step_id: String,
        enabled: bool,
        epoch: u64,
    },
    Goal {
        step_id: String,
        enabled: bool,
    },
}

pub fn state(store: &Store, guide: &Guide) -> Result<ReaderState> {
    let mut state = store
        .get::<ReaderState>(&format!("reader:{}", guide.id))?
        .unwrap_or_default();
    state.bookmarks.retain(|id| guide.step(id).is_some());
    Ok(state)
}

pub fn snapshot(store: &Store, guide: &Guide) -> Result<Value> {
    let frames = store
        .index(guide)?
        .map(|i| {
            i.frames.into_iter().map(|f| {
        json!({"id":f.id,"source_id":f.source_id,"at_ms":f.at_ms,"steps":f.step_ids})
    }).collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let mut source_metadata = serde_json::Map::new();
    for source in &guide.sources {
        if let Some(id) = source
            .url
            .as_deref()
            .and_then(|u| gamebuddy_core::youtube_id(u).ok())
            && let Some(metadata) = store.get::<Value>(&format!("source-metadata:{id}"))?
        {
            source_metadata.insert(source.id.clone(), metadata);
        }
    }
    Ok(json!({
        "guide_id":guide.id,"revision":guide.revision,"frames":frames,
        "history":store.revision_history(&guide.id)?, "atlas":crate::navigation::atlas(store,guide)?,
        "coverage":crate::walkthrough::coverage_report(guide,frames.len()),
        "reader":state(store,guide)?,"source_metadata":source_metadata,
        "method":"Video sections are interpreted by the configured reasoning model using twelve ordered screenshots and the available timed captions, or a local audio transcription. Chapters and imported text headings form the initial hierarchy. Generated tasks are checked for valid source spans, hierarchy and acyclic dependencies before a revision is saved. Route links describe the demonstrated ordering; prerequisites are model-inferred requirements. Maps are external reference images associated with places using source evidence. Reviewing every interval does not establish complete game coverage or verify the instructions through gameplay. See coverage notes and revision history for this guide's recorded history; older revisions do not record the exact model or prompt."
    }))
}

pub fn respond(store: &Store, request: &Request) -> Result<Value> {
    let guide = store.guide(&request.guide_id)?;
    ensure!(
        guide.revision == request.revision,
        "The outline changed. Reload the reader and try again."
    );
    match &request.action {
        Action::Snapshot => snapshot(store, &guide),
        Action::Images { ids } => {
            ensure!(ids.len() <= 8, "Load at most eight pictures at a time");
            let index = store
                .index(&guide)?
                .context("Pictures are still being prepared")?;
            let mut images = vec![];
            for id in ids {
                let frame = index
                    .frames
                    .iter()
                    .find(|f| &f.id == id)
                    .context("Unknown guide picture")?;
                // Resolve IDs from the index, never arbitrary paths supplied by HTML.
                let result = crate::image_base64(&frame.image_path);
                images.push(match result {
                    Ok(data) => json!({"id":id,"data":format!("data:image/jpeg;base64,{data}")}),
                    Err(error) => json!({"id":id,"error":format!("Picture unavailable: {error}")}),
                });
            }
            Ok(json!({"images":images}))
        }
        Action::Bookmark { step_id, enabled } => {
            ensure!(guide.step(step_id).is_some(), "Unknown step");
            let key = format!("reader:{}", guide.id);
            store.put_if_absent(&key, &ReaderState::default())?;
            let (mut reader, version) = store
                .get_versioned::<ReaderState>(&key)?
                .context("Reader unavailable")?;
            if *enabled {
                reader.bookmarks.insert(step_id.clone());
            } else {
                reader.bookmarks.remove(step_id);
            }
            ensure!(
                store.replace_version(&key, &version, &reader)?,
                "Bookmarks changed; try again"
            );
            Ok(json!({"reader":reader}))
        }
        Action::Complete {
            step_id,
            enabled,
            epoch,
        } => Ok(json!({"progress":store.check_step(&guide,*epoch,step_id,*enabled)?})),
        Action::Goal { step_id, enabled } => {
            let step = guide.step(step_id).context("Unknown step")?;
            let id = format!("reader-goal:{step_id}");
            let mut goals = guide.goals.clone();
            goals.retain(|g| g.id != id);
            if *enabled {
                goals.push(PlayerGoal {
                    id,
                    description: step.title.clone(),
                    step_ids: vec![step_id.clone()],
                });
            }
            let updated = edit_plan(
                store,
                &guide.id,
                PlanEdit {
                    expected_revision: guide.revision,
                    summary: format!(
                        "Player {} goal in guide library: {}",
                        if *enabled { "added" } else { "removed" },
                        step.title
                    ),
                    upsert_steps: vec![],
                    remove_step_ids: vec![],
                    goals: Some(goals),
                    reference_preferences: None,
                },
            )?;
            Ok(json!({"revision":updated.revision}))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reader_edits_are_persistent_scoped_and_do_not_move_the_player() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(temp.path()).unwrap();
        let guide = gamebuddy_core::import_markdown(
            "Test",
            "Guide",
            "# Town\n## Shop\nBuy key\n## Gate\nUse key",
        )
        .unwrap();
        store.save_guide(&guide).unwrap();
        let request = |action| Request {
            request_id: 1,
            guide_id: guide.id.clone(),
            revision: 1,
            action,
        };
        let decoded: Request = serde_json::from_value(json!({"request_id":7,"guide_id":guide.id,"revision":1,"action":"bookmark","step_id":"step-2","enabled":true})).unwrap();
        assert_eq!(decoded.request_id, 7);
        respond(
            &store,
            &request(Action::Bookmark {
                step_id: "step-2".into(),
                enabled: true,
            }),
        )
        .unwrap();
        assert!(
            state(&Store::open(temp.path()).unwrap(), &guide)
                .unwrap()
                .bookmarks
                .contains("step-2")
        );
        respond(
            &store,
            &request(Action::Complete {
                step_id: "step-2".into(),
                enabled: true,
                epoch: 0,
            }),
        )
        .unwrap();
        let p = store.progress(&guide).unwrap();
        assert!(p.completed.contains("step-2"));
        assert!(p.current_step.is_none());
        assert!(
            respond(
                &store,
                &request(Action::Complete {
                    step_id: "step-3".into(),
                    enabled: true,
                    epoch: 0
                })
            )
            .is_err()
        );
        respond(
            &store,
            &request(Action::Complete {
                step_id: "step-2".into(),
                enabled: false,
                epoch: p.epoch,
            }),
        )
        .unwrap();
        assert!(store.progress(&guide).unwrap().completed.is_empty());
        respond(
            &store,
            &request(Action::Goal {
                step_id: "step-3".into(),
                enabled: true,
            }),
        )
        .unwrap();
        let edited = store.guide(&guide.id).unwrap();
        assert_eq!(edited.goals[0].step_ids, vec!["step-3"]);
        assert!(
            respond(
                &store,
                &request(Action::Goal {
                    step_id: "step-2".into(),
                    enabled: true
                })
            )
            .is_err()
        );
        assert!(store.progress(&edited).unwrap().current_step.is_none());
        assert!(
            snapshot(&store, &edited).unwrap()["history"]
                .as_array()
                .unwrap()
                .len()
                >= 2
        );
        assert!(
            respond(
                &store,
                &Request {
                    revision: edited.revision,
                    ..request(Action::Images {
                        ids: vec!["/etc/passwd".into()]
                    })
                }
            )
            .is_err()
        );
    }
    #[test]
    fn unchecking_a_child_keeps_its_completed_siblings_and_rejects_old_capture() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(temp.path()).unwrap();
        let guide =
            gamebuddy_core::import_markdown("Test", "Guide", "# Chapter\n## One\n## Two").unwrap();
        store.save_guide(&guide).unwrap();
        let parent = store.check_step(&guide, 0, "step-1", true).unwrap();
        let edited = store
            .check_step(&guide, parent.epoch, "step-2", false)
            .unwrap();
        assert!(!gamebuddy_core::step_complete(&guide, &edited, "step-1"));
        assert!(!gamebuddy_core::step_complete(&guide, &edited, "step-2"));
        assert!(gamebuddy_core::step_complete(&guide, &edited, "step-3"));
        assert!(store.save_progress(&guide, &parent, "stale").is_err());
        let cleared = store
            .check_step(&guide, edited.epoch, "step-1", false)
            .unwrap();
        assert!(cleared.completed.is_empty());
    }
}
