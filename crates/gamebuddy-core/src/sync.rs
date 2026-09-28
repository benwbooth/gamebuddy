use crate::{Guide, VisualIndex, fingerprint_has_detail, similarity};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Candidate {
    pub step_id: String,
    pub reference_id: String,
    pub source_id: String,
    pub reference_ms: u64,
    pub image_path: String,
    pub visual_score: f32,
    pub rank_score: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SyncEstimate {
    pub observed_at_ms: u64,
    pub epoch: u64,
    pub status: String,
    pub candidates: Vec<Candidate>,
    pub located_step: Option<String>,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Progress {
    pub epoch: u64,
    pub current_step: Option<String>,
    pub completed: HashSet<String>,
    /// Explicit player reports received through the voice/agent harness.
    /// Kept distinct from the companion's direct confirmation controls.
    #[serde(default)]
    pub reported_completed: HashSet<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Synchronizer {
    pub progress: Progress,
    last_observation: Option<u64>,
    pending_step: Option<String>,
    pending_count: u32,
}

impl Synchronizer {
    pub fn new(progress: Progress) -> Self {
        Self {
            progress,
            ..Default::default()
        }
    }
    pub fn reset(&mut self) {
        self.progress.epoch += 1;
        self.progress.current_step = None;
        self.progress.completed.clear();
        self.progress.reported_completed.clear();
        self.last_observation = None;
        self.pending_step = None;
        self.pending_count = 0;
    }
    pub fn confirm_step(&mut self, guide: &Guide, step: &str) -> Result<()> {
        ensure!(guide.step(step).is_some(), "Unknown step");
        self.progress.current_step = Some(step.into());
        self.pending_step = None;
        self.pending_count = 0;
        Ok(())
    }
    pub fn confirm_complete(&mut self, guide: &Guide, step: &str) -> Result<()> {
        ensure!(guide.step(step).is_some(), "Unknown step");
        self.progress.completed.insert(step.into());
        Ok(())
    }
    pub fn observe(
        &mut self,
        guide: &Guide,
        index: &VisualIndex,
        vector: &[f32],
        at_ms: u64,
        epoch: u64,
    ) -> Result<SyncEstimate> {
        ensure!(
            index.guide_id == guide.id && index.guide_revision == guide.revision,
            "Visual index belongs to a different guide revision; rebuild it"
        );
        ensure!(
            epoch == self.progress.epoch,
            "Observation belongs to an earlier session"
        );
        ensure!(
            self.last_observation.is_none_or(|last| at_ms > last),
            "Stale observation ignored"
        );
        if !fingerprint_has_detail(vector) {
            self.last_observation = Some(at_ms);
            self.pending_step = None;
            self.pending_count = 0;
            return Ok(SyncEstimate {
                observed_at_ms: at_ms, epoch, status: "unmatched".into(),
                detail: "The frame has too little visible detail to locate the player (possibly a fade or loading screen).".into(),
                ..Default::default()
            });
        }
        let mut by_step: HashMap<String, Candidate> = HashMap::new();
        for frame in &index.frames {
            if !guide.source_enabled(&frame.source_id)
                || !fingerprint_has_detail(&frame.fingerprint)
            {
                continue;
            }
            let visual_score = similarity(vector, &frame.fingerprint)?;
            for step_id in &frame.step_ids {
                let Some(step) = guide.step(step_id) else {
                    continue;
                };
                let current = self.progress.current_step.as_deref();
                let continuity = if current == Some(step_id.as_str()) {
                    0.018
                } else if current
                    .and_then(|s| guide.step(s))
                    .is_some_and(|s| s.next.contains(step_id))
                {
                    0.01
                } else {
                    0.0
                };
                let missing = step
                    .prerequisites
                    .iter()
                    .filter(|id| !self.progress.completed.contains(*id))
                    .count();
                let rank_score = visual_score + continuity - (missing as f32 * 0.005).min(0.02);
                let candidate = Candidate {
                    step_id: step_id.clone(),
                    reference_id: frame.id.clone(),
                    source_id: frame.source_id.clone(),
                    reference_ms: frame.at_ms,
                    image_path: frame.image_path.clone(),
                    visual_score,
                    rank_score,
                };
                if by_step
                    .get(step_id)
                    .is_none_or(|existing| existing.rank_score < rank_score)
                {
                    by_step.insert(step_id.clone(), candidate);
                }
            }
        }
        let mut candidates: Vec<_> = by_step.into_values().collect();
        candidates.sort_by(|a, b| {
            b.rank_score
                .total_cmp(&a.rank_score)
                .then_with(|| a.step_id.cmp(&b.step_id))
        });
        // Preserve ambiguous task IDs sharing an image, but do not let them
        // occupy every slot intended for distinct visual alternatives.
        let mut images = HashSet::new();
        candidates.retain(|c| {
            images.contains(&c.image_path)
                || (images.len() < 4 && images.insert(c.image_path.clone()))
        });
        self.last_observation = Some(at_ms);
        let mut estimate = SyncEstimate {
            observed_at_ms: at_ms,
            epoch,
            status: "unmatched".into(),
            candidates,
            located_step: None,
            detail: "No reference confidently matches this frame.".into(),
        };
        if let Some(best) = estimate.candidates.first() {
            let margin = estimate
                .candidates
                .get(1)
                .map(|next| best.visual_score - next.visual_score)
                .unwrap_or(1.0);
            if best.visual_score >= 0.82 && margin >= 0.035 {
                if self.pending_step.as_deref() == Some(best.step_id.as_str()) {
                    self.pending_count += 1;
                } else {
                    self.pending_step = Some(best.step_id.clone());
                    self.pending_count = 1;
                }
                if self.pending_count >= 2 {
                    self.progress.current_step = Some(best.step_id.clone());
                    estimate.located_step = Some(best.step_id.clone());
                    estimate.status = "located".into();
                    estimate.detail="Repeated visual match. Objective completion still needs evidence or your confirmation.".into();
                } else {
                    estimate.status = "candidate".into();
                    estimate.detail = "Promising match; waiting for another observation.".into();
                }
            } else {
                self.pending_step = None;
                self.pending_count = 0;
                if best.visual_score >= 0.72 {
                    estimate.status = "ambiguous".into();
                    estimate.detail="Several steps look similar. Compare with the local vision model or confirm your location.".into();
                }
            }
        } else {
            self.pending_step = None;
            self.pending_count = 0;
        }
        Ok(estimate)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::*;
    #[test]
    fn cached_blank_references_cannot_outrank_dark_gameplay() {
        let (g, mut idx) = fixture();
        let mut blank = idx.frames[0].clone();
        blank.id = "fade".into();
        blank.image_path = "fade.png".into();
        blank.fingerprint.fill(0.0);
        let live = [0.0, 0.0, 0.0, 0.1, 0.05, 0.05];
        assert!(
            similarity(&live, &blank.fingerprint).unwrap()
                > similarity(&live, &idx.frames[0].fingerprint).unwrap()
        );
        idx.frames.push(blank);
        let estimate = Synchronizer::default()
            .observe(&g, &idx, &live, 1, 0)
            .unwrap();
        assert!(!estimate.candidates.is_empty());
        assert!(estimate.candidates.iter().all(|c| c.reference_id != "fade"));
    }
    #[test]
    fn live_fades_break_confirmation_votes_without_erasing_progress() {
        let (g, idx) = fixture();
        let mut sync = Synchronizer::default();
        let live = &idx.frames[0].fingerprint;
        assert_eq!(
            sync.observe(&g, &idx, live, 1, 0).unwrap().status,
            "candidate"
        );
        let faded = sync.observe(&g, &idx, &[0.0; 6], 2, 0).unwrap();
        assert!(faded.candidates.is_empty());
        assert_eq!(
            sync.observe(&g, &idx, live, 3, 0).unwrap().status,
            "candidate"
        );
        sync.observe(&g, &idx, live, 4, 0).unwrap();
        sync.observe(&g, &idx, &[0.0; 6], 5, 0).unwrap();
        assert_eq!(sync.progress.current_step.as_deref(), Some("step-1"));
        assert!(sync.progress.completed.is_empty());
    }
    #[test]
    fn shared_frame_tasks_preserve_ambiguity_without_crowding_out_other_images() {
        let (_, mut idx) = fixture();
        let g = import_markdown("Game", "Route", "# A\n# B\n# C\n# D\n# E").unwrap();
        idx.guide_id = g.id.clone();
        idx.frames[0].step_ids = (1..=4).map(|i| format!("step-{i}")).collect();
        idx.frames[1].step_ids = vec!["step-5".into()];
        let estimate = Synchronizer::default()
            .observe(&g, &idx, &idx.frames[0].fingerprint, 1, 0)
            .unwrap();
        assert_eq!(estimate.candidates.len(), 5);
        assert!(estimate.candidates.iter().any(|c| c.image_path == "b.png"));
        assert_eq!(estimate.status, "ambiguous");
        assert!(estimate.located_step.is_none());
    }
    fn fixture() -> (Guide, VisualIndex) {
        let g = import_markdown("Game", "Test", "# Town\n# Forest").unwrap();
        let idx = VisualIndex {
            guide_id: g.id.clone(),
            guide_revision: g.revision,
            encoder: FINGERPRINT_VERSION.into(),
            crop: Crop::default(),
            live_crop: Crop::default(),
            frames: vec![
                ReferenceFrame {
                    id: "a".into(),
                    source_id: "text-1".into(),
                    at_ms: 0,
                    image_path: "a.png".into(),
                    step_ids: vec!["step-1".into()],
                    fingerprint: vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
                },
                ReferenceFrame {
                    id: "b".into(),
                    source_id: "text-1".into(),
                    at_ms: 1000,
                    image_path: "b.png".into(),
                    step_ids: vec!["step-2".into()],
                    fingerprint: vec![1.0, 1.0, 1.0, 0.0, 0.0, 0.0],
                },
            ],
        };
        (g, idx)
    }
    #[test]
    fn requires_repetition_and_never_completes_by_similarity() {
        let (g, idx) = fixture();
        let mut s = Synchronizer::default();
        assert_eq!(
            s.observe(&g, &idx, &[0.0, 0.0, 0.0, 1.0, 1.0, 1.0], 1, 0)
                .unwrap()
                .status,
            "candidate"
        );
        assert_eq!(
            s.observe(&g, &idx, &[0.0, 0.0, 0.0, 1.0, 1.0, 1.0], 2, 0)
                .unwrap()
                .located_step
                .as_deref(),
            Some("step-1")
        );
        assert!(s.progress.completed.is_empty());
    }
    #[test]
    fn reload_stale_frames_and_guide_changes_are_handled() {
        let (mut g, idx) = fixture();
        let mut s = Synchronizer::default();
        s.confirm_complete(&g, "step-1").unwrap();
        s.observe(&g, &idx, &[0.0, 0.0, 0.0, 1.0, 1.0, 1.0], 10, 0)
            .unwrap();
        assert!(
            s.observe(&g, &idx, &[0.0, 0.0, 0.0, 1.0, 1.0, 1.0], 9, 0)
                .is_err()
        );
        s.reset();
        assert!(s.progress.completed.is_empty());
        assert!(
            s.observe(&g, &idx, &[0.0, 0.0, 0.0, 1.0, 1.0, 1.0], 11, 0)
                .is_err()
        );
        g.revision += 1;
        assert!(
            s.observe(&g, &idx, &[0.0, 0.0, 0.0, 1.0, 1.0, 1.0], 12, 1)
                .is_err()
        );
    }
    #[test]
    fn can_relocalize_backwards_and_preserves_ambiguity() {
        let (g, mut idx) = fixture();
        let mut s = Synchronizer::default();
        s.confirm_step(&g, "step-2").unwrap();
        s.observe(&g, &idx, &[0.0, 0.0, 0.0, 1.0, 1.0, 1.0], 1, 0)
            .unwrap();
        s.observe(&g, &idx, &[0.0, 0.0, 0.0, 1.0, 1.0, 1.0], 2, 0)
            .unwrap();
        assert_eq!(s.progress.current_step.as_deref(), Some("step-1"));
        idx.frames[1].fingerprint = vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0];
        assert_eq!(
            s.observe(&g, &idx, &[0.0, 0.0, 0.0, 1.0, 1.0, 1.0], 3, 0)
                .unwrap()
                .status,
            "ambiguous"
        );
    }
}
