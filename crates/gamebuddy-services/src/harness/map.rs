use super::*;
use base64::Engine;
use gamebuddy_core::Crop;

pub(super) fn read(
    store: &Store,
    guide: &Guide,
    state: &Value,
    requested: Option<&str>,
) -> Result<ToolOutput> {
    let atlas = crate::navigation::atlas(store, guide)?;
    let selected = requested.or_else(|| state["selected_map_id"].as_str());
    let map = atlas
        .maps
        .iter()
        .find(|m| m.enabled && Some(m.id.as_str()) == selected)
        .context("No current map is selected; choose an enabled map_id from get_atlas")?;
    let source = image::open(crate::navigation::cached_map_path(store, map))?;
    let marker = &state["geometric_marker"];
    let region = (marker["map_id"] == map.id)
        .then(|| serde_json::from_value::<Crop>(marker["region"].clone()).ok())
        .flatten();
    let crop = region
        .map(|r| {
            let x = (r.x - r.width * 0.75).max(0.);
            let y = (r.y - r.height * 0.75).max(0.);
            Crop {
                x,
                y,
                width: (r.width * 2.5).min(1. - x),
                height: (r.height * 2.5).min(1. - y),
            }
        })
        .unwrap_or_default();
    let mut image = crop
        .apply(&source)?
        .resize(1280, 960, image::imageops::FilterType::Triangle)
        .to_rgb8();
    if let Some(region) = region {
        let x = |v: f32| {
            (((v - crop.x) / crop.width * image.width() as f32).round() as u32)
                .min(image.width() - 1)
        };
        let y = |v: f32| {
            (((v - crop.y) / crop.height * image.height() as f32).round() as u32)
                .min(image.height() - 1)
        };
        let (left, right, top, bottom) = (
            x(region.x),
            x(region.x + region.width),
            y(region.y),
            y(region.y + region.height),
        );
        for v in top..=bottom {
            for u in left..=right {
                if u - left < 3 || right - u < 3 || v - top < 3 || bottom - v < 3 {
                    image.put_pixel(u, v, image::Rgb([255, 70, 70]));
                }
            }
        }
    }
    let mut bytes = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(image).write_to(&mut bytes, image::ImageFormat::Jpeg)?;
    Ok(ToolOutput {
        value: json!({"map_id":map.id,"title":map.title,"credit":map.credit,"source_url":map.source_url,
            "image_scope":if region.is_some(){"neighborhood_of_current_camera"}else{"whole_map"},"crop":crop,
            "camera_marker":if region.is_some(){marker.clone()}else{Value::Null},
            "instruction":if region.is_some(){"The red box marks the geometrically matched visible camera area, not the player's exact position. The separate game screenshot shows the player and immediate surroundings."}else{"This is an atlas reference image. The current player location has not been geometrically verified on it; no live marker is drawn."}}),
        images: vec![base64::engine::general_purpose::STANDARD.encode(bytes.into_inner())],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn named_area_map_is_prefetched_without_claiming_a_player_position() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let store = Store::open(dir.path())?;
        let guide =
            gamebuddy_core::import_markdown("Game", "Guide", "# Begin\nTalk to the guard.")?;
        store.save_guide(&guide)?;
        store.put("active_guide", &guide.id)?;
        let map = gamebuddy_core::MapAsset {
            id: "town-map".into(),
            title: "North Town".into(),
            url: "https://example.org/town.png".into(),
            source_url: "https://example.org/atlas".into(),
            credit: "Fixture".into(),
            enabled: true,
        };
        store.put(
            &format!("atlas:{}", guide.id),
            &gamebuddy_core::Atlas {
                guide_id: guide.id.clone(),
                guide_revision: guide.revision,
                maps: vec![map.clone()],
                ..Default::default()
            },
        )?;
        std::fs::create_dir_all(store.root.join("maps"))?;
        image::RgbImage::from_pixel(200, 200, image::Rgb([0, 180, 60]))
            .save(crate::navigation::cached_map_path(&store, &map))?;
        let path = dir.path().join("screen.png");
        image::RgbImage::from_pixel(96, 64, image::Rgb([200, 20, 30])).save(&path)?;
        let ticket = crate::screen::start(&store, &guide.game, true)?;
        crate::screen::publish(&store, &ticket, &path, "fixture")?;
        let context = crate::harness::context_for_request(
            &store,
            "I'm in North Town. There's no danger here.",
        )?;
        assert_eq!(context.images.len(), 2);
        assert_eq!(context.value["requested_map_image"]["map_id"], "town-map");
        assert_eq!(context.value["requested_map_image"]["image_index"], 1);
        assert!(context.value["requested_map_image"]["camera_marker"].is_null());
        assert!(context.value["navigation"]["map"]["current_place"].is_null());
        assert_eq!(
            context.screen_frame().unwrap().jpeg,
            crate::screen::snapshot(&store)?.jpeg
        );
        assert_eq!(
            crate::harness::context_for_request(&store, "I'm in North Township")?
                .images
                .len(),
            1
        );
        Ok(())
    }
    #[test]
    fn actual_map_pixels_are_available_with_or_without_a_live_match() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let store = Store::open(dir.path())?;
        let guide = gamebuddy_core::import_markdown("Test", "Guide", "# A")?;
        let map = gamebuddy_core::MapAsset {
            id: "map".into(),
            title: "Town".into(),
            url: "https://example.org/town.png".into(),
            source_url: "https://example.org/atlas".into(),
            credit: "Artist".into(),
            enabled: true,
        };
        store.put(
            &format!("atlas:{}", guide.id),
            &gamebuddy_core::Atlas {
                guide_id: guide.id.clone(),
                guide_revision: guide.revision,
                maps: vec![map.clone()],
                ..Default::default()
            },
        )?;
        std::fs::create_dir_all(store.root.join("maps"))?;
        image::RgbImage::from_pixel(400, 200, image::Rgb([0, 180, 60]))
            .save(crate::navigation::cached_map_path(&store, &map))?;
        let known = read(
            &store,
            &guide,
            &json!({"selected_map_id":"map","geometric_marker":{"map_id":"map","region":{"x":0.4,"y":0.2,"width":0.2,"height":0.5}}}),
            None,
        )?;
        assert_eq!(known.images.len(), 1);
        assert_eq!(known.value["camera_marker"]["map_id"], "map");
        let unknown = read(&store, &guide, &json!({}), Some("map"))?;
        assert!(unknown.value["camera_marker"].is_null());
        assert_eq!(unknown.value["image_scope"], "whole_map");
        assert_eq!(unknown.images.len(), 1);
        assert!(read(&store, &guide, &json!({}), Some("missing")).is_err());
        store.save_guide(&guide)?;
        store.put("active_guide", &guide.id)?;
        struct LocalMapReader(std::sync::atomic::AtomicBool);
        impl ReasoningProvider for LocalMapReader {
            fn complete(&self, request: ModelRequest) -> Result<String> {
                if !self.0.swap(true, std::sync::atomic::Ordering::SeqCst) {
                    return Ok(
                        json!({"tool":"get_location_map","arguments":{"map_id":"map"}}).to_string(),
                    );
                }
                assert_eq!(request.images.len(), 1);
                let pixels = image::load_from_memory(
                    &base64::engine::general_purpose::STANDARD.decode(&request.images[0])?,
                )?
                .to_rgb8();
                let center = pixels.get_pixel(pixels.width() / 2, pixels.height() / 2);
                assert!(center[1] > 150 && center[0] < 20);
                assert!(request.prompt.contains("whole_map"));
                Ok(json!({"answer":"The map is green."}).to_string())
            }
        }
        assert_eq!(
            coach(
                &LocalMapReader(Default::default()),
                &store,
                "What color is the map?"
            )?,
            "The map is green."
        );
        Ok(())
    }
}
