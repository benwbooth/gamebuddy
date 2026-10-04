//! Image-grounded 2D localization. A match needs distinctive patch agreement,
//! one coherent geometric transform, distributed inliers and room separation.
//! No language-model confidence or objective ordering enters the pixel matcher.
use image::{
    DynamicImage, GrayImage,
    imageops::{self, FilterType},
};
use serde::{Deserialize, Serialize};

pub const WIDTH: u32 = 256;
pub const HEIGHT: u32 = 192;
pub const ENCODER: &str = "brief256-affine-v1";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Feature {
    pub x: f32,
    pub y: f32,
    pub bits: [u64; 4],
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Features {
    pub points: Vec<Feature>,
    pub histogram: Vec<f32>,
    #[serde(default)]
    pub embedding: Vec<f32>,
    #[serde(default)]
    pub learned_key: Option<String>,
}

/// Frames must already be cropped to the gameplay background (HUD excluded).
pub fn normalize(image: &DynamicImage) -> GrayImage {
    imageops::blur(
        &image
            .resize_exact(WIDTH, HEIGHT, FilterType::Triangle)
            .to_luma8(),
        0.8,
    )
}

pub fn extract(image: &GrayImage) -> Features {
    let (w, h) = image.dimensions();
    let pixel = |x: u32, y: u32| image.get_pixel(x, y)[0] as f32;
    let mut corners = Vec::new();
    for y in (14..h.saturating_sub(14)).step_by(2) {
        for x in (14..w.saturating_sub(14)).step_by(2) {
            let (mut xx, mut yy, mut xy) = (0., 0., 0.);
            for v in y - 2..=y + 2 {
                for u in x - 2..=x + 2 {
                    let dx = pixel(u + 1, v) - pixel(u - 1, v);
                    let dy = pixel(u, v + 1) - pixel(u, v - 1);
                    xx += dx * dx;
                    yy += dy * dy;
                    xy += dx * dy;
                }
            }
            let score = xx * yy - xy * xy - 0.05 * (xx + yy).powi(2);
            if score > 1_000_000. {
                corners.push((score, x, y));
            }
        }
    }
    corners.sort_by(|a, b| b.0.total_cmp(&a.0));
    let mut points: Vec<Feature> = Vec::new();
    for (_, x, y) in corners {
        if points
            .iter()
            .any(|p| (p.x - x as f32).powi(2) + (p.y - y as f32).powi(2) < 49.)
        {
            continue;
        }
        let mut seed = 0x92fda673u32;
        let mut bits = [0u64; 4];
        for i in 0..256 {
            let mut offset = || {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                ((seed >> 16) % 25) as i32 - 12
            };
            let (ax, ay, bx, by) = (offset(), offset(), offset(), offset());
            if pixel((x as i32 + ax) as u32, (y as i32 + ay) as u32)
                > pixel((x as i32 + bx) as u32, (y as i32 + by) as u32)
            {
                bits[i / 64] |= 1u64 << (i % 64);
            }
        }
        points.push(Feature {
            x: x as f32,
            y: y as f32,
            bits,
        });
        if points.len() == 180 {
            break;
        }
    }
    // Spatial edge histograms shortlist candidates; geometry decides acceptance.
    let mut histogram = vec![0.; 4 * 3 * 8];
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let dx = pixel(x + 1, y) - pixel(x - 1, y);
            let dy = pixel(x, y + 1) - pixel(x, y - 1);
            let magnitude = (dx * dx + dy * dy).sqrt();
            let angle = (dy.atan2(dx) + std::f32::consts::PI) / (2. * std::f32::consts::PI);
            let bin = ((angle * 8.) as usize).min(7);
            let cell = ((y * 3 / h) * 4 + x * 4 / w) as usize;
            histogram[cell * 8 + bin] += magnitude;
        }
    }
    for cell in histogram.chunks_mut(8) {
        let norm = cell.iter().map(|v| v * v).sum::<f32>().sqrt().max(1.);
        for v in cell {
            *v /= norm;
        }
    }
    Features {
        points,
        histogram,
        embedding: vec![],
        learned_key: None,
    }
}

pub fn retrieval(a: &Features, b: &Features) -> f32 {
    if !a.embedding.is_empty() && a.embedding.len() == b.embedding.len() {
        return a
            .embedding
            .iter()
            .zip(&b.embedding)
            .map(|(a, b)| a * b)
            .sum();
    }
    a.histogram
        .iter()
        .zip(&b.histogram)
        .map(|(a, b)| a * b)
        .sum::<f32>()
        / 12.
}
fn distance(a: &Feature, b: &Feature) -> u32 {
    a.bits
        .iter()
        .zip(b.bits)
        .map(|(a, b)| (a ^ b).count_ones())
        .sum()
}

/// Maps query background coordinates into reference background coordinates.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Transform {
    pub xx: f32,
    pub xy: f32,
    pub tx: f32,
    pub yx: f32,
    pub yy: f32,
    pub ty: f32,
}
impl Transform {
    pub fn point(&self, x: f32, y: f32) -> [f32; 2] {
        [
            self.xx * x + self.xy * y + self.tx,
            self.yx * x + self.yy * y + self.ty,
        ]
    }
    fn plausible(&self) -> bool {
        self.xx.is_finite()
            && self.yy.is_finite()
            && self.tx.is_finite()
            && self.ty.is_finite()
            && (0.65..1.5).contains(&self.xx)
            && (0.65..1.5).contains(&self.yy)
            && self.xy.abs() < 0.25
            && self.yx.abs() < 0.25
            && (self.xx - self.yy).abs() < 0.4
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Registration {
    pub transform: Transform,
    pub inliers: usize,
    pub matches: usize,
    pub coverage: f32,
    pub residual: f32,
    pub score: f32,
    #[serde(default)]
    pub geometry: Geometry,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Geometry {
    #[default]
    Affine,
    Epipolar,
}
fn affine(pairs: &[(&Feature, &Feature)], ids: [usize; 3]) -> Option<Transform> {
    let (a, b, c) = (pairs[ids[0]], pairs[ids[1]], pairs[ids[2]]);
    let (dx1, dy1, dx2, dy2) = (b.0.x - a.0.x, b.0.y - a.0.y, c.0.x - a.0.x, c.0.y - a.0.y);
    let det = dx1 * dy2 - dx2 * dy1;
    if det.abs() < 80. {
        return None;
    }
    let solve = |u: f32, v: f32| [(u * dy2 - v * dy1) / det, (v * dx1 - u * dx2) / det];
    let x = solve(b.1.x - a.1.x, c.1.x - a.1.x);
    let y = solve(b.1.y - a.1.y, c.1.y - a.1.y);
    let t = Transform {
        xx: x[0],
        xy: x[1],
        tx: a.1.x - x[0] * a.0.x - x[1] * a.0.y,
        yx: y[0],
        yy: y[1],
        ty: a.1.y - y[0] * a.0.x - y[1] * a.0.y,
    };
    t.plausible().then_some(t)
}

pub fn register(query: &Features, reference: &Features) -> Option<Registration> {
    if query.points.len() < 10 || reference.points.len() < 10 {
        return None;
    }
    let mut pairs = Vec::new();
    for q in &query.points {
        let mut closest = [(u32::MAX, 0usize); 2];
        for (j, r) in reference.points.iter().enumerate() {
            let d = distance(q, r);
            if d < closest[0].0 {
                closest[1] = closest[0];
                closest[0] = (d, j);
            } else if d < closest[1].0 {
                closest[1] = (d, j);
            }
        }
        if closest[0].0 > 64
            || closest[0].0 + 5 >= closest[1].0
            || closest[0].0 as f32 >= closest[1].0 as f32 * 0.85
        {
            continue;
        }
        let r = &reference.points[closest[0].1];
        if query
            .points
            .iter()
            .any(|other| distance(other, r) < closest[0].0)
        {
            continue;
        }
        pairs.push((q, r));
    }
    if pairs.len() < 8 {
        return None;
    }
    let mut best: Option<Registration> = None;
    let mut seed = 0x193872aau32;
    for _ in 0..384 {
        let mut pick = || {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            seed as usize % pairs.len()
        };
        let Some(t) = affine(&pairs, [pick(), pick(), pick()]) else {
            continue;
        };
        let mut cells = std::collections::HashSet::new();
        let mut sum = 0.;
        let mut n = 0;
        let mut min = [f32::MAX; 2];
        let mut max = [f32::MIN; 2];
        for (q, r) in &pairs {
            let p = t.point(q.x, q.y);
            let error = ((p[0] - r.x).powi(2) + (p[1] - r.y).powi(2)).sqrt();
            if error <= 4.5 {
                n += 1;
                sum += error;
                cells.insert((q.x as u32 / 64, q.y as u32 / 48));
                min[0] = min[0].min(q.x);
                min[1] = min[1].min(q.y);
                max[0] = max[0].max(q.x);
                max[1] = max[1].max(q.y);
            }
        }
        let coverage = (max[0] - min[0]) * (max[1] - min[1]) / (WIDTH * HEIGHT) as f32;
        if n < 8 || cells.len() < 4 || coverage < 0.15 || (n as f32) / (pairs.len() as f32) < 0.45 {
            continue;
        }
        let score = (n as f32 / 35.).min(1.) * 0.5
            + coverage.min(0.8) * 0.4
            + (1. - sum / n as f32 / 4.5) * 0.1;
        if best.as_ref().is_none_or(|b| score > b.score) {
            best = Some(Registration {
                transform: t,
                inliers: n,
                matches: pairs.len(),
                coverage,
                residual: sum / n as f32,
                score,
                geometry: Geometry::Affine,
            });
        }
    }
    best
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Hypothesis {
    pub room_id: String,
    pub score: f32,
    pub reference_id: String,
    pub registration: Registration,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Tracker {
    #[serde(default)]
    pub capture_epoch: u64,
    pub session: String,
    pub epoch: u64,
    pub observed_at_ms: u64,
    pub room_id: Option<String>,
    pub pending: Option<String>,
    pub votes: u32,
    pub last_match_ms: u64,
    pub hypotheses: Vec<Hypothesis>,
    pub status: String,
}
impl Tracker {
    pub fn observe(&mut self, session: &str, epoch: u64, at: u64, mut hypotheses: Vec<Hypothesis>) {
        if self.session != session
            || self.epoch != epoch
            || at.saturating_sub(self.observed_at_ms) > 15_000
        {
            *self = Self {
                session: session.into(),
                epoch,
                ..Self::default()
            };
        }
        if at <= self.observed_at_ms {
            return;
        }
        self.observed_at_ms = at;
        hypotheses.sort_by(|a, b| b.score.total_cmp(&a.score));
        let candidate = hypotheses.first().filter(|a| {
            a.score >= 0.48
                && hypotheses
                    .iter()
                    .find(|b| b.room_id != a.room_id)
                    .is_none_or(|b| a.score - b.score > 0.09)
        });
        if let Some(best) = candidate {
            if self.pending.as_deref() == Some(&best.room_id) {
                self.votes += 1;
            } else {
                self.pending = Some(best.room_id.clone());
                self.votes = 1;
            }
            if self.votes >= 2 {
                self.room_id = Some(best.room_id.clone());
                self.last_match_ms = at;
                self.status = "tracked".into();
            } else {
                self.status = "candidate".into();
            }
        } else {
            self.votes = 0;
            self.pending = None;
            self.status = "uncertain".into();
            if at.saturating_sub(self.last_match_ms) > 5000 {
                self.room_id = None;
            }
        }
        self.hypotheses = hypotheses.into_iter().take(8).collect();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texture(seed: u32) -> GrayImage {
        let mut state = seed;
        let mut image = GrayImage::new(WIDTH, HEIGHT);
        for y in (8..HEIGHT - 8).step_by(8) {
            for x in (8..WIDTH - 8).step_by(8) {
                state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                let color = (state >> 24) as u8;
                for yy in y..y + 5 {
                    for xx in x..x + 5 {
                        image.put_pixel(xx, yy, image::Luma([color]));
                    }
                }
            }
        }
        image
    }
    #[test]
    fn localizes_a_scrolled_noisy_frame_and_rejects_unrelated_background() {
        let reference = texture(91);
        let mut shifted = GrayImage::new(WIDTH, HEIGHT);
        imageops::overlay(&mut shifted, &reference, 16, 0);
        // Changing a foreground object must not invalidate background geometry.
        for y in 90..125 {
            for x in 100..125 {
                shifted.put_pixel(x, y, image::Luma([220]));
            }
        }
        let a = extract(&normalize(&DynamicImage::ImageLuma8(shifted)));
        let b = extract(&normalize(&DynamicImage::ImageLuma8(reference)));
        let match_ = register(&a, &b).expect("Scrolling should preserve background landmarks");
        assert!((match_.transform.tx + 16.).abs() < 2.);
        assert!(match_.transform.ty.abs() < 2.);
        assert!(register(&a, &extract(&texture(7742))).is_none());
        assert!(register(&a, &extract(&GrayImage::new(WIDTH, HEIGHT))).is_none());
    }
    fn hypothesis(room: &str, score: f32) -> Hypothesis {
        Hypothesis {
            room_id: room.into(),
            score,
            reference_id: format!("{room}-frame"),
            registration: Registration {
                transform: Transform {
                    xx: 1.,
                    xy: 0.,
                    tx: 0.,
                    yx: 0.,
                    yy: 1.,
                    ty: 0.,
                },
                inliers: 32,
                matches: 36,
                coverage: 0.6,
                residual: 1.,
                score,
                geometry: Geometry::Affine,
            },
        }
    }
    #[test]
    fn repeated_views_do_not_outvote_an_ambiguous_room_and_restore_clears_identity() {
        let mut tracker = Tracker::default();
        for at in [1000, 2000, 3000] {
            tracker.observe(
                "a",
                0,
                at,
                vec![
                    hypothesis("left", 0.8),
                    hypothesis("left", 0.79),
                    hypothesis("right", 0.78),
                ],
            );
        }
        assert!(tracker.room_id.is_none());
        tracker.observe("a", 0, 4000, vec![hypothesis("left", 0.8)]);
        tracker.observe("a", 0, 5000, vec![hypothesis("left", 0.8)]);
        assert_eq!(tracker.room_id.as_deref(), Some("left"));
        tracker.observe("a", 1, 6000, vec![hypothesis("right", 0.85)]);
        assert!(tracker.room_id.is_none());
        tracker.observe("a", 1, 7000, vec![hypothesis("right", 0.85)]);
        assert_eq!(tracker.room_id.as_deref(), Some("right"));
        tracker.observe("new launch", 1, 8000, vec![]);
        assert!(tracker.room_id.is_none());
    }
}
