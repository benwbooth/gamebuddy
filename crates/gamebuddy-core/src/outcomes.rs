//! Source-backed outcomes are separate from the actions in a demonstrated run.
//! Evaluation is local: a new fact invalidates unnecessary advice immediately.
use crate::{Guide, Progress};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Condition {
    Always,
    #[default]
    Never,
    Fact {
        id: String,
        value: bool,
    },
    All {
        conditions: Vec<Condition>,
    },
    Any {
        conditions: Vec<Condition>,
    },
}
impl Condition {
    pub fn satisfied(&self, world: &WorldState) -> bool {
        match self {
            Self::Always => true,
            Self::Never => false,
            Self::Fact { id, value } => world.facts.get(id).is_some_and(|f| f.value == *value),
            Self::All { conditions } => conditions.iter().all(|c| c.satisfied(world)),
            Self::Any { conditions } => conditions.iter().any(|c| c.satisfied(world)),
        }
    }
    fn validate(&self, facts: &HashSet<&str>, depth: usize) -> Result<()> {
        ensure!(depth < 8, "Outcome condition is too deeply nested");
        match self {
            Self::Fact { id, .. } => ensure!(facts.contains(id.as_str()), "Unknown fact {id}"),
            Self::All { conditions } | Self::Any { conditions } => {
                ensure!(
                    !conditions.is_empty() && conditions.len() <= 32,
                    "Empty or oversized condition"
                );
                for condition in conditions {
                    condition.validate(facts, depth + 1)?;
                }
            }
            _ => (),
        }
        Ok(())
    }
    /// Alternatives are sets of facts that would satisfy this condition. Unknown
    /// is not false: a negative precondition also needs actual evidence.
    fn alternatives(&self, world: &WorldState) -> Vec<Vec<FactValue>> {
        if self.satisfied(world) {
            return vec![vec![]];
        }
        match self {
            Self::Always => vec![vec![]],
            Self::Never => vec![],
            Self::Fact { id, value } => vec![vec![FactValue {
                id: id.clone(),
                value: *value,
            }]],
            Self::Any { conditions } => conditions
                .iter()
                .flat_map(|c| c.alternatives(world))
                .take(64)
                .collect(),
            Self::All { conditions } => {
                let mut options = vec![vec![]];
                for condition in conditions {
                    let choices = condition.alternatives(world);
                    options = options
                        .iter()
                        .flat_map(|a| {
                            choices.iter().map(move |b| {
                                let mut together = a.clone();
                                together.extend(b.clone());
                                together
                            })
                        })
                        .take(64)
                        .collect();
                }
                options
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FactValue {
    pub id: String,
    pub value: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct KnownFact {
    pub value: bool,
    pub evidence: String,
    pub source: String,
    pub step_id: Option<String>,
    pub at_ms: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct WorldState {
    #[serde(default)]
    pub facts: BTreeMap<String, KnownFact>,
    /// Extra rewards are opt-in; a reference's "100%" title is not a player goal.
    #[serde(default)]
    pub include_optional: bool,
    #[serde(default)]
    pub goal: Option<OutcomeGoal>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OutcomeGoal {
    pub description: String,
    pub satisfied_when: Condition,
}

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FactDefinition {
    pub id: String,
    pub description: String,
    /// Exact source names for possession reports; leave empty for other facts.
    #[serde(default)]
    pub possession_names: Vec<String>,
    pub evidence_step_ids: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActionRule {
    pub step_id: String,
    /// When supplied these world requirements replace historical step ordering.
    /// None preserves the legacy prerequisites until they have been interpreted.
    pub requires: Option<Condition>,
    #[serde(default)]
    pub effects: Vec<FactValue>,
    /// Consumption/loss with an unknown remaining quantity invalidates
    /// possession instead of inventing either zero or another remaining item.
    #[serde(default)]
    pub invalidate_facts: Vec<String>,
    #[serde(default)]
    pub unnecessary_when: Condition,
    #[serde(default)]
    pub optional: bool,
    pub reason: String,
    pub evidence_step_ids: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Bypass {
    pub when: Condition,
    /// This shortcut is valid at these entry actions (including descendants).
    /// A player already inside the branch may still need its return traversal.
    #[serde(default)]
    pub entry_step_ids: Vec<String>,
    pub skip_step_ids: Vec<String>,
    /// A source-backed way out from this branch, never an invented teleport.
    pub resume_step_id: String,
    pub reason: String,
    pub evidence_step_ids: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OutcomeModel {
    #[serde(default)]
    pub facts: Vec<FactDefinition>,
    #[serde(default)]
    pub actions: Vec<ActionRule>,
    #[serde(default)]
    pub bypasses: Vec<Bypass>,
}

pub fn within(guide: &Guide, id: &str, parent: &str) -> bool {
    let mut at = Some(id);
    while let Some(id) = at {
        if id == parent {
            return true;
        }
        at = guide.step(id).and_then(|s| s.parent_id.as_deref());
    }
    false
}

impl OutcomeModel {
    pub fn merge(&mut self, incoming: Self) {
        for fact in incoming.facts {
            self.facts.retain(|f| f.id != fact.id);
            self.facts.push(fact);
        }
        for action in incoming.actions {
            self.actions.retain(|a| a.step_id != action.step_id);
            self.actions.push(action);
        }
        for bypass in incoming.bypasses {
            self.bypasses
                .retain(|b| b.skip_step_ids != bypass.skip_step_ids);
            self.bypasses.push(bypass);
        }
    }
    pub fn validate(&self, guide: &Guide) -> Result<()> {
        ensure!(
            self.facts.len() <= 512
                && self.actions.len() <= guide.steps.len()
                && self.bypasses.len() <= 128,
            "Oversized outcome model"
        );
        let facts: HashSet<_> = self.facts.iter().map(|f| f.id.as_str()).collect();
        ensure!(facts.len() == self.facts.len(), "Duplicate outcome facts");
        let evidence = |ids: &[String]| -> Result<()> {
            ensure!(
                !ids.is_empty()
                    && ids
                        .iter()
                        .all(|id| guide.step(id).is_some_and(|s| guide.step_enabled(s))),
                "Outcome rules need enabled source evidence"
            );
            Ok(())
        };
        for fact in &self.facts {
            ensure!(
                !fact.id.is_empty()
                    && fact.id.len() <= 100
                    && fact
                        .id
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || "_:-".contains(c))
                    && !fact.description.trim().is_empty(),
                "Invalid outcome fact"
            );
            ensure!(
                fact.possession_names.len() <= 8
                    && fact
                        .possession_names
                        .iter()
                        .all(|n| !n.trim().is_empty() && n.len() <= 80),
                "Invalid possession names"
            );
            evidence(&fact.evidence_step_ids)?;
        }
        let mut steps = HashSet::new();
        for rule in &self.actions {
            ensure!(
                steps.insert(&rule.step_id) && guide.step(&rule.step_id).is_some(),
                "Duplicate or unknown outcome action"
            );
            ensure!(!rule.reason.trim().is_empty(), "Explain outcome rule");
            evidence(&rule.evidence_step_ids)?;
            if let Some(condition) = &rule.requires {
                condition.validate(&facts, 0)?;
            }
            rule.unnecessary_when.validate(&facts, 0)?;
            let mut effects = HashSet::new();
            for effect in &rule.effects {
                ensure!(
                    facts.contains(effect.id.as_str()) && effects.insert(&effect.id),
                    "Unknown or conflicting fact effect"
                );
            }
            for id in &rule.invalidate_facts {
                ensure!(
                    facts.contains(id.as_str()) && effects.insert(id),
                    "Unknown or conflicting fact invalidation"
                );
            }
        }
        for bypass in &self.bypasses {
            bypass.when.validate(&facts, 0)?;
            evidence(&bypass.evidence_step_ids)?;
            ensure!(
                !bypass.reason.trim().is_empty() && !bypass.skip_step_ids.is_empty(),
                "Explain branch bypass"
            );
            ensure!(
                guide
                    .step(&bypass.resume_step_id)
                    .is_some_and(|s| guide.step_enabled(s)),
                "Unknown bypass continuation"
            );
            ensure!(
                !bypass.entry_step_ids.is_empty()
                    && bypass
                        .entry_step_ids
                        .iter()
                        .all(|id| guide.step(id).is_some_and(|s| guide.step_enabled(s))),
                "Bypass needs source-backed entry actions"
            );
            for id in &bypass.skip_step_ids {
                ensure!(
                    guide.step(id).is_some() && !within(guide, &bypass.resume_step_id, id),
                    "Bypass skips its own continuation"
                );
            }
        }
        Ok(())
    }
    pub fn validate_goal(&self, goal: &OutcomeGoal) -> Result<()> {
        ensure!(
            !goal.description.trim().is_empty(),
            "Describe the player goal"
        );
        goal.satisfied_when
            .validate(&self.facts.iter().map(|f| f.id.as_str()).collect(), 0)
    }
    pub fn view(&self, guide: &Guide, progress: &Progress) -> PlanningView {
        let mut view = PlanningView::default();
        for id in &progress.passed {
            view.skipped.insert(
                id.clone(),
                "Passed on the way to the player's reported location; not completed.".into(),
            );
        }
        let nodes: HashMap<_, _> = guide.steps.iter().map(|s| (s.id.as_str(), s)).collect();
        let ancestry: HashMap<_, Vec<_>> = guide
            .steps
            .iter()
            .map(|step| {
                let mut ids = vec![];
                let mut at = Some(step.id.as_str());
                while let Some(id) = at {
                    ids.push(id);
                    at = nodes.get(id).and_then(|s| s.parent_id.as_deref());
                }
                (step.id.as_str(), ids)
            })
            .collect();
        let within =
            |id: &str, parent: &str| ancestry.get(id).is_some_and(|ids| ids.contains(&parent));
        let rules: HashMap<_, _> = self
            .actions
            .iter()
            .map(|r| (r.step_id.as_str(), r))
            .collect();
        let inactive: HashMap<_, _> = self
            .actions
            .iter()
            .filter(|r| r.optional || r.unnecessary_when.satisfied(&progress.world))
            .map(|r| (r.step_id.as_str(), r.reason.as_str()))
            .collect();
        let bypasses: Vec<_> = self
            .bypasses
            .iter()
            .filter(|b| {
                b.when.satisfied(&progress.world)
                    && progress.current_step.as_deref().is_some_and(|cursor| {
                        b.entry_step_ids.iter().any(|entry| within(cursor, entry))
                    })
            })
            .collect();
        let mut wanted: HashSet<String> = guide
            .goals
            .iter()
            .flat_map(|g| g.step_ids.iter().cloned())
            .collect();
        // An explicitly requested reward remains a valid objective even when
        // its acquisition is optional on the normal progress route.
        if let Some(goal) = &progress.world.goal {
            for fact in goal
                .satisfied_when
                .alternatives(&progress.world)
                .into_iter()
                .flatten()
            {
                wanted.extend(
                    self.actions
                        .iter()
                        .filter(|r| r.effects.contains(&fact))
                        .map(|r| r.step_id.clone()),
                );
            }
        }
        while !wanted.is_empty() {
            let mut additions = vec![];
            for step in &guide.steps {
                if wanted.iter().any(|goal| within(&step.id, goal)) {
                    let mut at = Some(step);
                    while let Some(node) = at {
                        additions.extend(node.prerequisites.iter().cloned());
                        at = node.parent_id.as_deref().and_then(|id| guide.step(id));
                    }
                }
            }
            let before = wanted.len();
            wanted.extend(additions);
            if wanted.len() == before {
                break;
            }
        }
        let wants = |id: &str| wanted.iter().any(|goal| within(id, goal));
        if !progress.world.include_optional {
            for step in &guide.steps {
                if wants(&step.id) {
                    continue;
                }
                if ancestry[step.id.as_str()]
                    .iter()
                    .any(|id| nodes[id].optional && !rules.contains_key(id))
                {
                    view.skipped.insert(
                        step.id.clone(),
                        "Optional for the selected progress route.".into(),
                    );
                }
                for id in &ancestry[step.id.as_str()] {
                    if let Some(reason) = inactive.get(id) {
                        view.skipped.insert(step.id.clone(), (*reason).into());
                    }
                }
                for bypass in &bypasses {
                    if bypass.skip_step_ids.iter().any(|id| within(&step.id, id)) {
                        view.skipped.insert(step.id.clone(), bypass.reason.clone());
                        view.redirects
                            .insert(step.id.clone(), bypass.resume_step_id.clone());
                    }
                }
            }
        }
        // Source conditions can be inherited by substeps. The closest rule wins.
        for step in &guide.steps {
            let mut at = Some(step.id.as_str());
            while let Some(id) = at {
                if let Some(condition) = rules.get(id).and_then(|r| r.requires.as_ref()) {
                    match self.producers(guide, progress, &view, condition) {
                        Some(ids) => {
                            view.requirements.insert(step.id.clone(), ids);
                        }
                        None => {
                            view.blocked.insert(step.id.clone(), "A required world fact is unknown or unsatisfied; no supported acquisition route is available.".into());
                        }
                    }
                    break;
                }
                at = nodes.get(id).and_then(|s| s.parent_id.as_deref());
            }
        }
        if let Some(goal) = &progress.world.goal {
            view.goal_complete = goal.satisfied_when.satisfied(&progress.world);
            if !view.goal_complete {
                if let Some(ids) = self.producers(guide, progress, &view, &goal.satisfied_when) {
                    view.goal_steps = ids;
                } else {
                    view.goal_blocked = true;
                }
            }
        }
        view
    }
    fn producers(
        &self,
        guide: &Guide,
        progress: &Progress,
        view: &PlanningView,
        condition: &Condition,
    ) -> Option<Vec<String>> {
        condition
            .alternatives(&progress.world)
            .into_iter()
            .filter_map(|facts| {
                let mut ids = vec![];
                for fact in facts {
                    let rule = self.actions.iter().find(|r| {
                        r.effects.contains(&fact)
                            && !view.skipped.contains_key(&r.step_id)
                            && guide
                                .step(&r.step_id)
                                .is_some_and(|s| guide.step_enabled(s))
                            && !crate::step_complete(guide, progress, &r.step_id)
                    })?;
                    if !ids.contains(&rule.step_id) {
                        ids.push(rule.step_id.clone());
                    }
                }
                Some(ids)
            })
            .min_by_key(Vec::len)
    }
}

#[derive(Default, Debug)]
pub struct PlanningView {
    pub skipped: HashMap<String, String>,
    pub redirects: HashMap<String, String>,
    pub requirements: HashMap<String, Vec<String>>,
    pub blocked: HashMap<String, String>,
    pub goal_steps: Vec<String>,
    pub goal_complete: bool,
    pub goal_blocked: bool,
}
impl PlanningView {
    /// A known acquisition route is still an unmet requirement. Location
    /// evidence may bypass historical checklist order, never world state.
    pub fn requirements_met(&self, id: &str) -> bool {
        !self.blocked.contains_key(id) && self.requirements.get(id).is_none_or(Vec::is_empty)
    }
    pub fn resolved(&self, guide: &Guide, progress: &Progress, id: &str) -> bool {
        if self.skipped.contains_key(id) || crate::step_complete(guide, progress, id) {
            return true;
        }
        let children: Vec<_> = guide
            .steps
            .iter()
            .filter(|s| s.parent_id.as_deref() == Some(id))
            .collect();
        !children.is_empty()
            && children
                .iter()
                .all(|s| self.resolved(guide, progress, &s.id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PlaceTracker, guidance_with_outcomes, import_markdown};
    fn has(value: bool) -> Condition {
        Condition::Fact {
            id: "has_key".into(),
            value,
        }
    }
    fn fixture() -> (Guide, Progress, OutcomeModel) {
        let mut guide = import_markdown("Adventure", "A demonstrated route", "# Find key\nPick up the key from the chest.\n# Climb tower\nClimb toward the guardian for its key.\n# Defeat guardian\nDefeat the guardian for a key, or skip if you have one.\n# Leave tower\nUse the entrance door.\n# Open gate\nUse up the key to open the gate.").unwrap();
        guide.steps[2].prerequisites = vec!["step-2".into()];
        guide.steps[4].prerequisites = vec!["step-3".into()];
        let model = OutcomeModel {
            facts: vec![FactDefinition {
                id: "has_key".into(),
                description: "Currently has a key".into(),
                possession_names: vec!["key".into()],
                evidence_step_ids: vec!["step-1".into()],
            }],
            actions: vec![
                ActionRule {
                    step_id: "step-1".into(),
                    effects: vec![FactValue {
                        id: "has_key".into(),
                        value: true,
                    }],
                    reason: "Chest supplies the key.".into(),
                    evidence_step_ids: vec!["step-1".into()],
                    ..Default::default()
                },
                ActionRule {
                    step_id: "step-3".into(),
                    effects: vec![FactValue {
                        id: "has_key".into(),
                        value: true,
                    }],
                    unnecessary_when: has(true),
                    reason: "The guardian supplies a key you already have.".into(),
                    evidence_step_ids: vec!["step-3".into()],
                    ..Default::default()
                },
                ActionRule {
                    step_id: "step-5".into(),
                    requires: Some(has(true)),
                    effects: vec![FactValue {
                        id: "has_key".into(),
                        value: false,
                    }],
                    reason: "The gate needs the key, regardless of how it was obtained.".into(),
                    evidence_step_ids: vec!["step-5".into()],
                    ..Default::default()
                },
            ],
            bypasses: vec![Bypass {
                when: has(true),
                entry_step_ids: vec!["step-1".into(), "step-2".into()],
                skip_step_ids: vec!["step-2".into(), "step-3".into()],
                resume_step_id: "step-4".into(),
                reason: "The key objective is satisfied; the guardian detour is unnecessary."
                    .into(),
                evidence_step_ids: vec!["step-3".into(), "step-4".into()],
            }],
        };
        model.validate(&guide).unwrap();
        let mut progress = Progress {
            current_step: Some("step-1".into()),
            ..Default::default()
        };
        progress.reported_completed.insert("step-1".into());
        (guide, progress, model)
    }
    fn fact(progress: &mut Progress, value: bool) {
        progress.world.facts.insert(
            "has_key".into(),
            KnownFact {
                value,
                evidence: "I have the key".into(),
                source: "player_report".into(),
                step_id: None,
                at_ms: 1,
            },
        );
    }
    #[test]
    fn alternative_acquisition_retires_detour_without_completing_it() {
        let (guide, mut progress, model) = fixture();
        fact(&mut progress, true);
        let saved = serde_json::to_value(&progress).unwrap();
        let advice = guidance_with_outcomes(
            &guide,
            &progress,
            &PlaceTracker::default(),
            Some("step-2"),
            &model,
        );
        assert_eq!(advice.now.unwrap().id, "step-4");
        assert_eq!(advice.next[0].id, "step-5");
        assert!(!crate::step_complete(&guide, &progress, "step-3"));
        assert_eq!(saved, serde_json::to_value(progress).unwrap());
    }
    #[test]
    fn possession_is_an_outcome_goal_independent_of_the_acquisition_method() {
        let (guide, mut progress, model) = fixture();
        fact(&mut progress, true);
        progress.world.goal = Some(OutcomeGoal {
            description: "Get a key".into(),
            satisfied_when: has(true),
        });
        assert_eq!(
            guidance_with_outcomes(
                &guide,
                &progress,
                &PlaceTracker::default(),
                Some("step-3"),
                &model
            )
            .status,
            "goals_complete"
        );
        assert!(!crate::step_complete(&guide, &progress, "step-3"));
    }
    #[test]
    fn explicit_completion_choice_keeps_the_optional_fight_available() {
        let (guide, mut progress, model) = fixture();
        fact(&mut progress, true);
        progress.world.include_optional = true;
        assert_eq!(
            guidance_with_outcomes(&guide, &progress, &PlaceTracker::default(), None, &model)
                .now
                .unwrap()
                .id,
            "step-2"
        );
    }
    #[test]
    fn consuming_or_losing_an_item_reopens_a_supported_acquisition_route() {
        let (guide, mut progress, model) = fixture();
        fact(&mut progress, false);
        let advice = guidance_with_outcomes(
            &guide,
            &progress,
            &PlaceTracker::default(),
            Some("step-5"),
            &model,
        );
        assert_eq!(advice.now.unwrap().id, "step-2");
        assert!(advice.requirements.iter().any(|s| s.id == "step-3"));
    }
    #[test]
    fn missing_evidence_does_not_satisfy_a_negative_condition_and_restore_clears_facts() {
        let (_, mut progress, _) = fixture();
        assert!(!has(false).satisfied(&progress.world));
        fact(&mut progress, true);
        let mut sync = crate::Synchronizer::new(progress);
        sync.reset();
        assert!(!has(true).satisfied(&sync.progress.world));
        assert!(!has(false).satisfied(&sync.progress.world));
    }
    #[test]
    fn any_requirement_uses_one_alternative_and_unknown_producer_blocks() {
        let (guide, mut progress, mut model) = fixture();
        model.facts.push(FactDefinition {
            id: "knows_spell".into(),
            description: "Knows opening spell".into(),
            possession_names: vec![],
            evidence_step_ids: vec!["step-5".into()],
        });
        model.actions[2].requires = Some(Condition::Any {
            conditions: vec![
                has(true),
                Condition::Fact {
                    id: "knows_spell".into(),
                    value: true,
                },
            ],
        });
        progress.world.facts.insert(
            "knows_spell".into(),
            KnownFact {
                value: true,
                evidence: "I learned the spell".into(),
                source: "player_report".into(),
                step_id: None,
                at_ms: 2,
            },
        );
        let advice = guidance_with_outcomes(
            &guide,
            &progress,
            &PlaceTracker::default(),
            Some("step-5"),
            &model,
        );
        assert_eq!(advice.now.unwrap().id, "step-5");
        assert!(advice.requirements.is_empty());
        progress.world.facts.clear();
        model.actions[2].requires = Some(Condition::Fact {
            id: "knows_spell".into(),
            value: true,
        });
        assert_eq!(
            guidance_with_outcomes(
                &guide,
                &progress,
                &PlaceTracker::default(),
                Some("step-5"),
                &model
            )
            .status,
            "blocked"
        );
    }
    #[test]
    fn explicit_step_goal_overrides_optional_branch_suppression() {
        let (mut guide, mut progress, model) = fixture();
        fact(&mut progress, true);
        guide.goals.push(crate::PlayerGoal {
            id: "guardian".into(),
            description: "Defeat the guardian for fun".into(),
            step_ids: vec!["step-3".into()],
        });
        let advice =
            guidance_with_outcomes(&guide, &progress, &PlaceTracker::default(), None, &model);
        // The fight is requested, but its actual approach remains necessary too.
        assert_eq!(advice.now.unwrap().id, "step-2");
    }
    #[test]
    fn explicit_outcome_goal_can_choose_an_optional_reward() {
        let (guide, mut progress, mut model) = fixture();
        progress.reported_completed.clear();
        model.actions[0].optional = true;
        progress.world.goal = Some(OutcomeGoal {
            description: "Find the optional key".into(),
            satisfied_when: has(true),
        });
        assert_eq!(
            guidance_with_outcomes(&guide, &progress, &PlaceTracker::default(), None, &model)
                .now
                .unwrap()
                .id,
            "step-1"
        );
    }
    #[test]
    fn entrance_shortcut_does_not_skip_the_return_from_inside_a_branch() {
        let (guide, mut progress, mut model) = fixture();
        fact(&mut progress, true);
        model.bypasses[0].skip_step_ids.push("step-4".into());
        model.bypasses[0].resume_step_id = "step-5".into();
        model.validate(&guide).unwrap();
        assert_eq!(
            guidance_with_outcomes(&guide, &progress, &PlaceTracker::default(), None, &model)
                .now
                .unwrap()
                .id,
            "step-5"
        );
        progress.current_step = Some("step-3".into());
        assert_eq!(
            guidance_with_outcomes(&guide, &progress, &PlaceTracker::default(), None, &model)
                .now
                .unwrap()
                .id,
            "step-4"
        );
    }
    #[test]
    fn malformed_rules_cannot_create_unknown_facts_or_self_skipping_routes() {
        let (guide, _, mut model) = fixture();
        model.bypasses[0].resume_step_id = "step-2".into();
        assert!(model.validate(&guide).is_err());
        model.bypasses.clear();
        model.actions[0].effects[0].id = "invented".into();
        assert!(model.validate(&guide).is_err());
    }
}
