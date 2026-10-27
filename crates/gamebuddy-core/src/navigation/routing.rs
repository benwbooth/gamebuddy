//! Getting from one place to another, worked out from the walkthrough itself.
//!
//! The walkthrough visits places in order; each change of place is a link the
//! walkthrough player actually travelled, with the instruction that made the
//! move. A route between any two places is the shortest chain of such links,
//! walked forwards or backwards. Going back over a link flips its direction
//! (a passage entered by going right is left by going left).
use super::direction::ScreenDirection;
use crate::{Atlas, Guide};
use serde::Serialize;
use std::collections::{HashMap, VecDeque};

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Link {
    pub from: String,
    pub to: String,
    /// The step that arrived in `to`.
    pub step_id: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Hop {
    pub from: String,
    pub to: String,
    pub step_id: String,
    /// True when this hop goes back over the walkthrough's own move.
    pub backwards: bool,
    /// Which way to head on screen for this hop, when the instruction says.
    pub direction: Option<ScreenDirection>,
}

/// Every move between two places the walkthrough makes, once each.
pub fn links(guide: &Guide, atlas: &Atlas) -> Vec<Link> {
    let place: HashMap<&str, &str> = atlas
        .bindings
        .iter()
        .map(|b| (b.step_id.as_str(), b.place_id.as_str()))
        .collect();
    let mut out: Vec<Link> = vec![];
    let mut last: Option<&str> = None;
    for step in &guide.steps {
        let Some(&here) = place.get(step.id.as_str()) else {
            continue;
        };
        if let Some(previous) = last
            && previous != here
            && !out.iter().any(|l| {
                (l.from == previous && l.to == here) || (l.from == here && l.to == previous)
            })
        {
            out.push(Link {
                from: previous.into(),
                to: here.into(),
                step_id: step.id.clone(),
            });
        }
        last = Some(here);
    }
    out
}

/// The way an instruction says to travel, only when it says so with a verb
/// of movement ("Move right into...", "Continue left along...", "Climb the
/// ladder"). A direction that only describes where something is ("the
/// ladders at the upper left") is not a heading.
pub fn travel_direction(text: &str) -> Option<ScreenDirection> {
    const MOVES: [&str; 12] = [
        "move", "walk", "head", "go", "continue", "travel", "run", "proceed", "return", "exit",
        "cross", "follow",
    ];
    let first = text
        .split(['.', ';'])
        .next()
        .unwrap_or_default()
        .to_lowercase();
    let words: Vec<&str> = first
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    for (i, word) in words.iter().enumerate() {
        match *word {
            "climb" | "ascend" => return Some(ScreenDirection::Up),
            "descend" => return Some(ScreenDirection::Down),
            w if MOVES.contains(&w) => {
                // The heading must follow the verb closely, not as a place
                // ("go to the shop on the right").
                for next in words.iter().skip(i + 1).take(3) {
                    match *next {
                        "left" | "leftward" | "leftwards" => return Some(ScreenDirection::Left),
                        "right" | "rightward" | "rightwards" => {
                            return Some(ScreenDirection::Right);
                        }
                        "up" | "upward" | "upstairs" => return Some(ScreenDirection::Up),
                        "down" | "downward" | "downstairs" => return Some(ScreenDirection::Down),
                        "the" | "to" | "into" | "through" | "toward" | "towards" => break,
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    None
}

fn flip(direction: ScreenDirection) -> ScreenDirection {
    use ScreenDirection::*;
    match direction {
        Left => Right,
        Right => Left,
        Up => Down,
        Down => Up,
        UpLeft => DownRight,
        UpRight => DownLeft,
        DownLeft => UpRight,
        DownRight => UpLeft,
    }
}

/// The shortest chain of walkthrough moves from one place to another.
pub fn route(guide: &Guide, atlas: &Atlas, from: &str, to: &str) -> Option<Vec<Hop>> {
    if from == to {
        return Some(vec![]);
    }
    let links = links(guide, atlas);
    let mut previous: HashMap<String, (String, usize, bool)> = HashMap::new();
    let mut queue = VecDeque::from([from.to_owned()]);
    while let Some(here) = queue.pop_front() {
        if here == to {
            break;
        }
        for (i, link) in links.iter().enumerate() {
            let (next, backwards) = if link.from == here {
                (&link.to, false)
            } else if link.to == here {
                (&link.from, true)
            } else {
                continue;
            };
            if next != from && !previous.contains_key(next) {
                previous.insert(next.clone(), (here.clone(), i, backwards));
                queue.push_back(next.clone());
            }
        }
    }
    let mut hops = vec![];
    let mut at = to.to_owned();
    while at != from {
        let (before, i, backwards) = previous.get(&at)?.clone();
        let link = &links[i];
        let direction = guide
            .step(&link.step_id)
            .and_then(|s| travel_direction(&s.instructions))
            .map(|d| if backwards { flip(d) } else { d });
        hops.push(Hop {
            from: before.clone(),
            to: at.clone(),
            step_id: link.step_id.clone(),
            backwards,
            direction,
        });
        at = before;
    }
    hops.reverse();
    Some(hops)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Place, PlaceBinding, import_markdown};

    #[test]
    fn going_back_retraces_the_walkthrough_with_directions_flipped() {
        let guide = import_markdown(
            "Game",
            "Route",
            "# Leave town\nWalk right out of the gate.\n# Field\nMove right across the field to the tower.\n# Tower\nClimb the ladder to the top.\n# Back at the field\nFight here.",
        )
        .unwrap();
        let ids: Vec<String> = guide.steps.iter().map(|s| s.id.clone()).collect();
        let place = |id: &str| Place {
            id: id.into(),
            label: id.into(),
            map_id: None,
            landmarks: String::new(),
        };
        let bind = |step: &str, place: &str| PlaceBinding {
            step_id: step.into(),
            place_id: place.into(),
            evidence: String::new(),
        };
        let atlas = Atlas {
            places: vec![place("town"), place("field"), place("tower")],
            bindings: vec![
                bind(&ids[0], "town"),
                bind(&ids[1], "field"),
                bind(&ids[2], "tower"),
                bind(&ids[3], "field"),
            ],
            ..Default::default()
        };
        let forward = route(&guide, &atlas, "town", "tower").unwrap();
        assert_eq!(forward.len(), 2);
        assert_eq!(forward[0].direction, Some(ScreenDirection::Right));
        assert_eq!(forward[1].direction, Some(ScreenDirection::Up));
        let back = route(&guide, &atlas, "tower", "town").unwrap();
        assert_eq!(
            back.iter().map(|h| h.to.as_str()).collect::<Vec<_>>(),
            vec!["field", "town"]
        );
        assert!(back.iter().all(|h| h.backwards));
        assert_eq!(back[0].direction, Some(ScreenDirection::Down));
        assert_eq!(back[1].direction, Some(ScreenDirection::Left));
        assert!(route(&guide, &atlas, "town", "nowhere").is_none());
    }

    #[test]
    fn loose_directions_skip_place_names() {
        assert_eq!(
            travel_direction("Continue right along the bottom walkway."),
            Some(ScreenDirection::Right)
        );
        assert_eq!(
            travel_direction("Enter the door on the right and talk."),
            None
        );
        assert_eq!(
            travel_direction("Move toward the left side and climb the ladder"),
            None.or(Some(ScreenDirection::Up))
        );
    }
}
