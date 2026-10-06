use super::*;
use gamebuddy_core::import_markdown;

#[test]
fn journal_retains_original_images_search_pins_and_run_boundaries() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let store = Store::open(dir.path())?;
    let guide = import_markdown("Game", "Guide", "# Shop\nBuy boots.")?;
    store.save_guide(&guide)?;
    store.put("active_guide", &guide.id)?;
    let ticket = screen::start(&store, "Game", true)?;
    let path = dir.path().join("frame.jpg");
    image::RgbImage::from_pixel(64, 64, image::Rgb([20, 180, 90])).save(&path)?;
    screen::publish(&store, &ticket, &path, "game_window")?;
    store.report_complete(&guide, 0, "step-1", "I bought replacement boots")?;
    store.journal_sync(&guide)?;
    store.journal_sync(&guide)?;
    let entries = read(&store, &guide, &json!({"query":"replacement"}))?;
    let entry = entries["entries"][0]["id"].as_i64().unwrap();
    assert_eq!(entries["entries"].as_array().unwrap().len(), 1);
    assert_eq!(entries["entries"][0]["evidence_kind"], "player_report");
    assert_eq!(entries["entries"][0]["image_count"], 1);
    let pixels = image(&store, &guide, entry, 0)?;
    image::RgbImage::from_pixel(64, 64, image::Rgb([180, 20, 90])).save(&path)?;
    screen::publish(&store, &ticket, &path, "game_window")?;
    assert_eq!(
        image(&store, &guide, entry, 0)?,
        pixels,
        "future captures cannot overwrite historical evidence"
    );
    let note: Note = serde_json::from_value(
        json!({"title":"Locked tower","body":"The inscription says 0417. Hypothesis: this is the door code.","category":"clue","capture_screenshot":true,"pinned":true}),
    )?;
    record(&store, &guide, note, "agent_note")?;
    let found = read(&store, &guide, &json!({"query":"0417","pinned_only":true}))?;
    assert_eq!(found["entries"].as_array().unwrap().len(), 1);
    let old = store.journal_active(&guide)?;
    let new = store.journal_start(&guide, "Second playthrough")?;
    assert_ne!(old, new);
    assert_eq!(read(&store, &guide, &json!({}))?["entries"], json!([]));
    assert_eq!(
        read(&store, &guide, &json!({"run_id":old}))?["entries"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    drop(store);
    let store = Store::open(dir.path())?;
    assert_eq!(store.journal_active(&guide)?, new);
    assert_eq!(image(&store, &guide, entry, 0)?, pixels);
    let other = import_markdown("Other game", "Guide", "# Other")?;
    store.save_guide(&other)?;
    assert!(image(&store, &other, entry, 0).is_err());
    assert!(store.journal_pin(&other, entry, true).is_err());
    assert!(read(&store, &other, &json!({"run_id":old})).is_err());
    Ok(())
}

#[test]
fn backfill_does_not_attach_todays_capture_to_yesterdays_report() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let store = Store::open(dir.path())?;
    let guide = import_markdown("Game", "Guide", "# Gate\nOpen the gate.")?;
    store.save_guide(&guide)?;
    store.report_complete(&guide, 0, "step-1", "I opened the gate")?;
    let ticket = screen::start(&store, "Game", true)?;
    let path = dir.path().join("today.jpg");
    image::RgbImage::new(64, 64).save(&path)?;
    screen::publish(&store, &ticket, &path, "game_window")?;
    let records = read(&store, &guide, &json!({}))?;
    assert_eq!(records["entries"][0]["image_count"], 0);
    let before = store.progress(&guide)?;
    let note: Note = serde_json::from_value(
        json!({"title":"Reminder","body":"Check the eastern room later.","category":"note"}),
    )?;
    record(&store, &guide, note, "agent_note")?;
    assert_eq!(
        json!(store.progress(&guide)?),
        json!(before),
        "a journal plan is never completion evidence"
    );
    Ok(())
}
