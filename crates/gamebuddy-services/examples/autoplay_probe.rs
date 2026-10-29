//! A first player in plain code: look at the screen, find it on the area map,
//! take the next move of the room graph's route and turn it into held
//! buttons, a few times a second, until the player reaches the next area.
//! Runs against a RetroArch with its command port (screenshots) and network
//! gamepad on; for practice, a hidden one loaded from a save state:
//! autoplay_probe <data dir> <screenshot dir> <toward map title> [seconds]
//! AUTOPLAY_FRAMES=<dir> keeps every frame it looked at.
use anyhow::{Context, Result};
use gamebuddy_core::navigation::direction::ScreenDirection;
use gamebuddy_services::{
    navigation::{atlas, room_graph},
    retroarch::{Button, hold},
};
use std::{net::UdpSocket, path::Path, time::Duration};

const COMMAND_PORT: u16 = 55394;

/// The newest screenshot RetroArch writes after a SCREENSHOT command.
fn screenshot(dir: &Path) -> Result<image::DynamicImage> {
    let newest = || {
        std::fs::read_dir(dir)
            .ok()?
            .flatten()
            .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
            .max()
    };
    let before = newest();
    UdpSocket::bind("127.0.0.1:0")?.send_to(b"SCREENSHOT", ("127.0.0.1", COMMAND_PORT))?;
    for _ in 0..40 {
        std::thread::sleep(Duration::from_millis(25));
        if let Some((_, path)) = newest().filter(|n| Some(n) != before.as_ref())
            && let Ok(image) = image::open(&path)
        {
            return Ok(image);
        }
    }
    anyhow::bail!("No screenshot")
}

fn press(held: &[Button], ms: u64) -> Result<()> {
    hold(held)?;
    std::thread::sleep(Duration::from_millis(ms));
    hold(&[])
}

/// Three quick sword swings facing the way the hero last walked.
fn swing() -> Result<()> {
    for _ in 0..3 {
        press(&[Button::B], 60)?;
        std::thread::sleep(Duration::from_millis(100));
    }
    Ok(())
}

fn main() -> Result<()> {
    let a: Vec<String> = std::env::args().collect();
    let store = gamebuddy_services::Store::open(a.get(1).context("Data directory")?)?;
    let shots = Path::new(a.get(2).context("Screenshot directory")?);
    let toward_title = a.get(3).context("Toward map title")?;
    let seconds: u64 = a.get(4).map_or(Ok(120), |s| s.parse())?;
    let id: String = store.get("active_guide")?.context("No active guide")?;
    let guide = store.guide(&id)?;
    let atlas = atlas(&store, &guide)?;
    let toward = atlas
        .maps
        .iter()
        .find(|m| m.title == *toward_title)
        .context("Unknown map")?
        .id
        .clone();
    let world = gamebuddy_services::spatial::load_cached(&store, &guide)?.context("No world")?;
    let started = std::time::Instant::now();
    // Which way to sweep a screen looking for a ladder, and how long the
    // current screen has been tried.
    let mut sweep = ScreenDirection::Right;
    let mut last: Option<(String, i32, i32)> = None;
    let mut tries = 0;
    let mut previous: Option<Vec<u8>> = None;
    while started.elapsed().as_secs() < seconds {
        let frame = screenshot(shots)?;
        if let Ok(dir) = std::env::var("AUTOPLAY_FRAMES") {
            frame
                .save(Path::new(&dir).join(format!("{:06}.png", started.elapsed().as_millis())))?;
        }
        // Buttons held and nothing on screen moved: a message box is
        // waiting for a press.
        let pixels = frame.to_rgb8().into_raw();
        let frozen = previous.as_ref().is_some_and(|p: &Vec<u8>| {
            p.len() == pixels.len()
                && p.chunks(3)
                    .zip(pixels.chunks(3))
                    .filter(|(a, b)| a != b)
                    .count()
                    < 40
        });
        previous = Some(pixels);
        if frozen {
            println!(
                "{:5.1}s  screen frozen; pressing A",
                started.elapsed().as_secs_f32()
            );
            press(&[Button::A], 100)?;
            press(&[], 150)?;
            continue;
        }
        let marker = gamebuddy_services::spatial_maps::locate(&store, &guide, &world, &frame)?;
        let Some(marker) = marker else {
            // A transition or a screen the matcher cannot place: keep going
            // the way we were.
            println!("{:5.1}s  unplaced", started.elapsed().as_secs_f32());
            press(&[], 150)?;
            continue;
        };
        let map = marker["map_id"].as_str().unwrap_or_default().to_owned();
        if map == toward {
            println!(
                "{:5.1}s  reached {toward_title}",
                started.elapsed().as_secs_f32()
            );
            hold(&[])?;
            return Ok(());
        }
        let r = &marker["region"];
        let at = [
            (r["x"].as_f64().unwrap_or(0.) + r["width"].as_f64().unwrap_or(0.) / 2.) as f32,
            (r["y"].as_f64().unwrap_or(0.) + r["height"].as_f64().unwrap_or(0.) / 2.) as f32,
        ];
        let here = (map.clone(), (at[0] * 100.) as i32, (at[1] * 100.) as i32);
        if last.as_ref() == Some(&here) {
            tries += 1;
        } else {
            tries = 0;
            last = Some(here);
        }
        let moves = room_graph::way_out(&store, &guide, &map, at, &toward)?;
        let first = moves.as_ref().and_then(|m| m.first().cloned());
        let direction = match &first {
            Some(m) => m.direction,
            // On the screen that leads out: the run left this map from
            // here; keep walking the way that got us here.
            None => sweep,
        };
        println!(
            "{:5.1}s  at ({:.2},{:.2}) next {:?} {}",
            started.elapsed().as_secs_f32(),
            at[0],
            at[1],
            direction,
            moves
                .as_ref()
                .map(|m| room_graph::words(m))
                .unwrap_or_else(|| "no route from here".into())
        );
        match direction {
            ScreenDirection::Left | ScreenDirection::Right => {
                sweep = direction;
                let button = if direction == ScreenDirection::Left {
                    Button::Left
                } else {
                    Button::Right
                };
                // Turn and swing a few times before each step: enemies walk
                // the same corridors, and walking into one costs more than
                // waiting for it.
                press(&[button], 30)?;
                swing()?;
                // Stuck on one screen: a ledge or a gap; jump while walking.
                if tries >= 4 && tries % 2 == 0 {
                    press(&[button, Button::A], 400)?;
                } else {
                    press(&[button], 250)?;
                }
            }
            ScreenDirection::Up | ScreenDirection::Down => {
                // The ladder's place on the screen is not known: walk a step,
                // then try the ladder; turn back after a while.
                if tries > 0 && tries % 8 == 0 {
                    sweep = if sweep == ScreenDirection::Left {
                        ScreenDirection::Right
                    } else {
                        ScreenDirection::Left
                    };
                }
                let walk = if sweep == ScreenDirection::Left {
                    Button::Left
                } else {
                    Button::Right
                };
                let climb = if direction == ScreenDirection::Up {
                    Button::Up
                } else {
                    Button::Down
                };
                press(&[walk], 180)?;
                swing()?;
                press(&[climb], 500)?;
            }
            _ => press(&[], 200)?,
        }
    }
    hold(&[])?;
    println!("gave up after {seconds} s");
    Ok(())
}
