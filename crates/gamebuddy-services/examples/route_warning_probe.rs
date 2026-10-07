//! Replay a source route position in a new private store. This establishes
//! warning selection, not that the player is physically at the supplied step.
use anyhow::{Context, Result, ensure};
use gamebuddy_core::Progress;
use gamebuddy_services::{Store, harness, navigation, screen, voice::narration::Narrator};
use serde_json::json;
use std::{path::PathBuf, time::Instant};

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let source = Store::open(args.next().context("Private source fixture")?)?;
    ensure!(source.root.starts_with("/tmp"), "Use a private source copy");
    let root = PathBuf::from(args.next().context("New private output directory")?);
    ensure!(
        root.starts_with("/tmp") && !root.exists(),
        "Use a new private directory"
    );
    let id: String = source.get("active_guide")?.context("Active guide")?;
    let guide = source.guide(&id)?;
    let step = guide
        .step(&args.next().context("Source entrance step ID")?)
        .context("Source step")?;
    let store = Store::open(root)?;
    store.save_guide(&guide)?;
    store.put("active_guide", &id)?;
    let progress = Progress {
        current_step: Some(step.id.clone()),
        ..Default::default()
    };
    store.save_progress(&guide, &progress, "private_route_replay")?;
    let ticket = screen::start(&store, &guide.game, true)?;
    screen::publish(
        &store,
        &ticket,
        &source.root.join("saved-screen.jpg"),
        "recorded_replay",
    )?;
    // Seed only the route position being tested. No completion or inventory is
    // inferred from the independently recorded screenshot.
    store.put(
        &format!("next-action:{id}"),
        &json!({"revision":guide.revision,"epoch":progress.epoch,
        "session":ticket.session,"updated_at_ms":screen::now_ms(),"step_id":step.id,
        "instructions":step.instructions,"location_evidence":{"kind":"private_route_fixture"}}),
    )?;
    let started = Instant::now();
    let mut nav = navigation::context(&store, &guide, &progress, &Default::default())?;
    ensure!(
        nav["guidance"]["now"]["id"] == step.id,
        "Route fixture was redirected"
    );
    ensure!(
        nav["guidance"]["warning"].is_null(),
        "An unlocated route produced a warning"
    );
    // Explicit synthetic proximity for this source-route replay. The recorded
    // screenshot is not evidence that the real player reached this entrance.
    let mut atlas = navigation::atlas(&source, &guide)?;
    let place = atlas
        .place_for_step(&guide, &step.id)
        .map(|p| p.id.clone())
        .unwrap_or_else(|| {
            atlas.places.push(gamebuddy_core::Place {
                id: "fixture-entrance".into(),
                label: "Replay entrance".into(),
                map_id: None,
                landmarks: String::new(),
            });
            atlas.bindings.push(gamebuddy_core::PlaceBinding {
                step_id: step.id.clone(),
                place_id: "fixture-entrance".into(),
                evidence: "Explicit replay assumption".into(),
            });
            "fixture-entrance".into()
        });
    store.put(&format!("atlas:{id}"), &atlas)?;
    let mut location = gamebuddy_core::PlaceTracker::for_session(ticket.session.clone());
    location.report_place(
        &place,
        progress.epoch,
        screen::now_ms(),
        "Synthetic source entrance for replay",
    );
    store.put(&navigation::tracker_key(&guide), &location)?;
    nav = navigation::context(&store, &guide, &progress, &Default::default())?;
    let mut context = json!({"game":guide.game,"guide_id":guide.id,"screen":screen::status(&store)?,
        "guidance":nav["guidance"],"player_speaking":false,"conversation_active":true});
    let mut narrator = Narrator::default();
    ensure!(
        narrator.next_cue(&context, Instant::now()).is_none(),
        "Warning interrupted a player request"
    );
    context["conversation_active"] = json!(false);
    let cue = narrator
        .next_cue(&context, Instant::now() + std::time::Duration::from_secs(1))
        .context("No warning selected")?;
    ensure!(
        cue["urgent"] == false,
        "Source preview was treated as an observed emergency"
    );
    println!(
        "{}",
        json!({"elapsed_ms":started.elapsed().as_millis(),"warning":nav["guidance"]["warning"],"cue":cue})
    );
    nav["player_steps"] = json!(gamebuddy_core::player_outline(&guide));
    let state = json!({"game":guide.game,"guide":guide,"progress":progress,"navigation":nav,
        "screen":screen::status(&store)?,"widgets":harness::widgets(&store, &guide)?,"layouts":{}});
    std::fs::write(
        store.root.join("state.json"),
        serde_json::to_vec_pretty(&state)?,
    )?;
    ensure!(
        store.progress(&guide)?.completed.is_empty(),
        "Warning changed completion"
    );
    Ok(())
}
