//! retroarch_frame_probe config <root>  → writes and prints the RetroArch config
//! retroarch_frame_probe grab <root>    → asks for five frames and times them
fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let root = std::path::PathBuf::from(&args[2]);
    if args[1] == "config" {
        println!("{}", gamebuddy_services::retroarch::write_config(&root)?.display());
        return Ok(());
    }
    for i in 0..5 {
        let started = std::time::Instant::now();
        let out = root.join(format!("frame-{i}.png"));
        let got = gamebuddy_services::retroarch::frame(&root, &out)?;
        println!("frame {i}: {:?} in {} ms", got.map(|p| image::image_dimensions(p).ok()), started.elapsed().as_millis());
        std::thread::sleep(std::time::Duration::from_millis(400));
    }
    Ok(())
}
