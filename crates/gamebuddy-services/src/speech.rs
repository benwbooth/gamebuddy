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
    // LFM3-Omni on the GPU transcribes in a fraction of a second; Whisper on
    // the CPU is the fallback when it is not installed or not answering.
    if crate::liquid::voice_installed() && crate::liquid::wait_for_voice(15) {
        match std::fs::read(&audio)
            .map_err(anyhow::Error::from)
            .and_then(|wav| crate::liquid::transcribe(&wav))
        {
            Ok(text) if !text.is_empty() => return Ok(text),
            Ok(_) => anyhow::bail!("No speech was detected. Try speaking again."),
            Err(error) => eprintln!("LFM3 transcription: {error:#}"),
        }
    }
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

/// Caption fallback for a bounded walkthrough interval. Cache original timings;
/// these are machine transcripts, not publisher captions or verified game facts.
pub fn transcribe_reference(
    root: &Path,
    media: &Path,
    start: u64,
    end: u64,
) -> Result<Vec<crate::captions::CaptionCue>> {
    use sha2::{Digest, Sha256};
    use std::process::Stdio;
    ensure!(
        end > start && end - start <= 600_000,
        "Audio reference interval must be at most ten minutes"
    );
    let key = format!(
        "{:x}",
        Sha256::digest(format!("{}:{start}:{end}", media.display()))
    );
    let directory = root.join("transcripts");
    std::fs::create_dir_all(&directory)?;
    let cached = directory.join(format!("{key}.json"));
    if cached.is_file() {
        return Ok(serde_json::from_slice(&std::fs::read(cached)?)?);
    }
    let model = speech_model(root)?;
    let temporary = tempfile::tempdir_in(&directory)?;
    let audio = temporary.path().join("audio.wav");
    let converted = Command::new("ffmpeg")
        .args([
            "-nostdin",
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-ss",
        ])
        .arg(format!("{:.3}", start as f64 / 1000.0))
        .arg("-i")
        .arg(media)
        .arg("-t")
        .arg(format!("{:.3}", (end - start) as f64 / 1000.0))
        .args(["-vn", "-ar", "16000", "-ac", "1", "-c:a", "pcm_s16le"])
        .arg(&audio)
        .output()?;
    ensure!(
        converted.status.success(),
        "Reference audio unavailable: {}",
        String::from_utf8_lossy(&converted.stderr)
    );
    let output = temporary.path().join("result");
    let mut child = Command::new(
        std::env::var_os("GAMEBUDDY_WHISPER_CLI").unwrap_or_else(|| "whisper-cli".into()),
    )
    .arg("-m")
    .arg(model)
    .arg("-f")
    .arg(audio)
    .args(["-ng", "-oj", "-t", "4", "-of"])
    .arg(&output)
    .stdin(Stdio::null())
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .spawn()
    .context("Reference transcription requires whisper-cli")?;
    let deadline = std::time::Instant::now() + Duration::from_secs(300);
    loop {
        if let Some(status) = child.try_wait()? {
            ensure!(status.success(), "Reference transcription failed");
            break;
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!("Reference transcription timed out");
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(output.with_extension("json"))?)?;
    let cues = value["transcription"]
        .as_array()
        .context("Whisper returned no timestamped transcript")?
        .iter()
        .filter_map(|v| {
            let from = v["offsets"]["from"].as_u64()?;
            let to = v["offsets"]["to"].as_u64()?;
            let text = v["text"].as_str()?.trim();
            (to > from && !text.is_empty() && start + from < end).then(|| {
                crate::captions::CaptionCue {
                    start_ms: start + from,
                    end_ms: (start + to).min(end),
                    text: text.into(),
                }
            })
        })
        .collect::<Vec<_>>();
    let mut file = tempfile::NamedTempFile::new_in(directory)?;
    serde_json::to_writer(&mut file, &cues)?;
    file.persist(cached)?;
    Ok(cues)
}
