use anyhow::{Result, ensure};
use image::{DynamicImage, GenericImageView, imageops::FilterType};
use serde::{Deserialize, Serialize};

pub const FINGERPRINT_VERSION: &str = "spatial-rgb-v1";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct Crop {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}
impl Default for Crop {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
        }
    }
}
impl Crop {
    pub fn validate(self) -> Result<()> {
        ensure!(
            [self.x, self.y, self.width, self.height]
                .iter()
                .all(|v| v.is_finite()),
            "Crop must be finite"
        );
        ensure!(
            self.x >= 0.0
                && self.y >= 0.0
                && self.width > 0.0
                && self.height > 0.0
                && self.x + self.width <= 1.001
                && self.y + self.height <= 1.001,
            "Crop must fit inside the image"
        );
        Ok(())
    }
    pub fn apply(self, image: &DynamicImage) -> Result<DynamicImage> {
        self.validate()?;
        let (w, h) = image.dimensions();
        ensure!(w > 0 && h > 0, "Empty image");
        let x = ((self.x * w as f32) as u32).min(w - 1);
        let y = ((self.y * h as f32) as u32).min(h - 1);
        Ok(image.crop_imm(
            x,
            y,
            ((self.width * w as f32) as u32).max(1).min(w - x),
            ((self.height * h as f32) as u32).max(1).min(h - y),
        ))
    }
}

/// Cheap candidate retrieval only. This is not a trained vision model and cannot verify progress.
pub fn fingerprint(image: &DynamicImage, crop: Crop) -> Result<Vec<f32>> {
    let small = crop
        .apply(image)?
        .resize_exact(24, 18, FilterType::Triangle)
        .to_rgb8();
    Ok(small.as_raw().iter().map(|&v| v as f32 / 255.0).collect())
}

/// Uniform fades/loading frames carry no usable location evidence. Measure
/// spatial variation within each channel, so a solid coloured frame also fails.
/// Applies to existing cached fingerprints; no source re-download is required.
pub fn fingerprint_has_detail(vector: &[f32]) -> bool {
    if vector.len() < 6
        || !vector.len().is_multiple_of(3)
        || vector
            .iter()
            .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
    {
        return false;
    }
    let pixels = (vector.len() / 3) as f32;
    let mut mean = [0.0; 3];
    for pixel in vector.as_chunks::<3>().0 {
        for channel in 0..3 {
            mean[channel] += pixel[channel] / pixels;
        }
    }
    let variance = vector
        .as_chunks::<3>()
        .0
        .iter()
        .map(|p| (0..3).map(|c| (p[c] - mean[c]).powi(2)).sum::<f32>())
        .sum::<f32>()
        / vector.len() as f32;
    variance >= 0.025_f32.powi(2)
}

pub fn similarity(a: &[f32], b: &[f32]) -> Result<f32> {
    ensure!(
        !a.is_empty() && a.len() == b.len(),
        "Incompatible fingerprints"
    );
    ensure!(
        a.iter().chain(b).all(|v| v.is_finite()),
        "Nonfinite fingerprint"
    );
    let rms = (a.iter().zip(b).map(|(a, b)| (a - b).powi(2)).sum::<f32>() / a.len() as f32).sqrt();
    Ok((1.0 - rms).clamp(0.0, 1.0))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReferenceFrame {
    pub id: String,
    pub source_id: String,
    pub at_ms: u64,
    pub image_path: String,
    pub step_ids: Vec<String>,
    pub fingerprint: Vec<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VisualIndex {
    pub guide_id: String,
    pub guide_revision: u64,
    pub encoder: String,
    pub crop: Crop,
    /// The player's frame may have a different border/layout than the guide video.
    #[serde(default)]
    pub live_crop: Crop,
    pub frames: Vec<ReferenceFrame>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_fades_and_uniform_colour_but_keeps_dark_gameplay_detail() {
        for pixel in [[0.0, 0.0, 0.0], [1.0, 1.0, 1.0], [1.0, 0.0, 0.0]] {
            assert!(!fingerprint_has_detail(&pixel.repeat(432)));
        }
        assert!(!fingerprint_has_detail(
            &[0.0, 0.001, 0.002, 0.003, 0.0, 0.001].repeat(216)
        ));
        assert!(fingerprint_has_detail(
            &[0.0, 0.0, 0.0, 0.1, 0.05, 0.05].repeat(216)
        ));
    }
    #[test]
    fn matching_survives_resize_and_detects_different_scene() {
        let a = DynamicImage::ImageRgb8(image::RgbImage::from_fn(256, 240, |x, y| {
            image::Rgb([(x % 256) as u8, (y % 256) as u8, 30])
        }));
        let b = a.resize_exact(768, 720, FilterType::Nearest);
        let c = DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            256,
            240,
            image::Rgb([0, 0, 255]),
        ));
        let af = fingerprint(&a, Crop::default()).unwrap();
        assert!(similarity(&af, &fingerprint(&b, Crop::default()).unwrap()).unwrap() > 0.99);
        assert!(similarity(&af, &fingerprint(&c, Crop::default()).unwrap()).unwrap() < 0.6);
    }
    #[test]
    fn crop_and_vectors_reject_invalid_input() {
        assert!(
            Crop {
                x: f32::NAN,
                ..Crop::default()
            }
            .validate()
            .is_err()
        );
        assert!(similarity(&[1.0], &[]).is_err());
        assert!(similarity(&[f32::NAN], &[0.0]).is_err());
    }
}
