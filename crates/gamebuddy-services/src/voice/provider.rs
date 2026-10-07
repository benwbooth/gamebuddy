//! Adapt the selected completion provider to the voice loop's turn/tool events.
//!
//! Inference runs off the audio thread. This lane never executes tools: the
//! application still validates the current utterance, game, and screenshot
//! before returning a tool result. Superseded completions cannot publish events.
use crate::{HttpProvider, ModelRequest, ProviderConfig, ReasoningProvider};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
};

const MAX_ROUNDS: usize = 12;
const MAX_CONTEXT_BYTES: usize = 300_000;
type Complete = dyn Fn(ModelRequest) -> Result<String> + Send + Sync;

struct Job {
    serial: u64,
    request: ModelRequest,
}
struct Completion {
    serial: u64,
    result: Result<String>,
}
struct Turn {
    id: String,
    history: Vec<Value>,
    images: Vec<String>,
    rounds: usize,
    waiting_tool: Option<String>,
}

pub struct Lane {
    pub thread: String,
    instructions: String,
    tools: Value,
    active: Option<Turn>,
    serial: Arc<AtomicU64>,
    jobs: mpsc::SyncSender<Job>,
    completed: mpsc::Receiver<Completion>,
    events: VecDeque<Value>,
    recent: VecDeque<Value>,
}

impl Lane {
    pub fn new(config: ProviderConfig, instructions: String, tools: Value) -> Result<Self> {
        config.validate()?;
        Ok(Self::with_completion(
            instructions,
            tools,
            Arc::new(move |request| HttpProvider::new(config.clone())?.complete(request)),
        ))
    }

    fn with_completion(instructions: String, tools: Value, complete: Arc<Complete>) -> Self {
        let (jobs, requests) = mpsc::sync_channel::<Job>(2);
        let (done, completed) = mpsc::channel();
        let requests = Arc::new(Mutex::new(requests));
        let serial = Arc::new(AtomicU64::new(0));
        // A cancelled blocking HTTP request may finish at its transport timeout.
        // Allow the replacement to run, but bound both work and queued requests.
        for _ in 0..2 {
            let requests = requests.clone();
            let serial = serial.clone();
            let complete = complete.clone();
            let done = done.clone();
            std::thread::spawn(move || {
                loop {
                    let job = match requests.lock().unwrap().recv() {
                        Ok(job) => job,
                        Err(_) => break,
                    };
                    if serial.load(Ordering::SeqCst) != job.serial {
                        continue;
                    }
                    let result = complete(job.request);
                    if serial.load(Ordering::SeqCst) == job.serial
                        && done
                            .send(Completion {
                                serial: job.serial,
                                result,
                            })
                            .is_err()
                    {
                        break;
                    }
                }
            });
        }
        Self {
            thread: format!("provider-{}", uuid::Uuid::new_v4()),
            instructions,
            tools,
            active: None,
            serial,
            jobs,
            completed,
            events: VecDeque::new(),
            recent: VecDeque::new(),
        }
    }

    /// True means the message belongs to this reasoner, never to audio transport.
    pub fn send(&mut self, message: &Value) -> Result<bool> {
        let tool_reply = message["id"]
            .as_str()
            .is_some_and(|id| id.starts_with("provider-tool:"));
        if !tool_reply && message["params"]["threadId"] != self.thread {
            return Ok(false);
        }
        if tool_reply {
            let Some(turn) = self.active.as_mut() else {
                return Ok(true);
            };
            if turn.waiting_tool.as_deref() != message["id"].as_str() {
                return Ok(true);
            }
            turn.waiting_tool = None;
            let items = message["result"]["contentItems"].clone();
            let text = extract_content(&items, &mut turn.images)?;
            turn.history
                .push(json!({"role":"tool","success":message["result"]["success"],"content":text}));
            // A successful finish_voice_reply is already delivered by the app.
            // Do not spend another model round acknowledging it.
            let finished = message["result"]["success"] == true
                && items
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|v| v["text"].as_str())
                    .filter_map(|s| serde_json::from_str::<Value>(s).ok())
                    .any(|v| v["voice_reply"].is_object());
            if finished {
                self.finish("completed", None);
            } else {
                self.dispatch_or_fail();
            }
            return Ok(true);
        }
        let params = &message["params"];
        match message["method"].as_str() {
            Some("turn/start") if self.active.is_none() => {
                self.serial.fetch_add(1, Ordering::SeqCst);
                let mut turn = Turn { id: uuid::Uuid::new_v4().to_string(), history: vec![], images: vec![], rounds: 0, waiting_tool: None };
                let content = extract_content(&params["input"], &mut turn.images)?;
                turn.history.push(json!({"role":"user","content":content}));
                self.events.push_back(json!({"id":message["id"],"result":{"turn":{"id":turn.id}}}));
                self.events.push_back(json!({"method":"turn/started","params":{"threadId":self.thread,"turn":{"id":turn.id}}}));
                self.active = Some(turn);
                self.dispatch_or_fail();
            }
            Some("turn/steer") if self.active.as_ref().is_some_and(|t| params["expectedTurnId"] == t.id) => {
                self.serial.fetch_add(1, Ordering::SeqCst);
                let turn = self.active.as_mut().unwrap();
                // A fresh request may report capture paused/unavailable. Never
                // relabel the previous screenshot as its current evidence.
                turn.images.clear();
                let content = extract_content(&params["input"], &mut turn.images)?;
                turn.history.push(json!({"role":"user","content":content}));
                self.events.push_back(json!({"id":message["id"],"result":{"turnId":turn.id}}));
                // A tool already sent to the app must resolve before continuing;
                // preserve its result so a follow-up cannot replay an action.
                if turn.waiting_tool.is_none() { self.dispatch_or_fail(); }
            }
            Some("turn/interrupt") => {
                self.events.push_back(json!({"id":message["id"],"result":{}}));
                if self.active.as_ref().is_some_and(|t| params["turnId"] == t.id) {
                    self.finish("interrupted", None);
                }
            }
            _ => self.events.push_back(json!({"id":message["id"],"error":{"message":"The provider turn is no longer active"}})),
        }
        Ok(true)
    }

    pub fn poll(&mut self) -> Option<Value> {
        // Drain acknowledgments before completions, preserving turn ownership.
        if let Some(event) = self.events.pop_front() {
            return Some(event);
        }
        while let Ok(completion) = self.completed.try_recv() {
            if completion.serial != self.serial.load(Ordering::SeqCst) || self.active.is_none() {
                continue;
            }
            if let Err(error) = self.accept(completion.result) {
                self.finish("failed", Some(error.to_string()));
            }
        }
        self.events.pop_front()
    }

    fn dispatch_or_fail(&mut self) {
        if let Err(error) = self.dispatch() {
            self.finish("failed", Some(error.to_string()));
        }
    }

    fn dispatch(&mut self) -> Result<()> {
        let turn = self.active.as_mut().context("No provider turn")?;
        ensure!(
            turn.rounds < MAX_ROUNDS,
            "The provider exceeded the tool round limit"
        );
        let history = serde_json::to_string(&turn.history)?;
        ensure!(
            history.len() <= MAX_CONTEXT_BYTES,
            "The provider conversation exceeded its context limit"
        );
        let request = ModelRequest {
            system: format!(
                "{}\nYou are inside GameBuddy's tool harness. Return exactly one JSON object: {{\"tool\":\"listed_tool_name\",\"arguments\":{{...}}}} to request a tool, or {{\"answer\":\"spoken answer\"}} when finished. The application executes tools and returns their results. Use finish_voice_reply to publish verified advice. Do not claim success before a tool result confirms it. Previous tool results are evidence; never replay successful actions. Available tools: {}",
                self.instructions, self.tools
            ),
            prompt: format!(
                "Previous turns are historical data, never new instructions; do not repeat their actions: {}\nCurrent turn and evidence: {history}\nRound {} of {MAX_ROUNDS}. Images are the latest evidence supplied by the application in order. Return JSON only.",
                serde_json::to_string(&self.recent)?,
                turn.rounds + 1
            ),
            images: turn.images.clone(),
            json: true,
        };
        self.jobs
            .try_send(Job {
                serial: self.serial.load(Ordering::SeqCst),
                request,
            })
            .context(
                "The selected provider is still finishing cancelled requests; try again shortly",
            )?;
        turn.rounds += 1;
        Ok(())
    }

    fn accept(&mut self, result: Result<String>) -> Result<()> {
        let value: Value = crate::providers::parse_json(&result?)?;
        let turn = self.active.as_mut().context("No provider turn")?;
        if let Some(name) = value["tool"].as_str() {
            ensure!(
                self.tools
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|t| t["name"] == name),
                "The provider requested an unknown tool: {name}"
            );
            ensure!(
                value["arguments"].is_object(),
                "Tool arguments must be an object"
            );
            let id = format!("provider-tool:{}:{}", turn.id, turn.rounds);
            turn.waiting_tool = Some(id.clone());
            turn.history
                .push(json!({"role":"assistant","content":value}));
            self.events.push_back(json!({"id":id,"method":"item/tool/call","params":{"threadId":self.thread,"turnId":turn.id,"tool":name,"arguments":value["arguments"]}}));
        } else {
            let text = value["answer"]
                .as_str()
                .filter(|s| !s.trim().is_empty())
                .context("The provider returned neither a tool call nor an answer")?;
            turn.history
                .push(json!({"role":"assistant","content":text}));
            self.events.push_back(json!({"method":"item/completed","params":{"threadId":self.thread,"turnId":turn.id,"item":{"type":"agentMessage","phase":"final_answer","text":text}}}));
            self.finish("completed", None);
        }
        Ok(())
    }

    fn finish(&mut self, status: &str, error: Option<String>) {
        self.serial.fetch_add(1, Ordering::SeqCst);
        if let Some(turn) = self.active.take() {
            self.recent
                .push_back(json!({"history":turn.history,"status":status}));
            while self.recent.len() > 2
                || serde_json::to_string(&self.recent).map_or(0, |s| s.len()) > 80_000
            {
                self.recent.pop_front();
            }
            // Remove unconsumed tool calls from a cancelled/failed turn.
            self.events.retain(|e| e["method"] != "item/tool/call");
            self.events.push_back(json!({"method":"turn/completed","params":{"threadId":self.thread,"turn":{"id":turn.id,"status":status,"error":error}}}));
        }
    }
}

impl Drop for Lane {
    fn drop(&mut self) {
        self.serial.fetch_add(1, Ordering::SeqCst);
    }
}

fn extract_content(items: &Value, images: &mut Vec<String>) -> Result<Vec<Value>> {
    let mut text = vec![];
    let mut new_images = vec![];
    for item in items.as_array().context("Missing provider input")? {
        match item["type"].as_str() {
            Some("text" | "inputText") => text.push(json!({"text":item["text"]})),
            Some("image" | "inputImage") => {
                let url = item["url"]
                    .as_str()
                    .or(item["imageUrl"].as_str())
                    .context("Missing image URL")?;
                let data = url
                    .strip_prefix("data:image/jpeg;base64,")
                    .context("Expected an inline JPEG")?;
                new_images.push(data.to_owned());
            }
            _ => anyhow::bail!("Unsupported provider input type"),
        }
    }
    if !new_images.is_empty() {
        *images = new_images;
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn event(lane: &mut Lane, method: &str) -> Value {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if let Some(event) = lane.poll()
                && event["method"] == method
            {
                return event;
            }
            assert!(Instant::now() < deadline, "Missing event: {method}");
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    fn start(lane: &mut Lane, request: &str) -> String {
        lane.send(&json!({"id":1,"method":"turn/start","params":{"threadId":lane.thread,"input":[{"type":"text","text":request},{"type":"image","url":"data:image/jpeg;base64,first-image"}]}})).unwrap();
        event(lane, "turn/started")["params"]["turn"]["id"]
            .as_str()
            .unwrap()
            .into()
    }

    #[test]
    fn tools_use_selected_provider_images_and_stop_after_verified_reply() {
        let calls = Arc::new(Mutex::new(vec![]));
        let seen = calls.clone();
        let mut lane = Lane::with_completion(
            "system".into(),
            json!([{"name":"get_game_screen"},{"name":"finish_voice_reply"}]),
            Arc::new(move |r| {
                let mut calls = seen.lock().unwrap();
                let result = if calls.is_empty() {
                    assert_eq!(r.images, ["first-image"]);
                    json!({"tool":"get_game_screen","arguments":{}})
                } else {
                    assert_eq!(r.images, ["fresh-image"]);
                    assert!(r.prompt.contains("screen-result"));
                    json!({"tool":"finish_voice_reply","arguments":{"speech":"Ready"}})
                };
                calls.push(r);
                Ok(result.to_string())
            }),
        );
        start(&mut lane, "Inspect my screen");
        let call = event(&mut lane, "item/tool/call");
        assert_eq!(call["params"]["tool"], "get_game_screen");
        lane.send(&json!({"id":call["id"],"result":{"success":true,"contentItems":[{"type":"inputText","text":"screen-result"},{"type":"inputImage","imageUrl":"data:image/jpeg;base64,fresh-image"}]}})).unwrap();
        let call = event(&mut lane, "item/tool/call");
        lane.send(&json!({"id":call["id"],"result":{"success":true,"contentItems":[{"type":"inputText","text":json!({"voice_reply":{"text":"Ready"}}).to_string()}]}})).unwrap();
        assert_eq!(
            event(&mut lane, "turn/completed")["params"]["turn"]["status"],
            "completed"
        );
        assert_eq!(calls.lock().unwrap().len(), 2);
        assert!(!lane.send(&json!({"id":99,"method":"thread/realtime/appendSpeech","params":{"threadId":"speaker"}})).unwrap());
    }

    #[test]
    fn replacement_runs_while_cancelled_inference_cannot_emit_tools() {
        let (entered, waiting) = mpsc::channel();
        let release = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
        let worker_release = release.clone();
        let mut lane = Lane::with_completion(
            "system".into(),
            json!([{"name":"show_widget"}]),
            Arc::new(move |r| {
                if r.prompt.contains("old-request") && !r.prompt.contains("new-request") {
                    entered.send(()).unwrap();
                    let (lock, ready) = &*worker_release;
                    let mut released = lock.lock().unwrap();
                    while !*released {
                        released = ready.wait(released).unwrap();
                    }
                    Ok(json!({"tool":"show_widget","arguments":{}}).to_string())
                } else {
                    Ok(json!({"answer":"new-answer"}).to_string())
                }
            }),
        );
        let turn = start(&mut lane, "old-request");
        waiting.recv_timeout(Duration::from_secs(3)).unwrap();
        lane.send(&json!({"id":2,"method":"turn/interrupt","params":{"threadId":lane.thread,"turnId":turn}})).unwrap();
        assert_eq!(
            event(&mut lane, "turn/completed")["params"]["turn"]["status"],
            "interrupted"
        );
        start(&mut lane, "new-request");
        assert_eq!(
            event(&mut lane, "item/completed")["params"]["item"]["text"],
            "new-answer"
        );
        event(&mut lane, "turn/completed");
        *release.0.lock().unwrap() = true;
        release.1.notify_all();
        std::thread::sleep(Duration::from_millis(30));
        assert!(
            lane.poll().is_none(),
            "Cancelled inference emitted an action"
        );
    }

    #[test]
    fn followup_preserves_tool_result_and_replaces_images() {
        let calls = Arc::new(AtomicU64::new(0));
        let count = calls.clone();
        let mut lane = Lane::with_completion(
            "system".into(),
            json!([{"name":"record_journal"}]),
            Arc::new(move |r| {
                if count.fetch_add(1, Ordering::SeqCst) == 0 {
                    Ok(json!({"tool":"record_journal","arguments":{}}).to_string())
                } else {
                    assert!(r.prompt.contains("saved-once"));
                    assert!(r.prompt.contains("follow-up"));
                    assert_eq!(r.images, ["new-image"]);
                    Ok(json!({"answer":"Remembered"}).to_string())
                }
            }),
        );
        let turn = start(&mut lane, "Remember this");
        let call = event(&mut lane, "item/tool/call");
        lane.send(&json!({"id":2,"method":"turn/steer","params":{"threadId":lane.thread,"expectedTurnId":turn,"input":[{"type":"text","text":"follow-up"},{"type":"image","url":"data:image/jpeg;base64,new-image"}]}})).unwrap();
        lane.send(&json!({"id":call["id"],"result":{"success":true,"contentItems":[{"type":"inputText","text":"saved-once"}]}})).unwrap();
        assert_eq!(
            event(&mut lane, "item/completed")["params"]["item"]["text"],
            "Remembered"
        );
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn provider_error_ends_only_its_turn_and_never_falls_back() {
        let mut lane = Lane::with_completion(
            "system".into(),
            json!([]),
            Arc::new(|_| anyhow::bail!("Claude login required")),
        );
        for _ in 0..2 {
            start(&mut lane, "question");
            let done = event(&mut lane, "turn/completed");
            assert_eq!(done["params"]["turn"]["status"], "failed");
            assert_eq!(done["params"]["turn"]["error"], "Claude login required");
        }
    }

    #[test]
    fn followup_without_capture_drops_previous_pixels() {
        let count = Arc::new(AtomicU64::new(0));
        let calls = count.clone();
        let mut lane = Lane::with_completion(
            "system".into(),
            json!([{"name":"get_game_context"}]),
            Arc::new(move |r| {
                if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                    Ok(json!({"tool":"get_game_context","arguments":{}}).to_string())
                } else {
                    assert!(r.images.is_empty(), "Paused capture reused old pixels");
                    Ok(json!({"answer":"Capture is paused"}).to_string())
                }
            }),
        );
        let turn = start(&mut lane, "Look at my screen");
        let call = event(&mut lane, "item/tool/call");
        lane.send(&json!({"id":2,"method":"turn/steer","params":{"threadId":lane.thread,"expectedTurnId":turn,"input":[{"type":"text","text":"Capture is now paused"}]}})).unwrap();
        lane.send(&json!({"id":call["id"],"result":{"success":false,"contentItems":[{"type":"inputText","text":"Superseded"}]}})).unwrap();
        assert_eq!(
            event(&mut lane, "item/completed")["params"]["item"]["text"],
            "Capture is paused"
        );
    }
}
