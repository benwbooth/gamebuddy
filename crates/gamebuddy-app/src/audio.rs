#[cxx_qt::bridge]
pub mod ffi {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }
    unsafe extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qproperty(QString, status)]
        type GameAudio = super::GameAudioRust;
        #[qinvokable]
        fn poll(self: Pin<&mut GameAudio>, pid: QString, active: bool, enabled: bool);
    }
}
#[cfg(target_os = "linux")]
use cxx_qt::CxxQtType;
use cxx_qt_lib::QString;
use std::pin::Pin;

#[derive(Default)]
pub struct GameAudioRust {
    status: QString,
    #[cfg(target_os = "linux")]
    worker: Option<Worker>,
}
#[cfg(target_os = "linux")]
struct Worker {
    pid: u32,
    child: Option<std::process::Child>,
    input: Option<std::process::ChildStdin>,
    updates: std::sync::mpsc::Receiver<String>,
}
#[cfg(target_os = "linux")]
impl Worker {
    fn start(pid: u32) -> anyhow::Result<Self> {
        use std::{
            io::BufRead,
            process::{Command, Stdio},
        };
        let mut child = Command::new(std::env::current_exe()?)
            .args(["--audio-duck-worker", &pid.to_string()])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let input = child.stdin.take();
        let output = child.stdout.take().unwrap();
        let (sender, updates) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for line in std::io::BufReader::new(output).lines() {
                let Ok(line) = line else { break };
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
        Ok(Self {
            pid,
            child: Some(child),
            input,
            updates,
        })
    }
}
#[cfg(target_os = "linux")]
impl Drop for Worker {
    fn drop(&mut self) {
        self.input.take(); // EOF restores volume, including on a normal Qt exit.
        if let Some(mut child) = self.child.take() {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
    }
}
impl ffi::GameAudio {
    pub fn poll(mut self: Pin<&mut Self>, pid: QString, active: bool, enabled: bool) {
        #[cfg(target_os = "linux")]
        {
            use std::io::Write;
            let pid = pid.to_string().parse::<u32>().ok().filter(|p| *p > 1);
            if !enabled || pid.is_none() {
                self.as_mut().rust_mut().worker = None;
                self.set_status(QString::from(if enabled {
                    "Launch a game to enable audio ducking"
                } else {
                    "Game audio ducking off"
                }));
                return;
            }
            let pid = pid.unwrap();
            if self.rust().worker.as_ref().is_some_and(|w| w.pid != pid) {
                self.as_mut().rust_mut().worker = None;
            }
            if self.rust().worker.is_none() {
                match Worker::start(pid) {
                    Ok(worker) => self.as_mut().rust_mut().worker = Some(worker),
                    Err(e) => {
                        self.set_status(QString::from(format!("Game audio: {e}")));
                        return;
                    }
                }
            }
            let mut worker = self.as_mut().rust_mut();
            let worker = worker.worker.as_mut().unwrap();
            let result =
                worker
                    .input
                    .as_mut()
                    .unwrap()
                    .write_all(if active { b"1\n" } else { b"0\n" });
            let mut latest = None;
            while let Ok(update) = worker.updates.try_recv() {
                latest = Some(update)
            }
            if result.is_err() {
                self.as_mut().rust_mut().worker = None;
                self.set_status(QString::from("Game audio helper stopped; reconnecting"));
            } else if let Some(update) = latest
                && let Ok(value) = serde_json::from_str::<serde_json::Value>(&update)
                && let Some(status) = value["status"].as_str()
            {
                self.set_status(QString::from(status));
            }
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (pid, active, enabled);
            self.set_status(QString::from(
                "Per-game audio ducking is currently supported on Linux",
            ));
        }
    }
}
