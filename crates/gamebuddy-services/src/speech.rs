//! Local speech recognition for automatic and push-to-talk turns. Reuses
//! whisper.cpp; orchestration, model caching and the coaching loop stay in Rust.
use anyhow::{Context, Result, ensure};
use std::{
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

pub fn speech_model(root: &Path) -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("GAMEBUDDY_WHISPER_MODEL") {
        let path = PathBuf::from(path);
        ensure!(
            path.is_file(),
            "GAMEBUDDY_WHISPER_MODEL does not point to a model file"
        );
        return Ok(path);
    }
    let directory = root.join("models");
    std::fs::create_dir_all(&directory)?;
    let model = directory.join("ggml-base.en.bin");
    if model.is_file() {
        return Ok(model);
    }
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(240))
        .build()?;
    let mut response = client
        .get("https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin")
        .send()?
        .error_for_status()?;
    let mut file = tempfile::NamedTempFile::new_in(directory)?;
    let bytes = std::io::copy(&mut response, &mut file)?;
    ensure!(
        bytes > 100_000_000,
        "The speech model download was incomplete"
    );
    file.as_file().sync_all()?;
    file.persist(&model)
        .context("Saving the local speech model")?;
    Ok(model)
}

pub fn transcribe(root: &Path, recording: &Path) -> Result<String> {
    ensure!(recording.is_file(), "No microphone recording is available");
    let model = speech_model(root)?;
    let work = tempfile::tempdir_in(root)?;
    let audio = work.path().join("speech.wav");
    let converted = Command::new("ffmpeg")
        .args(["-nostdin", "-hide_banner", "-loglevel", "error", "-y", "-i"])
        .arg(recording)
        .args(["-t", "45", "-ar", "16000", "-ac", "1", "-c:a", "pcm_s16le"])
        .arg(&audio)
        .output()
        .context("Speech recognition requires ffmpeg")?;
    ensure!(
        converted.status.success(),
        "Could not decode the microphone recording: {}",
        String::from_utf8_lossy(&converted.stderr)
    );
    let output = work.path().join("transcript");
    let executable =
        std::env::var_os("GAMEBUDDY_WHISPER_CLI").unwrap_or_else(|| "whisper-cli".into());
    let result = Command::new(executable)
        .arg("-m")
        .arg(model)
        .arg("-f")
        .arg(audio)
        .args(["-ng", "-nt", "-otxt", "-t", "4", "-of"])
        .arg(&output)
        .output()
        .context("Install whisper.cpp and put whisper-cli in PATH, or set GAMEBUDDY_WHISPER_CLI")?;
    ensure!(
        result.status.success(),
        "Speech recognition failed: {}",
        String::from_utf8_lossy(&result.stderr)
            .chars()
            .take(600)
            .collect::<String>()
    );
    let text = std::fs::read_to_string(output.with_extension("txt"))?
        .trim()
        .to_owned();
    ensure!(
        !text.is_empty(),
        "No speech was detected. Try speaking again."
    );
    Ok(text)
}
