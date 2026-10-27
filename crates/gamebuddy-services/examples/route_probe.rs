//! The walkthrough route between two places: route_probe <data dir> <guide id> <from place id> <to place text>
fn main() -> anyhow::Result<()> {
    use anyhow::Context;
    let args: Vec<String> = std::env::args().collect();
    let store = gamebuddy_services::Store::open(args.get(1).context("Data directory")?)?;
    let guide = store.guide(args.get(2).context("Guide id")?)?;
    let atlas = gamebuddy_services::navigation::atlas(&store, &guide)?;
    let from = args.get(3).context("From place id")?;
    let to = gamebuddy_services::navigation::routing::named_place(
        &atlas,
        args.get(4).context("Destination")?,
    )
    .context("No place named")?;
    let goal = gamebuddy_services::navigation::routing::Goal {
        place_id: to.clone(),
        label: to,
        set_at_ms: 0,
    };
    println!(
        "{}",
        serde_json::to_string_pretty(&gamebuddy_services::navigation::routing::route_view(
            &guide, &atlas, from, &goal
        ))?
    );
    Ok(())
}
