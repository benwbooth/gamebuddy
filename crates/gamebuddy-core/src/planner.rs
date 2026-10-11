//! A dependency plan over the walkthrough, so advice comes from where the
//! player is and what they have, not from replaying a recorded run.
//!
//! - Objectives are the walkthrough's tasks. Each has an area, the world
//!   facts it needs (a key, an item), the facts it produces or uses up, and
//!   the earlier main-line objective it follows.
//! - The area graph links areas the walkthrough travels between; a crossing
//!   carries the requirements of the steps at that boundary (a locked door).
//! - The planner takes the player's area and world state, picks the next
//!   main-line objective that is not done, routes there through the area
//!   graph, and puts objectives that obtain anything missing first.
//!
//! A death or detour only changes the player's area, so the plan simply
//! starts from the checkpoint; used-up keys show up as missing requirements.
use crate::{
    Atlas, Guide, Progress,
    outcomes::{Condition, OutcomeModel, WorldState},
};
use serde::Serialize;
use std::collections::{BTreeSet, HashMap, VecDeque};

#[derive(Clone, Debug, Serialize)]
pub struct Objective {
    pub id: String,
    pub title: String,
    pub summary: String,
    pub chapter: String,
    pub area: Option<String>,
    pub optional: bool,
    /// World facts its steps need before they can be done.
    pub requires: Vec<String>,
    pub produces: Vec<String>,
    pub consumes: Vec<String>,
    /// The main-line objective before it (walkthrough order).
    pub after: Option<String>,
    /// Leaf steps, in order.
    pub steps: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Crossing {
    pub from: String,
    pub to: String,
    /// Facts needed to make this crossing (a key for a locked door).
    pub requires: Vec<String>,
    /// The walkthrough step that makes the crossing.
    pub step: String,
    /// The last step before it, in the area being left (the way out).
    pub exit_step: String,
}

/// One crossing on a route: from, to, what it needs, the step leading out.
type Leg = (String, String, Vec<String>, String);

#[derive(Clone, Debug, Default, Serialize)]
pub struct PlanGraph {
    pub objectives: Vec<Objective>,
    pub crossings: Vec<Crossing>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PlanItem {
    /// Obtain something a later item needs.
    Get {
        need: String,
        objective: String,
        title: String,
        area: Option<String>,
        first_step: Option<String>,
    },
    Travel {
        from: String,
        to: String,
        requires: Vec<String>,
        /// A walkthrough step that leads out of `from` toward `to`.
        via_step: Option<String>,
    },
    Do {
        objective: String,
        title: String,
        area: Option<String>,
        first_step: Option<String>,
    },
}

#[derive(Clone, Debug, Serialize)]
pub struct Plan {
    pub area: Option<String>,
    pub objective: Option<String>,
    pub items: Vec<PlanItem>,
    /// Needed facts with no known way to get them in the walkthrough.
    pub unresolved: Vec<String>,
    /// The main-line objectives after this one, for the overall direction.
    pub upcoming: Vec<String>,
}

fn facts(condition: &Condition, out: &mut BTreeSet<String>) {
    match condition {
        Condition::Fact { id, value: true } => {
            out.insert(id.clone());
        }
        Condition::All { conditions } => conditions.iter().for_each(|c| facts(c, out)),
        // One alternative is enough; plan for the first.
        Condition::Any { conditions } => {
            if let Some(first) = conditions.first() {
                facts(first, out)
            }
        }
        _ => {}
    }
}

/// The area a place belongs to: its area map, or the area name its label
/// starts with ("Mascon tool shop" is in Mascon).
pub fn place_area(atlas: &Atlas, place_id: &str) -> Option<String> {
    let place = atlas.places.iter().find(|p| p.id == place_id)?;
    if let Some(map) = place
        .map_id
        .as_deref()
        .and_then(|m| atlas.maps.iter().find(|x| x.id == m))
        .filter(|m| !m.title.contains(" To "))
    {
        return Some(map.title.clone());
    }
    let label = place.label.to_lowercase();
    atlas
        .maps
        .iter()
        .map(|m| m.title.as_str())
        .filter(|t| !t.contains(" To "))
        .filter(|t| {
            label.starts_with(&t.to_lowercase())
                || label.contains(&format!("{} ", t.to_lowercase()))
        })
        .max_by_key(|t| t.len())
        .map(str::to_owned)
        .or_else(|| {
            // "Misty dwarf platform chamber" names the misty area.
            place.label.split_whitespace().next().map(|w| {
                let w = w.trim_end_matches(['-', ',']);
                if w.eq_ignore_ascii_case("misty") {
                    "Misty area".to_owned()
                } else {
                    w.to_owned()
                }
            })
        })
        .filter(|a| a.chars().next().is_some_and(char::is_uppercase))
}

impl PlanGraph {
    pub fn build(guide: &Guide, atlas: &Atlas, model: &OutcomeModel) -> Self {
        let parents: BTreeSet<&str> = guide
            .steps
            .iter()
            .filter_map(|s| s.parent_id.as_deref())
            .collect();
        let leaves: Vec<&crate::Step> = guide
            .steps
            .iter()
            .filter(|s| !parents.contains(s.id.as_str()) && guide.step_enabled(s))
            .collect();
        // Every leaf gets an area: its bound place, else the last known one.
        let mut areas: HashMap<&str, String> = HashMap::new();
        let mut last: Option<String> = None;
        for leaf in &leaves {
            let area = atlas
                .bindings
                .iter()
                .find(|b| b.step_id == leaf.id)
                .and_then(|b| place_area(atlas, &b.place_id));
            if area.is_some() {
                last = area;
            }
            if let Some(area) = &last {
                areas.insert(leaf.id.as_str(), area.clone());
            }
        }
        let rule_for = |id: &str| {
            // The closest rule on the step or an ancestor applies.
            let mut at = Some(id);
            while let Some(step) = at {
                if let Some(rule) = model.actions.iter().find(|r| r.step_id == step) {
                    return Some(rule);
                }
                at = guide.step(step).and_then(|s| s.parent_id.as_deref());
            }
            None
        };
        let task_of = |leaf: &crate::Step| -> String {
            let mut chain = vec![leaf.id.as_str()];
            let mut at = leaf.parent_id.as_deref();
            while let Some(parent) = at {
                chain.push(parent);
                at = guide.step(parent).and_then(|s| s.parent_id.as_deref());
            }
            chain.reverse();
            // Chapter, timed part, task.
            chain
                .get(2)
                .or(chain.last())
                .map(|s| s.to_string())
                .unwrap_or_default()
        };
        let mut objectives: Vec<Objective> = vec![];
        let mut index: HashMap<String, usize> = HashMap::new();
        for leaf in &leaves {
            let task = task_of(leaf);
            let at = *index.entry(task.clone()).or_insert_with(|| {
                let step = guide.step(&task);
                let mut root = step;
                while let Some(parent) = root
                    .and_then(|s| s.parent_id.as_deref())
                    .and_then(|p| guide.step(p))
                {
                    root = Some(parent);
                }
                let mut optional =
                    step.is_some_and(|s| s.optional || s.title.starts_with("Optionally"));
                let mut ancestor = step.and_then(|s| s.parent_id.as_deref());
                while let Some(id) = ancestor {
                    optional |= guide.step(id).is_some_and(|s| s.optional);
                    ancestor = guide.step(id).and_then(|s| s.parent_id.as_deref());
                }
                objectives.push(Objective {
                    id: task.clone(),
                    title: step.map(|s| s.title.clone()).unwrap_or_default(),
                    summary: step
                        .map(|s| s.instructions.chars().take(300).collect())
                        .unwrap_or_default(),
                    chapter: root.map(|s| s.title.clone()).unwrap_or_default(),
                    area: None,
                    optional,
                    requires: vec![],
                    produces: vec![],
                    consumes: vec![],
                    after: None,
                    steps: vec![],
                });
                objectives.len() - 1
            });
            let objective = &mut objectives[at];
            objective.steps.push(leaf.id.clone());
            if let Some(rule) = rule_for(&leaf.id) {
                let mut needed = BTreeSet::new();
                if let Some(requires) = &rule.requires {
                    facts(requires, &mut needed);
                }
                for fact in needed {
                    // Needed before this objective produces it itself.
                    if !objective.produces.contains(&fact) && !objective.requires.contains(&fact) {
                        objective.requires.push(fact);
                    }
                }
                for effect in rule.effects.iter().filter(|e| e.value) {
                    if !objective.produces.contains(&effect.id) {
                        objective.produces.push(effect.id.clone());
                    }
                }
                for fact in &rule.invalidate_facts {
                    if !objective.consumes.contains(fact) {
                        objective.consumes.push(fact.clone());
                    }
                }
            }
        }
        for objective in &mut objectives {
            // The area where most of its steps happen.
            let mut counts: HashMap<&String, usize> = HashMap::new();
            for step in &objective.steps {
                if let Some(area) = areas.get(step.as_str()) {
                    *counts.entry(area).or_default() += 1;
                }
            }
            objective.area = counts
                .into_iter()
                .max_by_key(|(_, n)| *n)
                .map(|(a, _)| a.clone());
        }
        let mut previous: Option<String> = None;
        for objective in &mut objectives {
            objective.after = previous.clone();
            if !objective.optional {
                previous = Some(objective.id.clone());
            }
        }
        // Crossings: where consecutive steps change area. The steps at the
        // boundary carry the crossing's requirements (the locked door).
        let mut crossings: Vec<Crossing> = vec![];
        // Facts some action uses up (a key spent on a door) have to be met
        // again on every pass, so a locked door anywhere on the way through
        // an area belongs to the crossing out of it.
        let consumable: BTreeSet<&str> = model
            .actions
            .iter()
            .flat_map(|r| r.invalidate_facts.iter().map(String::as_str))
            .collect();
        for (at, pair) in leaves.windows(2).enumerate() {
            let (Some(from), Some(to)) = (
                areas.get(pair[0].id.as_str()),
                areas.get(pair[1].id.as_str()),
            ) else {
                continue;
            };
            if from == to {
                continue;
            }
            let mut needed = BTreeSet::new();
            for leaf in pair {
                if let Some(requires) = rule_for(&leaf.id).and_then(|r| r.requires.as_ref()) {
                    facts(requires, &mut needed);
                }
            }
            for leaf in leaves[..=at]
                .iter()
                .rev()
                .take_while(|l| areas.get(l.id.as_str()) == Some(from))
            {
                if let Some(requires) = rule_for(&leaf.id).and_then(|r| r.requires.as_ref()) {
                    let mut run = BTreeSet::new();
                    facts(requires, &mut run);
                    needed.extend(run.into_iter().filter(|f| consumable.contains(f.as_str())));
                }
            }
            // A door that uses up its key needs one on every pass; facts
            // that stay true (a raised ladder) are met once done.
            let needed: Vec<String> = needed.into_iter().collect();
            if let Some(existing) = crossings
                .iter_mut()
                .find(|c| c.from == *from && c.to == *to)
            {
                for fact in needed {
                    if !existing.requires.contains(&fact) {
                        existing.requires.push(fact);
                    }
                }
            } else {
                crossings.push(Crossing {
                    from: from.clone(),
                    to: to.clone(),
                    requires: needed,
                    step: pair[1].id.clone(),
                    exit_step: pair[0].id.clone(),
                });
            }
        }
        Self {
            objectives,
            crossings,
        }
    }

    fn done(&self, guide: &Guide, progress: &Progress, objective: &Objective) -> bool {
        objective
            .steps
            .iter()
            .all(|id| crate::step_complete(guide, progress, id) || progress.passed.contains(id))
    }

    /// Areas from `from` to `to`, walking crossings in either direction (a
    /// player can walk back the way they came).
    fn route(&self, from: &str, to: &str) -> Option<Vec<Leg>> {
        if from == to {
            return Some(vec![]);
        }
        let mut seen: HashMap<String, (String, Vec<String>, String)> = HashMap::new();
        let mut queue = VecDeque::from([from.to_owned()]);
        seen.insert(from.to_owned(), (String::new(), vec![], String::new()));
        while let Some(area) = queue.pop_front() {
            for crossing in &self.crossings {
                // Walking a crossing backwards leaves through its far step.
                let (next, exit) = if crossing.from == area {
                    (&crossing.to, &crossing.exit_step)
                } else if crossing.to == area {
                    (&crossing.from, &crossing.step)
                } else {
                    continue;
                };
                if seen.contains_key(next) {
                    continue;
                }
                seen.insert(
                    next.clone(),
                    (area.clone(), crossing.requires.clone(), exit.clone()),
                );
                if next == to {
                    let mut path = vec![];
                    let mut at = to.to_owned();
                    while at != from {
                        let (prev, requires, exit) = seen[&at].clone();
                        path.push((prev.clone(), at.clone(), requires, exit));
                        at = prev;
                    }
                    path.reverse();
                    return Some(path);
                }
                queue.push_back(next.clone());
            }
        }
        None
    }

    /// The plan from the player's area and world state.
    pub fn plan(&self, guide: &Guide, progress: &Progress, area: Option<&str>) -> Plan {
        let world: &WorldState = &progress.world;
        let have = |fact: &str| world.facts.get(fact).is_some_and(|f| f.value);
        // Progress only moves forward through the main line: the furthest
        // objective with any finished step marks where the run is. Anything
        // skipped before it that still matters shows up as a missing fact.
        let touched = |o: &Objective| {
            o.steps
                .iter()
                .any(|id| crate::step_complete(guide, progress, id) || progress.passed.contains(id))
        };
        // The route cursor is the established position; stray completions
        // far ahead (or recovery steps) do not move it.
        let mark = progress
            .current_step
            .as_deref()
            .and_then(|c| {
                self.objectives
                    .iter()
                    .position(|o| o.steps.iter().any(|s| s == c) || o.id == c)
            })
            .or_else(|| self.objectives.iter().rposition(touched))
            .unwrap_or(0);
        let target = self
            .objectives
            .iter()
            .skip(mark)
            .find(|o| !o.optional && !self.done(guide, progress, o));
        let mut items = vec![];
        let mut unresolved = vec![];
        let upcoming = target
            .map(|t| {
                self.objectives
                    .iter()
                    .skip_while(|o| o.id != t.id)
                    .skip(1)
                    .filter(|o| !o.optional)
                    .take(5)
                    .map(|o| o.title.clone())
                    .collect()
            })
            .unwrap_or_default();
        let Some(target) = target else {
            return Plan {
                area: area.map(str::to_owned),
                objective: None,
                items,
                unresolved,
                upcoming,
            };
        };
        let path = match (area, target.area.as_deref()) {
            (Some(from), Some(to)) => self.route(from, to).unwrap_or_default(),
            _ => vec![],
        };
        // Everything needed on the way and for the objective itself.
        let mut needed: Vec<String> = vec![];
        for (_, _, requires, _) in &path {
            needed.extend(requires.iter().cloned());
        }
        needed.extend(target.requires.iter().cloned());
        let mut seen = BTreeSet::new();
        needed.retain(|f| !have(f) && seen.insert(f.clone()));
        let order = |id: &str| {
            self.objectives
                .iter()
                .position(|o| o.id == id)
                .unwrap_or(usize::MAX)
        };
        let target_at = order(&target.id);
        for need in &needed {
            // A source that gets it: the latest one before the target (the
            // shop the player used last time), else any.
            let source = self
                .objectives
                .iter()
                .filter(|o| o.produces.contains(need) && o.id != target.id)
                .filter(|o| order(&o.id) < target_at)
                .max_by_key(|o| {
                    // Prefer one in the player's area.
                    (o.area.as_deref() == area, order(&o.id))
                })
                .or_else(|| self.objectives.iter().find(|o| o.produces.contains(need)));
            match source {
                Some(source) => items.push(PlanItem::Get {
                    need: need.clone(),
                    objective: source.id.clone(),
                    title: source.title.clone(),
                    area: source.area.clone(),
                    first_step: source.steps.first().cloned(),
                }),
                None => unresolved.push(need.clone()),
            }
        }
        for (from, to, requires, exit) in path {
            items.push(PlanItem::Travel {
                from,
                to,
                requires,
                via_step: Some(exit),
            });
        }
        items.push(PlanItem::Do {
            objective: target.id.clone(),
            title: target.title.clone(),
            area: target.area.clone(),
            first_step: target
                .steps
                .iter()
                .find(|id| {
                    !crate::step_complete(guide, progress, id) && !progress.passed.contains(*id)
                })
                .cloned(),
        });
        Plan {
            area: area.map(str::to_owned),
            objective: Some(target.id.clone()),
            items,
            unresolved,
            upcoming,
        }
    }

    pub fn area_of_step(&self, step: &str) -> Option<&str> {
        self.objectives
            .iter()
            .find(|o| o.steps.iter().any(|s| s == step) || o.id == step)
            .and_then(|o| o.area.as_deref())
    }
}
