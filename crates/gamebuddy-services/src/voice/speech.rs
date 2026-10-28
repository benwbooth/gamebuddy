//! Optional prepared neural speech, independent of the live microphone and GPU.
//! The bounded worker/cache also accepts foreground synthesis at higher priority.
use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, VecDeque},
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::{Arc, Condvar, Mutex, OnceLock, mpsc},
    time::{Duration, Instant},
};
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Config {
    #[serde(default = "enabled")]
    pub enabled: bool,
    pub python: String,
    pub model: PathBuf,
}
fn enabled() -> bool {
    true
}
impl Config {
    pub fn validate(&self) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }
        ensure!(
            !self.python.trim().is_empty(),
            "Choose a Python runtime with Piper installed"
        );
        ensure!(self.model.is_file(), "Prepared speech model is missing");
        // A .kokoro file names a Kokoro voice; Piper voices need their JSON.
        ensure!(
            self.model
                .extension()
                .is_some_and(|e| e == "kokoro" || e == "omni" || e == "orpheus")
                || self.model.with_extension("onnx.json").is_file(),
            "Prepared speech model configuration is missing"
        );
        Ok(())
    }
}

pub fn key(config: &Config, text: &str) -> String {
    let stamp = |path: &Path| {
        std::fs::metadata(path)
            .ok()
            .map(|m| (m.len(), m.modified().ok()))
    };
    format!(
        "{:x}",
        Sha256::digest(format!(
            "{}:{}:{:?}:{:?}:{text}",
            config.python,
            config.model.display(),
            stamp(&config.model),
            stamp(&config.model.with_extension("onnx.json"))
        ))
    )
}
#[derive(Default)]
struct Cache {
    values: HashMap<String, Arc<Vec<u8>>>,
    order: VecDeque<String>,
    bytes: usize,
}
static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
pub fn cached(key: &str) -> Option<Arc<Vec<u8>>> {
    CACHE.get()?.lock().ok()?.values.get(key).cloned()
}
fn save(key: String, audio: Vec<u8>) {
    if audio.len() > 8 * 1024 * 1024 {
        return;
    }
    let mut cache = CACHE.get_or_init(Default::default).lock().unwrap();
    if cache.values.contains_key(&key) {
        return;
    }
    cache.bytes += audio.len();
    cache.order.push_back(key.clone());
    cache.values.insert(key, Arc::new(audio));
    while cache.order.len() > 32 || cache.bytes > 32 * 1024 * 1024 {
        if let Some(key) = cache.order.pop_front()
            && let Some(audio) = cache.values.remove(&key)
        {
            cache.bytes -= audio.len();
        }
    }
}

struct Worker {
    /// Orpheus waits for its server and writes a sentence at about 1.3x
    /// speaking speed: it gets longer to load and to answer.
    slow: bool,
    child: Child,
    input: ChildStdin,
    output: mpsc::Receiver<Result<Value>>,
}
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
impl Worker {
    fn start(root: &Path, config: &Config) -> Result<Self> {
        config.validate()?;
        std::fs::create_dir_all(root)?;
        let script = root.join("speech-worker.py");
        std::fs::write(
            &script,
            include_str!("../../../../scripts/speech-worker.py"),
        )?;
        let mut child = Command::new(&config.python)
            .arg("-u")
            .arg(script)
            .arg(&config.model)
            .env("OMP_NUM_THREADS", "2")
            .env("OPENBLAS_NUM_THREADS", "2")
            // A Nix shell's PYTHONPATH points at another Python's packages
            // and breaks the voice runtimes' own.
            .env_remove("PYTHONPATH")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .context("Starting prepared speech")?;
        let input = child.stdin.take().context("Speech input")?;
        let output = child.stdout.take().context("Speech output")?;
        let (tx, rx) = mpsc::sync_channel(2);
        std::thread::spawn(move || {
            for line in BufReader::new(output).lines() {
                if tx
                    .send(
                        line.map_err(anyhow::Error::from)
                            .and_then(|s| Ok(serde_json::from_str(&s)?)),
                    )
                    .is_err()
                {
                    break;
                }
            }
        });
        let worker = Self {
            slow: config.model.extension().is_some_and(|e| e == "orpheus"),
            child,
            input,
            output: rx,
        };
        let value = worker
            .output
            .recv_timeout(Duration::from_secs(if worker.slow { 90 } else { 20 }))
            .context("Loading prepared speech timed out")??;
        ensure!(value["ready"] == true, "Prepared speech failed: {value}");
        Ok(worker)
    }
    fn synthesize(&mut self, text: &str) -> Result<Vec<u8>> {
        serde_json::to_writer(&mut self.input, &json!({"text":text}))?;
        self.input.write_all(b"\n")?;
        self.input.flush()?;
        let result = self
            .output
            .recv_timeout(Duration::from_secs(if self.slow { 40 } else { 15 }))
            .context("Speech synthesis timed out")??;
        let wav = STANDARD.decode(
            result["wav"]
                .as_str()
                .with_context(|| format!("Speech synthesis: {}", result["error"]))?,
        )?;
        ensure!(
            wav.starts_with(b"RIFF") && wav.len() > 44,
            "Invalid synthesized audio"
        );
        Ok(wav)
    }
}

#[derive(Debug, Serialize)]
pub struct Chunk {
    pub key: String,
    pub text: String,
    pub index: usize,
    pub last: bool,
    pub answer: Option<String>,
}
type Completed = Arc<dyn Fn(usize, Result<Chunk>) + Send + Sync>;
struct Job {
    text: String,
    index: usize,
    last: bool,
    answer: Option<String>,
    done: Option<Completed>,
}

/// Unicode sentence boundaries preserve decimals and avoid cutting words or
/// dropping text. The first sentence can play while later ones synthesize.
fn sentences(text: &str) -> Vec<String> {
    text.unicode_sentences()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}
#[derive(Default)]
struct Queue {
    foreground: VecDeque<Job>,
    background: VecDeque<Job>,
    stopped: bool,
}
pub struct Speech {
    queue: Arc<(Mutex<Queue>, Condvar)>,
    config: Config,
}
impl Speech {
    pub fn start(root: PathBuf, config: Config) -> Self {
        let queue = Arc::new((Mutex::new(Queue::default()), Condvar::new()));
        let shared = queue.clone();
        let worker_config = config.clone();
        std::thread::spawn(move || {
            let mut worker = Worker::start(&root, &worker_config).ok();
            loop {
                let job = {
                    let (lock, wake) = &*shared;
                    let mut q = lock.lock().unwrap();
                    while !q.stopped && q.foreground.is_empty() && q.background.is_empty() {
                        q = wake.wait(q).unwrap();
                    }
                    if q.stopped {
                        return;
                    }
                    q.foreground
                        .pop_front()
                        .or_else(|| q.background.pop_front())
                        .unwrap()
                };
                let id = key(&worker_config, &job.text);
                let started = Instant::now();
                let result = (|| -> Result<String> {
                    if cached(&id).is_none() {
                        if worker.is_none() {
                            worker = Some(Worker::start(&root, &worker_config)?);
                        }
                        let audio = worker.as_mut().unwrap().synthesize(&job.text)?;
                        save(id.clone(), audio);
                    }
                    Ok(id)
                })();
                if std::env::args().any(|a| a == "--voice-trace") {
                    eprintln!(
                        "GAMEBUDDY_PREPARED_SPEECH elapsed_ms={} success={}",
                        started.elapsed().as_millis(),
                        result.is_ok()
                    );
                }
                if result.is_err() {
                    worker = None;
                }
                if let Some(done) = job.done {
                    done(
                        job.index,
                        result.map(|key| Chunk {
                            key,
                            text: job.text,
                            index: job.index,
                            last: job.last,
                            answer: job.answer,
                        }),
                    );
                }
            }
        });
        Self { queue, config }
    }
    pub fn prepare(&self, phrases: Vec<String>) {
        let (lock, wake) = &*self.queue;
        let mut q = lock.lock().unwrap();
        q.background.clear();
        for text in phrases
            .into_iter()
            .filter(|s| !s.is_empty() && s.len() <= 4000)
            .take(8)
            .flat_map(|s| sentences(&s))
            .take(32)
        {
            if cached(&key(&self.config, &text)).is_none() {
                q.background.push_back(Job {
                    text,
                    index: 0,
                    last: true,
                    answer: None,
                    done: None,
                });
            }
        }
        wake.notify_one();
    }
    pub fn speak(&self, text: String, done: Completed) {
        let parts = sentences(&text);
        let (lock, wake) = &*self.queue;
        let mut q = lock.lock().unwrap();
        // Obsolete speech must never wait behind older requests.
        q.foreground.clear();
        if parts.is_empty() || parts.len() > 128 || text.len() > 16000 {
            drop(q);
            done(
                0,
                Err(anyhow::anyhow!(
                    "Prepared speech exceeds its response budget"
                )),
            );
            return;
        }
        let mut ready = vec![];
        let count = parts.len();
        for (index, part) in parts.into_iter().enumerate() {
            let id = key(&self.config, &part);
            let last = index + 1 == count;
            let answer = (index == 0).then(|| text.clone());
            if cached(&id).is_some() {
                ready.push(Chunk {
                    key: id,
                    text: part,
                    index,
                    last,
                    answer,
                });
            } else {
                q.foreground.push_back(Job {
                    text: part,
                    index,
                    last,
                    answer,
                    done: Some(done.clone()),
                });
            }
        }
        drop(q);
        for chunk in ready {
            done(chunk.index, Ok(chunk));
        }
        wake.notify_one();
    }
    pub fn cancel(&self) {
        self.queue.0.lock().unwrap().foreground.clear();
    }
}

impl Drop for Speech {
    fn drop(&mut self) {
        let (lock, wake) = &*self.queue;
        lock.lock().unwrap().stopped = true;
        wake.notify_one();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sentence_chunks_preserve_numbers_unicode_and_the_final_instruction() {
        let input = "Use 1.5 meters of rope. Then enter the door!\nDo not jump… wait.";
        let parts = sentences(input);
        assert_eq!(parts[0], "Use 1.5 meters of rope.");
        assert_eq!(parts.join(" "), input.replace('\n', " "));
        assert_eq!(parts.last().unwrap(), "Do not jump… wait.");
    }
}
