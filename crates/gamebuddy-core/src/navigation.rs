//! Source-derived places are independent of objectives. A screen match never
//! proves that a purchase, fight, or other objective has been completed.
use crate::{Guide, Progress, Step, SyncEstimate};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
pub struct MapAsset {
    pub id: String,
    pub title: String,
    pub url: String,
    pub source_url: String,
    pub credit: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
pub struct Place {
    pub id: String,
    pub label: String,
    pub map_id: Option<String>,
    pub landmarks: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
pub struct PlaceBinding {
    pub step_id: String,
    pub place_id: String,
    pub evidence: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct Atlas {
    pub guide_id: String,
    pub guide_revision: u64,
    pub maps: Vec<MapAsset>,
    pub places: Vec<Place>,
    pub bindings: Vec<PlaceBinding>,
    pub reviewed_steps: Vec<String>,
    pub status: String,
    pub notes: Vec<String>,
    #[serde(default)]
    pub map_links: Vec<MapLink>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MapLink {
    pub place_id: String,
    pub map_id: Option<String>,
    pub evidence: String,
}

pub fn public_https(value: &str) -> Result<()> {
    let url = url::Url::parse(value)?;
    ensure!(
        url.scheme() == "https" && url.username().is_empty() && url.password().is_none(),
        "Map sources require HTTPS without credentials"
    );
    let host = url.host_str().unwrap_or_default();
    ensure!(
        !host.is_empty()
            && host != "localhost"
            && !host.ends_with(".local")
            && !host.ends_with(".localhost")
            && host.parse::<std::net::IpAddr>().is_err()
            && !host.starts_with('['),
        "Use a public map source hostname"
    );
    Ok(())
}

impl Atlas {
    pub fn validate(&self, guide: &Guide) -> Result<()> {
        ensure!(
            self.guide_id == guide.id && self.guide_revision == guide.revision,
            "Atlas belongs to another guide revision"
        );
        let maps: HashSet<_> = self.maps.iter().map(|m| &m.id).collect();
        let places: HashSet<_> = self.places.iter().map(|p| &p.id).collect();
        ensure!(
            maps.len() == self.maps.len() && places.len() == self.places.len(),
            "Duplicate atlas identifiers"
        );
        for map in &self.maps {
            ensure!(
                !map.id.is_empty() && !map.title.trim().is_empty(),
                "Map identity is missing"
            );
            public_https(&map.url)?;
            public_https(&map.source_url)?;
        }
        for place in &self.places {
            ensure!(
                !place.id.is_empty() && !place.label.trim().is_empty(),
                "Place identity is missing"
            );
            ensure!(
                place.map_id.as_ref().is_none_or(|id| maps.contains(id)),
                "Place references a missing map"
            );
        }
        let mut bound = HashSet::new();
        for binding in &self.bindings {
            ensure!(
                guide.step(&binding.step_id).is_some() && places.contains(&binding.place_id),
                "Atlas binding references an unknown step or place"
            );
            ensure!(
                bound.insert(&binding.step_id) && !binding.evidence.trim().is_empty(),
                "Each step needs one unambiguous, evidenced place binding"
            );
        }
        ensure!(
            self.reviewed_steps
                .iter()
                .all(|id| guide.step(id).is_some()),
            "Unknown reviewed atlas step"
        );
        let mut links = HashSet::new();
        for link in &self.map_links {
            ensure!(
                places.contains(&link.place_id)
                    && links.insert(&link.place_id)
                    && link.map_id.as_ref().is_none_or(|id| maps.contains(id))
                    && !link.evidence.trim().is_empty(),
                "Invalid or unevidenced map association"
            );
        }
        Ok(())
    }

    pub fn place_for_step(&self, guide: &Guide, id: &str) -> Option<&Place> {
        let step = guide.step(id)?;
        if !guide.step_enabled(step) {
            return None;
        }
        let binding = self.bindings.iter().find(|b| b.step_id == id)?;
        self.places.iter().find(|p| p.id == binding.place_id)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(default)]
pub struct PlaceTracker {
    pub observed_at_ms: u64,
    pub epoch: u64,
    pub place_id: Option<String>,
    pub reference_step_id: Option<String>,
    pub status: String,
    pub detail: String,
    pub located_at_ms: u64,
    pub session_id: String,
    pending: Option<String>,
    votes: u32,
    last_place: Option<String>,
    room_entry_step: Option<String>,
}

impl PlaceTracker {
    pub fn for_session(session_id: String) -> Self {
        Self {
            session_id,
            ..Default::default()
        }
    }
    pub fn adopt_visual(&mut self, other: &Self, now: u64) {
        if other.epoch != self.epoch
            || other.status != "estimated"
            || other.located_at_ms <= self.located_at_ms
            || now.saturating_sub(other.located_at_ms) > 30_000
        {
            return;
        }
        self.located_at_ms = other.located_at_ms;
        if self.status == "reported" && self.place_id == other.place_id {
            return;
        }
        if self.place_id != other.place_id {
            self.reference_step_id = other.reference_step_id.clone();
            self.room_entry_step = other.reference_step_id.clone();
        }
        self.place_id = other.place_id.clone();
        self.last_place = other.place_id.clone();
        self.status = "estimated".into();
        self.detail = other.detail.clone();
    }
    /// A changed screen invalidates a visual estimate immediately. Explicit
    /// player reports remain authoritative until newer evidence locates them.
    pub fn invalidate_visual(&mut self) {
        if self.status == "reported" {
            return;
        }
        self.place_id = None;
        self.reference_step_id = None;
        self.pending = None;
        self.votes = 0;
        self.status = "uncertain".into();
        self.detail =
            "Matching the current screen. The previous room is no longer a live location estimate."
                .into();
    }
    /// Changes the location cursor only after geometric frame agreement.
    /// This cannot change completion or inventory.
    pub fn ground(&mut self, place: &str, step: &str, epoch: u64, at: u64, evidence: &str) {
        self.report(place, step, epoch, at, evidence);
        self.status = "estimated".into();
        self.detail = evidence.into();
    }
    /// Atlas geometry can identify a room without identifying a task there.
    pub fn ground_place(&mut self, place: &str, epoch: u64, at: u64, evidence: &str) {
        self.ground(place, "", epoch, at, evidence);
        self.reference_step_id = None;
        self.room_entry_step = None;
    }
    pub fn report(&mut self, place: &str, step: &str, epoch: u64, at: u64, report: &str) {
        *self = Self {
            epoch,
            observed_at_ms: at,
            located_at_ms: at,
            place_id: Some(place.into()),
            reference_step_id: Some(step.into()),
            status: "reported".into(),
            detail: format!("Your location report: {report}"),
            last_place: Some(place.into()),
            room_entry_step: Some(step.into()),
            session_id: self.session_id.clone(),
            ..Default::default()
        };
    }
    pub fn observe(
        &mut self,
        guide: &Guide,
        atlas: &Atlas,
        estimate: &SyncEstimate,
        epoch: u64,
        now: u64,
    ) {
        if self.epoch != epoch {
            *self = Self {
                epoch,
                session_id: self.session_id.clone(),
                ..Default::default()
            };
        }
        let retained = self.place_id.is_some()
            && (self.status == "reported" || now.saturating_sub(self.located_at_ms) <= 30_000);
        if estimate.epoch != epoch
            || estimate.observed_at_ms == 0
            || now.saturating_sub(estimate.observed_at_ms) > 30_000
        {
            if retained {
                return;
            }
            self.place_id = None;
            self.reference_step_id = None;
            self.pending = None;
            self.votes = 0;
            self.status = "unknown".into();
            self.detail =
                "Waiting for a fresh location observation. No player coordinates are available."
                    .into();
            return;
        }
        if estimate.observed_at_ms <= self.observed_at_ms {
            return;
        }
        self.observed_at_ms = estimate.observed_at_ms;
        if !retained {
            self.place_id = None;
            self.reference_step_id = None;
        }
        let direct = estimate
            .located_step
            .as_deref()
            .filter(|_| {
                matches!(
                    estimate.status.as_str(),
                    "located" | "vision_estimate" | "confirmed"
                )
            })
            .and_then(|id| {
                atlas
                    .place_for_step(guide, id)
                    .map(|p| (p.id.clone(), id.to_owned()))
            });
        // Several reference steps can describe the same room. Aggregate by place
        // before comparing margins; never ignore an unmapped strong competitor.
        let inferred = estimate.candidates.first().and_then(|best| {
            let place = atlas.place_for_step(guide, &best.step_id)?;
            let other = estimate
                .candidates
                .iter()
                .skip(1)
                .filter(|c| {
                    atlas
                        .place_for_step(guide, &c.step_id)
                        .is_none_or(|p| p.id != place.id)
                })
                .map(|c| c.visual_score)
                .fold(0.0_f32, f32::max);
            (best.visual_score >= 0.82 && best.visual_score - other >= 0.035)
                .then(|| (place.id.clone(), best.step_id.clone()))
        });
        if let Some((place, step)) = direct.clone().or(inferred) {
            if self.pending.as_ref() == Some(&place) {
                self.votes += 1;
            } else {
                self.pending = Some(place.clone());
                self.votes = 1;
            }
            // One frame must not overrule what the player just told us. Later
            // repeated observations may move the location as play continues.
            if (direct.is_some() && self.status != "reported") || self.votes >= 2 {
                if self.status == "reported" && self.place_id.as_ref() == Some(&place) {
                    self.located_at_ms = estimate.observed_at_ms;
                    return;
                }
                // Keep the action cursor stable within a room. Repeated visits
                // can have identical screenshots but different source steps.
                if self.last_place.as_ref() != Some(&place) || estimate.status == "confirmed" {
                    self.last_place = Some(place.clone());
                    self.room_entry_step = Some(step);
                }
                self.place_id = Some(place);
                self.reference_step_id = self.room_entry_step.clone();
                self.located_at_ms = estimate.observed_at_ms;
                self.status = "estimated".into();
                self.detail = "Estimated room from reference evidence; player position within the map is unknown.".into();
                return;
            }
        } else {
            self.pending = None;
            self.votes = 0;
        }
        if retained {
            return;
        }
        self.status = "uncertain".into();
        self.detail = "Location is uncertain; waiting for a repeated, distinct room match.".into();
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Guidance {
    pub status: String,
    pub now: Option<Step>,
    pub next: Vec<Step>,
    pub target: Option<Step>,
    pub requirements: Vec<Step>,
    pub reason: String,
}

/// Compiler bookkeeping remains in the source graph for coverage/provenance,
/// but is never an instruction for the player (including older saved guides).
pub fn is_player_action(step: &Step) -> bool {
    let instructions = step.instructions.trim();
    !instructions.is_empty()
        && ![
            "Video segment awaiting interpretation.",
            "Chapter supplied by the video's author.",
            "Reference reviewed:",
            "Detailed tasks below",
        ]
        .iter()
        .any(|prefix| instructions.starts_with(prefix))
        && !(instructions.starts_with("Follow the ")
            && instructions.contains("Expand each timed section"))
}

/// A player-facing projection, preserving original IDs and source anchors.
/// Chapters group actions; source sampling intervals are flattened out.
pub fn player_outline(guide: &Guide) -> Vec<Step> {
    fn visit(
        guide: &Guide,
        parent: Option<&str>,
        visible_parent: Option<&str>,
        out: &mut Vec<Step>,
    ) {
        for step in guide
            .steps
            .iter()
            .filter(|s| s.parent_id.as_deref() == parent)
        {
            if !guide.step_enabled(step) {
                continue;
            }
            let mut children = vec![];
            visit(guide, Some(&step.id), Some(&step.id), &mut children);
            let section = step.instructions.starts_with("Detailed tasks below")
                || step.instructions.starts_with("Reference reviewed:")
                || step
                    .instructions
                    .starts_with("Video segment awaiting interpretation.");
            if section || (!is_player_action(step) && children.is_empty()) {
                for child in &mut children {
                    if child.parent_id.as_deref() == Some(&step.id) {
                        child.parent_id = visible_parent.map(str::to_owned);
                    }
                }
                out.extend(children);
            } else {
                let mut node = step.clone();
                node.parent_id = visible_parent.map(str::to_owned);
                out.push(node);
                out.extend(children);
            }
        }
    }
    let mut result = vec![];
    visit(guide, None, None, &mut result);
    result
}

pub fn step_complete(guide: &Guide, progress: &Progress, id: &str) -> bool {
    let mut current = Some(id);
    while let Some(id) = current {
        if progress.completed.contains(id) || progress.reported_completed.contains(id) {
            return true;
        }
        current = guide.step(id).and_then(|s| s.parent_id.as_deref());
    }
    let children: Vec<_> = guide
        .steps
        .iter()
        .filter(|s| s.parent_id.as_deref() == Some(id))
        .collect();
    !children.is_empty()
        && children
            .iter()
            .all(|s| step_complete(guide, progress, &s.id))
}

fn unfinished<'a>(
    guide: &'a Guide,
    progress: &Progress,
    id: &str,
    visited: &mut HashSet<String>,
) -> Option<&'a Step> {
    if !visited.insert(id.to_owned()) {
        return None;
    }
    let step = guide.step(id)?;
    if !guide.step_enabled(step) {
        return None;
    }
    if !step_complete(guide, progress, id) {
        let children: Vec<_> = guide
            .steps
            .iter()
            .filter(|s| s.parent_id.as_deref() == Some(id))
            .collect();
        if children.is_empty() && is_player_action(step) {
            return Some(step);
        }
        for child in children {
            if !step_complete(guide, progress, &child.id)
                && let Some(s) = unfinished(guide, progress, &child.id, visited)
            {
                return Some(s);
            }
        }
    }
    for next in route_successors(guide, step) {
        if let Some(s) = unfinished(guide, progress, next, visited) {
            return Some(s);
        }
    }
    None
}

/// A container's outgoing route starts after its last descendant. Video
/// chapters often store the cross-chapter edge only on that container.
pub fn route_successors<'a>(guide: &'a Guide, step: &'a Step) -> Vec<&'a str> {
    let mut result = vec![];
    let mut cursor = step;
    loop {
        for next in &cursor.next {
            let mut target = guide.step(next);
            let mut enters_self = false;
            while let Some(node) = target {
                if node.id == cursor.id {
                    enters_self = true;
                    break;
                }
                target = node.parent_id.as_deref().and_then(|id| guide.step(id));
            }
            if !enters_self && !result.contains(&next.as_str()) {
                result.push(next.as_str());
            }
        }
        let Some(parent) = cursor.parent_id.as_deref().and_then(|id| guide.step(id)) else {
            break;
        };
        if guide
            .steps
            .iter()
            .rev()
            .find(|s| s.parent_id.as_deref() == Some(&parent.id))
            .is_none_or(|last| last.id != cursor.id)
        {
            break;
        }
        cursor = parent;
    }
    result
}

fn requirements<'a>(guide: &'a Guide, progress: &Progress, step: &'a Step) -> Vec<&'a Step> {
    let mut result = vec![];
    let mut current = Some(step);
    while let Some(s) = current {
        for id in &s.prerequisites {
            if !step_complete(guide, progress, id)
                && let Some(required) = guide.step(id)
                && !result.iter().any(|r: &&Step| r.id == required.id)
            {
                result.push(required);
            }
        }
        current = s.parent_id.as_deref().and_then(|id| guide.step(id));
    }
    result
}

/// Deterministic advice from the editable graph. Unknown prerequisites remain
/// outstanding; previewing a route does not persist hypothetical completion.
pub fn guidance(guide: &Guide, progress: &Progress, location: &PlaceTracker) -> Guidance {
    guidance_from(guide, progress, location, None)
}

/// A conversational recommendation selects a plan action without claiming
/// that the player reached it or completed any preceding task.
pub fn guidance_from(
    guide: &Guide,
    progress: &Progress,
    location: &PlaceTracker,
    recommendation: Option<&str>,
) -> Guidance {
    let goal_ids: Vec<_> = guide.goals.iter().flat_map(|g| &g.step_ids).collect();
    if !goal_ids.is_empty() && goal_ids.iter().all(|id| step_complete(guide, progress, id)) {
        return Guidance { status: "goals_complete".into(), reason: "Your selected goals are complete according to saved confirmations and player reports.".into(), ..Default::default() };
    }
    let goal = guide
        .goals
        .iter()
        .flat_map(|g| &g.step_ids)
        .filter(|id| !step_complete(guide, progress, id))
        .find_map(|id| unfinished(guide, progress, id, &mut HashSet::new()));
    if !goal_ids.is_empty() && goal.is_none() {
        return Guidance { status: "blocked".into(), reason: "The selected goal has no enabled unfinished instructions. Enable its reference or adjust the outline.".into(), ..Default::default() };
    }
    let start = recommendation
        .and_then(|id| unfinished(guide, progress, id, &mut HashSet::new()))
        .or(goal)
        .or_else(|| {
            location
                .reference_step_id
                .as_deref()
                .or(progress.current_step.as_deref())
                .and_then(|id| unfinished(guide, progress, id, &mut HashSet::new()))
        })
        .or_else(|| {
            guide
                .steps
                .iter()
                .filter(|s| s.parent_id.is_none())
                .find_map(|s| unfinished(guide, progress, &s.id, &mut HashSet::new()))
        });
    let Some(target) = start else {
        return Guidance {
            status: "waiting".into(),
            reason: "No unfinished player actions are available. Reference preparation may still be in progress."
                .into(),
            ..Default::default()
        };
    };
    let mut selected = target;
    let mut seen = HashSet::new();
    let mut missing = vec![];
    loop {
        if !seen.insert(selected.id.clone()) {
            return Guidance { status: "blocked".into(), target: Some(target.clone()), reason: "The inherited prerequisites conflict. Adjust the outline before following this route.".into(), ..Default::default() };
        }
        let required = requirements(guide, progress, selected);
        missing.extend(
            required
                .iter()
                .map(|s| (*s).clone())
                .filter(|s| !missing.iter().any(|r: &Step| r.id == s.id))
                .collect::<Vec<_>>(),
        );
        let Some(first) = required.first() else {
            break;
        };
        let Some(next) = unfinished(guide, progress, &first.id, &mut HashSet::new()) else {
            return Guidance { status: "blocked".into(), target: Some(target.clone()), requirements: missing, reason: "A prerequisite has no enabled instructions. Enable its reference or adjust the outline.".into(), ..Default::default() };
        };
        selected = next;
    }
    let reason = if !missing.is_empty() {
        "A prerequisite is still unconfirmed. Complete or confirm it before the target task."
    } else if goal.is_some() {
        "Following your selected goal and its prerequisites."
    } else if location.status == "reported" {
        "Suggested from the location you reported and unfinished walkthrough tasks."
    } else if location.place_id.is_some()
        && location.reference_step_id.is_none()
        && progress.current_step.is_none()
    {
        "Your location is matched, but task progress is unconfirmed. Showing the next unfinished action in the saved plan."
    } else if location.place_id.is_some() {
        "Suggested from the estimated room and unfinished walkthrough tasks."
    } else {
        "Next unfinished action in your plan. Your location is not yet confirmed; this does not mean you have reached this step."
    };
    let mut preview = progress.clone();
    preview.completed.insert(selected.id.clone());
    let mut next = vec![];
    let mut cursor = selected;
    for _ in 0..2 {
        let candidate = if selected.id != target.id && !step_complete(guide, &preview, &target.id) {
            Some(target)
        } else {
            route_successors(guide, cursor)
                .into_iter()
                .find_map(|id| unfinished(guide, &preview, id, &mut HashSet::new()))
        };
        let Some(mut candidate) = candidate else {
            break;
        };
        let mut seen = HashSet::new();
        let mut blocked = false;
        loop {
            if !seen.insert(&candidate.id) {
                blocked = true;
                break;
            }
            let required = requirements(guide, &preview, candidate);
            let Some(first) = required.first() else {
                break;
            };
            let Some(required) = unfinished(guide, &preview, &first.id, &mut HashSet::new()) else {
                blocked = true;
                break;
            };
            candidate = required;
        }
        if blocked {
            break;
        }
        if candidate.id == selected.id || next.iter().any(|s: &Step| s.id == candidate.id) {
            break;
        }
        next.push(candidate.clone());
        preview.completed.insert(candidate.id.clone());
        cursor = candidate;
    }
    Guidance {
        status: if location.place_id.is_none() {
            "unlocated"
        } else if location.reference_step_id.is_none()
            && progress.current_step.is_none()
            && goal.is_none()
        {
            "plan"
        } else {
            "ready"
        }
        .into(),
        now: Some(selected.clone()),
        next,
        target: Some(target.clone()),
        requirements: missing,
        reason: reason.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Candidate, PlayerGoal, ReferencePreference, import_markdown};
    #[test]
    fn player_outline_skips_source_intervals_and_plans_without_a_location_lock() {
        let guide = import_markdown("Game", "Video", "# Introduction\n## Intro · 0:00\nReference reviewed: no actionable steps.\n# Town\nFollow the 2 detailed actions below. Expand each timed section for tasks.\n## Town · 0:04\nDetailed tasks below follow the demonstrated route.\n### Enter town\nWalk through the right door.\n### Get the ring\nTalk to the guru.\n# Unreviewed\nVideo segment awaiting interpretation.").unwrap();
        let visible = player_outline(&guide);
        assert_eq!(
            visible.iter().map(|s| s.title.as_str()).collect::<Vec<_>>(),
            vec!["Town", "Enter town", "Get the ring"]
        );
        assert_eq!(visible[1].parent_id.as_deref(), Some("step-3"));
        assert_eq!(visible[1].anchors, guide.steps[4].anchors);
        let mut progress = Progress::default();
        let advice = guidance(&guide, &progress, &PlaceTracker::default());
        assert_eq!(advice.now.unwrap().id, "step-5");
        assert_eq!(advice.status, "unlocated");
        assert!(progress.completed.is_empty());
        progress.completed.insert("step-5".into());
        assert_eq!(
            guidance(&guide, &progress, &PlaceTracker::default())
                .now
                .unwrap()
                .id,
            "step-6"
        );
        progress.completed.insert("step-6".into());
        assert!(
            guidance(&guide, &progress, &PlaceTracker::default())
                .now
                .is_none()
        );
    }
    fn guide() -> Guide {
        let mut guide = import_markdown(
            "Game",
            "Route",
            "# Farm\nEarn gold.\n# Buy\nBuy the spell.\n# Exit\nLeave the shop.",
        )
        .unwrap();
        guide.steps[1].prerequisites = vec!["step-1".into()];
        guide
    }
    fn atlas(guide: &Guide) -> Atlas {
        Atlas {
            guide_id: guide.id.clone(),
            guide_revision: guide.revision,
            places: vec![Place {
                id: "shop".into(),
                label: "Shop".into(),
                map_id: None,
                landmarks: "Counter".into(),
            }],
            bindings: ["step-2", "step-3"]
                .into_iter()
                .map(|id| PlaceBinding {
                    step_id: id.into(),
                    place_id: "shop".into(),
                    evidence: "Shop counter in reference".into(),
                })
                .collect(),
            ..Default::default()
        }
    }
    fn location(step: &str) -> PlaceTracker {
        PlaceTracker {
            place_id: Some("shop".into()),
            reference_step_id: Some(step.into()),
            ..Default::default()
        }
    }
    #[test]
    fn same_room_changes_advice_with_verified_progress_without_completing_by_location() {
        let guide = guide();
        let mut progress = Progress::default();
        let before = guidance(&guide, &progress, &location("step-2"));
        assert_eq!(before.now.unwrap().id, "step-1");
        assert_eq!(before.requirements.len(), 1);
        assert!(progress.completed.is_empty());
        progress.completed.insert("step-1".into());
        assert_eq!(
            guidance(&guide, &progress, &location("step-2"))
                .now
                .unwrap()
                .id,
            "step-2"
        );
        progress.completed.insert("step-2".into());
        assert_eq!(
            guidance(&guide, &progress, &location("step-2"))
                .now
                .unwrap()
                .id,
            "step-3"
        );
        assert_eq!(progress.completed.len(), 2);
    }
    #[test]
    fn lookahead_resolves_all_dependencies_before_previewing_the_target() {
        let mut guide = guide();
        guide.steps[1].prerequisites.clear();
        guide.steps[2].prerequisites = vec!["step-1".into(), "step-2".into()];
        let advice = guidance(&guide, &Progress::default(), &location("step-3"));
        assert_eq!(advice.now.unwrap().id, "step-1");
        assert_eq!(
            advice
                .next
                .iter()
                .map(|s| s.id.as_str())
                .collect::<Vec<_>>(),
            vec!["step-2", "step-3"]
        );
    }
    #[test]
    fn completing_a_chapters_last_substep_follows_the_container_route() {
        let mut guide = import_markdown(
            "Game",
            "Guide",
            "# Town\n## Leave\nExit town\n# Forest\n## Enter\nGo inside",
        )
        .unwrap();
        guide.steps[0].next = vec!["step-3".into()];
        guide.steps[1].next.clear();
        let progress = Progress {
            completed: HashSet::from(["step-2".into()]),
            ..Default::default()
        };
        assert_eq!(
            guidance(&guide, &progress, &location("step-2"))
                .now
                .unwrap()
                .id,
            "step-4"
        );
        assert!(
            route_successors(&guide, &guide.steps[2]).is_empty(),
            "Containment must not become a route back into a completed chapter"
        );
    }
    #[test]
    fn changed_goal_and_inherited_prerequisites_are_respected() {
        let mut guide = guide();
        guide.goals = vec![PlayerGoal {
            id: "spell".into(),
            description: "Get the spell".into(),
            step_ids: vec!["step-2".into()],
        }];
        assert_eq!(
            guidance(&guide, &Progress::default(), &location("step-3"))
                .now
                .unwrap()
                .id,
            "step-1"
        );
        let mut child = guide.steps[1].clone();
        child.id = "purchase".into();
        child.parent_id = Some("step-2".into());
        child.prerequisites.clear();
        child.next.clear();
        guide.steps.push(child);
        let advice = guidance(&guide, &Progress::default(), &location("purchase"));
        assert_eq!(advice.now.unwrap().id, "step-1");
        let mut progress = Progress::default();
        progress.completed.insert("purchase".into());
        assert!(step_complete(&guide, &progress, "step-2"));
    }
    #[test]
    fn disabled_prerequisite_blocks_instead_of_silently_skipping_it() {
        let mut guide = guide();
        let mut source = guide.sources[0].clone();
        source.id = "other".into();
        guide.sources.push(source);
        guide.steps[0].anchors[0].source_id = "other".into();
        guide.reference_preferences = vec![ReferencePreference {
            source_id: "other".into(),
            enabled: false,
            preference: String::new(),
        }];
        assert_eq!(
            guidance(&guide, &Progress::default(), &location("step-2")).status,
            "blocked"
        );
    }
    #[test]
    fn repeated_room_matches_ignore_visit_identity_but_abstain_for_unknown_competitors() {
        let guide = guide();
        let atlas = atlas(&guide);
        let candidate = |step: &str, score| Candidate {
            step_id: step.into(),
            reference_id: step.into(),
            source_id: "text-1".into(),
            reference_ms: 0,
            image_path: String::new(),
            visual_score: score,
            rank_score: score,
        };
        let mut estimate = SyncEstimate {
            observed_at_ms: 100,
            candidates: vec![candidate("step-2", 0.91), candidate("step-3", 0.90)],
            ..Default::default()
        };
        let mut tracker = PlaceTracker::default();
        tracker.observe(&guide, &atlas, &estimate, 0, 100);
        assert!(tracker.place_id.is_none());
        tracker.observe(&guide, &atlas, &estimate, 0, 101);
        assert!(
            tracker.place_id.is_none(),
            "Polling one frame must not add votes"
        );
        estimate.observed_at_ms = 200;
        tracker.observe(&guide, &atlas, &estimate, 0, 200);
        assert_eq!(tracker.place_id.as_deref(), Some("shop"));
        estimate.observed_at_ms = 250;
        estimate.candidates.swap(0, 1);
        tracker.observe(&guide, &atlas, &estimate, 0, 250);
        assert_eq!(
            tracker.reference_step_id.as_deref(),
            Some("step-2"),
            "A different screenshot of the same room must not move the objective cursor"
        );
        estimate.observed_at_ms = 300;
        estimate.candidates.push(candidate("step-1", 0.90));
        tracker.observe(&guide, &atlas, &estimate, 0, 300);
        assert_eq!(
            tracker.place_id.as_deref(),
            Some("shop"),
            "One ambiguous frame retains a recent positive location"
        );
        let mut unknown = PlaceTracker::default();
        unknown.observe(&guide, &atlas, &estimate, 0, 300);
        assert!(
            unknown.place_id.is_none(),
            "Ambiguous evidence cannot establish a new location"
        );
        estimate.located_step = Some("step-2".into());
        estimate.status = "located".into();
        estimate.observed_at_ms = 400;
        tracker.observe(&guide, &atlas, &estimate, 0, 400);
        assert!(tracker.place_id.is_some());
        tracker.observe(&guide, &atlas, &estimate, 0, 31_000);
        assert!(
            tracker.place_id.is_none(),
            "Stale observations must clear live room selection"
        );
        tracker.observe(&guide, &atlas, &estimate, 1, 400);
        assert!(
            tracker.place_id.is_none(),
            "An old epoch must not relocalize a reloaded game"
        );
    }
    #[test]
    fn no_observation_does_not_claim_a_location() {
        let progress = Progress {
            current_step: Some("step-2".into()),
            ..Default::default()
        };
        let advice = guidance(&guide(), &progress, &PlaceTracker::default());
        assert_eq!(advice.status, "unlocated");
    }
    #[test]
    fn completed_goal_stops_and_disabled_goal_does_not_fall_through_to_an_unrelated_route() {
        let mut guide = guide();
        guide.goals = vec![PlayerGoal {
            id: "spell".into(),
            description: "Get the spell".into(),
            step_ids: vec!["step-2".into()],
        }];
        let progress = Progress {
            reported_completed: HashSet::from(["step-2".into()]),
            ..Default::default()
        };
        assert_eq!(
            guidance(&guide, &progress, &location("step-3")).status,
            "goals_complete"
        );
        guide.reference_preferences = vec![ReferencePreference {
            source_id: "text-1".into(),
            enabled: false,
            preference: "Use a different reference".into(),
        }];
        assert_eq!(
            guidance(&guide, &Progress::default(), &location("step-3")).status,
            "blocked"
        );
    }
}
