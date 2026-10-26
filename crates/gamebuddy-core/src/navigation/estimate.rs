//! One location estimate, fused from every source that knows something about
//! where the player is: the player's own report, the geometric match of the
//! screen against map pixels, the visual room tracker and, as a weak prior,
//! where the plan expects the player to be. Each source carries a strength
//! and a time; agreement adds up, disagreement is settled by weight, and the
//! result says how sure it is instead of letting each source speak alone.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    /// The player said where they are.
    PlayerReport,
    /// The screen registered geometrically against a map's pixels.
    MapRegistration,
    /// The screen matched walkthrough frames of a known room.
    VisualRoom,
    /// Where the walkthrough's current step takes place.
    PlanExpectation,
}

impl EvidenceKind {
    /// How much a fresh, full-strength piece of this evidence counts.
    fn base_weight(self) -> f32 {
        match self {
            Self::PlayerReport => 1.0,
            Self::MapRegistration => 0.9,
            Self::VisualRoom => 0.7,
            Self::PlanExpectation => 0.25,
        }
    }
    /// After this long the evidence is worth nothing (the plan never ages).
    fn lifetime_ms(self) -> Option<u64> {
        match self {
            Self::PlayerReport => Some(180_000),
            Self::MapRegistration | Self::VisualRoom => Some(15_000),
            Self::PlanExpectation => None,
        }
    }
    /// Fresh evidence keeps its full weight for this long before fading.
    fn fresh_ms(self) -> u64 {
        match self {
            Self::PlayerReport => 60_000,
            Self::MapRegistration | Self::VisualRoom => 5_000,
            Self::PlanExpectation => 0,
        }
    }
}

/// A point on a map, in fractions of its width and height.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MapPosition {
    pub map_id: String,
    pub x: f32,
    pub y: f32,
    /// The visible camera area around the point, `[x, y, width, height]`.
    pub region: Option<[f32; 4]>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Evidence {
    pub kind: EvidenceKind,
    pub place_id: Option<String>,
    pub map_id: Option<String>,
    pub position: Option<MapPosition>,
    pub at_ms: u64,
    /// 0..=1: how sure this source itself is.
    pub strength: f32,
    pub detail: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Source {
    pub kind: EvidenceKind,
    pub weight: f32,
    pub age_ms: u64,
    pub agrees: bool,
    pub detail: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LocationEstimate {
    /// `confident`, `likely`, `uncertain` or `unknown`.
    pub status: String,
    pub confidence: f32,
    pub place_id: Option<String>,
    pub map_id: Option<String>,
    pub position: Option<MapPosition>,
    pub located_at_ms: u64,
    pub sources: Vec<Source>,
}

impl LocationEstimate {
    pub fn is_known(&self) -> bool {
        self.status == "confident" || self.status == "likely"
    }
}

fn weight(evidence: &Evidence, now: u64) -> f32 {
    let age = now.saturating_sub(evidence.at_ms);
    let decay = match evidence.kind.lifetime_ms() {
        None => 1.0,
        Some(lifetime) if age >= lifetime => 0.0,
        Some(lifetime) => {
            let fresh = evidence.kind.fresh_ms();
            if age <= fresh {
                1.0
            } else {
                1.0 - (age - fresh) as f32 / (lifetime - fresh) as f32
            }
        }
    };
    evidence.kind.base_weight() * evidence.strength.clamp(0.0, 1.0) * decay
}

/// What a piece of evidence votes for: a place when it names one, else a map.
fn candidate(evidence: &Evidence) -> Option<String> {
    evidence
        .place_id
        .as_ref()
        .map(|p| format!("place:{p}"))
        .or_else(|| evidence.map_id.as_ref().map(|m| format!("map:{m}")))
}

/// Evidence that names only a map agrees with a place on that map.
fn compatible(a: &Evidence, b: &Evidence) -> bool {
    match (&a.place_id, &b.place_id) {
        (Some(x), Some(y)) => x == y,
        _ => match (&a.map_id, &b.map_id) {
            (Some(x), Some(y)) => x == y,
            _ => false,
        },
    }
}

pub fn fuse(evidence: &[Evidence], now: u64) -> LocationEstimate {
    let weighted: Vec<(&Evidence, f32)> = evidence
        .iter()
        .map(|e| (e, weight(e, now)))
        .filter(|(e, w)| *w > 0.0 && candidate(e).is_some())
        .collect();
    let total: f32 = weighted.iter().map(|(_, w)| w).sum();
    if total <= 0.0 {
        return LocationEstimate {
            status: "unknown".into(),
            sources: evidence
                .iter()
                .map(|e| Source {
                    kind: e.kind,
                    weight: 0.0,
                    age_ms: now.saturating_sub(e.at_ms),
                    agrees: false,
                    detail: e.detail.clone(),
                })
                .collect(),
            ..Default::default()
        };
    }
    // Each candidate's support is everything compatible with it, so a map-only
    // registration backs the one place on that map rather than competing.
    let mut best: Option<(&Evidence, f32)> = None;
    for (anchor, _) in &weighted {
        if anchor.place_id.is_none()
            && weighted
                .iter()
                .any(|(e, _)| e.place_id.is_some() && compatible(anchor, e))
        {
            continue;
        }
        let support: f32 = weighted
            .iter()
            .filter(|(e, _)| compatible(anchor, e))
            .map(|(_, w)| w)
            .sum();
        if best.is_none_or(|(_, s)| support > s) {
            best = Some((anchor, support));
        }
    }
    let (winner, support) = best.expect("some weighted evidence");
    // Confidence needs both agreement (the winner's share of all evidence)
    // and substance (enough real evidence; the plan alone is a guess).
    let confidence = ((support / (total + 0.15)) * (support / 0.9).min(1.0)).clamp(0.0, 1.0);
    let agreeing: Vec<&Evidence> = weighted
        .iter()
        .filter(|(e, _)| compatible(winner, e))
        .map(|(e, _)| *e)
        .collect();
    let position = agreeing
        .iter()
        .filter(|e| e.position.is_some())
        .max_by_key(|e| e.at_ms)
        .and_then(|e| e.position.clone());
    let map_id = winner
        .map_id
        .clone()
        .or_else(|| agreeing.iter().find_map(|e| e.map_id.clone()));
    let located_at_ms = agreeing
        .iter()
        .filter(|e| e.kind != EvidenceKind::PlanExpectation)
        .map(|e| e.at_ms)
        .max()
        .unwrap_or(0);
    let status = if confidence >= 0.75 {
        "confident"
    } else if confidence >= 0.5 {
        "likely"
    } else {
        "uncertain"
    };
    LocationEstimate {
        status: status.into(),
        confidence,
        place_id: winner.place_id.clone(),
        map_id,
        position,
        located_at_ms,
        sources: evidence
            .iter()
            .map(|e| Source {
                kind: e.kind,
                weight: weight(e, now),
                age_ms: now.saturating_sub(e.at_ms),
                agrees: weight(e, now) > 0.0 && compatible(winner, e),
                detail: e.detail.clone(),
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn evidence(
        kind: EvidenceKind,
        place: Option<&str>,
        map: Option<&str>,
        at: u64,
        strength: f32,
    ) -> Evidence {
        Evidence {
            kind,
            place_id: place.map(str::to_owned),
            map_id: map.map(str::to_owned),
            position: map
                .filter(|_| kind == EvidenceKind::MapRegistration)
                .map(|m| MapPosition {
                    map_id: m.into(),
                    x: 0.5,
                    y: 0.5,
                    region: None,
                }),
            at_ms: at,
            strength,
            detail: String::new(),
        }
    }

    #[test]
    fn plan_alone_is_only_a_guess() {
        let e = fuse(
            &[evidence(
                EvidenceKind::PlanExpectation,
                Some("town"),
                Some("m1"),
                0,
                1.0,
            )],
            10_000,
        );
        assert_eq!(e.status, "uncertain");
        assert_eq!(e.place_id.as_deref(), Some("town"));
        assert_eq!(e.located_at_ms, 0);
    }

    #[test]
    fn agreeing_screen_and_plan_are_confident_and_carry_the_position() {
        let e = fuse(
            &[
                evidence(
                    EvidenceKind::PlanExpectation,
                    Some("town"),
                    Some("m1"),
                    0,
                    1.0,
                ),
                evidence(EvidenceKind::MapRegistration, None, Some("m1"), 9_000, 0.9),
                evidence(EvidenceKind::VisualRoom, Some("town"), None, 9_500, 0.8),
            ],
            10_000,
        );
        assert_eq!(e.status, "confident", "{e:?}");
        assert_eq!(e.place_id.as_deref(), Some("town"));
        assert_eq!(e.position.as_ref().map(|p| p.map_id.as_str()), Some("m1"));
        assert_eq!(e.located_at_ms, 9_500);
        assert!(e.sources.iter().all(|s| s.agrees));
    }

    #[test]
    fn a_fresh_player_report_beats_the_screen_and_the_plan() {
        let e = fuse(
            &[
                evidence(
                    EvidenceKind::PlanExpectation,
                    Some("town"),
                    Some("m1"),
                    0,
                    1.0,
                ),
                evidence(EvidenceKind::VisualRoom, Some("town"), None, 9_500, 0.8),
                evidence(
                    EvidenceKind::PlayerReport,
                    Some("fountain"),
                    None,
                    9_800,
                    1.0,
                ),
            ],
            10_000,
        );
        assert_eq!(e.place_id.as_deref(), Some("fountain"));
        assert_eq!(e.status, "likely", "{e:?}");
        assert!(e.position.is_none());
    }

    #[test]
    fn stale_screen_evidence_expires_and_the_estimate_falls_back_to_the_plan() {
        let e = fuse(
            &[
                evidence(
                    EvidenceKind::PlanExpectation,
                    Some("town"),
                    Some("m1"),
                    0,
                    1.0,
                ),
                evidence(EvidenceKind::MapRegistration, None, Some("m2"), 1_000, 1.0),
            ],
            40_000,
        );
        assert_eq!(e.place_id.as_deref(), Some("town"));
        assert_eq!(e.map_id.as_deref(), Some("m1"));
        assert!(e.position.is_none());
    }

    #[test]
    fn nothing_known_is_unknown() {
        assert_eq!(fuse(&[], 5).status, "unknown");
        let e = fuse(
            &[evidence(EvidenceKind::VisualRoom, Some("x"), None, 0, 1.0)],
            20_000,
        );
        assert_eq!(e.status, "unknown");
    }
}
