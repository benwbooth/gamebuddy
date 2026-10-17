//! Resumable source-wide compilation. Publish each reviewed interval immediately;
//! missing evidence remains a visible gap rather than an invented route.
use crate::{HttpProvider, Settings, Store, compile_segment};
use anyhow::Result;
use gamebuddy_core::{Guide, Step};
use serde_json::json;

const INTERVAL: u64 = 120_000;
pub(crate) fn pending(step: &Step) -> bool {
    step.instructions
        .starts_with("Chapter supplied by the video's author.")
        || step
            .instructions
            .starts_with("Video segment awaiting interpretation.")
}

pub fn split_pending(guide: &Guide) -> Result<Guide> {
    let mut result = guide.clone();
    for parent in &guide.steps {
        if !pending(parent)
            || guide
                .steps
                .iter()
                .any(|s| s.parent_id.as_deref() == Some(&parent.id))
        {
            continue;
        }
        let Some(anchor) = parent
            .anchors
            .iter()
            .find(|a| a.start_ms.is_some() && a.end_ms.is_some())
        else {
            continue;
        };
        let (start, end) = (anchor.start_ms.unwrap(), anchor.end_ms.unwrap());
        if end - start <= INTERVAL {
            continue;
        }
        let mut children = vec![];
        for (i, at) in (start..end).step_by(INTERVAL as usize).enumerate() {
            let mut child = parent.clone();
            child.id = format!("{}.part-{}", parent.id, i + 1);
            child.parent_id = Some(parent.id.clone());
            child.title = format!("{} · {}", parent.title, crate::timestamp(at));
            child.instructions = "Video segment awaiting interpretation. Preparation will extract source-backed actions here.".into();
            child.next.clear();
            child.prerequisites.clear();
            child.anchors = vec![gamebuddy_core::Anchor {
                start_ms: Some(at),
                end_ms: Some((at + INTERVAL).min(end)),
                ..anchor.clone()
            }];
            children.push(child);
        }
        for i in 0..children.len().saturating_sub(1) {
            children[i].next = vec![children[i + 1].id.clone()];
        }
        let pos = result.steps.iter().position(|s| s.id == parent.id).unwrap() + 1;
        result.steps.splice(pos..pos, children);
    }
    if result.steps != guide.steps {
        result.revision += 1;
    }
    result.validate()?;
    Ok(result)
}

pub(crate) fn finish_chapters(store: &Store, guide: &Guide) -> Result<Guide> {
    let mut result = guide.clone();
    let parents: std::collections::HashSet<_> = guide
        .steps
        .iter()
        .filter_map(|s| s.parent_id.as_deref())
        .collect();
    for parent in result
        .steps
        .iter_mut()
        .filter(|s| pending(s) && parents.contains(s.id.as_str()))
    {
        let descendants: Vec<_> = guide
            .steps
            .iter()
            .filter(|s| !parents.contains(s.id.as_str()))
            .filter(|s| {
                let mut current = *s;
                while let Some(id) = current.parent_id.as_deref() {
                    if id == parent.id {
                        return true;
                    }
                    let Some(next) = guide.step(id) else {
                        break;
                    };
                    current = next;
                }
                false
            })
            .collect();
        if !descendants.is_empty() && descendants.iter().all(|s| !pending(s)) {
            parent.instructions = format!(
                "Follow the {} detailed actions below in the demonstrated order. Expand each timed section for tasks, substeps and source evidence.",
                descendants.len()
            );
        }
    }
    if result.steps != guide.steps {
        result.revision += 1;
        store.revise_guide(&result, guide.revision, "Summarize fully reviewed chapters")?;
    }
    Ok(result)
}

pub fn compile_walkthrough(
    store: &Store,
    guide: &Guide,
    progress: &dyn Fn(String),
) -> Result<Guide> {
    let split = split_pending(guide)?;
    if split.revision != guide.revision {
        store.revise_guide(
            &split,
            guide.revision,
            "Divide walkthrough into reviewable video intervals",
        )?;
    }
    let mut guide = split;
    let pending: Vec<_> = guide
        .steps
        .iter()
        .filter(|s| {
            pending(s)
                && guide.step_enabled(s)
                && !guide
                    .steps
                    .iter()
                    .any(|child| child.parent_id.as_deref() == Some(&s.id))
        })
        .map(|s| s.id.clone())
        .collect();
    if pending.is_empty() {
        return finish_chapters(store, &guide);
    }
    let settings = store.get::<Settings>("settings")?.unwrap_or_default();
    let provider = HttpProvider::new(settings.reasoning)?;
    let key = format!("preparation:{}", guide.id);
    let mut failed = vec![];
    let mut reviewed = 0;
    for (i, id) in pending.iter().enumerate() {
        // Respect concurrent goal/reference edits rather than overwriting them.
        guide = store.guide(&guide.id)?;
        if !guide
            .step(id)
            .is_some_and(|s| self::pending(s) && guide.step_enabled(s))
            || guide
                .steps
                .iter()
                .any(|s| s.parent_id.as_deref() == Some(id))
        {
            continue;
        }
        let message = format!(
            "Building walkthrough: section {} of {} · {}",
            i + 1,
            pending.len(),
            guide.step(id).unwrap().title
        );
        store.put(&key, &json!({"phase":"compiling","reviewed":reviewed,"total":pending.len(),"message":message,"errors":failed}))?;
        progress(message);
        let attempt = || {
            crate::with_backoff(&|m| progress(m), || {
                compile_segment(&provider, &guide, id, &store.root.join("work"))
            })
        };
        let compiled = match attempt() {
            Err(error) if crate::is_rate_limited(&error) => Err(error),
            Ok(g) => Ok(g),
            Err(first_error) => {
                progress(format!(
                    "Retrying section after validation/provider failure: {first_error}"
                ));
                attempt()
            }
        };
        match compiled.and_then(|g| {
            store.revise_guide(&g, guide.revision, "Compile automatic walkthrough section")?;
            Ok(g)
        }) {
            Ok(updated) => {
                guide = updated;
                reviewed += 1;
            }
            Err(e) => {
                let limited = crate::is_rate_limited(&e);
                failed.push(format!("{id}: {e:#}"));
                // Still limited after waiting it out: stop rather than fail
                // every remaining section; they stay pending for a retry.
                if limited {
                    break;
                }
                // Avoid spending minutes repeatedly retrying a broken provider.
                if reviewed == 0 && failed.len() == 3 {
                    break;
                }
            }
        }
    }
    let message = if failed.is_empty() {
        "Source review finished. Instructions use sampled frames and available captions; coverage gaps remain listed in the guide.".to_string()
    } else {
        format!(
            "Reviewed {reviewed} of {} pending sections. {} sections failed; retry preparation to continue{}.",
            pending.len(),
            failed.len(),
            if failed.last().is_some_and(|e| e.contains("429") || e.contains("Too Many Requests")) {
                " (the model kept limiting requests, so the rest were left for later)"
            } else {
                ""
            }
        )
    };
    store.put(&key, &json!({"phase":if failed.is_empty(){"reviewed"}else{"partial"},"reviewed":reviewed,"total":pending.len(),"message":message,"errors":failed}))?;
    progress(message);
    finish_chapters(store, &store.guide(&guide.id)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn chapters_remain_pending_until_their_leaf_sections_are_reviewed() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let mut g=gamebuddy_core::import_markdown("Game","Guide","# Chapter\nChapter supplied by the video's author.\n## Section\nVideo segment awaiting interpretation.").unwrap();
        store.save_guide(&g).unwrap();
        assert_eq!(finish_chapters(&store, &g).unwrap(), g);
        g.steps[1].instructions = "Follow the doorway.".into();
        g.revision += 1;
        store.revise_guide(&g, 1, "Reviewed section").unwrap();
        let finished = finish_chapters(&store, &g).unwrap();
        assert!(!pending(&finished.steps[0]));
        assert_eq!(finished.steps[1], g.steps[1]);
    }
    #[test]
    fn splits_long_chapters_without_gaps_and_is_idempotent() {
        let mut g = gamebuddy_core::import_markdown("Test", "Guide", "# Chapter\nText").unwrap();
        g.sources[0].duration_ms = Some(601_234);
        g.steps[0].instructions = "Chapter supplied by the video's author.".into();
        g.steps[0].anchors[0].start_ms = Some(1234);
        g.steps[0].anchors[0].end_ms = Some(601_234);
        let split = split_pending(&g).unwrap();
        let children: Vec<_> = split
            .steps
            .iter()
            .filter(|s| s.parent_id.is_some())
            .collect();
        assert_eq!(children.len(), 5);
        assert_eq!(children[0].anchors[0].start_ms, Some(1234));
        assert_eq!(children[4].anchors[0].end_ms, Some(601_234));
        for pair in children.windows(2) {
            assert_eq!(pair[0].anchors[0].end_ms, pair[1].anchors[0].start_ms);
        }
        assert_eq!(split_pending(&split).unwrap(), split);
    }

    #[test]
    fn revision_rebinds_video_frames_to_new_leaf_steps() {
        use gamebuddy_core::{Anchor, Crop, ReferenceFrame, VisualIndex};
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path()).unwrap();
        let mut guide =
            gamebuddy_core::import_markdown("Game", "Guide", "# Chapter\nReference").unwrap();
        guide.sources[0].duration_ms = Some(200);
        guide.steps[0].anchors = vec![Anchor {
            source_id: guide.sources[0].id.clone(),
            start_ms: Some(0),
            end_ms: Some(200),
            section: None,
        }];
        store.save_guide(&guide).unwrap();
        store
            .save_index(&VisualIndex {
                guide_id: guide.id.clone(),
                guide_revision: guide.revision,
                encoder: gamebuddy_core::FINGERPRINT_VERSION.into(),
                crop: Crop::default(),
                live_crop: Crop::default(),
                frames: [50, 150]
                    .into_iter()
                    .map(|at| ReferenceFrame {
                        id: format!("frame-{at}"),
                        source_id: guide.sources[0].id.clone(),
                        at_ms: at,
                        image_path: "reference.png".into(),
                        step_ids: vec![guide.steps[0].id.clone()],
                        fingerprint: vec![0.5; 1296],
                    })
                    .collect(),
            })
            .unwrap();
        let mut next = guide.clone();
        next.revision += 1;
        let mut child = guide.steps[0].clone();
        child.id = "action".into();
        child.parent_id = Some(guide.steps[0].id.clone());
        child.anchors[0].end_ms = Some(100);
        next.steps.push(child);
        store
            .revise_guide(&next, guide.revision, "Compile action")
            .unwrap();
        let index = store.index(&next).unwrap().unwrap();
        assert_eq!(index.frames[0].step_ids, vec!["action"]);
        assert_eq!(index.frames[1].step_ids, vec!["step-1"]);
        assert_eq!(index.frames[0].fingerprint, vec![0.5; 1296]);
        assert_eq!(index.guide_revision, next.revision);
    }
}
