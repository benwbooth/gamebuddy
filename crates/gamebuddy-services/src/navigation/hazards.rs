//! Immediate route warnings from existing source instructions. No inference or
//! inferred enemy positions: these describe what the selected route leads into.
use gamebuddy_core::{
    Guidance, Guide, Progress, Step, navigation::route_successors, outcomes::PlanningView,
};
use serde::Serialize;
use std::collections::HashSet;

#[derive(Clone, Debug, Serialize)]
pub struct Warning {
    pub id: String,
    pub text: String,
    pub source_step_id: String,
    pub encounter_step_id: String,
    pub trigger_step_id: String,
    pub evidence_kind: &'static str,
}

fn dangerous(step: &Step) -> bool {
    let words: Vec<_> = format!("{} {}", step.title, step.instructions)
        .to_lowercase()
        .split(|c: char| !c.is_alphabetic())
        .filter(|w| !w.is_empty())
        .map(str::to_owned)
        .collect();
    words.iter().any(|w| {
        matches!(
            w.as_str(),
            "boss"
                | "bosses"
                | "ambush"
                | "poison"
                | "lava"
                | "spikes"
                | "trap"
                | "traps"
                | "fireball"
                | "fireballs"
                | "defeat"
                | "fight"
                | "battle"
        )
    })
}

fn usable(guide: &Guide, progress: &Progress, view: &PlanningView, step: &Step) -> bool {
    guide.step_enabled(step)
        && !step.anchors.is_empty()
        && !gamebuddy_core::step_complete(guide, progress, &step.id)
        && !view.skipped.contains_key(&step.id)
}

fn immediate_section(guide: &Guide, progress: &Progress, view: &PlanningView, step: &Step) -> bool {
    let children: Vec<_> = guide
        .steps
        .iter()
        .filter(|s| s.parent_id.as_deref() == Some(&step.id))
        .collect();
    children.is_empty()
        || (children.len() <= 4
            && children
                .first()
                .is_some_and(|s| usable(guide, progress, view, s))
            && children.iter().all(|child| {
                !guide
                    .steps
                    .iter()
                    .any(|s| s.parent_id.as_deref() == Some(&child.id))
            }))
}

fn description(encounter: &Step, strategy: &Step) -> String {
    // Preserve the source wording, including protection limits and cautions.
    // A warning is a small excerpt, not a generated combat strategy.
    let mut sentences = Vec::new();
    for sentence in strategy.instructions.split_inclusive(['.', '!', '?']) {
        if sentences.len() == 2 {
            break;
        }
        if !sentence.trim().is_empty() {
            sentences.push(sentence.trim());
        }
    }
    let body = sentences.join(" ");
    format!(
        "Danger ahead: {}. {}",
        encounter.title.trim_end_matches(['.', '!', '?']),
        body
    )
}

pub fn upcoming(
    guide: &Guide,
    progress: &Progress,
    guidance: &Guidance,
    view: &PlanningView,
) -> Option<Warning> {
    let current = guide.step(&guidance.now.as_ref()?.id)?;
    let mut queue = std::collections::VecDeque::from([(current, 0usize)]);
    let mut seen = HashSet::new();
    while let Some((step, distance)) = queue.pop_front() {
        if !seen.insert(&step.id) || seen.len() > 12 || !usable(guide, progress, view, step) {
            continue;
        }
        let parent = step
            .parent_id
            .as_deref()
            .and_then(|id| guide.step(id))
            .filter(|p| {
                guide
                    .steps
                    .iter()
                    .find(|s| s.parent_id.as_deref() == Some(&p.id))
                    .is_some_and(|first| first.id == step.id)
                    && immediate_section(guide, progress, view, p)
            })
            .filter(|p| usable(guide, progress, view, p) && dangerous(p));
        if let Some(hazard) = parent.or_else(|| {
            (dangerous(step) && immediate_section(guide, progress, view, step)).then_some(step)
        }) {
            // Encounter summaries often describe the reward or destination.
            // Prefer the actual combat substep so the player hears how to win
            // before entry, including attacks, dodges and protection limits.
            let strategy = guide
                .steps
                .iter()
                .find(|child| {
                    child.parent_id.as_deref() == Some(&hazard.id)
                        && usable(guide, progress, view, child)
                        && dangerous(child)
                })
                .unwrap_or(hazard);
            return Some(Warning {
                id: format!("{}:{}:{}", guide.id, guide.revision, hazard.id),
                text: description(hazard, strategy),
                source_step_id: strategy.id.clone(),
                encounter_step_id: hazard.id.clone(),
                trigger_step_id: current.id.clone(),
                evidence_kind: "upcoming_source_hazard",
            });
        }
        // Give the player a few source actions to prepare. Never scan a
        // whole chapter or infer that two route nodes are adjacent rooms.
        if distance < 3 {
            for id in route_successors(guide, step) {
                if let Some(next) = guide.step(id) {
                    queue.push_back((next, distance + 1));
                }
            }
            if let Some(child) = guide
                .steps
                .iter()
                .find(|s| s.parent_id.as_deref() == Some(&step.id))
            {
                queue.push_front((child, distance));
            }
        }
    }
    None
}

/// Warn about a nearby departure/gate before the player crosses it. Requirements
/// are compiled from source mechanics, never inferred from scene identity.
pub fn readiness(
    guide: &Guide,
    progress: &Progress,
    guidance: &Guidance,
    model: &gamebuddy_core::outcomes::OutcomeModel,
    view: &PlanningView,
) -> Option<Warning> {
    let current = guide.step(&guidance.now.as_ref()?.id)?;
    let mut queue = std::collections::VecDeque::from([(current, 0usize)]);
    let mut seen = HashSet::new();
    while let Some((step, distance)) = queue.pop_front() {
        if !seen.insert(&step.id) || seen.len() > 16 || !guide.step_enabled(step) {
            continue;
        }
        if !gamebuddy_core::step_complete(guide, progress, &step.id)
            && !view.skipped.contains_key(&step.id)
            && let Some(required) = view.requirements.get(&step.id).filter(|r| !r.is_empty())
        {
            let source = guide.step(&required[0])?;
            let mut at = Some(step);
            while let Some(node) = at {
                if let Some(rule) = model
                    .actions
                    .iter()
                    .find(|r| r.step_id == node.id && r.requires.is_some())
                {
                    return Some(Warning {
                        id: format!("{}:{}:readiness:{}", guide.id, guide.revision, step.id),
                        text: format!(
                            "Before continuing: {}. {}",
                            required
                                .iter()
                                .filter_map(|id| guide.step(id))
                                .map(|s| s.title.trim_end_matches('.'))
                                .collect::<Vec<_>>()
                                .join("; "),
                            rule.reason
                        ),
                        source_step_id: source.id.clone(),
                        encounter_step_id: step.id.clone(),
                        trigger_step_id: current.id.clone(),
                        evidence_kind: "source_route_requirement",
                    });
                }
                at = node.parent_id.as_deref().and_then(|id| guide.step(id));
            }
        }
        if distance < 3 {
            for id in route_successors(guide, step) {
                if let Some(next) = guide.step(id) {
                    queue.push_back((next, distance + 1));
                }
            }
            if let Some(child) = guide
                .steps
                .iter()
                .find(|s| s.parent_id.as_deref() == Some(&step.id))
            {
                queue.push_front((child, distance));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use gamebuddy_core::{import_markdown, outcomes::OutcomeModel};

    #[test]
    fn warns_at_the_entrance_without_waiting_for_the_monster_to_be_visible() {
        let mut guide = import_markdown("Game", "Source", "# Enter the tower\nUse the key at the doorway.\n# Defeat the monster\nWatch for its fireballs. A shield does not block them.\n# Speak to the guardian\nTalk to the guardian.").unwrap();
        let progress = Progress::default();
        let view = OutcomeModel::default().view(&guide, &progress);
        let guidance = Guidance {
            now: Some(guide.steps[0].clone()),
            ..Default::default()
        };
        let warning = upcoming(&guide, &progress, &guidance, &view).unwrap();
        assert_eq!(warning.source_step_id, guide.steps[1].id);
        assert!(warning.text.contains("shield does not block"));
        let mut completed = progress.clone();
        completed.completed.insert(guide.steps[1].id.clone());
        assert!(upcoming(&guide, &completed, &guidance, &view).is_none());
        guide
            .reference_preferences
            .push(gamebuddy_core::ReferencePreference {
                source_id: guide.sources[0].id.clone(),
                enabled: false,
                preference: String::new(),
            });
        assert!(upcoming(&guide, &progress, &guidance, &view).is_none());
    }

    #[test]
    fn distant_or_bypassed_hazards_do_not_warn() {
        let guide = import_markdown("Game", "Source", "# Buy supplies\nBuy supplies.\n# Leave town\nLeave town.\n# Cross the bridge\nCross the bridge.\n# Climb to the tower\nClimb.\n# Poison trap\nAvoid the poison.").unwrap();
        let progress = Progress::default();
        let mut view = OutcomeModel::default().view(&guide, &progress);
        let mut guidance = Guidance {
            now: Some(guide.steps[0].clone()),
            ..Default::default()
        };
        assert!(upcoming(&guide, &progress, &guidance, &view).is_none());
        guidance.now = Some(guide.steps[3].clone());
        assert!(upcoming(&guide, &progress, &guidance, &view).is_some());
        view.skipped
            .insert(guide.steps[4].id.clone(), "Alternate route".into());
        assert!(upcoming(&guide, &progress, &guidance, &view).is_none());
    }

    #[test]
    fn a_finished_fight_does_not_warn_during_its_following_conversation() {
        let guide = import_markdown("Game", "Source", "# Enter the tower\nUse the key.\n# Defeat the guardian\nWatch for fireballs.\n## Clear the chamber\nFight the guardian.\n## Speak to the elder\nTalk to the elder.").unwrap();
        let mut progress = Progress::default();
        let mut guidance = Guidance {
            now: Some(guide.steps[2].clone()),
            ..Default::default()
        };
        let view = OutcomeModel::default().view(&guide, &progress);
        assert_eq!(
            upcoming(&guide, &progress, &guidance, &view)
                .unwrap()
                .encounter_step_id,
            guide.steps[1].id
        );
        guidance.now = Some(guide.steps[3].clone());
        assert!(upcoming(&guide, &progress, &guidance, &view).is_none());
        progress.completed.insert(guide.steps[2].id.clone());
        guidance.now = Some(guide.steps[0].clone());
        assert!(upcoming(&guide, &progress, &guidance, &view).is_none());
    }

    #[test]
    fn approach_warns_with_combat_tactics_before_the_boss_room() {
        let guide = import_markdown("Game", "Source", "# Pick up protection\nTake the potion.\n# Cross the gap\nJump across.\n# Open the tower door\nUse the key.\n# Defeat the guardian\nFight inside, then collect the reward.\n## Fight the guardian\nAttack its exposed head after it lands. Dodge fireballs; your shield does not stop magic.\n## Claim the reward\nCollect the reward.").unwrap();
        let progress = Progress::default();
        let guidance = Guidance {
            now: Some(guide.steps[0].clone()),
            ..Default::default()
        };
        let view = OutcomeModel::default().view(&guide, &progress);
        let warning = upcoming(&guide, &progress, &guidance, &view).unwrap();
        assert_eq!(warning.source_step_id, guide.steps[4].id);
        assert!(warning.text.contains("exposed head after it lands"));
        assert!(warning.text.contains("shield does not stop magic"));
        assert!(!warning.text.contains("collect the reward"));
    }
}
