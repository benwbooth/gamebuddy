//! Run the walkthrough chart scan for the active guide, or with `widgets`
//! print the reference widgets it would show now:
//! reference_scan <data directory> [widgets]
fn main() -> anyhow::Result<()> {
    use anyhow::Context;
    let mut args = std::env::args().skip(1);
    let store = gamebuddy_services::Store::open(args.next().context("Data directory")?)?;
    let id: String = store.get("active_guide")?.context("No active guide")?;
    let guide = store.guide(&id)?;
    if args.next().as_deref() == Some("widgets") {
        gamebuddy_services::game_data::sync_widgets(&store, &guide)?;
        let widgets = gamebuddy_services::harness::widgets(&store, &guide)?;
        println!("{}", serde_json::to_string_pretty(&widgets)?);
        return Ok(());
    }
    let count = gamebuddy_services::reference_scan::scan(&store, &guide, &|m| eprintln!("{m}"))?;
    println!("{count} charts and data cards");
    Ok(())
}
