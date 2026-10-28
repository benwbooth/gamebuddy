//! Speak a line with the player's chosen local voice and time each sentence:
//! speak_probe <out dir> <text>
fn main() -> anyhow::Result<()> {
    use anyhow::Context;
    let a: Vec<String> = std::env::args().collect();
    let out = std::path::PathBuf::from(a.get(1).context("Out dir")?);
    let text = a.get(2).context("Text")?.clone();
    let config = gamebuddy_services::providers::local_voice().context("No local voice")?;
    println!("voice: {}", config.model.display());
    let started = std::time::Instant::now();
    let speech = gamebuddy_services::voice::speech::Speech::start(out.join("worker"), config);
    let (tx, rx) = std::sync::mpsc::channel();
    speech.speak(
        text,
        std::sync::Arc::new(move |_, chunk| {
            let _ = tx.send(chunk.map(|c| (c.key, c.text, c.last)));
        }),
    );
    loop {
        let (key, text, last) = rx
            .recv_timeout(std::time::Duration::from_secs(180))?
            .map_err(|e| anyhow::anyhow!("{e:#}"))?;
        let wav = gamebuddy_services::voice::speech::cached(&key).context("No audio")?;
        let path = out.join(format!("{key}.wav"));
        std::fs::write(&path, wav.as_slice())?;
        println!(
            "{:.2} s: {text} -> {}",
            started.elapsed().as_secs_f64(),
            path.display()
        );
        if last {
            return Ok(());
        }
    }
}
