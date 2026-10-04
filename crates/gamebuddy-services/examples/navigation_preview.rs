//! Render production OSD widgets with controlled location inputs for UI review.
//! Reads an app-built atlas; never authors game content or modifies live progress.
use anyhow::{Context, Result, ensure};
use gamebuddy_core::{Progress, SyncEstimate};
use gamebuddy_services::{Store, navigation};
use serde_json::json;
use std::{
    collections::HashSet,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let id = args.next().context("guide ID required")?;
    let output = PathBuf::from(args.next().context("output directory required")?);
    let source = Store::open(gamebuddy_services::data_dir()?)?;
    let guide = source.guide(&id)?;
    let atlas = navigation::atlas(&source, &guide)?;
    let expected = gamebuddy_services::walkthrough::ordering_graph(&guide);
    let expected_routes: HashSet<_> = expected["edges"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["kind"] == "route")
        .map(|e| {
            (
                e["from"].as_str().unwrap().to_owned(),
                e["to"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    let mut planner_routes = HashSet::new();
    for step in guide.steps.iter().filter(|s| {
        !guide
            .steps
            .iter()
            .any(|c| c.parent_id.as_deref() == Some(&s.id))
    }) {
        for id in gamebuddy_core::route_successors(&guide, step) {
            let mut target = guide.step(id).unwrap();
            while let Some(child) = guide
                .steps
                .iter()
                .find(|s| s.parent_id.as_deref() == Some(&target.id))
            {
                target = child;
            }
            if target.id != step.id {
                planner_routes.insert((step.id.clone(), target.id.clone()));
            }
        }
    }
    ensure!(
        planner_routes == expected_routes,
        "Planner routes differ from the exported walkthrough ordering graph"
    );
    let temporary = tempfile::tempdir()?;
    let store = Store::open(temporary.path())?;
    store.save_guide(&guide)?;
    store.put(&format!("atlas:{}", guide.id), &atlas)?;
    let mut scenarios = vec![];
    let mut map_ids = HashSet::new();
    let mut at = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as u64;
    for place in &atlas.places {
        let Some(map_id) = &place.map_id else {
            continue;
        };
        if !map_ids.insert(map_id) {
            continue;
        }
        let Some(binding) = atlas.bindings.iter().find(|b| b.place_id == place.id) else {
            continue;
        };
        at += 1;
        let estimate = SyncEstimate {
            observed_at_ms: at,
            status: "confirmed".into(),
            located_step: Some(binding.step_id.clone()),
            ..Default::default()
        };
        let progress = Progress {
            current_step: Some(binding.step_id.clone()),
            ..Default::default()
        };
        let navigation = navigation::context(&store, &guide, &progress, &estimate)?;
        scenarios.push(json!({"label":place.label,"game":guide.game,"guide":guide,"current":navigation["guidance"]["now"],"progress":progress,"estimate":estimate,"navigation":navigation,"locked":false}));
        if scenarios.len() == 4 {
            break;
        }
    }
    ensure!(
        !scenarios.is_empty(),
        "Wait for the application to build a map association"
    );
    let navigation = navigation::context(
        &store,
        &guide,
        &Progress::default(),
        &SyncEstimate::default(),
    )?;
    scenarios.push(json!({"label":"Uncertain location","game":guide.game,"guide":guide,"progress":Progress::default(),"navigation":navigation,"locked":false}));
    let data = serde_json::to_string(&scenarios)?.replace('<', "\\u003c");
    let script = format!(
        r#"<script>
const scenarios={data};const controls=document.createElement('nav');controls.style.cssText='position:fixed;bottom:0;left:0;right:0;z-index:999;background:#111d29;padding:8px;display:flex;gap:8px;font:12px sans-serif;color:white';controls.append(document.createTextNode('Controlled locations · UI test: '));
scenarios.forEach(state=>{{const b=document.createElement('button');b.textContent=state.label;b.onclick=()=>gamebuddy.render(state);controls.append(b)}});document.body.append(controls);gamebuddy.render(scenarios[0]);
</script>"#
    );
    let html = include_str!("../../gamebuddy-app/assets/widgets.html")
        .replace(
            "/* GUIDE_LIBRARY_SCRIPT */",
            include_str!("../../gamebuddy-app/assets/guide-library.js"),
        )
        .replace("</body>", &format!("{script}</body>"));
    std::fs::create_dir_all(&output)?;
    std::fs::write(output.join("index.html"), html)?;
    std::fs::write(
        output.join("atlas.json"),
        serde_json::to_vec_pretty(&atlas)?,
    )?;
    std::fs::write(
        output.join("validation.json"),
        serde_json::to_vec_pretty(
            &json!({"route_edges_checked":planner_routes.len(),"places":atlas.places.len(),"map_associations":atlas.map_links.iter().filter(|l|l.map_id.is_some()).count(),"scenario_count":scenarios.len(),"scope":"Controlled reference locations and graph consistency; independent gameplay untested"}),
        )?,
    )?;
    println!("{}", output.join("index.html").display());
    Ok(())
}
