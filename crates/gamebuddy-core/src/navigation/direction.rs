//! Screen-relative movement from the current action, never a compass bearing
//! inferred from atlas artwork or an unverified straight-line path.
use super::{Atlas, Guidance, PlaceTracker};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ScreenDirection {
    Left,
    Right,
    Up,
    Down,
    UpLeft,
    UpRight,
    DownLeft,
    DownRight,
}

#[derive(Debug, Serialize)]
pub struct DirectionCue {
    pub direction: ScreenDirection,
    pub step_id: String,
    pub label: String,
    pub instruction: String,
    pub valid_until_ms: u64,
}

/// Deliberately accept only a leading, explicit movement instruction. Words
/// such as "right key", conditional routes and later steps are not directions.
pub fn movement(instructions: &str) -> Option<ScreenDirection> {
    let first = instructions.split(['.', '!', '?', '\n', ';']).next()?;
    let lower = first.to_lowercase();
    let words: Vec<_> = lower
        .split(|c: char| !c.is_alphabetic() && c != '\'')
        .filter(|w| !w.is_empty())
        .collect();
    if words.iter().any(|w| {
        [
            "if", "unless", "not", "never", "don't", "avoid", "before", "after", "then", "instead",
        ]
        .contains(w)
    }) {
        return None;
    }
    let words = words.strip_prefix(&["now"]).unwrap_or(&words);
    let (verb, rest) = words.split_first()?;
    if ![
        "go", "walk", "move", "head", "run", "climb", "jump", "swim", "fly", "travel", "exit",
        "continue", "proceed",
    ]
    .contains(verb)
    {
        return None;
    }
    let rest = rest.strip_prefix(&["to", "the"]).unwrap_or(rest);
    let rest = rest.strip_prefix(&["towards", "the"]).unwrap_or(rest);
    let rest = rest.strip_prefix(&["toward", "the"]).unwrap_or(rest);
    use ScreenDirection::*;
    let (direction, used) = match rest {
        ["up", "and", "left", ..] | ["up", "left", ..] => {
            (UpLeft, if rest[1] == "and" { 3 } else { 2 })
        }
        ["up", "and", "right", ..] | ["up", "right", ..] => {
            (UpRight, if rest[1] == "and" { 3 } else { 2 })
        }
        ["down", "and", "left", ..] | ["down", "left", ..] => {
            (DownLeft, if rest[1] == "and" { 3 } else { 2 })
        }
        ["down", "and", "right", ..] | ["down", "right", ..] => {
            (DownRight, if rest[1] == "and" { 3 } else { 2 })
        }
        ["left", ..] => (Left, 1),
        ["right", ..] => (Right, 1),
        ["up", ..] => (Up, 1),
        ["down", ..] => (Down, 1),
        _ => return None,
    };
    if used == 1
        && ((matches!(direction, Right)
            && rest
                .get(1)
                .is_some_and(|w| ["away", "through", "into", "back"].contains(w)))
            || (matches!(direction, Up | Down)
                && rest.get(1) == Some(&"to")
                && !["climb", "jump", "fly", "swim"].contains(verb)))
    {
        return None;
    }
    if rest[used..].iter().any(|w| {
        [
            "left", "right", "up", "down", "north", "south", "east", "west", "or",
        ]
        .contains(w)
    }) {
        return None;
    }
    Some(direction)
}

pub fn cue(
    guidance: &Guidance,
    atlas: &Atlas,
    location: &PlaceTracker,
    now: u64,
) -> Option<DirectionCue> {
    let action = guidance.now.as_ref()?;
    let place = location.place_id.as_deref()?;
    if guidance.status != "ready"
        || !["estimated", "reported"].contains(&location.status.as_str())
        || location.located_at_ms == 0
        || now < location.located_at_ms
        || now - location.located_at_ms >= 15_000
        || !atlas
            .bindings
            .iter()
            .any(|b| b.step_id == action.id && b.place_id == place)
    {
        return None;
    }
    Some(DirectionCue {
        direction: movement(&action.instructions)?,
        step_id: action.id.clone(),
        label: action.title.clone(),
        instruction: action.instructions.clone(),
        valid_until_ms: location.located_at_ms.saturating_add(15_000),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PlaceBinding, import_markdown};

    #[test]
    fn reads_only_immediate_explicit_screen_movement() {
        use ScreenDirection::*;
        for (text, direction) in [
            ("Walk right to the next door.", Right),
            ("Now head to the left.", Left),
            ("Climb up the ladder.", Up),
            ("Jump down to the platform.", Down),
            ("Move up-right through the opening.", UpRight),
            ("Go down and left to the stairs.", DownLeft),
        ] {
            assert_eq!(movement(text), Some(direction), "{text}");
        }
        for text in [
            "Buy the right key.",
            "Do not go left.",
            "Walk right if you have the key.",
            "After buying the key, walk left.",
            "Go right, then left.",
            "Go right or left.",
            "Talk to the king. Go right.",
            "Go east to the town.",
            "Move forward.",
            "Go to the shop on the right.",
            "Walk right, not left.",
            "Turn left.",
            "Go right through the doorway.",
            "Walk up to the king.",
        ] {
            assert_eq!(movement(text), None, "{text}");
        }
    }

    #[test]
    fn current_action_needs_fresh_matching_place_evidence() {
        let guide =
            import_markdown("Game", "Route", "# Leave town\nWalk right to the gate.").unwrap();
        let mut guidance = Guidance {
            status: "ready".into(),
            now: Some(guide.steps[0].clone()),
            ..Default::default()
        };
        let atlas = Atlas {
            bindings: vec![PlaceBinding {
                step_id: guide.steps[0].id.clone(),
                place_id: "town".into(),
                evidence: "Route begins in town".into(),
            }],
            ..Default::default()
        };
        let mut location = PlaceTracker {
            status: "estimated".into(),
            place_id: Some("town".into()),
            located_at_ms: 1_000,
            ..Default::default()
        };
        assert_eq!(
            cue(&guidance, &atlas, &location, 1_200).unwrap().direction,
            ScreenDirection::Right
        );
        assert!(cue(&guidance, &atlas, &location, 16_000).is_none());
        location.place_id = Some("forest".into());
        assert!(cue(&guidance, &atlas, &location, 1_200).is_none());
        location.place_id = Some("town".into());
        location.status = "uncertain".into();
        assert!(cue(&guidance, &atlas, &location, 1_200).is_none());
        location.status = "reported".into();
        guidance.status = "blocked".into();
        assert!(cue(&guidance, &atlas, &location, 1_200).is_none());
        guidance.status = "ready".into();
        guidance.now = None;
        assert!(cue(&guidance, &atlas, &location, 1_200).is_none());
    }
}
