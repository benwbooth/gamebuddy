//! Check that the LFM3 reasoning server starts for the "liquid" setup:
//! liquid_probe <data dir with that setup saved>
fn main() -> anyhow::Result<()> {
    let store = gamebuddy_services::Store::open(std::env::args().nth(1).unwrap_or_default())?;
    gamebuddy_services::liquid::ensure(&store);
    for i in 0..60 {
        if gamebuddy_services::liquid::answering(11440) {
            println!("answering after {:.1} s", i as f64 * 0.5);
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    anyhow::bail!("not answering after 30 s")
}
