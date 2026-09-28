//! Detailed guide editions and portable illustrated exports.
use crate::{HttpProvider, Settings, Store, compile_segment};
use anyhow::{Context, Result, ensure};
use gamebuddy_core::Guide;
use serde_json::json;
use std::{collections::HashSet, path::Path};

/// Preserve the original guide and player edits in a separate edition.
pub fn new_edition(store: &Store, original: &Guide) -> Result<Guide> {
    let mut guide = original.clone();
    guide.id = uuid::Uuid::new_v4().to_string();
    guide.revision = 1;
    guide.title = format!("{} · Illustrated walkthrough", original.game);
    guide.steps.retain(|s| {
        s.parent_id.is_none()
            && s.anchors
                .iter()
                .any(|a| a.start_ms.is_some() && a.end_ms.is_some())
    });
    ensure!(
        !guide.steps.is_empty(),
        "A detailed video edition requires timed source chapters"
    );
    for source in &guide.sources {
        if let Some(duration) = source.duration_ms {
            let last = guide
                .steps
                .iter_mut()
                .flat_map(|s| s.anchors.iter_mut())
                .filter(|a| a.source_id == source.id)
                .max_by_key(|a| a.end_ms);
            if let Some(anchor) = last {
                anchor.end_ms = Some(duration);
            }
        }
    }
    let ids: HashSet<_> = guide.steps.iter().map(|s| s.id.clone()).collect();
    for step in &mut guide.steps {
        step.instructions =
            "Chapter supplied by the video's author. Awaiting detailed review.".into();
        step.prerequisites.retain(|id| ids.contains(id));
        step.next.retain(|id| ids.contains(id));
        step.completion_signals.clear();
    }
    for goal in &mut guide.goals {
        goal.step_ids.retain(|id| ids.contains(id));
    }
    guide.coverage_notes = vec!["This edition follows one demonstrated route. Source coverage and factual correctness are separate; unresolved details are listed with each section.".into()];
    store.save_guide(&guide)?;
    if let Some(mut index) = store.index(original)? {
        index.guide_id = guide.id.clone();
        index.guide_revision = guide.revision;
        for frame in &mut index.frames {
            frame.step_ids = guide
                .steps
                .iter()
                .filter(|s| {
                    s.anchors.iter().any(|a| {
                        a.source_id == frame.source_id
                            && a.start_ms.is_some_and(|t| t <= frame.at_ms)
                            && a.end_ms.is_some_and(|t| frame.at_ms < t)
                    })
                })
                .map(|s| s.id.clone())
                .collect();
        }
        store.save_index(&index)?;
    }
    let split = crate::preparation::split_pending(&guide)?;
    if split.revision != guide.revision {
        store.revise_guide(&split, guide.revision, "Create detailed review intervals")?;
    }
    store.put(&format!("detailed-edition:{}", split.id), &original.id)?;
    Ok(split)
}

/// Model requests can run concurrently; revisions are merged serially and reject
/// an interval changed by the player in the meantime. Successful work is resumable.
pub fn build_edition(
    store: &Store,
    id: &str,
    jobs: usize,
    output: &Path,
    notify: &dyn Fn(String),
) -> Result<Guide> {
    ensure!(
        (1..=4).contains(&jobs),
        "Choose between one and four concurrent requests"
    );
    ensure!(
        store
            .get::<String>(&format!("detailed-edition:{id}"))?
            .is_some(),
        "Resume a detailed edition, or create one from a source guide first"
    );
    let guide = store.guide(id)?;
    let pending: Vec<_> = guide
        .steps
        .iter()
        .filter(|s| {
            crate::preparation::pending(s)
                && guide.step_enabled(s)
                && !guide
                    .steps
                    .iter()
                    .any(|c| c.parent_id.as_deref() == Some(&s.id))
        })
        .map(|s| s.id.clone())
        .collect();
    let settings = store.get::<Settings>("settings")?.unwrap_or_default();
    let mut errors = vec![];
    let mut reviewed = 0;
    export_book(store, &guide, output)?;
    for batch in pending.chunks(jobs) {
        let snapshot = store.guide(id)?;
        let results = std::thread::scope(|scope| {
            let handles: Vec<_> = batch
                .iter()
                .map(|step_id| {
                    let guide = &snapshot;
                    let config = settings.reasoning.clone();
                    let work = store.root.join("work");
                    scope.spawn(move || {
                        let provider = HttpProvider::new(config)?;
                        // A rejected JSON structure is retried once, without changing
                        // the provider or billing source.
                        compile_segment(&provider, guide, step_id, &work)
                            .or_else(|_| compile_segment(&provider, guide, step_id, &work))
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|h| {
                    h.join()
                        .unwrap_or_else(|_| Err(anyhow::anyhow!("Compilation worker panicked")))
                })
                .collect::<Vec<_>>()
        });
        for (step_id, result) in batch.iter().zip(results) {
            match result.and_then(|compiled| {
                let mut latest = store.guide(id)?;
                ensure!(
                    latest.sources == snapshot.sources
                        && latest.step(step_id).is_some_and(|s| latest.step_enabled(s)),
                    "Reference changed during compilation; retry against current evidence"
                );
                ensure!(
                    latest.step(step_id) == snapshot.step(step_id)
                        && !latest
                            .steps
                            .iter()
                            .any(|s| s.parent_id.as_deref() == Some(step_id)),
                    "This section changed during compilation; preserving the player's edit"
                );
                let new_ids: HashSet<_> = compiled
                    .steps
                    .iter()
                    .filter(|s| snapshot.step(&s.id).is_none())
                    .map(|s| s.id.clone())
                    .collect();
                let pos = latest
                    .steps
                    .iter()
                    .position(|s| &s.id == step_id)
                    .context("Section removed")?;
                latest.steps[pos] = compiled
                    .step(step_id)
                    .context("Compiled section missing")?
                    .clone();
                latest.steps.splice(
                    pos + 1..pos + 1,
                    compiled
                        .steps
                        .iter()
                        .filter(|s| new_ids.contains(&s.id))
                        .cloned(),
                );
                latest.coverage_notes.extend(
                    compiled
                        .coverage_notes
                        .iter()
                        .skip(snapshot.coverage_notes.len())
                        .cloned(),
                );
                let previous = latest.revision;
                latest.revision += 1;
                store.revise_guide(
                    &latest,
                    previous,
                    "Compile detailed illustrated tasks and substeps",
                )?;
                Ok(latest)
            }) {
                Ok(_) => reviewed += 1,
                Err(e) => errors.push(format!("{step_id}: {e:#}")),
            }
            let message = format!(
                "Detailed review: {reviewed}/{} sections, {} failures · {step_id}",
                pending.len(),
                errors.len()
            );
            notify(message.clone());
            store.put(&format!("preparation:{id}"),&json!({"phase":"compiling","reviewed":reviewed,"total":pending.len(),"message":message,"errors":errors}))?;
        }
        export_book(store, &store.guide(id)?, output)?;
        if reviewed == 0 && errors.len() >= jobs {
            break;
        }
    }
    let guide = crate::preparation::finish_chapters(store, &store.guide(id)?)?;
    let message = if errors.is_empty() {
        format!(
            "Detailed review finished: {reviewed} sections reviewed. See section notes for unresolved source details."
        )
    } else {
        format!(
            "Detailed review incomplete: {} sections failed; resume this edition to retry.",
            errors.len()
        )
    };
    store.put(&format!("preparation:{id}"),&json!({"phase":if errors.is_empty(){"reviewed"}else{"partial"},"reviewed":reviewed,"total":pending.len(),"message":message,"errors":errors}))?;
    export_book(store, &guide, output)?;
    notify(message);
    Ok(guide)
}

/// A graph view resolves container links to their first actionable descendant.
/// Route edges express the demonstrated order, separately from prerequisites.
pub fn ordering_graph(guide: &Guide) -> serde_json::Value {
    fn first_leaf<'a>(guide: &'a Guide, mut id: &'a str) -> &'a str {
        while let Some(child) = guide
            .steps
            .iter()
            .find(|s| s.parent_id.as_deref() == Some(id))
        {
            id = &child.id;
        }
        id
    }
    let mut edges = vec![];
    for step in &guide.steps {
        for next in &step.next {
            let mut target = guide.step(next);
            let mut enters_self = false;
            while let Some(node) = target {
                if node.id == step.id {
                    enters_self = true;
                    break;
                }
                target = node.parent_id.as_deref().and_then(|id| guide.step(id));
            }
            // Imported Markdown may link a container to its first child.
            // That is containment, not a route back from its last descendant.
            if enters_self {
                continue;
            }
            let mut tail = step;
            while let Some(child) = guide
                .steps
                .iter()
                .rev()
                .find(|s| s.parent_id.as_deref() == Some(&tail.id))
            {
                tail = child;
            }
            let edge = json!({"from":tail.id,"to":first_leaf(guide,next),"kind":"route"});
            if edge["from"] != edge["to"] && !edges.contains(&edge) {
                edges.push(edge);
            }
        }
        for prerequisite in &step.prerequisites {
            edges.push(json!({"from":prerequisite,"to":step.id,"kind":"requires"}));
        }
    }
    json!({"nodes":guide.steps.iter().map(|s|json!({"id":s.id,"parent":s.parent_id,"title":s.title})).collect::<Vec<_>>(),"edges":edges})
}

pub fn export_book(store: &Store, guide: &Guide, output: &Path) -> Result<()> {
    guide.validate()?;
    std::fs::create_dir_all(output.join("images"))?;
    let index = store
        .index(guide)?
        .context("Build a visual index before exporting an illustrated walkthrough")?;
    let mut frames = vec![];
    for frame in &index.frames {
        use sha2::{Digest, Sha256};
        let digest = format!(
            "{:x}",
            Sha256::digest(format!("{}:{}", frame.image_path, frame.at_ms))
        );
        let relative = format!("images/{}.jpg", &digest[..24]);
        let target = output.join(&relative);
        if !target.is_file() {
            // The video index already contains web-sized JPEGs. Preserve their
            // bytes instead of recompressing hundreds of images on every export.
            if Path::new(&frame.image_path)
                .extension()
                .is_some_and(|e| e == "jpg" || e == "jpeg")
            {
                std::fs::copy(&frame.image_path, &target)?;
            } else {
                image::open(&frame.image_path)
                    .with_context(|| format!("Read source frame {}", frame.image_path))?
                    .thumbnail(960, 720)
                    .save(&target)?;
            }
        }
        frames.push(json!({"source_id":frame.source_id,"at_ms":frame.at_ms,"image":relative,"steps":frame.step_ids}));
    }
    let graph = ordering_graph(guide);
    let coverage = coverage_report(guide, index.frames.len());
    let status = store.get::<serde_json::Value>(&format!("preparation:{}", guide.id))?;
    let data = json!({"guide":guide,"frames":frames,"graph":graph,"preparation":status,"coverage":coverage});
    // Escape script terminators even though imported/model text is rendered only
    // through textContent. The export opens without external scripts or services.
    let encoded = serde_json::to_string(&data)?
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026");
    let html = include_str!("../assets/walkthrough.html").replace("__GAMEBUDDY_DATA__", &encoded);
    std::fs::write(output.join("index.html.tmp"), html)?;
    std::fs::rename(output.join("index.html.tmp"), output.join("index.html"))?;
    std::fs::write(
        output.join("guide.json"),
        serde_json::to_string_pretty(guide)?,
    )?;
    std::fs::write(
        output.join("ordering-graph.json"),
        serde_json::to_string_pretty(&graph)?,
    )?;
    std::fs::write(
        output.join("coverage.json"),
        serde_json::to_string_pretty(&coverage)?,
    )?;
    store.put(
        &format!("book:{}", guide.id),
        &output
            .join("index.html")
            .canonicalize()?
            .to_string_lossy()
            .into_owned(),
    )?;
    Ok(())
}

/// Interval coverage is independent of task count. It says the source was
/// reviewed, not that every visual event was understood or the game was beaten.
pub fn coverage_report(guide: &Guide, screenshots: usize) -> serde_json::Value {
    let parents: HashSet<_> = guide
        .steps
        .iter()
        .filter_map(|s| s.parent_id.as_deref())
        .collect();
    let leaves: Vec<_> = guide
        .steps
        .iter()
        .filter(|s| !parents.contains(s.id.as_str()))
        .collect();
    let pending: Vec<_> = leaves
        .iter()
        .filter(|s| crate::preparation::pending(s))
        .map(|s| s.id.as_str())
        .collect();
    let reviewed: Vec<_> = guide
        .steps
        .iter()
        .filter(|s| {
            s.instructions.starts_with("Detailed tasks below")
                || s.instructions.starts_with("Reference reviewed:")
        })
        .collect();
    let mut sources = vec![];
    for source in guide.sources.iter().filter(|s| guide.source_enabled(&s.id)) {
        let Some(duration) = source.duration_ms else {
            continue;
        };
        let mut spans: Vec<_> = reviewed
            .iter()
            .flat_map(|s| &s.anchors)
            .filter(|a| a.source_id == source.id)
            .filter_map(|a| Some((a.start_ms?, a.end_ms?)))
            .collect();
        spans.sort_unstable();
        let mut cursor = 0;
        let mut gaps = vec![];
        for (start, end) in spans {
            if start > cursor {
                gaps.push(json!({"start_ms":cursor,"end_ms":start}));
            }
            cursor = cursor.max(end);
        }
        if cursor < duration {
            gaps.push(json!({"start_ms":cursor,"end_ms":duration}));
        }
        let missing: u64 = gaps
            .iter()
            .map(|g| g["end_ms"].as_u64().unwrap() - g["start_ms"].as_u64().unwrap())
            .sum();
        sources.push(json!({"source_id":source.id,"duration_ms":duration,"reviewed_ms":duration.saturating_sub(missing),"unreviewed_intervals":gaps}));
    }
    json!({"guide_id":guide.id,"revision":guide.revision,"chapter_count":guide.steps.iter().filter(|s|s.parent_id.is_none()).count(),"node_count":guide.steps.len(),"leaf_count":leaves.len(),"screenshot_count":screenshots,"reviewed_section_count":reviewed.len(),"pending_step_ids":pending,"source_review_complete":!sources.is_empty()&&sources.iter().all(|s|s["unreviewed_intervals"].as_array().unwrap().is_empty())&&pending.is_empty(),"independent_playthrough_verified":false,"sources":sources})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ordering_enters_children_and_connects_across_chapters() {
        let g = gamebuddy_core::import_markdown(
            "Game",
            "Route",
            "# Start\nText\n## Buy key\nText\n## Open door\nText\n# Castle\nText\n## Boss\nText",
        )
        .unwrap();
        let graph = ordering_graph(&g);
        let edges = graph["edges"].as_array().unwrap();
        assert!(edges.contains(&json!({"from":"step-2","to":"step-3","kind":"route"})));
        assert!(edges.contains(&json!({"from":"step-3","to":"step-5","kind":"route"})));
        assert!(edges.iter().all(|e| e["from"] != e["to"]));
        assert_eq!(edges.len(), 2);
    }
    #[test]
    fn coverage_does_not_hide_a_gap_behind_a_large_step_count() {
        let mut g = gamebuddy_core::import_markdown(
            "Game",
            "Guide",
            "# Route\nDetailed tasks below follow the route.",
        )
        .unwrap();
        g.sources[0].duration_ms = Some(1000);
        g.steps[0].anchors[0].start_ms = Some(100);
        g.steps[0].anchors[0].end_ms = Some(900);
        let report = coverage_report(&g, 100);
        assert_eq!(report["source_review_complete"], false);
        assert_eq!(report["sources"][0]["reviewed_ms"], 800);
        g.steps[0].anchors[0].start_ms = Some(0);
        g.steps[0].anchors[0].end_ms = Some(1000);
        assert_eq!(coverage_report(&g, 100)["source_review_complete"], true);
    }
}
