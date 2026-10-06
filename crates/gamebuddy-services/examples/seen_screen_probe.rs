//! Exercise the same screenshot storage/bridge as the native UI, in a fixture.
use anyhow::{Context, Result, ensure};
use gamebuddy_services::{Store, screen};
use serde_json::json;

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let root = std::path::PathBuf::from(args.next().context("Private store")?);
    ensure!(root.starts_with("/tmp"), "Use a private fixture store");
    let frame = args.next().context("Recorded screenshot")?;
    let store = Store::open(root)?;
    let id: String = store.get("active_guide")?.context("Active guide")?;
    let guide = store.guide(&id)?;
    let ticket = screen::start(&store, &guide.game, true)?;
    screen::publish(
        &store,
        &ticket,
        std::path::Path::new(&frame),
        "recorded_fixture",
    )?;
    let output =
        gamebuddy_services::harness::call_tool_with_images(&store, "get_game_screen", json!({}))?;
    let shared = output.screen_frame().context("Attached screen")?;
    screen::seen::record(&store, &shared, "assistant")?;
    let status = screen::status(&store)?;
    println!(
        "{}",
        json!({"screen":status,"seen_screen":screen::seen::context(&store,&status)?})
    );
    Ok(())
}
