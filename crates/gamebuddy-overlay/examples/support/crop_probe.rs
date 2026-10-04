//! Exercise the production Qt selector using only the private Gamescope display.
use anyhow::{Context, Result, ensure};
use gamebuddy_core::Crop;
use gamebuddy_overlay::x11::cardinal;
use gamebuddy_services::{Store, screen};
use serde_json::Value;
use std::{
    thread,
    time::{Duration, Instant},
};
use x11rb::{
    connection::Connection,
    protocol::{xproto::*, xtest::ConnectionExt as _},
    rust_connection::RustConnection,
};

fn wait(mut condition: impl FnMut() -> Result<bool>) -> Result<()> {
    let until = Instant::now() + Duration::from_secs(12);
    while !condition()? {
        ensure!(Instant::now() < until, "Crop UI condition timed out");
        thread::sleep(Duration::from_millis(100));
    }
    Ok(())
}
pub fn run(
    conn: &RustConnection,
    game: u32,
    overlay: u32,
    focus: u32,
    root: u32,
    toggle: impl Fn() -> Result<()>,
    screenshot: impl Fn(&str) -> Result<()>,
) -> Result<()> {
    let store = Store::open(std::env::var("GAMEBUDDY_TEST_DATA")?)?;
    let log = std::env::var("GAMEBUDDY_TEST_LOG")?;
    let gc = conn.generate_id()?;
    conn.create_gc(gc, game, &CreateGCAux::new().foreground(0xee2020))?
        .check()?;
    conn.poly_fill_rectangle(
        game,
        gc,
        &[
            Rectangle {
                x: 0,
                y: 0,
                width: 320,
                height: 720,
            },
            Rectangle {
                x: 960,
                y: 0,
                width: 320,
                height: 720,
            },
        ],
    )?
    .check()?;
    conn.free_gc(gc)?.check()?;
    conn.flush()?;
    let painted = screen::now_ms();
    wait(|| {
        Ok(screen::status(&store)?["captured_at_ms"]
            .as_u64()
            .unwrap_or_default()
            > painted)
    })?;
    let mouse = |kind, detail, x: f64, y: f64| -> Result<()> {
        conn.xtest_fake_input(kind, detail, 0, root, x.round() as i16, y.round() as i16, 0)?
            .check()?;
        conn.flush()?;
        thread::sleep(Duration::from_millis(160));
        Ok(())
    };
    let click = |point: &Value| -> Result<()> {
        mouse(
            MOTION_NOTIFY_EVENT,
            0,
            point[0].as_f64().context("x")?,
            point[1].as_f64().context("y")?,
        )?;
        mouse(BUTTON_PRESS_EVENT, 1, 0., 0.)?;
        mouse(BUTTON_RELEASE_EVENT, 1, 0., 0.)
    };
    let geometry = || -> Result<Value> {
        std::fs::read_to_string(&log)?
            .lines()
            .rev()
            .find_map(|l| {
                l.split_once("GAMEBUDDY_CROP_EDITOR ")
                    .map(|(_, v)| serde_json::from_str(v))
            })
            .context("Crop editor did not open")?
            .map_err(Into::into)
    };
    toggle()?;
    screenshot("crop-entry.png")?;
    click(&serde_json::json!([422, 358]))?;
    wait(|| Ok(geometry().is_ok()))?;
    ensure!(
        cardinal(conn, overlay, focus)? == 1,
        "Crop editor did not take input focus"
    );
    let g = geometry()?;
    let p = &g["plane"];
    let x = p[0].as_f64().unwrap();
    let y = p[1].as_f64().unwrap();
    let w = p[2].as_f64().unwrap();
    let h = p[3].as_f64().unwrap();
    // Select the central half, inset vertically to exercise letterbox transforms.
    mouse(MOTION_NOTIFY_EVENT, 0, x + w * 0.25, y + h * 0.10)?;
    mouse(BUTTON_PRESS_EVENT, 1, 0., 0.)?;
    mouse(MOTION_NOTIFY_EVENT, 0, x + w * 0.75, y + h * 0.90)?;
    mouse(BUTTON_RELEASE_EVENT, 1, 0., 0.)?;
    screenshot("crop-selection.png")?;
    click(&g["save"])?;
    wait(|| Ok(screen::game_crop(&store, "Faxanadu")?.width < 0.6))?;
    let mut crop = screen::game_crop(&store, "Faxanadu")?;
    ensure!(
        (crop.x - 0.25).abs() < 0.005
            && (crop.y - 0.1).abs() < 0.005
            && (crop.width - 0.5).abs() < 0.005
            && (crop.height - 0.8).abs() < 0.005,
        "Selection coordinates are wrong: {crop:?}"
    );
    wait(|| {
        Ok(screen::status(&store)?["status"] == "live"
            && screen::status(&store)?["width"]
                .as_u64()
                .unwrap_or_default()
                < 700)
    })?;
    let snapshot = screen::snapshot(&store)?;
    use base64::Engine;
    let image =
        image::load_from_memory(&base64::engine::general_purpose::STANDARD.decode(snapshot.jpeg)?)?
            .to_rgb8();
    ensure!(
        image.get_pixel(image.width() / 2, image.height() / 2).0[2] > 50,
        "Crop lost the blue play area"
    );
    ensure!(
        image.pixels().filter(|p| p.0[0] > 150).count() < image.width() as usize * 2,
        "Crop still contains the red bezel"
    );
    // Move the persisted selection, then resize its lower-right corner.
    click(&serde_json::json!([422, 358]))?;
    thread::sleep(Duration::from_millis(400));
    mouse(MOTION_NOTIFY_EVENT, 0, x + w * 0.5, y + h * 0.5)?;
    mouse(BUTTON_PRESS_EVENT, 1, 0., 0.)?;
    mouse(MOTION_NOTIFY_EVENT, 0, x + w * 0.55, y + h * 0.55)?;
    mouse(BUTTON_RELEASE_EVENT, 1, 0., 0.)?;
    click(&g["save"])?;
    wait(|| Ok(screen::game_crop(&store, "Faxanadu")?.x > 0.29))?;
    crop = screen::game_crop(&store, "Faxanadu")?;
    ensure!(
        (crop.width - 0.5).abs() < 0.005 && (crop.y - 0.15).abs() < 0.005,
        "Moving changed the selection size: {crop:?}"
    );
    click(&serde_json::json!([422, 358]))?;
    thread::sleep(Duration::from_millis(400));
    mouse(
        MOTION_NOTIFY_EVENT,
        0,
        x + w * (crop.x + crop.width) as f64 - 2.,
        y + h * (crop.y + crop.height) as f64 - 2.,
    )?;
    mouse(BUTTON_PRESS_EVENT, 1, 0., 0.)?;
    mouse(MOTION_NOTIFY_EVENT, 0, x + w * 0.7, y + h * 0.85)?;
    mouse(BUTTON_RELEASE_EVENT, 1, 0., 0.)?;
    screenshot("crop-resized.png")?;
    click(&g["save"])?;
    wait(|| Ok(screen::game_crop(&store, "Faxanadu")?.width < 0.45))?;
    crop = screen::game_crop(&store, "Faxanadu")?;
    ensure!(
        (crop.x - 0.3).abs() < 0.005
            && (crop.width - 0.4).abs() < 0.005
            && (crop.height - 0.7).abs() < 0.005,
        "Corner resizing produced the wrong crop: {crop:?}"
    );
    // Reopen, reset the draft, cancel: saved preference must survive.
    click(&serde_json::json!([422, 358]))?;
    thread::sleep(Duration::from_millis(400));
    let g = geometry()?;
    click(&g["reset"])?;
    click(&g["cancel"])?;
    ensure!(
        screen::game_crop(&store, "Faxanadu")? == crop,
        "Cancel changed the saved crop"
    );
    click(&serde_json::json!([422, 358]))?;
    thread::sleep(Duration::from_millis(400));
    let g = geometry()?;
    screenshot("crop-reopened.png")?;
    click(&g["reset"])?;
    click(&g["save"])?;
    wait(|| Ok(screen::game_crop(&store, "Faxanadu")? == Crop::default()))?;
    wait(|| Ok(screen::snapshot(&store).is_ok_and(|s| s.metadata["width"] == 1280)))?;
    toggle()?;
    ensure!(
        cardinal(conn, overlay, focus)? == 0,
        "Closing crop did not restore game focus"
    );
    println!(
        "GAMEBUDDY_CROP_TEST_PASS selection coordinates, move, corner resize, cropped native image, persistence, cancel, reset, game focus"
    );
    Ok(())
}
