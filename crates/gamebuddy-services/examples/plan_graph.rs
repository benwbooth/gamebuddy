//! Print the plan graph's areas and crossings: plan_graph <data directory>
fn main() -> anyhow::Result<()> {
    use anyhow::Context;
    let store =
        gamebuddy_services::Store::open(std::env::args().nth(1).context("Data directory")?)?;
    let id: String = store.get("active_guide")?.context("No active guide")?;
    let guide = store.guide(&id)?;
    let atlas = gamebuddy_services::navigation::atlas(&store, &guide)?;
    let model = gamebuddy_services::outcomes::prepared(&store, &guide)?
        .map(|p| p.model.clone())
        .unwrap_or_default();
    let graph = gamebuddy_core::planner::PlanGraph::build(&guide, &atlas, &model);
    for c in &graph.crossings {
        println!(
            "{} -> {} needs {:?} via {} / {}",
            c.from, c.to, c.requires, c.exit_step, c.step
        );
    }
    for o in graph.objectives.iter().filter(|o| !o.optional) {
        println!(
            "OBJ {} [{}] {:?} req {:?} prod {:?}",
            o.id,
            o.area.as_deref().unwrap_or("-"),
            o.title,
            o.requires,
            o.produces
        );
    }
    Ok(())
}
