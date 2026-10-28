//! Which destination a sentence sets: goal_probe <data dir copy> <guide id> <sentence>
fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let store = gamebuddy_services::Store::open(&args[1])?;
    let guide = store.guide(&args[2])?;
    let atlas = gamebuddy_services::navigation::atlas(&store, &guide)?;
    let goal = gamebuddy_services::navigation::routing::set_from_text(&store, &guide, &atlas, &args[3], None)?;
    println!("{:?}", goal.map(|g| g.label));
    Ok(())
}
