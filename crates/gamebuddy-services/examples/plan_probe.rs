//! Print the dependency plan for a profile: plan_probe <data directory>
fn main() -> anyhow::Result<()> {
    use anyhow::Context;
    let store =
        gamebuddy_services::Store::open(std::env::args().nth(1).context("Data directory")?)?;
    let id: String = store.get("active_guide")?.context("No active guide")?;
    let guide = store.guide(&id)?;
    let plan = gamebuddy_services::navigation::planning::context(&store, &guide)?;
    println!("{}", serde_json::to_string_pretty(&plan)?);
    if std::env::var_os("NEARBY").is_some() {
        let nearby = gamebuddy_services::navigation::planning::nearby_requirements(&store, &guide)?;
        eprintln!("{}", serde_json::to_string_pretty(&nearby)?);
    }
    Ok(())
}
