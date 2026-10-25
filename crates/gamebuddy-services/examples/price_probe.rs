//! price_probe <data dir> "<question>"
fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let store = gamebuddy_services::Store::open(&args[1])?;
    let id: String = store.get("active_guide")?.unwrap();
    let game = store.guide(&id)?.game;
    let answer = gamebuddy_services::voice::briefing::price_reply(&store, &game, &args[2])?;
    println!("{:?}", answer.map(|a| a.text));
    Ok(())
}
