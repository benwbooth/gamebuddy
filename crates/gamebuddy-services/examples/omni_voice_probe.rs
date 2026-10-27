//! Start GameBuddy's LFM3-Omni voice server, transcribe a WAV with it and
//! time it: omni_voice_probe <speech.wav>
fn main() -> anyhow::Result<()> {
    let path = std::env::args().nth(1).unwrap_or_default();
    let started = std::time::Instant::now();
    anyhow::ensure!(gamebuddy_services::liquid::wait_for_voice(60), "Voice server did not start");
    println!("server ready in {:.1} s", started.elapsed().as_secs_f64());
    let wav = std::fs::read(path)?;
    for _ in 0..2 {
        let t = std::time::Instant::now();
        let text = gamebuddy_services::liquid::transcribe(&wav)?;
        println!("{:.2} s: {text}", t.elapsed().as_secs_f64());
    }
    Ok(())
}
