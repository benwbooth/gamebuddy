//! GameBuddy's own llama-server for Liquid's LFM3 models (the "liquid"
//! setup): one small vision-language model, LFM3-VL-2B, serving reasoning,
//! screen reading and the voice's answers on this computer.
//!
//! The runtime is llama.cpp built for this GPU (`models/lfm3/runtime`, from
//! the flake beside it) and the weights sit in `models/lfm3`. The server is
//! started when the setup is selected and stops with GameBuddy.
use crate::{Settings, Store};
use anyhow::{Context, Result};
use std::{
    path::PathBuf,
    process::{Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

const PORT: u16 = 11440;

fn models() -> Result<PathBuf> {
    Ok(crate::store::data_dir()?.join("models").join("lfm3"))
}

/// True when this setup is in use and its files are installed.
pub fn selected(settings: &Settings) -> bool {
    [&settings.reasoning, &settings.vision]
        .iter()
        .any(|c| c.endpoint == crate::providers::LIQUID_ENDPOINT)
}

fn answering() -> bool {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .and_then(|c| c.get(format!("http://127.0.0.1:{PORT}/health")).send())
        .is_ok_and(|r| r.status().is_success())
}

/// Start the server in the background if the setup is selected and it is not
/// running yet. It dies with this process.
pub fn ensure(store: &Store) {
    static STARTED: AtomicBool = AtomicBool::new(false);
    let Ok(Some(settings)) = store.get::<Settings>("settings") else {
        return;
    };
    if !selected(&settings) || STARTED.swap(true, Ordering::AcqRel) {
        return;
    }
    std::thread::spawn(|| {
        if let Err(error) = start() {
            eprintln!("LFM3 server: {error:#}");
            STARTED.store(false, Ordering::Release);
        }
    });
}

fn start() -> Result<()> {
    if answering() {
        return Ok(());
    }
    let dir = models()?;
    let server = dir.join("runtime").join("bin").join("llama-server");
    let model = dir.join("LFM3-VL-2B-Q8_0.gguf");
    let projector = dir.join("mmproj-LFM3-VL-2B-F16.gguf");
    for path in [&server, &model, &projector] {
        anyhow::ensure!(path.is_file(), "Missing {}", path.display());
    }
    let mut command = Command::new(server);
    command
        .arg("-m")
        .arg(model)
        .arg("--mmproj")
        .arg(projector)
        .args([
            "--host",
            "127.0.0.1",
            "--port",
            &PORT.to_string(),
            "--alias",
            crate::providers::LIQUID_MODEL,
            "-ngl",
            "99",
            "-c",
            "65536",
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
        // Stop with GameBuddy, so its video memory is not held after a game.
        unsafe {
            command.pre_exec(|| {
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
                Ok(())
            });
        }
    }
    command.spawn().context("Starting llama-server")?;
    for _ in 0..120 {
        if answering() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    anyhow::bail!("llama-server did not become ready within a minute")
}
