//! Time screen publication for a captured frame: decode, crop, resize, JPEG
//! encode and store, at full size and at the 2560 px capture size.
//! Usage: capture_bench <data directory> <frame.png>
use anyhow::{Context, Result};
use gamebuddy_services::{Store, screen};
use std::time::Instant;

fn main() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    let store = Store::open(args.next().context("Data directory")?)?;
    let frame = std::path::PathBuf::from(args.next().context("Frame")?);
    let ticket = screen::start(&store, "Faxanadu", true)?;
    // The capture path keeps every second pixel of a 5120 px surface and
    // writes a fast-compressed PNG.
    let full = image::open(&frame)?.to_rgb8();
    let reduced = image::RgbImage::from_fn(full.width() / 2, full.height() / 2, |x, y| {
        *full.get_pixel(x * 2, y * 2)
    });
    let small = std::env::temp_dir().join("capture-bench-small.png");
    for _ in 0..3 {
        let started = Instant::now();
        let file = std::io::BufWriter::new(std::fs::File::create(&small)?);
        image::ImageEncoder::write_image(
            image::codecs::png::PngEncoder::new_with_quality(
                file,
                image::codecs::png::CompressionType::Fast,
                image::codecs::png::FilterType::Sub,
            ),
            reduced.as_raw(),
            reduced.width(),
            reduced.height(),
            image::ExtendedColorType::Rgb8,
        )?;
        let written = started.elapsed();
        let publish = Instant::now();
        screen::publish(&store, &ticket, &small, "bench")?;
        println!(
            "reduced: png_encode_ms={} publish_ms={}",
            written.as_millis(),
            publish.elapsed().as_millis()
        );
    }
    for _ in 0..3 {
        let started = Instant::now();
        let image = image::open(&frame)?;
        let decoded = started.elapsed();
        let encode = Instant::now();
        image.save(std::env::temp_dir().join("capture-bench.png"))?;
        let png = encode.elapsed();
        let publish = Instant::now();
        screen::publish(&store, &ticket, &frame, "bench")?;
        println!(
            "decode_ms={} png_encode_ms={} publish_ms={}",
            decoded.as_millis(),
            png.as_millis(),
            publish.elapsed().as_millis()
        );
    }
    Ok(())
}
