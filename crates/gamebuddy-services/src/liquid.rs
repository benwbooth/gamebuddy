//! GameBuddy's own llama-servers for Liquid's LFM3 models:
//! - LFM3-VL-2B on port 11440 for the "liquid" setup (reasoning, screen
//!   reading, spoken answers);
//! - LFM3-Omni-2B on port 11441 as GameBuddy's ears and voice: speech
//!   recognition (`/v1/audio/transcriptions`) and speech (`/tts`), in a
//!   fraction of a second each on the GPU.
//!
//! The runtime is llama.cpp built for this GPU (`models/lfm3/runtime`, from
//! the flake beside it) and the weights sit in `models/lfm3`. Servers start
//! on demand and stop with GameBuddy.
use crate::{Settings, Store};
use anyhow::{Context, Result};
use std::{
    path::PathBuf,
    process::{Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

const VL_PORT: u16 = 11440;
pub const VOICE_PORT: u16 = 11441;

fn models() -> Result<PathBuf> {
    Ok(crate::store::data_dir()?.join("models").join("lfm3"))
}

/// True when the LFM3 reasoning setup is in use.
pub fn selected(settings: &Settings) -> bool {
    [&settings.reasoning, &settings.vision]
        .iter()
        .any(|c| c.endpoint == crate::providers::LIQUID_ENDPOINT)
}

pub fn answering(port: u16) -> bool {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .and_then(|c| c.get(format!("http://127.0.0.1:{port}/health")).send())
        .is_ok_and(|r| r.status().is_success())
}

struct Server {
    port: u16,
    model: &'static str,
    projector: &'static str,
    alias: &'static str,
    context: u32,
}

const VL: Server = Server {
    port: VL_PORT,
    model: "LFM3-VL-2B-Q8_0.gguf",
    projector: "mmproj-LFM3-VL-2B-F16.gguf",
    alias: crate::providers::LIQUID_MODEL,
    context: 65_536,
};
const VOICE: Server = Server {
    port: VOICE_PORT,
    model: "LFM3-Omni-2B-Q8_0.gguf",
    projector: "mmproj-LFM3-Omni-2B-F16.gguf",
    alias: "lfm3-omni-2b",
    context: 8_192,
};

/// The voice model is installed (runtime and weights).
pub fn voice_installed() -> bool {
    models().is_ok_and(|d| {
        d.join("runtime/bin/llama-server").is_file()
            && d.join(VOICE.model).is_file()
            && d.join(VOICE.projector).is_file()
    })
}

/// Start the reasoning server in the background when its setup is selected.
pub fn ensure(store: &Store) {
    static STARTED: AtomicBool = AtomicBool::new(false);
    if store
        .get::<Settings>("settings")
        .ok()
        .flatten()
        .is_some_and(|s| selected(&s))
    {
        run(&VL, &STARTED);
    }
}

/// Start the voice server in the background if it is installed.
pub fn ensure_voice() {
    static STARTED: AtomicBool = AtomicBool::new(false);
    if voice_installed() {
        run(&VOICE, &STARTED);
    }
}

/// Wait up to `seconds` for the voice server to answer.
pub fn wait_for_voice(seconds: u64) -> bool {
    ensure_voice();
    for _ in 0..seconds * 4 {
        if answering(VOICE_PORT) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    false
}

fn run(server: &'static Server, started: &'static AtomicBool) {
    if started.swap(true, Ordering::AcqRel) {
        return;
    }
    // This thread owns the server for its whole life: the child is told to
    // stop when its parent thread ends, which is when GameBuddy exits.
    std::thread::spawn(move || {
        if let Err(error) = serve(server) {
            eprintln!("LFM3 server on {}: {error:#}", server.port);
        }
        started.store(false, Ordering::Release);
    });
}

fn serve(server: &Server) -> Result<()> {
    if answering(server.port) {
        return Ok(());
    }
    let dir = models()?;
    let binary = dir.join("runtime").join("bin").join("llama-server");
    let model = dir.join(server.model);
    let projector = dir.join(server.projector);
    for path in [&binary, &model, &projector] {
        anyhow::ensure!(path.is_file(), "Missing {}", path.display());
    }
    let mut command = Command::new(binary);
    command
        .arg("-m")
        .arg(model)
        .arg("--mmproj")
        .arg(projector)
        .args([
            "--host",
            "127.0.0.1",
            "--port",
            &server.port.to_string(),
            "--alias",
            server.alias,
            "-ngl",
            "99",
            "-c",
            &server.context.to_string(),
            "-np",
            "2",
            "--jinja",
            "--reasoning",
            "off",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::process::CommandExt;
        unsafe {
            command.pre_exec(|| {
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
                Ok(())
            });
        }
    }
    let mut child = command.spawn().context("Starting llama-server")?;
    child.wait()?;
    Ok(())
}

/// Speech recognition with LFM3-Omni: 16 kHz mono WAV in, text out.
pub fn transcribe(wav: &[u8]) -> Result<String> {
    let boundary = format!("gamebuddy{}", crate::screen::now_ms());
    let mut body = Vec::new();
    for (name, value) in [("model", VOICE.alias), ("language", "en")] {
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n"
            )
            .as_bytes(),
        );
    }
    body.extend_from_slice(
        format!("--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"speech.wav\"\r\nContent-Type: audio/wav\r\n\r\n")
            .as_bytes(),
    );
    body.extend_from_slice(wav);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    let reply: serde_json::Value = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()?
        .post(format!(
            "http://127.0.0.1:{VOICE_PORT}/v1/audio/transcriptions"
        ))
        .header(
            "Content-Type",
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(body)
        .send()?
        .error_for_status()?
        .json()?;
    Ok(reply["text"].as_str().unwrap_or_default().trim().to_owned())
}
