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
    [&settings.reasoning, &settings.vision].iter().any(|c| {
        c.endpoint == crate::providers::LIQUID_ENDPOINT
            || c.endpoint == crate::providers::LIQUID_TEXT_ENDPOINT
            || c.endpoint == crate::providers::LIQUID_OMNI_ENDPOINT
    })
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
    // Screen reads are short; a big context only reserved video memory the
    // game needed.
    context: 16_384,
};
const TEXT: Server = Server {
    port: 11442,
    model: "LFM3-1.7B-Q8_0.gguf",
    projector: "",
    alias: crate::providers::LIQUID_TEXT_MODEL,
    context: 32_768,
};
const VOICE: Server = Server {
    port: VOICE_PORT,
    model: "LFM3-Omni-2B-Q8_0.gguf",
    projector: "mmproj-LFM3-Omni-2B-F16.gguf",
    alias: "lfm3-omni-2b",
    context: 8_192,
};

/// Orpheus 3B, GameBuddy's speaking voice when the player picks it
/// (`orpheus:<voice>`): it writes SNAC audio codes that the speech worker
/// decodes. Its weights live in `models/orpheus`, beside its Python runtime.
const ORPHEUS: Server = Server {
    port: 11443,
    model: "../orpheus/Orpheus-3b-FT-Q8_0.gguf",
    projector: "",
    alias: "orpheus-3b",
    context: 4_096,
};

/// Orpheus is installed: the server runtime, its weights and the decoder's
/// Python.
pub fn orpheus_installed() -> bool {
    models().is_ok_and(|d| {
        d.join("runtime/bin/llama-server").is_file()
            && d.join(ORPHEUS.model).is_file()
            && d.join("../orpheus/venv/bin/python").is_file()
    })
}

/// Start the Orpheus voice server in the background if it is installed.
pub fn ensure_orpheus() {
    if orpheus_installed() {
        run(&ORPHEUS, &ORPHEUS_STARTED);
    }
}

/// The LFM3-VL screen reader is installed (runtime, weights, projector).
pub fn installed() -> bool {
    models().is_ok_and(|d| {
        d.join("runtime/bin/llama-server").is_file()
            && d.join(VL.model).is_file()
            && d.join(VL.projector).is_file()
    })
}

/// The voice model is installed (runtime and weights).
pub fn voice_installed() -> bool {
    models().is_ok_and(|d| {
        d.join("runtime/bin/llama-server").is_file()
            && d.join(VOICE.model).is_file()
            && d.join(VOICE.projector).is_file()
    })
}

static VL_STARTED: AtomicBool = AtomicBool::new(false);
static VOICE_STARTED: AtomicBool = AtomicBool::new(false);
static TEXT_STARTED: AtomicBool = AtomicBool::new(false);
static ORPHEUS_STARTED: AtomicBool = AtomicBool::new(false);

/// Start the reasoning server in the background when its setup is selected.
pub fn ensure(store: &Store) {
    let Some(settings) = store.get::<Settings>("settings").ok().flatten() else {
        return;
    };
    // Only the servers this setup uses: each holds video memory the game needs.
    for config in [&settings.reasoning, &settings.vision] {
        if config.endpoint == crate::providers::LIQUID_ENDPOINT {
            run(&VL, &VL_STARTED);
        } else if config.endpoint == crate::providers::LIQUID_TEXT_ENDPOINT {
            run(&TEXT, &TEXT_STARTED);
        } else if config.endpoint == crate::providers::LIQUID_OMNI_ENDPOINT && voice_installed() {
            run(&VOICE, &VOICE_STARTED);
        }
    }
}

/// Before a request to GameBuddy's own LFM3 server: start it if it is not
/// running (switching setups mid-game, a crash) and wait for it, so no
/// request fails just because the server was not up yet.
pub fn ready_for(config: &crate::ProviderConfig) {
    let (server, started) = if config.endpoint == crate::providers::LIQUID_ENDPOINT {
        (&VL, &VL_STARTED)
    } else if config.endpoint == crate::providers::LIQUID_TEXT_ENDPOINT {
        (&TEXT, &TEXT_STARTED)
    } else if config.endpoint == crate::providers::LIQUID_OMNI_ENDPOINT {
        (&VOICE, &VOICE_STARTED)
    } else {
        return;
    };
    if answering(server.port) {
        return;
    }
    run(server, started);
    for _ in 0..240 {
        if answering(server.port) {
            return;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

/// Start the voice server in the background if it is installed.
pub fn ensure_voice() {
    if voice_installed() {
        run(&VOICE, &VOICE_STARTED);
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

/// Stop LFM3 servers left behind by an earlier GameBuddy (their parent is
/// gone): they hold video memory, and one answering on our port with an
/// exhausted heap failed every request. Only GameBuddy's own runtime binary
/// is touched.
fn clear_orphans() {
    static DONE: AtomicBool = AtomicBool::new(false);
    if DONE.swap(true, Ordering::AcqRel) {
        return;
    }
    let Ok(dir) = models() else { return };
    let binary = dir.join("runtime").join("bin").join("llama-server");
    let me = std::process::id();
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return;
    };
    for entry in entries.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|p| p.parse::<i32>().ok())
        else {
            continue;
        };
        let Ok(exe) = std::fs::read_link(entry.path().join("exe")) else {
            continue;
        };
        if exe != binary && !exe.ends_with("llama-server") {
            continue;
        }
        let Ok(cmdline) = std::fs::read(entry.path().join("cmdline")) else {
            continue;
        };
        let cmdline = String::from_utf8_lossy(&cmdline);
        if !cmdline.starts_with(binary.to_string_lossy().as_ref()) {
            continue;
        }
        // Still owned by a running GameBuddy (this one or another)?
        let parent = std::fs::read_to_string(entry.path().join("stat"))
            .ok()
            .and_then(|s| s.rsplit(')').next().map(str::to_owned))
            .and_then(|s| {
                s.split_whitespace()
                    .nth(1)
                    .and_then(|p| p.parse::<u32>().ok())
            });
        let owner = parent
            .and_then(|p| std::fs::read_link(format!("/proc/{p}/exe")).ok())
            .map(|e| e.to_string_lossy().into_owned())
            .unwrap_or_default();
        if parent == Some(me) || owner.contains("gamebuddy") {
            continue;
        }
        eprintln!("Stopping a leftover LFM3 server (pid {pid})");
        unsafe {
            libc::kill(pid, libc::SIGKILL);
        }
    }
    std::thread::sleep(Duration::from_millis(300));
}

fn run(server: &'static Server, started: &'static AtomicBool) {
    if started.swap(true, Ordering::AcqRel) {
        return;
    }
    clear_orphans();
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
    for path in [&binary, &model] {
        anyhow::ensure!(path.is_file(), "Missing {}", path.display());
    }
    let mut command = Command::new(binary);
    command.arg("-m").arg(model);
    // Text-only models (LFM3-1.7B) have no vision/audio projector.
    if !server.projector.is_empty() {
        let projector = dir.join(server.projector);
        anyhow::ensure!(projector.is_file(), "Missing {}", projector.display());
        command.arg("--mmproj").arg(projector);
    }
    command
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
            "1",
            "--jinja",
            "--reasoning",
            "off",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        // Its own log beside the weights: why a server stopped is otherwise lost.
        .stderr(
            std::fs::File::create(dir.join(format!("server-{}.log", server.port)))
                .map(Stdio::from)
                .unwrap_or_else(|_| Stdio::null()),
        );
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::process::CommandExt;
        unsafe {
            let parent = libc::getpid();
            command.pre_exec(move || {
                // SIGKILL: llama-server catches SIGTERM for a graceful stop
                // and could linger, holding video memory, after GameBuddy.
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL);
                // GameBuddy already gone before the signal was armed.
                if libc::getppid() != parent {
                    libc::_exit(0);
                }
                Ok(())
            });
        }
    }
    let mut child = command.spawn().context("Starting llama-server")?;
    eprintln!("LFM3 server on {} started", server.port);
    let status = child.wait()?;
    anyhow::bail!("stopped ({status}); see server-{}.log", server.port)
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
