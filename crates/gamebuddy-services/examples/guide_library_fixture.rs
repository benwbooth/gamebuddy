//! Isolated data for testing the actual Qt reader/SQLite bridge, without a model.
use anyhow::{Context, Result};
use gamebuddy_core::*;
use gamebuddy_services::{Store, navigation};
use serde_json::json;

fn main() -> Result<()> {
    let root = std::env::args()
        .nth(1)
        .context("temporary output directory required")?;
    let store = Store::open(root)?;
    let mut guide = import_markdown(
        "Reader fixture",
        "Reader integration fixture",
        "# Chapter\nExplore the town.\n## Church\nTalk to the guru.\n## Shop\nBuy the key.\n# Forest\nEnter the forest.",
    )?;
    guide.sources[0].kind = SourceKind::Youtube;
    guide.sources[0].url = Some("https://www.youtube.com/watch?v=abcdefghijk".into());
    guide.sources[0].duration_ms = Some(120_000);
    for (i, step) in guide.steps.iter_mut().enumerate() {
        step.anchors[0].start_ms = Some(if i == 0 { 0 } else { (i as u64 - 1) * 40_000 });
        step.anchors[0].end_ms = Some(if i == 0 { 80_000 } else { i as u64 * 40_000 });
    }
    guide.steps[3].prerequisites = vec!["step-3".into()];
    store.save_guide(&guide)?;
    let mut revised = guide.clone();
    revised.revision += 1;
    revised
        .coverage_notes
        .push("Test fixture: generated pictures, not real gameplay.".into());
    store.revise_guide(&revised, guide.revision, "Fixture source review")?;
    guide = revised;
    store.put("active_guide", &guide.id)?;
    let mut index = VisualIndex {
        guide_id: guide.id.clone(),
        guide_revision: guide.revision,
        encoder: "fixture".into(),
        crop: Crop::default(),
        live_crop: Crop::default(),
        frames: vec![],
    };
    for i in 0..12 {
        let path = store.root.join(format!("picture-{i}.jpg"));
        image::RgbImage::from_fn(320, 200, |x, y| {
            image::Rgb([((x + i * 20) % 255) as u8, (y % 255) as u8, 100])
        })
        .save(&path)?;
        index.frames.push(ReferenceFrame {
            id: format!("frame-{i}"),
            source_id: guide.sources[0].id.clone(),
            at_ms: u64::from(i) * 10_000,
            image_path: path.to_string_lossy().into(),
            step_ids: vec![format!("step-{}", 2 + i / 4)],
            fingerprint: vec![],
        });
    }
    store.save_index(&index)?;
    let map = MapAsset {
        id: "town".into(),
        title: "Town reference map".into(),
        url: "https://example.org/map.jpg".into(),
        source_url: "https://example.org/maps".into(),
        credit: "Generated fixture author".into(),
        enabled: true,
    };
    std::fs::create_dir_all(store.root.join("maps"))?;
    image::RgbImage::from_pixel(600, 300, image::Rgb([34, 110, 80]))
        .save(navigation::cached_map_path(&store, &map))?;
    let atlas = Atlas {
        guide_id: guide.id.clone(),
        guide_revision: guide.revision,
        maps: vec![map],
        places: vec![Place {
            id: "church".into(),
            label: "Town church".into(),
            map_id: Some("town".into()),
            landmarks: "Fixture entrance".into(),
        }],
        bindings: vec![PlaceBinding {
            step_id: "step-2".into(),
            place_id: "church".into(),
            evidence: "Fixture screenshot/source binding".into(),
        }],
        status: "ready".into(),
        ..Default::default()
    };
    store.put(&format!("atlas:{}", guide.id), &atlas)?;
    store.save_progress(
        &guide,
        &Progress {
            epoch: 1,
            current_step: Some("step-2".into()),
            ..Default::default()
        },
        "fixture",
    )?;
    store.put(&format!("widget-visibility:{}",guide.id),&json!({"guide-library":true,"walkthrough-outline":false,"location-map":false,"current-step":false}))?;
    let ticket = gamebuddy_services::screen::start(&store, &guide.game, true)?;
    gamebuddy_services::screen::publish(
        &store,
        &ticket,
        &store.root.join("picture-0.jpg"),
        "generated_test_frame",
    )?;
    gamebuddy_services::journal::record(
        &store,
        &guide,
        serde_json::from_value(
            json!({"title":"Tower inscription", "body":"The inscription reads 0417. Generated integration fixture, not real gameplay.", "category":"clue", "capture_screenshot":true}),
        )?,
        "agent_note",
    )?;
    gamebuddy_services::screen::set_enabled(&store, &ticket.session, &guide.game, false)?;
    store.put(&format!("widget-visibility:{}",guide.id),&json!({"guide-library":true,"run-journal":false,"walkthrough-outline":false,"location-map":false,"current-step":false}))?;
    store.put(
        "widget-layout",
        &json!({"guide-library":{"x":110,"y":15,"width":1100,"height":685}}),
    )?;
    let mut settings = gamebuddy_services::Settings::default();
    settings.vision.endpoint = "http://127.0.0.1:9".into();
    store.put("settings", &settings)?;
    println!("{}", guide.id);
    Ok(())
}
