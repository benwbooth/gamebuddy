//! Follow authored route edges independently of prerequisites and room matches.
use crate::{Guide, Progress, is_player_action, outcomes::PlanningView, route_successors};
use serde::Serialize;
use std::collections::HashSet;

#[derive(Debug, Default, Serialize)]
pub struct AuthoredRoute {
    pub step_ids: Vec<String>,
    pub stop: String,
    pub choices: Vec<String>,
}

/// Return the unfinished actions on one source route. A branch, disabled
/// reference or cycle stops the preview instead of selecting an arbitrary path.
/// Skipped actions retain their continuation and never become completed facts.
pub fn authored_route(
    guide: &Guide,
    progress: &Progress,
    plan: &PlanningView,
    start: &str,
    limit: usize,
) -> AuthoredRoute {
    fn entry<'a>(guide: &'a Guide, mut id: &'a str) -> &'a str {
        while let Some(child) = guide
            .steps
            .iter()
            .find(|s| s.parent_id.as_deref() == Some(id))
        {
            id = &child.id;
        }
        id
    }
    let mut route = AuthoredRoute::default();
    let mut at = start;
    let mut visited = HashSet::new();
    loop {
        if !visited.insert(at) {
            route.stop = "cycle".into();
            break;
        }
        let Some(step) = guide.step(at) else {
            route.stop = "missing_step".into();
            break;
        };
        if !guide.step_enabled(step) {
            route.stop = "disabled_source".into();
            break;
        }
        if plan.skipped.contains_key(at) {
            if let Some(next) = plan.redirects.get(at) {
                at = next;
                continue;
            }
        } else if !crate::step_complete(guide, progress, at) {
            if let Some(child) = guide
                .steps
                .iter()
                .find(|s| s.parent_id.as_deref() == Some(at))
            {
                at = &child.id;
                continue;
            }
            if is_player_action(step) {
                route.step_ids.push(at.to_owned());
                if route.step_ids.len() >= limit {
                    route.stop = "preview_limit".into();
                    break;
                }
            }
        }
        let mut next: Vec<&str> = vec![];
        for id in route_successors(guide, step) {
            // A leaf and its enclosing chapter may name the same continuation
            // using different container IDs.
            if !next
                .iter()
                .any(|other| entry(guide, other) == entry(guide, id))
            {
                next.push(id);
            }
        }
        match next.as_slice() {
            [] => {
                route.stop = "end".into();
                break;
            }
            [id] => at = id,
            _ => {
                route.stop = "branch".into();
                route.choices = next.iter().map(|id| entry(guide, id).to_owned()).collect();
                break;
            }
        }
    }
    route
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ReferencePreference, import_markdown};

    #[test]
    fn source_order_keeps_unmodelled_pickups_and_return_visits() {
        let guide = import_markdown("Game", "Walkthrough", "# First visit\n## Reward\nSpeak again for the seal.\n## Leave\nExit to the forest.\n# Return visit\n## Door\nUnlock the same room again.").unwrap();
        let progress = Progress::default();
        let route = authored_route(&guide, &progress, &PlanningView::default(), "step-1", 20);
        assert_eq!(route.step_ids, ["step-2", "step-3", "step-5"]);
        assert_eq!(route.stop, "end");
        assert!(progress.completed.is_empty());
    }

    #[test]
    fn completed_containers_optional_bypasses_and_disabled_sources_remain_distinct() {
        let mut guide = import_markdown("Game", "Walkthrough", "# Start\n## Fight\nWin.\n# Detour\nClimb for an optional item.\n# Continue\nTake the exit.").unwrap();
        guide.steps[0].next = vec!["step-3".into()];
        let mut progress = Progress::default();
        progress.reported_completed.insert("step-1".into());
        let mut plan = PlanningView::default();
        plan.skipped
            .insert("step-3".into(), "Item already owned".into());
        plan.redirects.insert("step-3".into(), "step-4".into());
        assert_eq!(
            authored_route(&guide, &progress, &plan, "step-1", 20).step_ids,
            ["step-4"]
        );
        guide.reference_preferences.push(ReferencePreference {
            source_id: "text-1".into(),
            enabled: false,
            preference: String::new(),
        });
        let route = authored_route(&guide, &progress, &plan, "step-1", 20);
        assert!(route.step_ids.is_empty());
        assert_eq!(route.stop, "disabled_source");
    }

    #[test]
    fn unresolved_branch_and_route_cycle_do_not_invent_a_path() {
        let mut guide = import_markdown(
            "Game",
            "Walkthrough",
            "# Start\nWalk.\n# Left\nGo left.\n# Right\nGo right.",
        )
        .unwrap();
        guide.steps[0].next = vec!["step-2".into(), "step-3".into()];
        let route = authored_route(
            &guide,
            &Progress::default(),
            &PlanningView::default(),
            "step-1",
            20,
        );
        assert_eq!(route.step_ids, ["step-1"]);
        assert_eq!(route.stop, "branch");
        assert_eq!(route.choices, ["step-2", "step-3"]);
        guide.steps[0].next = vec!["step-2".into()];
        guide.steps[1].next = vec!["step-1".into()];
        assert_eq!(
            authored_route(
                &guide,
                &Progress::default(),
                &PlanningView::default(),
                "step-1",
                20
            )
            .stop,
            "cycle"
        );
    }
}
