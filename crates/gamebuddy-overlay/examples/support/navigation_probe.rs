//! Exercise the real HTML map menus and shared voice/MCP location state inside
//! the private Gamescope fixture. Generated maps are test artwork, not game data.
use anyhow::{Context, Result, ensure};
use gamebuddy_core::{Atlas, Crop, MapAsset, Place, PlaceBinding, Progress};
use gamebuddy_services::{Store, harness, navigation};
use serde_json::{Value, json};
use std::{
    thread,
    time::{Duration, Instant},
};
use x11rb::{
    connection::Connection,
    protocol::{xproto::*, xtest::ConnectionExt as _},
    rust_connection::RustConnection,
};

pub fn prepare(store: &Store) -> Result<()> {
    let mut guide = gamebuddy_core::import_markdown(
        "Faxanadu",
        "Navigation fixture",
        "# Church\nTalk to the guru.\n# Shop\nBuy the key.\n# Forest\nEnter the forest.",
    )?;
    let mut video = guide.sources[0].clone();
    video.id = "fixture-video".into();
    video.title = "Fixture video walkthrough".into();
    video.kind = gamebuddy_core::SourceKind::Youtube;
    video.url = Some("https://www.youtube.com/watch?v=abcdefghijk".into());
    video.duration_ms = Some(300_000);
    guide.sources.push(video);
    guide.steps[2].anchors = vec![gamebuddy_core::Anchor {
        source_id: "fixture-video".into(),
        start_ms: Some(120_000),
        end_ms: Some(130_000),
        section: Some("Forest route".into()),
    }];
    store.save_guide(&guide)?;
    store.put("active_guide", &guide.id)?;
    store.save_progress(&guide, &Progress::default(), "fixture")?;
    let mut atlas = Atlas {
        guide_id: guide.id.clone(),
        guide_revision: guide.revision,
        status: "ready".into(),
        ..Default::default()
    };
    std::fs::create_dir_all(store.root.join("maps"))?;
    for (i, name) in ["church", "forest"].iter().enumerate() {
        let map = MapAsset {
            id: name.to_string(),
            title: format!("{name} area"),
            url: format!("https://example.org/{name}.jpg"),
            source_url: "https://example.org/maps".into(),
            credit: "Generated test fixture".into(),
            enabled: true,
        };
        let picture = image::RgbImage::from_fn(600, 300, |x, y| {
            if (80..240).contains(&x) && (70..210).contains(&y) {
                image::Rgb([180, 90, 60])
            } else if i == 0 {
                image::Rgb([35, 60, 100])
            } else {
                image::Rgb([30, 100, 55])
            }
        });
        picture.save(navigation::cached_map_path(store, &map))?;
        atlas.maps.push(map);
    }
    for (i, name) in ["church", "shop", "forest"].iter().enumerate() {
        let map = if i == 2 { "forest" } else { "church" };
        atlas.places.push(Place {
            id: name.to_string(),
            label: name.to_string(),
            map_id: Some(map.into()),
            landmarks: format!("Source-backed {name} fixture"),
        });
        atlas.bindings.push(PlaceBinding {
            step_id: guide.steps[i].id.clone(),
            place_id: name.to_string(),
            evidence: "Fixture source".into(),
        });
        store.put(
            &format!("map-placement:{}:{}:{name}", guide.id, guide.revision),
            &navigation::placement::Placement {
                updated_at_ms: gamebuddy_services::screen::now_ms(),
                status: "ready".into(),
                detail: "Controlled marker".into(),
                marker: Some(navigation::placement::Marker {
                    place_id: name.to_string(),
                    map_id: map.into(),
                    region: Crop {
                        x: 0.12,
                        y: 0.22,
                        width: 0.3,
                        height: 0.5,
                    },
                    evidence: "Generated brick rectangle".into(),
                    confidence: 1.0,
                }),
            },
        )?;
    }
    store.put(&format!("atlas:{}", guide.id), &atlas)?;
    store.put("widget-layout",&json!({"location-map":{"x":250,"y":20,"width":600,"height":670},"walkthrough-outline":{"x":870,"y":20,"width":380,"height":550},"provenance-table":{"x":870,"y":20,"width":380,"height":350}}))?;
    store.put(
        &format!("widget-visibility:{}", guide.id),
        &json!({"current-step":false}),
    )?;
    // No real provider is needed for this deterministic UI/state test.
    let mut settings = gamebuddy_services::Settings::default();
    settings.vision.endpoint = "http://127.0.0.1:9".into();
    store.put("settings", &settings)?;
    Ok(())
}

fn wait(mut test: impl FnMut() -> Result<bool>) -> Result<()> {
    let until = Instant::now() + Duration::from_secs(20);
    while !test()? {
        ensure!(Instant::now() < until, "Navigation assertion timed out");
        thread::sleep(Duration::from_millis(100));
    }
    Ok(())
}
fn latest(log: &str, kind: &str) -> Value {
    std::fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .rev()
        .find_map(|line| {
            line.split_once(kind)
                .and_then(|(_, json)| serde_json::from_str(json).ok())
        })
        .unwrap_or(Value::Null)
}
pub fn run(
    conn: &RustConnection,
    root: u32,
    screenshot: impl Fn(&str) -> Result<()>,
) -> Result<()> {
    let store = Store::open(std::env::var("GAMEBUDDY_TEST_DATA")?)?;
    let id: String = store.get("active_guide")?.context("Guide")?;
    let guide = store.guide(&id)?;
    let log = std::env::var("GAMEBUDDY_TEST_LOG")?;
    let report = |place: &str, step: &str| -> Result<()> {
        let p = store.progress(&guide)?;
        harness::call_tool(
            &store,
            "report_location",
            json!({"place_id":place,"step_id":step,"player_report":format!("I am at the {place}"),"expected_revision":guide.revision,"epoch":p.epoch}),
        )?;
        Ok(())
    };
    let click = |rect: &Value| -> Result<()> {
        let r = rect.as_array().context("Control bounds")?;
        let x = (r[0].as_f64().unwrap() + r[2].as_f64().unwrap() / 2.0) as i16;
        let y = (r[1].as_f64().unwrap() + r[3].as_f64().unwrap() / 2.0) as i16;
        conn.xtest_fake_input(MOTION_NOTIFY_EVENT, 0, 0, root, x, y, 0)?
            .check()?;
        conn.flush()?;
        thread::sleep(Duration::from_millis(350));
        conn.xtest_fake_input(BUTTON_PRESS_EVENT, 1, 0, root, 0, 0, 0)?
            .check()?;
        conn.xtest_fake_input(BUTTON_RELEASE_EVENT, 1, 0, root, 0, 0, 0)?
            .check()?;
        conn.flush()?;
        Ok(())
    };
    let hover = |rect: &Value| -> Result<()> {
        let r = rect.as_array().context("Hover target bounds")?;
        let x = (r[0].as_f64().unwrap() + r[2].as_f64().unwrap() / 2.0) as i16;
        let y = (r[1].as_f64().unwrap() + r[3].as_f64().unwrap() / 2.0) as i16;
        // Continue motion through Gamescope's asynchronous focus transfer. A
        // single synthetic warp can be consumed by the game's input surface.
        for offset in 0..12 {
            conn.xtest_fake_input(MOTION_NOTIFY_EVENT, 0, 0, root, x + offset % 2, y, 0)?
                .check()?;
            conn.flush()?;
            thread::sleep(Duration::from_millis(100));
        }
        thread::sleep(Duration::from_millis(400));
        Ok(())
    };
    wait(
        || Ok(store.get::<String>("game-guide:faxanadu:")?.as_deref() == Some(guide.id.as_str())),
    )?;
    report("church", "step-1")?;
    wait(|| {
        let v = latest(&log, "GAMEBUDDY_MAP_VIEW ");
        Ok(v["place"] == "church" && v["map"] == "church" && v["marker"] == true)
    })?;
    wait(|| Ok(latest(&log, "GAMEBUDDY_OUTLINE ")["current"] == "step-1"))?;
    thread::sleep(Duration::from_secs(1));
    screenshot("navigation-church.png")?;
    hover(&latest(&log, "GAMEBUDDY_MAP_VIEW ")["credit"])?;
    wait(|| {
        Ok(
            latest(&log, "GAMEBUDDY_PROVENANCE ")["entries"][0]["detail"]
                == "Reference map · Generated test fixture",
        )
    })?;
    screenshot("navigation-map-provenance.png")?;
    hover(&json!([900, 600, 1, 1]))?;
    click(&latest(&log, "GAMEBUDDY_MAP_VIEW ")["mapPicker"])?;
    wait(|| Ok(latest(&log, "GAMEBUDDY_PICKER ")["label"] == "Location map"))?;
    report("forest", "step-3")?;
    wait(|| Ok(latest(&log, "GAMEBUDDY_OUTLINE ")["current"] == "step-3"))?;
    thread::sleep(Duration::from_secs(3));
    screenshot("navigation-menu-survives-update.png")?;
    let menu = latest(&log, "GAMEBUDDY_PICKER ");
    let option = menu["options"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["id"] == "place:church")
        .unwrap();
    click(&option["rect"])?;
    wait(|| {
        let v = latest(&log, "GAMEBUDDY_MAP_VIEW ");
        Ok(v["place"] == "forest"
            && v["map"] == "church"
            && v["following"] == false
            && v["marker"] == false)
    })?;
    click(&latest(&log, "GAMEBUDDY_MAP_VIEW ")["followButton"])?;
    wait(|| {
        let v = latest(&log, "GAMEBUDDY_MAP_VIEW ");
        Ok(v["map"] == "forest" && v["following"] == true && v["marker"] == true)
    })?;
    thread::sleep(Duration::from_secs(1));
    screenshot("navigation-forest.png")?;
    let outline = latest(&log, "GAMEBUDDY_OUTLINE ");
    hover(
        &outline["rows"]
            .as_array()
            .context("Outline rows")?
            .iter()
            .find(|r| r["id"] == "step-3")
            .context("Forest row")?["rect"],
    )?;
    wait(|| {
        Ok(latest(&log, "GAMEBUDDY_PROVENANCE ")["entries"][0]["url"]
            == "https://www.youtube.com/watch?v=abcdefghijk&t=120s")
    })?;
    screenshot("navigation-video-provenance.png")?;
    hover(&json!([900, 600, 1, 1]))?;
    click(&latest(&log, "GAMEBUDDY_MAP_VIEW ")["placePicker"])?;
    wait(|| Ok(latest(&log, "GAMEBUDDY_PICKER ")["label"] == "Browse known places"))?;
    screenshot("navigation-places-menu.png")?;
    let menu = latest(&log, "GAMEBUDDY_PICKER ");
    click(
        &menu["options"]
            .as_array()
            .unwrap()
            .iter()
            .find(|o| o["id"] == "church")
            .unwrap()["rect"],
    )?;
    wait(|| {
        let v = latest(&log, "GAMEBUDDY_MAP_VIEW ");
        Ok(v["map"] == "church" && v["place"] == "forest" && v["following"] == false)
    })?;
    click(&latest(&log, "GAMEBUDDY_MAP_VIEW ")["followButton"])?;
    report("church", "step-1")?;
    let p = store.progress(&guide)?;
    store.report_complete(&guide, p.epoch, "step-1", "I spoke to the guru")?;
    wait(|| Ok(latest(&log, "GAMEBUDDY_OUTLINE ")["current"] == "step-2"))?;
    wait(|| Ok(latest(&log, "GAMEBUDDY_MAP_VIEW ")["place"] == "church"))?;
    screenshot("navigation-next-step.png")?;
    hover(&json!([900, 600, 1, 1]))?;
    store.put(
        &format!("widget-visibility:{}", guide.id),
        &json!({"current-step":false,"walkthrough-outline":false}),
    )?;
    harness::call_tool(
        &store,
        "show_widget",
        json!({"widget":{
            "id":"provenance-table","title":"Mixed references",
            "content":{"kind":"table","columns":["Action"],"rows":[["Forest route"],["My own goal"]]},
            "source_step_ids":["step-1"],
            "item_sources":[{"item_index":0,"source_step_ids":["step-3"]},{"item_index":1,"source_step_ids":[]}]
        }}),
    )?;
    wait(|| Ok(latest(&log, "GAMEBUDDY_WIDGET_SOURCES ")["id"] == "provenance-table"))?;
    let rows = latest(&log, "GAMEBUDDY_WIDGET_SOURCES ")["rows"].clone();
    hover(&rows[0])?;
    wait(|| {
        Ok(
            latest(&log, "GAMEBUDDY_PROVENANCE ")["title"] == "Row sources"
                && latest(&log, "GAMEBUDDY_PROVENANCE ")["entries"][0]["title"]
                    == "Fixture video walkthrough",
        )
    })?;
    screenshot("navigation-widget-row-provenance.png")?;
    hover(&json!([900, 600, 1, 1]))?;
    hover(&rows[1])?;
    wait(|| {
        Ok(latest(&log, "GAMEBUDDY_PROVENANCE ")["entries"][0]["title"] == "No reference attached")
    })?;
    screenshot("navigation-widget-uncited-row.png")?;
    println!(
        "GAMEBUDDY_NAVIGATION_TEST_PASS reported location, stable dropdown through refresh, manual map browsing, Follow, cached map images, area marker, room menu, current-step outline, completion advances without losing room, automatic cached-guide preparation, map and timestamped outline tooltips, per-row citations and uncited player input"
    );
    Ok(())
}
