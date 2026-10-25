//! Publish a frame as the live screen, prefetch the "what now?" answer and
//! ask it: prefetch_probe <data dir> <frame.jpg>
fn main() -> anyhow::Result<()> {
    use anyhow::Context;
    use gamebuddy_services::{Store, screen, voice::prefetch};
    let mut args = std::env::args().skip(1);
    let store = Store::open(args.next().context("data dir")?)?;
    let frame = std::path::PathBuf::from(args.next().context("frame")?);
    let id: String = store.get("active_guide")?.context("No active guide")?;
    let guide = store.guide(&id)?;
    let ticket = screen::start(&store, &guide.game, true)?;
    screen::publish(&store, &ticket, &frame, "probe")?;
    let started = std::time::Instant::now();
    let stored = prefetch::refresh(&store, &guide)?;
    println!("refresh stored={stored} in {:.1}s", started.elapsed().as_secs_f32());
    let started = std::time::Instant::now();
    let answer = prefetch::reply(&store, &guide.game, "What now?")?;
    println!("reply in {} ms: {:?}", started.elapsed().as_millis(), answer.map(|a| a.text));
    Ok(())
}
