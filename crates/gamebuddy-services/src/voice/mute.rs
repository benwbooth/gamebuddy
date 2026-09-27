//! Read-only system mute monitoring. No device is muted/unmuted by GameBuddy.
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub device: String,
    pub muted: Option<bool>,
    pub error: String,
}

pub struct Monitor {
    device: Arc<Mutex<String>>,
    stop: Arc<AtomicBool>,
    updates: mpsc::Receiver<Snapshot>,
}
impl Default for Monitor {
    fn default() -> Self {
        let device = Arc::new(Mutex::new(String::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (sender, updates) = mpsc::channel();
        let (input, stopped) = (device.clone(), stop.clone());
        std::thread::spawn(move || {
            let mut previous = None;
            while !stopped.load(Ordering::Relaxed) {
                let device = input.lock().unwrap().clone();
                if !device.is_empty() {
                    let state = match query(&device) {
                        Ok(muted) => Snapshot {
                            device,
                            muted: Some(muted),
                            error: String::new(),
                        },
                        Err(error) => Snapshot {
                            device,
                            muted: None,
                            error: format!("{error:#}"),
                        },
                    };
                    if previous.as_ref() != Some(&state) {
                        previous = Some(state.clone());
                        if sender.send(state).is_err() {
                            break;
                        }
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(250));
            }
        });
        Self {
            device,
            stop,
            updates,
        }
    }
}
impl Monitor {
    pub fn select(&self, device: &str) {
        *self.device.lock().unwrap() = device.into();
    }
    pub fn poll(&self) -> Option<Snapshot> {
        let mut latest = None;
        while let Ok(state) = self.updates.try_recv() {
            latest = Some(state);
        }
        latest
    }
}
impl Drop for Monitor {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

#[cfg(target_os = "linux")]
fn query(device: &str) -> anyhow::Result<bool> {
    use std::{
        io::Read,
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    // The device ID is the exact Qt-selected PulseAudio source, not the default
    // source alias. This also works with PipeWire's PulseAudio server.
    let mut child = Command::new("pactl")
        .args(["get-source-mute", device])
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| anyhow::anyhow!("System mic mute monitoring requires pactl: {e}"))?;
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if let Some(status) = child.try_wait()? {
            anyhow::ensure!(
                status.success(),
                "Cannot read this microphone's system mute state. Use Listen or hold to talk."
            );
            let mut output = String::new();
            child
                .stdout
                .take()
                .unwrap()
                .take(256)
                .read_to_string(&mut output)?;
            return parse(&output);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!("System microphone mute query timed out");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
#[cfg(not(target_os = "linux"))]
fn query(_device: &str) -> anyhow::Result<bool> {
    anyhow::bail!(
        "System mute activation is currently available on Linux. Use Listen or hold to talk."
    )
}
#[cfg(any(target_os = "linux", test))]
fn parse(output: &str) -> anyhow::Result<bool> {
    match output.trim() {
        "Mute: yes" => Ok(true),
        "Mute: no" => Ok(false),
        _ => anyhow::bail!("Unrecognized system microphone mute state"),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unknown_mute_state_is_never_treated_as_unmuted() {
        assert!(parse("Mute: yes\n").unwrap());
        assert!(!parse("Mute: no\n").unwrap());
        assert!(parse("Connection failure").is_err());
        assert!(parse("").is_err());
    }
}
