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
        #[qproperty(QString, event_json)]
        type RealtimeVoice = super::RealtimeVoiceRust;
        #[qinvokable]
        fn page_url(self: Pin<&mut RealtimeVoice>) -> QString;
        #[qinvokable]
        fn connect_offer(
            self: Pin<&mut RealtimeVoice>,
            offer: QString,
            directory: QString,
            context: QString,
        );
        #[qinvokable]
        fn speak(self: Pin<&mut RealtimeVoice>, text: QString);
        #[qinvokable]
        fn speak_prepared(self: Pin<&mut RealtimeVoice>, text: QString, visual: bool) -> bool;
        #[qinvokable]
        fn prepare_speech(self: Pin<&mut RealtimeVoice>, texts: QString);
        #[qinvokable]
        fn cancel_speech(self: Pin<&mut RealtimeVoice>);
        #[qinvokable]
        fn check_speech(self: Pin<&mut RealtimeVoice>);
        #[qinvokable]
        fn interrupt_reasoning(self: Pin<&mut RealtimeVoice>);
        #[qinvokable]
        fn record_prepared(self: Pin<&mut RealtimeVoice>, text: QString);
        #[qinvokable]
        fn trace_event(self: Pin<&mut RealtimeVoice>, kind: QString, value: QString);
        #[qinvokable]
        fn next_narration(self: Pin<&mut RealtimeVoice>, context: QString) -> QString;
        #[qinvokable]
        fn advice_spoken(self: Pin<&mut RealtimeVoice>, context: QString);
        #[qinvokable]
        fn detect_speech(self: Pin<&mut RealtimeVoice>, samples: QString) -> bool;
        #[qinvokable]
        fn reset_speech(self: Pin<&mut RealtimeVoice>);
        #[qinvokable]
        fn player_speech(self: Pin<&mut RealtimeVoice>, active: bool);
        #[qinvokable]
        fn recognize_input(self: Pin<&mut RealtimeVoice>, text: QString, complete: bool);
        #[qinvokable]
        fn delegate_request(self: Pin<&mut RealtimeVoice>, delegation: QString);
        #[qinvokable]
        fn disconnect(self: Pin<&mut RealtimeVoice>);
    }
    impl cxx_qt::Threading for RealtimeVoice {}
}
use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::QString;
use gamebuddy_services::{Store, conversation, subscription::CodexClient};
use serde_json::{Value, json};
use std::{
    pin::Pin,
    sync::mpsc,
    time::{Duration, Instant},
};

#[derive(Default)]
pub struct RealtimeVoiceRust {
    status: QString,
    event_json: QString,
    generation: u64,
    commands: Option<mpsc::Sender<Command>>,
    narrator: gamebuddy_services::voice::narration::Narrator,
    page_server: Option<std::sync::Arc<()>>,
    activity: gamebuddy_services::voice::Segmenter,
    input: std::sync::Arc<std::sync::Mutex<gamebuddy_services::voice::input::PlayerInput>>,
    prepared_speech: Option<gamebuddy_services::voice::speech::Speech>,
    speech_sequence: u64,
    root: Option<std::path::PathBuf>,
    speech_scope: Option<std::sync::Arc<gamebuddy_services::voice::prepared::PlaybackGuard>>,
    speech_check_running: bool,
}
enum Command {
    Urgent,
    Spoken(String),
    Speech(String),
    Diagnostic(String, Value),
    Delegate(String, u64, String),
}

impl ffi::RealtimeVoice {
    pub fn trace_event(self: Pin<&mut Self>, kind: QString, value: QString) {
        let kind = kind.to_string();
        // This allowlist deliberately excludes transcripts, speech-ready and PCM.
        if [
            "capture",
            "input-active",
            "output-active",
            "reply-pending",
            "cue-timeout",
            "connection",
            "ready",
            "work-pending",
        ]
        .contains(&kind.as_str())
            && let Ok(value) = serde_json::from_str::<Value>(&value.to_string())
            && (value.is_boolean() || value.as_str().is_some_and(|s| s.len() <= 32))
            && let Some(tx) = &self.rust().commands
        {
            let _ = tx.send(Command::Diagnostic(kind, value));
        }
    }
    pub fn player_speech(self: Pin<&mut Self>, active: bool) {
        self.rust()
            .input
            .lock()
            .unwrap()
            .activity(active, Instant::now());
    }
    pub fn record_prepared(self: Pin<&mut Self>, text: QString) {
        if let Some(tx) = &self.rust().commands {
            let _ = tx.send(Command::Spoken(text.to_string()));
        }
    }
    pub fn interrupt_reasoning(self: Pin<&mut Self>) {
        if let Some(tx) = &self.rust().commands {
            let _ = tx.send(Command::Urgent);
        }
    }
    pub fn cancel_speech(mut self: Pin<&mut Self>) {
        self.as_mut().rust_mut().speech_sequence += 1;
        self.as_mut().rust_mut().speech_scope = None;
        if let Some(speech) = &self.rust().prepared_speech {
            speech.cancel();
        }
    }
    pub fn check_speech(mut self: Pin<&mut Self>) {
        if self.rust().speech_check_running {
            return;
        }
        let Some(scope) = self.rust().speech_scope.clone() else {
            return;
        };
        let Some(root) = self.rust().root.clone() else {
            return;
        };
        let sequence = self.rust().speech_sequence;
        let generation = self.rust().generation;
        self.as_mut().rust_mut().speech_check_running = true;
        let ui = self.qt_thread();
        std::thread::spawn(move || {
            let current = Store::open(root).is_ok_and(|store| scope.current(&store));
            let _ = ui.queue(move |mut object| {
                if object.rust().generation != generation {
                    return;
                }
                object.as_mut().rust_mut().speech_check_running = false;
                if object.rust().speech_sequence == sequence && !current {
                    object.as_mut().rust_mut().speech_scope = None;
                    object.as_mut().set_event_json(QString::from(
                        json!({"method":"gamebuddy/speech-expired","params":{}}).to_string(),
                    ));
                }
            });
        });
    }
    pub fn prepare_speech(self: Pin<&mut Self>, texts: QString) {
        if let Some(speech) = &self.rust().prepared_speech
            && let Ok(texts) = serde_json::from_str::<Vec<String>>(&texts.to_string())
        {
            speech.prepare(texts);
        }
    }
    pub fn speak_prepared(mut self: Pin<&mut Self>, text: QString, visual: bool) -> bool {
        // Native speech keeps its conversational delivery. Pixel changes and
        // progress bookkeeping are not reasons to cut it off mid-sentence.
        // The cache's strict evidence guard belongs only to prepared audio.
        if self.rust().prepared_speech.is_none() {
            self.as_mut().rust_mut().speech_scope = None;
            return false;
        }
        self.as_mut().rust_mut().speech_sequence += 1;
        let sequence = self.rust().speech_sequence;
        let generation = self.rust().generation;
        let ui = self.qt_thread();
        let root = self.rust().root.clone();
        let guard = root
            .as_ref()
            .and_then(|root| Store::open(root).ok())
            .and_then(|store| {
                gamebuddy_services::voice::prepared::PlaybackGuard::capture(&store, visual).ok()
            })
            .map(std::sync::Arc::new);
        self.as_mut().rust_mut().speech_scope = guard.clone();
        if guard.is_none() {
            self.as_mut().set_event_json(QString::from(
                json!({"method":"gamebuddy/speech-expired","params":{}}).to_string(),
            ));
            return true;
        }
        let text = text.to_string();
        self.rust().prepared_speech.as_ref().unwrap().speak(text.clone(), std::sync::Arc::new(move |index,result| {
            let fresh=guard.as_ref().is_some_and(|guard|root.as_ref().and_then(|root|Store::open(root).ok()).is_some_and(|store|guard.current(&store)));
            let event = if !fresh {json!({"method":"gamebuddy/speech-expired","params":{}})} else {match result {
                Ok(chunk) => json!({"method":"gamebuddy/local-speech","params":{"key":chunk.key,"text":chunk.text,"index":chunk.index,"last":chunk.last,"answer":chunk.answer,"request":format!("{generation}:{sequence}")}}),
                Err(error) => json!({"method":"gamebuddy/local-speech-error","params":{"text":text,"index":index,"error":error.to_string()}}),
            }}.to_string();
            let _ = ui.queue(move |mut object| {
                if object.rust().generation == generation && object.rust().speech_sequence == sequence {
                    object.as_mut().set_event_json(QString::from(event));
                }
            });
        }));
        true
    }
    pub fn recognize_input(self: Pin<&mut Self>, text: QString, complete: bool) {
        let text = text.to_string();
        let mut input = self.rust().input.lock().unwrap();
        if complete {
            input.finish(&text, Instant::now());
        } else {
            input.append(&text, Instant::now());
        }
        if let Some(tx) = &self.rust().commands {
            let _ = tx.send(Command::Diagnostic(
                "recognized-input".into(),
                json!({"text_bytes":text.len(),"complete":complete,"utterance":input.utterance()}),
            ));
        }
    }
    pub fn delegate_request(self: Pin<&mut Self>, delegation: QString) {
        let Ok(value) = serde_json::from_str::<Value>(&delegation.to_string()) else {
            return;
        };
        let Some(id) = value["id"].as_str().filter(|id| !id.is_empty()) else {
            return;
        };
        let input = self.rust().input.lock().unwrap();
        if let Some(tx) = &self.rust().commands {
            let _ = tx.send(Command::Delegate(
                id.to_owned(),
                input.utterance(),
                input.text(Instant::now()),
            ));
        }
    }
    pub fn detect_speech(mut self: Pin<&mut Self>, samples: QString) -> bool {
        if let Ok(samples) = serde_json::from_str::<Vec<i16>>(&samples.to_string())
            && samples.len() == 320
        {
            let _ = self.as_mut().rust_mut().activity.feed(&samples);
            return self.rust().activity.live_speaking();
        }
        false
    }
    pub fn reset_speech(mut self: Pin<&mut Self>) {
        self.as_mut().rust_mut().activity = Default::default();
        self.rust()
            .input
            .lock()
            .unwrap()
            .activity(false, Instant::now());
    }
    pub fn page_url(mut self: Pin<&mut Self>) -> QString {
        // Give Chromium a real localhost origin with narrowly scoped capture
        // permission, rather than granting it to generic HTML/data documents.
        let Ok(server) = tiny_http::Server::http("127.0.0.1:0") else {
            return QString::default();
        };
        let url = format!("http://{}/voice", server.server_addr());
        let guard = std::sync::Arc::new(());
        let alive = std::sync::Arc::downgrade(&guard);
        self.as_mut().rust_mut().page_server = Some(guard);
        std::thread::spawn(move || {
            while alive.upgrade().is_some() {
                if let Ok(Some(request)) = server.recv_timeout(Duration::from_millis(100)) {
                    let response = if request.url() == "/voice"
                        && request.method() == &tiny_http::Method::Get
                    {
                        tiny_http::Response::from_string(include_str!("../assets/voice.html"))
                            .with_header(
                                tiny_http::Header::from_bytes(
                                    "Content-Type",
                                    "text/html; charset=utf-8",
                                )
                                .unwrap(),
                            )
                            .with_header(
                                tiny_http::Header::from_bytes("Cache-Control", "no-store").unwrap(),
                            )
                    } else if request.method() == &tiny_http::Method::Get
                        && let Some(key) = request
                            .url()
                            .strip_prefix("/speech/")
                            .and_then(|s| s.strip_suffix(".wav"))
                        && let Some(wav) = gamebuddy_services::voice::speech::cached(key)
                    {
                        tiny_http::Response::from_data(wav.as_ref().clone()).with_header(
                            tiny_http::Header::from_bytes("Content-Type", "audio/wav").unwrap(),
                        )
                    } else {
                        tiny_http::Response::from_string("Not found").with_status_code(404)
                    };
                    let _ = request.respond(response);
                }
            }
        });
        QString::from(url)
    }
    pub fn disconnect(mut self: Pin<&mut Self>) {
        self.as_mut().rust_mut().commands = None;
        self.as_mut().rust_mut().prepared_speech = None;
        self.as_mut().rust_mut().root = None;
        self.as_mut().rust_mut().speech_scope = None;
        self.as_mut().rust_mut().speech_check_running = false;
        self.as_mut().rust_mut().speech_sequence += 1;
        self.as_mut().rust_mut().generation += 1;
        self.set_status(QString::from("Voice disconnected"));
    }
    pub fn speak(self: Pin<&mut Self>, text: QString) {
        if let Some(tx) = &self.rust().commands {
            let _ = tx.send(Command::Speech(text.to_string()));
        }
    }
    pub fn advice_spoken(mut self: Pin<&mut Self>, context: QString) {
        if let Ok(context) = serde_json::from_str(&context.to_string()) {
            self.as_mut()
                .rust_mut()
                .narrator
                .heard_advice(&context, Instant::now());
        }
    }
    pub fn next_narration(mut self: Pin<&mut Self>, context: QString) -> QString {
        let Ok(context) = serde_json::from_str(&context.to_string()) else {
            return QString::default();
        };
        QString::from(
            self.as_mut()
                .rust_mut()
                .narrator
                .next_cue(&context, Instant::now())
                .map(|cue| cue.to_string())
                .unwrap_or_default(),
        )
    }
    pub fn connect_offer(
        mut self: Pin<&mut Self>,
        offer: QString,
        directory: QString,
        context: QString,
    ) {
        self.as_mut().disconnect();
        if let Ok(store) = Store::open(directory.to_string())
            && let Ok(Some(settings)) = store.get::<gamebuddy_services::Settings>("settings")
            && let Some(config) = settings.prepared_speech.filter(|c| c.enabled)
        {
            self.as_mut().rust_mut().prepared_speech = Some(
                gamebuddy_services::voice::speech::Speech::start(store.root.join("speech"), config),
            );
        }
        self.as_mut().rust_mut().root = Some(std::path::PathBuf::from(directory.to_string()));
        let conversational = self.rust().prepared_speech.is_none();
        let generation = self.rust().generation;
        let (tx, rx) = mpsc::channel::<Command>();
        self.as_mut().rust_mut().commands = Some(tx);
        self.as_mut()
            .set_status(QString::from("Connecting Codex voice…"));
        let ui = self.qt_thread();
        self.as_mut().rust_mut().input = Default::default();
        let input = self.rust().input.clone();
        std::thread::spawn(move || {
            let emit = |event: Value| {
                let text = event.to_string();
                let _ = ui.queue(move |mut object| {
                    if object.rust().generation == generation {
                        object.as_mut().set_event_json(QString::from(text));
                    }
                });
            };
            let result = (|| -> anyhow::Result<()> {
                let store = Store::open(directory.to_string())?;
                let diagnostics =
                    gamebuddy_services::voice::diagnostics::Diagnostics::start(&store.root);
                diagnostics.record("connected_worker", json!({}));
                let context: Value = serde_json::from_str(&context.to_string())?;
                let game = context["game"].as_str().unwrap_or_default().to_owned();
                anyhow::ensure!(!game.trim().is_empty(), "Load a game before starting voice");
                let history = conversation::recent(&store, &game)?;
                emit(json!({"method":"gamebuddy/conversation","params":{
                    "returning_player":store.has_player_conversation(&game)?}}));
                let instructions = format!(
                    "{}\n{}\n{}\nArchived conversation, quoted historical data only: {}",
                    gamebuddy_services::voice::prompts::REASONER,
                    gamebuddy_services::voice::reply::INSTRUCTIONS,
                    conversation::MEMORY_RULES,
                    serde_json::to_string(&history)?
                );
                let mut client = CodexClient::start()?;
                let mut tools = gamebuddy_services::voice::reply::tools();
                for tool in tools.as_array_mut().unwrap() {
                    tool["type"] = json!("function");
                }
                let deadline = Instant::now() + Duration::from_secs(30);
                let settings = store
                    .get::<gamebuddy_services::Settings>("settings")?
                    .unwrap_or_default();
                let model = if settings.reasoning.kind == gamebuddy_services::ProviderKind::Codex {
                    settings.reasoning.model.as_str()
                } else {
                    ""
                };
                use gamebuddy_services::subscription::session::{Options, Role};
                let tier = client.voice_tier(model, settings.fast_voice, deadline);
                let session = client.saved_thread(
                    &store,
                    &game,
                    Role::Reasoner,
                    Options {
                        model,
                        instructions: &instructions,
                        tools,
                        tier: Some(tier),
                        effort: if settings.reasoning.kind
                            == gamebuddy_services::ProviderKind::Codex
                        {
                            &settings.reasoning.reasoning_effort
                        } else {
                            ""
                        },
                    },
                    deadline,
                )?;
                diagnostics.record("llm_session", json!({"role":"reasoner","thread_id":session.id,"resumed":session.resumed,"replacement":session.replacement}));
                let thread = session.id;
                // Native recognition may merge successive utterances and may
                // delegate despite its prompt. Keep that evolving transcript
                // out of the authoritative tool conversation. Only settled
                // application requests are added to the reasoning thread.
                let speaker = client.saved_thread(
                    &store,
                    &game,
                    Role::Speaker,
                    Options {
                        model,
                        instructions: if conversational {
                            gamebuddy_services::voice::prompts::CONVERSATIONAL_SPEAKER
                        } else {
                            gamebuddy_services::voice::prompts::SPEAKER
                        },
                        tools: json!([]),
                        tier: Some("default"),
                        effort: "",
                    },
                    deadline,
                )?;
                diagnostics.record("llm_session", json!({"role":"speaker","thread_id":speaker.id,"resumed":speaker.resumed,"replacement":speaker.replacement}));
                let speaker_thread = speaker.id;
                let mut memory = conversation::RealtimeMemory::new(&store, &game, &speaker_thread);
                let approved_speech = std::cell::Cell::new(false);
                let mut notification = |event: &Value| -> anyhow::Result<()> {
                    if std::env::args().any(|a| a == "--voice-trace") {
                        eprintln!(
                            "GAMEBUDDY_VOICE_NOTIFICATION method={} at_ms={} role={} status={}",
                            event["method"].as_str().unwrap_or_default(),
                            std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_millis(),
                            event["params"]["role"].as_str().unwrap_or_default(),
                            event["params"]["turn"]["status"]
                                .as_str()
                                .unwrap_or_default()
                        );
                    }
                    if matches!(
                        event["method"].as_str(),
                        Some(
                            "turn/started"
                                | "turn/completed"
                                | "thread/realtime/closed"
                                | "thread/realtime/error"
                                | "thread/realtime/transcript/done"
                        )
                    ) {
                        diagnostics.record(event["method"].as_str().unwrap_or_default(), json!({"role":event["params"]["role"],"status":event["params"]["turn"]["status"],"text_bytes":event["params"]["text"].as_str().map(str::len)}));
                    }
                    // The model can generate an answer before delegation finishes.
                    // Locally discarded audio is not something the player heard.
                    if conversational
                        || event["params"]["role"] != "assistant"
                        || approved_speech.get()
                    {
                        memory.record(event)?;
                    }
                    if event["method"] == "thread/realtime/transcript/done"
                        && event["params"]["role"] == "assistant"
                    {
                        approved_speech.set(false);
                    }
                    // The browser delivers recognition directly to shared
                    // input state. Reading the duplicate app-server transcript
                    // here delays recognition behind tools and can split or
                    // repeat an already dispatched request.
                    if matches!(
                        event["method"].as_str(),
                        Some("turn/started" | "turn/completed")
                    ) && event["params"]["threadId"] == thread
                    {
                        emit(
                            json!({"method":"gamebuddy/reasoning","params":{"active":event["method"]=="turn/started"}}),
                        );
                    }
                    if event["method"]
                        .as_str()
                        .is_some_and(|s| s.starts_with("thread/realtime/"))
                    {
                        emit(event.clone());
                    }
                    Ok(())
                };
                emit(
                    json!({"method":"gamebuddy/voice-mode","params":{"conversational":conversational}}),
                );
                client.request_with_notifications("thread/realtime/start", json!({"threadId":speaker_thread,"outputModality":"audio","version":"v3","delegationAckFiller":false,"clientManagedHandoffs":true,"codexResponseHandoffMode":"bemTags","includeStartupContext":false,"prompt":if conversational {gamebuddy_services::voice::prompts::CONVERSATIONAL_SPEAKER} else {gamebuddy_services::voice::prompts::SPEAKER},"initialItems":conversation::initial_items(&context,&history),"transport":{"type":"webrtc","sdp":offer.to_string()}}), deadline, &mut notification)?;
                let mut decision_settings = settings.clone();
                decision_settings.reasoning.model = model.into();
                if settings.reasoning.kind != gamebuddy_services::ProviderKind::Codex {
                    decision_settings.reasoning.reasoning_effort.clear();
                }
                let mut decision =
                    gamebuddy_services::voice::decision::Lane::start(decision_settings);
                let mut decision_ready: Option<gamebuddy_services::voice::decision::Proposal> =
                    None;
                let mut speculation_budget = (0u64, 0u8);
                let mut last_speculation = Instant::now();
                let mut sequence = 1000;
                let mut context_requests = std::collections::HashMap::<
                    u64,
                    (
                        Value,
                        u64,
                        String,
                        Option<gamebuddy_services::screen::Snapshot>,
                    ),
                >::new();
                let mut refused_context: Option<(Value, u64, String)> = None;
                let mut start_requests = std::collections::HashMap::new();
                let mut prepared_context: Option<(
                    Value,
                    u64,
                    String,
                    Option<gamebuddy_services::screen::Snapshot>,
                )> = None;
                let mut reasoning_since: Option<Instant> = None;
                let mut interrupt_requests = std::collections::HashMap::new();
                let mut context_turn = None;
                let mut reasoning_turn: Option<Value> = None;
                let mut reasoning_input = None;
                let mut supplied_request = String::new();
                let mut final_answer = String::new();
                let mut supplied_frame = None;
                let mut last_frame_check = Instant::now();
                let mut refreshed_view = false;
                let mut spatial_advice = false;
                let mut stale_retries = (0u64, 0u8);
                let mut delivery = gamebuddy_services::voice::delivery::Delivery::default();
                let mut response_step: Option<(u64, String)> = None;
                let mut discussed_step: Option<String> = None;
                let mut live_requests = gamebuddy_services::voice::live::Requests::default();
                let mut responsiveness = gamebuddy_services::voice::live::Responsiveness::default();
                let mut listener_interrupts = std::collections::HashSet::new();
                let mut pending_tools = std::collections::VecDeque::<(Value, bool)>::new();
                loop {
                    match rx.recv_timeout(Duration::from_millis(20)) {
                        Ok(command) => {
                            sequence += 1;
                            let (method, params) = match command {
                                Command::Delegate(id, utterance, request) => {
                                    if conversational {
                                        live_requests.delegate(id, utterance, request);
                                        diagnostics.record(
                                            "native_delegation",
                                            json!({"utterance":utterance}),
                                        );
                                    }
                                    continue;
                                }
                                Command::Spoken(text) => {
                                    store.append_conversation(
                                        &game,
                                        &thread,
                                        &format!("local-{sequence}"),
                                        "assistant",
                                        &text,
                                    )?;
                                    continue;
                                }
                                Command::Urgent => {
                                    decision.cancel();
                                    decision_ready = None;
                                    delivery.abandon(if conversational {
                                        live_requests.utterance()
                                    } else {
                                        input.lock().unwrap().utterance()
                                    });
                                    if let Some(turn_id) = reasoning_turn.take() {
                                        interrupt_requests.insert(sequence, turn_id.clone());
                                        client.send(json!({"id":sequence,"method":"turn/interrupt","params":{"threadId":thread,"turnId":turn_id}}))?;
                                    }
                                    reasoning_input = None;
                                    reasoning_since = None;
                                    context_turn = None;
                                    for (tool, _) in pending_tools.drain(..) {
                                        client.send(json!({"id":tool["id"],"result":{"success":false,"contentItems":[{"type":"inputText","text":"A new urgent screen event superseded this action; it was not executed."}]}}))?;
                                    }
                                    emit(
                                        json!({"method":"gamebuddy/reasoning","params":{"active":false}}),
                                    );
                                    continue;
                                }
                                Command::Diagnostic(kind, value) => {
                                    if kind == "output-active" && value == true {
                                        responsiveness.output(Instant::now());
                                    }
                                    diagnostics.record(&format!("browser/{kind}"), value);
                                    continue;
                                }
                                Command::Speech(text) => {
                                    approved_speech.set(true);
                                    (
                                        "thread/realtime/appendSpeech",
                                        json!({"threadId":speaker_thread,"text":text}),
                                    )
                                }
                            };
                            client.send(json!({"id":sequence,"method":method,"params":params}))?;
                        }
                        Err(mpsc::RecvTimeoutError::Disconnected) => {
                            client.request_with_notifications(
                                "thread/realtime/stop",
                                json!({"threadId":speaker_thread}),
                                Instant::now() + Duration::from_secs(3),
                                &mut notification,
                            )?;
                            return Ok(());
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                    }
                    while let Some(event) = client.poll()? {
                        if event["id"]
                            .as_u64()
                            .is_some_and(|id| listener_interrupts.remove(&id))
                        {
                            // A listener delegation can finish before its cancel
                            // reaches Codex. That race must not end live voice.
                            continue;
                        }
                        if event["params"]["threadId"] == speaker_thread
                            && !event["method"]
                                .as_str()
                                .is_some_and(|m| m.starts_with("thread/realtime/"))
                        {
                            if event["method"] == "turn/started" {
                                sequence += 1;
                                listener_interrupts.insert(sequence);
                                client.send(json!({"id":sequence,"method":"turn/interrupt","params":{"threadId":speaker_thread,"turnId":event["params"]["turn"]["id"]}}))?;
                            }
                            if event.get("id").is_some() && event.get("method").is_some() {
                                client.reject(&event)?;
                            }
                            continue;
                        }
                        if event["method"] == "turn/started" {
                            let unsolicited = start_requests.is_empty()
                                && !prepared_context
                                    .as_ref()
                                    .is_some_and(|(id, ..)| *id == event["params"]["turn"]["id"]);
                            reasoning_since = Some(Instant::now());
                            context_turn = Some(event["params"]["turn"]["id"].clone());
                            reasoning_turn = context_turn.clone();
                            reasoning_input = None;
                            supplied_request.clear();
                            final_answer.clear();
                            supplied_frame = None;
                            refreshed_view = false;
                            spatial_advice = false;
                            refused_context = None;
                            if unsolicited {
                                // The live speaker can still delegate despite
                                // its prompt, sometimes on the first few words.
                                // Only the application's complete transcript
                                // owns a reasoning turn. Do not spend the rest
                                // of the utterance steering a partial request.
                                sequence += 1;
                                let turn_id = event["params"]["turn"]["id"].clone();
                                interrupt_requests.insert(sequence, turn_id.clone());
                                client.send(json!({"id":sequence,"method":"turn/interrupt","params":{"threadId":thread,"turnId":turn_id}}))?;
                                context_turn = None;
                                if std::env::args().any(|a| a == "--voice-trace") {
                                    eprintln!("GAMEBUDDY_VOICE_EARLY_DELEGATION_CANCELLED");
                                }
                                continue;
                            }
                            if prepared_context
                                .as_ref()
                                .is_some_and(|(id, ..)| Some(id) == reasoning_turn.as_ref())
                            {
                                let (_, utterance, request, frame) =
                                    prepared_context.take().unwrap();
                                reasoning_input = Some(utterance);
                                supplied_request = request;
                                supplied_frame = frame;
                                last_frame_check = Instant::now();
                            }
                        }
                        if event["method"] == "item/completed"
                            && event["params"]["item"]["type"] == "agentMessage"
                            && event["params"]["item"]["phase"] != "commentary"
                        {
                            final_answer = event["params"]["item"]["text"]
                                .as_str()
                                .unwrap_or_default()
                                .trim()
                                .to_owned();
                        }
                        if event["method"] == "turn/completed" {
                            let current_input = input.lock().unwrap();
                            let utterance = if conversational {
                                live_requests.snapshot(&current_input, Instant::now()).2
                            } else {
                                current_input.utterance()
                            };
                            if event["params"]["turn"]["status"] == "completed"
                                && !delivery.has_reply(utterance)
                                && reasoning_turn.as_ref() == Some(&event["params"]["turn"]["id"])
                                && reasoning_input == Some(utterance)
                                && !final_answer.is_empty()
                            {
                                if !spatial_advice
                                    || supplied_frame.as_ref().is_none_or(|frame| {
                                        gamebuddy_services::screen::still_current(&store, frame)
                                            .unwrap_or(false)
                                    })
                                {
                                    delivery.complete(utterance, &supplied_request, &final_answer);
                                } else {
                                    if stale_retries.0 != utterance {
                                        stale_retries = (utterance, 0);
                                    }
                                    diagnostics.record(
                                        "answer_view_changed",
                                        json!({"utterance":utterance}),
                                    );
                                    if stale_retries.1 == 0 {
                                        stale_retries.1 += 1;
                                        delivery.retry(utterance);
                                    } else {
                                        delivery.complete(utterance,&supplied_request,"Your screen changed while I was checking. Pause briefly so I can give directions for where you are now.");
                                    }
                                }
                            } else if !delivery.has_reply(utterance)
                                && reasoning_input == Some(utterance)
                                && reasoning_turn.as_ref() == Some(&event["params"]["turn"]["id"])
                            {
                                delivery.abandon(utterance);
                                emit(
                                    json!({"method":"gamebuddy/reply-error","params":{"text":"I couldn't finish that request. You can still speak to me; please try again."}}),
                                );
                            }
                            drop(current_input);
                            if reasoning_turn.as_ref() == Some(&event["params"]["turn"]["id"]) {
                                reasoning_turn = None;
                                reasoning_input = None;
                                reasoning_since = None;
                            }
                            if context_turn.as_ref() == Some(&event["params"]["turn"]["id"]) {
                                context_turn = None
                            }
                            let completed = &event["params"]["turn"]["id"];
                            for (tool, _) in pending_tools
                                .iter()
                                .filter(|(tool, _)| tool["params"]["turnId"] == *completed)
                            {
                                client.send(json!({"id":tool["id"],"result":{"success":false,"contentItems":[{"type":"inputText","text":"The turn ended before this tool ran; no action was performed."}]}}))?;
                            }
                            pending_tools
                                .retain(|(tool, _)| tool["params"]["turnId"] != *completed);
                        }
                        if let Some((request_utterance, request, frame)) = event["id"]
                            .as_u64()
                            .and_then(|id| start_requests.remove(&id))
                        {
                            // Native delegation can win the same start race.
                            // Its active turn will be steered with the request.
                            if event.get("error").is_some() && reasoning_turn.is_none() {
                                delivery.abandon(request_utterance);
                                reasoning_since = None;
                                emit(
                                    json!({"method":"gamebuddy/reply-error","params":{"text":"The request could not start. You can still speak to me; please try again."}}),
                                );
                            } else if event.get("error").is_none() {
                                let turn_id = event["result"]["turn"]["id"].clone();
                                if delivery.finished(request_utterance) {
                                    sequence += 1;
                                    interrupt_requests.insert(sequence, turn_id.clone());
                                    client.send(json!({"id":sequence,"method":"turn/interrupt","params":{"threadId":thread,"turnId":turn_id}}))?;
                                    continue;
                                }
                                if reasoning_turn.as_ref() == Some(&turn_id) {
                                    reasoning_input = Some(request_utterance);
                                    supplied_request = request;
                                    supplied_frame = frame;
                                    last_frame_check = Instant::now();
                                } else {
                                    prepared_context =
                                        Some((turn_id, request_utterance, request, frame));
                                }
                            }
                            continue;
                        }
                        if let Some(turn_id) = event["id"]
                            .as_u64()
                            .and_then(|id| interrupt_requests.remove(&id))
                        {
                            if reasoning_turn.as_ref() == Some(&turn_id)
                                && reasoning_input.is_none()
                            {
                                reasoning_turn = None;
                                context_turn = None;
                                reasoning_since = None;
                            }
                            if std::env::args().any(|a| a == "--voice-trace") {
                                eprintln!(
                                    "GAMEBUDDY_VOICE_BACKEND_INTERRUPT success={}",
                                    event.get("error").is_none()
                                );
                            }
                            // Completion can race interruption; a finished old
                            // turn is not a failure of the new voice session.
                            continue;
                        }
                        if let Some((turn_id, updated_utterance, updated_request, updated_frame)) =
                            event["id"]
                                .as_u64()
                                .and_then(|id| context_requests.remove(&id))
                        {
                            let accepted = event.get("error").is_none();
                            diagnostics.record(
                                "request_update",
                                json!({"utterance":updated_utterance,"accepted":accepted}),
                            );
                            if reasoning_turn.as_ref() == Some(&turn_id) {
                                if accepted {
                                    // Only an acknowledged update owns the
                                    // result. A rejected/racing steer cannot
                                    // relabel the old answer as the new one.
                                    reasoning_input = Some(updated_utterance);
                                    supplied_request = updated_request;
                                    supplied_frame = updated_frame;
                                    delivery.started(updated_utterance);
                                } else {
                                    refused_context =
                                        Some((turn_id, updated_utterance, updated_request));
                                }
                            }
                            if std::env::args().any(|a| a == "--voice-trace") {
                                eprintln!("GAMEBUDDY_VOICE_CONTEXT_ACCEPTED success={accepted}",);
                            }
                            continue;
                        }
                        if event["method"] == "item/tool/call" {
                            let readonly = event["params"]["tool"].as_str().is_some_and(|name| {
                                name.starts_with("get_")
                                    || matches!(name, "search_guide" | "read_reference")
                            });
                            let reject = interrupt_requests
                                .values()
                                .any(|id| *id == event["params"]["turnId"])
                                || delivery.finished(if conversational {
                                    live_requests.utterance()
                                } else {
                                    input.lock().unwrap().utterance()
                                })
                                || (!readonly
                                    && (!input.lock().unwrap().settled(Instant::now())
                                        || (conversational
                                            && reasoning_input
                                                != Some(live_requests.utterance()))));
                            pending_tools.push_back((event, reject));
                        } else {
                            client.reject(&event)?;
                            if let Some(error) = event.get("error") {
                                anyhow::bail!("Codex voice: {error}");
                            }
                            let ended = event["method"] == "thread/realtime/error"
                                || event["method"] == "thread/realtime/closed";
                            notification(&event)?;
                            if ended {
                                return Ok(());
                            }
                        }
                    }
                    if conversational {
                        let recovery = responsiveness.poll(
                            &input.lock().unwrap(),
                            live_requests.utterance(),
                            Instant::now(),
                        );
                        match recovery {
                            Some(gamebuddy_services::voice::live::Recovery::Presence(text)) => {
                                diagnostics.record("presence_recovery", json!({}));
                                emit(json!({"method":"gamebuddy/presence","params":{"text":text}}));
                            }
                            Some(gamebuddy_services::voice::live::Recovery::Request {
                                utterance,
                                text,
                            }) => {
                                diagnostics.record(
                                    "request_recovered",
                                    json!({"utterance":utterance,"text_bytes":text.len()}),
                                );
                                live_requests.recover(utterance, text);
                            }
                            None => {}
                        }
                    }
                    let (quiet, request, utterance) = {
                        let input = input.lock().unwrap();
                        let now = Instant::now();
                        if conversational {
                            live_requests.snapshot(&input, now)
                        } else {
                            (input.settled(now), input.text(now), input.utterance())
                        }
                    };
                    gamebuddy_services::voice::priority::update(
                        &store.root,
                        !quiet || reasoning_since.is_some(),
                    );
                    // Speculation is read-only and never shortens endpointing.
                    // Any changed wording invalidates its result before effects.
                    if decision
                        .pending
                        .as_ref()
                        .is_some_and(|(u, q)| *u != utterance || q != &request)
                    {
                        decision.cancel();
                        decision_ready = None;
                    }
                    if let Some(proposal) = decision.poll() {
                        if proposal.decision.is_ok() {
                            let mut shared = proposal.evidence.frame.clone();
                            shared.jpeg = proposal.evidence.image.clone();
                            let _ = gamebuddy_services::screen::seen::record(
                                &store,
                                &shared,
                                "assistant",
                            );
                        }
                        decision_ready = Some(proposal);
                    }
                    if quiet
                        && let Some(proposal) = decision_ready.take()
                        && proposal.utterance == utterance
                        && proposal.request == request
                    {
                        let elapsed = proposal.elapsed_ms;
                        let selected = proposal
                            .decision
                            .as_ref()
                            .ok()
                            .and_then(|d| d.step_id.clone());
                        let answer = proposal
                            .decision
                            .and_then(|d| proposal.evidence.apply(&store, d));
                        diagnostics.record("decision_ready",json!({"utterance":utterance,"elapsed_ms":elapsed,"accepted":matches!(&answer,Ok(Some(_)))}));
                        match answer {
                            Ok(Some(answer)) => {
                                response_step = selected.map(|id| (utterance, id));
                                delivery.started(utterance);
                                delivery.complete(utterance, &request, &answer);
                                emit(json!({"method":"gamebuddy/state-changed","params":{}}));
                            }
                            _ => {
                                delivery.retry(utterance);
                            }
                        }
                        decision.cancel();
                    }
                    if speculation_budget.0 != utterance {
                        speculation_budget = (utterance, 0);
                    }
                    if !conversational
                        && !quiet
                        && speculation_budget.1 < 2
                        && last_speculation.elapsed() >= Duration::from_secs(1)
                        && reasoning_since.is_none()
                        && !delivery.finished(utterance)
                        && let Some(preview) = input.lock().unwrap().preview(Instant::now())
                        && gamebuddy_services::voice::decision::eligible_request(&preview)
                        && decision.pending.is_none()
                    {
                        last_speculation = Instant::now();
                        speculation_budget.1 += 1;
                        if decision.propose(&store, &game, utterance, &preview).is_ok() {
                            diagnostics.record("decision_preview", json!({"utterance":utterance}));
                        }
                    }
                    // A failed or missing completion must not strand subsequent
                    // requests. This bounds reasoning only, never speech playback.
                    if reasoning_since
                        .is_some_and(|since| since.elapsed() >= Duration::from_secs(60))
                    {
                        if let Some(turn_id) = reasoning_turn.take() {
                            sequence += 1;
                            interrupt_requests.insert(sequence, turn_id.clone());
                            client.send(json!({"id":sequence,"method":"turn/interrupt","params":{"threadId":thread,"turnId":turn_id}}))?;
                        }
                        context_turn = None;
                        reasoning_input = None;
                        reasoning_since = None;
                        delivery.abandon(utterance);
                        emit(
                            json!({"method":"gamebuddy/reply-error","params":{"text":"That request took too long. You can still speak to me; please try again."}}),
                        );
                    }
                    // Native speech owns audible turn-taking. Return verified
                    // results immediately even during a backchannel, before a
                    // subsequent handoff can orphan a completed answer.
                    if let Some(answer) =
                        delivery.take_reply(utterance, &request, conversational || quiet)
                    {
                        diagnostics.record(
                            "answer_ready",
                            json!({"utterance":utterance,"text_bytes":answer.len()}),
                        );
                        emit(
                            json!({"method":"gamebuddy/speech","params":{"text":answer,"step_id":response_step.as_ref().filter(|(u,_)|*u==utterance).map(|(_,id)|id),"discussed_step_id":discussed_step,"delegation_id":if conversational {live_requests.delegation_id()} else {None}}}),
                        );
                    }
                    // Native follow-ups revise the active turn below, keeping
                    // its evidence and completed actions. Repeated questions
                    // must not keep cancelling slow work before it can answer.
                    // Explicit Stop still cancels; composed speech retains its
                    // existing replacement-turn behavior.
                    if reasoning_input.is_some_and(|prior| prior != utterance)
                        && (!conversational || live_requests.stopped())
                        && let Some(turn_id) = &reasoning_turn
                    {
                        sequence += 1;
                        interrupt_requests.insert(sequence, turn_id.clone());
                        client.send(json!({"id":sequence,"method":"turn/interrupt","params":{"threadId":thread,"turnId":turn_id}}))?;
                        reasoning_input = None;
                        context_turn = None;
                    }
                    if quiet {
                        if conversational
                            && reasoning_input.is_some_and(|prior| prior != utterance)
                            && request == supplied_request
                            && reasoning_turn.is_some()
                        {
                            reasoning_input = Some(utterance);
                            delivery.started(utterance);
                            diagnostics.record("request_repeated", json!({"utterance":utterance}));
                        }
                        // Dispatch every recognized local utterance even when
                        // the live model neither delegates nor answers. Waiting
                        // for an assistant transcript creates a deadlock when
                        // the speaker correctly stays silent for backend approval.
                        if delivery.dispatch(utterance, &request, reasoning_since.is_some()) {
                            // A preview is an optimization only if it finished
                            // before endpointing. Never wait for it and then
                            // start a second model pass after a stale result.
                            decision.cancel();
                            decision_ready = None;
                            response_step = None;
                            discussed_step = None;
                            if let Some(action) =
                                gamebuddy_services::voice::overlay_command(&request)
                            {
                                if std::env::args().any(|a| a == "--voice-trace") {
                                    eprintln!(
                                        "GAMEBUDDY_VOICE_LOCAL action={action} at_ms={}",
                                        gamebuddy_services::screen::now_ms()
                                    );
                                }
                                emit(
                                    json!({"method":"gamebuddy/overlay-action","params":{"action":action}}),
                                );
                                delivery.abandon(utterance);
                                diagnostics.record(
                                    "local_command",
                                    json!({"utterance":utterance,"action":action}),
                                );
                                continue;
                            }
                            if let Some(answer) = gamebuddy_services::voice::quick_reply(&request) {
                                if std::env::args().any(|a| a == "--voice-trace") {
                                    eprintln!(
                                        "GAMEBUDDY_VOICE_LOCAL reply=true at_ms={}",
                                        gamebuddy_services::screen::now_ms()
                                    );
                                }
                                delivery.complete(utterance, &request, answer);
                                diagnostics.record("local_reply", json!({"utterance":utterance}));
                                continue;
                            }
                            if let Ok(Some(answer)) =
                                gamebuddy_services::voice::prepared::reply(&store, &game, &request)
                            {
                                response_step = answer.step_id.map(|id| (utterance, id));
                                delivery.complete(utterance, &request, &answer.text);
                                diagnostics.record(
                                    "prepared_reply",
                                    json!({"utterance":utterance,"kind":answer.kind}),
                                );
                                continue;
                            }
                            if let Ok(Some(answer)) =
                                gamebuddy_services::outcomes::quick_report(&store, &game, &request)
                            {
                                emit(json!({"method":"gamebuddy/state-changed","params":{}}));
                                delivery.complete(utterance, &request, &answer);
                                diagnostics
                                    .record("local_world_report", json!({"utterance":utterance}));
                                continue;
                            }
                            let context_started = Instant::now();
                            let evidence =
                                gamebuddy_services::harness::context_for_request(&store, &request)
                                    .ok()
                                    .filter(|c| {
                                        c.value["game"]
                                            .as_str()
                                            .is_some_and(|g| g.eq_ignore_ascii_case(&game))
                                    });
                            let frame = evidence.as_ref().and_then(|c| c.screen_frame());
                            discussed_step = evidence
                                .as_ref()
                                .and_then(|c| {
                                    c.value["navigation"]["guidance"]["now"]["id"].as_str()
                                })
                                .map(str::to_owned);
                            let label = format!(
                                "CURRENT_GAME_CONTEXT for this request. Complete live player request: {request}. request_id: {}. Inspect the supplied images directly; do not fetch the same context again.",
                                gamebuddy_services::voice::reply::request_id(utterance, &request)
                            );
                            let turn_input = evidence.as_ref().map(|c| c.codex_input(&label))
                                .unwrap_or_else(|| vec![json!({"type":"text","text":format!("Complete live player request: {request}. request_id: {}", gamebuddy_services::voice::reply::request_id(utterance, &request))})]);
                            sequence += 1;
                            start_requests
                                .insert(sequence, (utterance, request.clone(), frame.clone()));
                            reasoning_since = Some(Instant::now());
                            diagnostics.record(
                                "request_dispatched",
                                json!({"utterance":utterance,"text_bytes":request.len()}),
                            );
                            if std::env::args().any(|a| a == "--voice-trace") {
                                eprintln!(
                                    "GAMEBUDDY_VOICE_CONTEXT images={} text_bytes={} initial=true",
                                    evidence.as_ref().map_or(0, |c| c.images.len()),
                                    request.len()
                                );
                                eprintln!(
                                    "GAMEBUDDY_VOICE_DISPATCH utterance={utterance} text_bytes={} images={} context_ms={} at_ms={}",
                                    request.len(),
                                    evidence.as_ref().map_or(0, |c| c.images.len()),
                                    context_started.elapsed().as_millis(),
                                    gamebuddy_services::screen::now_ms()
                                );
                            }
                            client.send(json!({"id":sequence,"method":"turn/start","params":{"threadId":thread,"input":turn_input,"environments":[]}}))?;
                            if let Some(frame) = &frame {
                                let _ = gamebuddy_services::screen::seen::record(
                                    &store,
                                    frame,
                                    "assistant",
                                );
                            }
                        }
                        // Refresh native visual evidence while a long reasoning
                        // turn is in flight. A new camera view supersedes its old
                        // route before a queued instruction can be published.
                        // A continuously moving camera must not keep restarting
                        // reasoning. Allow one automatic refresh per request;
                        // tools still obtain fresh evidence before mutations.
                        let changed_view = !refreshed_view
                            && reasoning_since
                                .is_some_and(|since| since.elapsed() >= Duration::from_secs(3))
                            && last_frame_check.elapsed() >= Duration::from_secs(1)
                            && supplied_frame.as_ref().is_some_and(|frame| {
                                !gamebuddy_services::screen::still_current(&store, frame)
                                    .unwrap_or(false)
                            });
                        if last_frame_check.elapsed() >= Duration::from_secs(1) {
                            last_frame_check = Instant::now();
                        }
                        // Native delegation may begin midway through a request.
                        // Supply the finished utterance and current image before
                        // the next reasoning step, avoiding a context-fetch round
                        // trip. Rejected steers leave normal tools available.
                        if !request.is_empty()
                            && !delivery.finished(utterance)
                            && start_requests.is_empty()
                            && (request != supplied_request || changed_view)
                            && let Some(turn_id) = context_turn.as_ref()
                            && !context_requests.values().any(|(id, ..)| id == turn_id)
                            && refused_context.as_ref()
                                != Some(&(turn_id.clone(), utterance, request.clone()))
                        {
                            refreshed_view |= changed_view;
                            if !supplied_request.is_empty() {
                                for (event, reject) in &mut pending_tools {
                                    let name = event["params"]["tool"].as_str().unwrap_or_default();
                                    let source_reply = name
                                        == gamebuddy_services::voice::reply::TOOL
                                        && serde_json::from_value::<
                                            gamebuddy_services::voice::reply::Reply,
                                        >(
                                            event["params"]["arguments"].clone()
                                        )
                                        .is_ok_and(|r| !r.requires_current_screen());
                                    if !name.starts_with("get_")
                                        && !matches!(name, "search_guide" | "read_reference")
                                        && (request != supplied_request || !source_reply)
                                    {
                                        *reject = true;
                                    }
                                }
                            }
                            let mut updated_frame = supplied_frame.clone();
                            let evidence = if supplied_request.is_empty() || changed_view {
                                gamebuddy_services::harness::context_for_request(&store, &request)
                                    .ok()
                                    .filter(|c| {
                                        c.value["game"]
                                            .as_str()
                                            .is_some_and(|g| g.eq_ignore_ascii_case(&game))
                                    })
                            } else {
                                None
                            };
                            sequence += 1;
                            if let Some(context) = &evidence {
                                updated_frame = context.screen_frame();
                            }
                            context_requests.insert(
                                sequence,
                                (
                                    turn_id.clone(),
                                    utterance,
                                    request.clone(),
                                    updated_frame.clone(),
                                ),
                            );
                            let label = format!(
                                "The latest complete live player request is: {request}. Treat this as a follow-up to the conversation already in this turn. Use evidence and successful actions already obtained; do not repeat completed tools. Answer the latest question and retain earlier requested actions unless the player replaces or cancels them. Resolve any location correction before repeating advice. If a newer image accompanies this update, reassess the changed view before giving directions. request_id: {}.",
                                gamebuddy_services::voice::reply::request_id(utterance, &request)
                            );
                            let turn_input = evidence.as_ref().map(|context| context.codex_input(&format!("CURRENT_GAME_CONTEXT for this request. {label} Inspect these images; refresh if the screen changes.")))
                                .unwrap_or_else(|| vec![json!({"type":"text","text":label})]);
                            if std::env::args().any(|a| a == "--voice-trace") {
                                eprintln!(
                                    "GAMEBUDDY_VOICE_CONTEXT images={} text_bytes={}",
                                    evidence.as_ref().map_or(0, |c| c.images.len()),
                                    request.len()
                                );
                            }
                            diagnostics.record("request_update_sent", json!({"utterance":utterance,"text_bytes":request.len(),"images":evidence.as_ref().map_or(0,|c|c.images.len())}));
                            client.send(json!({"id":sequence,"method":"turn/steer","params":{"threadId":thread,"expectedTurnId":turn_id,"input":turn_input}}))?;
                            if evidence.is_some() {
                                if let Some(frame) = &updated_frame {
                                    let _ = gamebuddy_services::screen::seen::record(
                                        &store,
                                        frame,
                                        "assistant",
                                    );
                                }
                            }
                        }
                        if let Some((event, reject)) = pending_tools.pop_front() {
                            let outcome = answer_tool(
                                &mut client,
                                &store,
                                &game,
                                &event,
                                VoiceEvidence {
                                    utterance,
                                    request: &request,
                                    frame: supplied_frame.as_ref(),
                                },
                                reject,
                                &diagnostics,
                            )?;
                            if let Some(frame) = outcome.frame {
                                supplied_frame = Some(frame);
                            }
                            if let Some(input) = outcome.retry_input
                                && let Some(turn_id) =
                                    event["params"].get("turnId").filter(|id| id.is_string())
                                && reasoning_turn.as_ref() == Some(turn_id)
                                && !context_requests.values().any(|(id, ..)| id == turn_id)
                            {
                                // A native steer makes replacement pixels available
                                // at the next reasoning step, rather than leaving the
                                // model to rediscover them inside a failed tool result.
                                sequence += 1;
                                context_requests.insert(
                                    sequence,
                                    (
                                        turn_id.clone(),
                                        utterance,
                                        request.clone(),
                                        supplied_frame.clone(),
                                    ),
                                );
                                client.send(json!({"id":sequence,"method":"turn/steer","params":{"threadId":thread,"expectedTurnId":turn_id,"input":input}}))?;
                                diagnostics
                                    .record("reply_retry_steered", json!({"utterance":utterance}));
                            }
                            if let Some(reply) = outcome.voice_reply {
                                response_step = reply["step_id"]
                                    .as_str()
                                    .map(|id| (utterance, id.to_owned()));
                                delivery.complete(
                                    utterance,
                                    &request,
                                    reply["text"].as_str().unwrap_or_default(),
                                );
                                diagnostics.record("reply_tool_ready", json!({"utterance":utterance,"elapsed_ms":reasoning_since.map(|since|since.elapsed().as_millis())}));
                            }
                            if outcome.state_changed {
                                spatial_advice |= event["params"]["tool"] == "set_next_action";
                                if event["params"]["tool"] == "set_next_action" {
                                    let args = event["params"]["arguments"]
                                        .as_str()
                                        .and_then(|s| serde_json::from_str::<Value>(s).ok())
                                        .unwrap_or_else(|| event["params"]["arguments"].clone());
                                    response_step = args["step_id"]
                                        .as_str()
                                        .map(|id| (utterance, id.to_owned()));
                                }
                                emit(json!({"method":"gamebuddy/state-changed","params":{}}));
                            }
                        }
                    }
                }
            })();
            if let Err(error) = result {
                emit(
                    json!({"method":"thread/realtime/error","params":{"message":format!("{error:#}")}}),
                );
            }
        });
    }
}

struct VoiceEvidence<'a> {
    utterance: u64,
    request: &'a str,
    frame: Option<&'a gamebuddy_services::screen::Snapshot>,
}
struct ToolOutcome {
    state_changed: bool,
    voice_reply: Option<Value>,
    frame: Option<gamebuddy_services::screen::Snapshot>,
    retry_input: Option<Vec<Value>>,
}

fn answer_tool(
    client: &mut CodexClient,
    store: &Store,
    game: &str,
    event: &Value,
    evidence: VoiceEvidence<'_>,
    rejected: bool,
    diagnostics: &gamebuddy_services::voice::diagnostics::Diagnostics,
) -> anyhow::Result<ToolOutcome> {
    let request = evidence.request;
    let tool_started = Instant::now();
    let params = &event["params"];
    let active_game = if params["tool"] == "get_game_screen" {
        gamebuddy_services::screen::status(store)?["game"]
            .as_str()
            .map(str::to_owned)
    } else {
        store
            .get::<String>("active_guide")?
            .and_then(|id| store.guide(&id).ok())
            .map(|g| g.game)
    };
    let result = if rejected {
        Err(anyhow::anyhow!(
            "The player continued speaking while this action was planned. It was not executed. Read live_player_request and carry out the complete request."
        ))
    } else if params["tool"] == "get_conversation" {
        // Bind recall to the voice session's game even
        // while its walkthrough is still being prepared.
        conversation::read(store, game, &params["arguments"]).map(|value| {
            gamebuddy_services::harness::ToolOutput {
                value,
                images: vec![],
            }
        })
    } else if active_game
        .as_ref()
        .is_none_or(|g| g.eq_ignore_ascii_case(game))
    {
        if params["tool"] == gamebuddy_services::voice::reply::TOOL {
            gamebuddy_services::voice::reply::apply(
                store,
                params["arguments"].clone(),
                &gamebuddy_services::voice::reply::request_id(evidence.utterance, request),
                evidence.frame,
            )
        } else {
            gamebuddy_services::harness::call_tool_with_images(
                store,
                params["tool"].as_str().unwrap_or(""),
                params["arguments"].clone(),
            )
        }
    } else {
        Err(anyhow::anyhow!(
            "The active guide belongs to a different game and this game's guide may still be preparing. get_game_screen remains available for the current game."
        ))
    };
    let result = result.map(|mut output| {
        if output.value.is_object() && !request.is_empty() {
            output.value["live_player_request"] = json!(request)
        }
        output
    });
    let success = result.is_ok();
    let voice_reply = result
        .as_ref()
        .ok()
        .filter(|_| params["tool"] == gamebuddy_services::voice::reply::TOOL)
        .and_then(|output| output.value.get("voice_reply").cloned());
    diagnostics.record("tool_result", json!({"tool":params["tool"],"success":success,"rejected":rejected,"error":result.as_ref().err().map(|e|e.to_string().chars().take(300).collect::<String>()),"elapsed_ms":tool_started.elapsed().as_millis()}));
    let mut frame = result.as_ref().ok().and_then(|r| r.screen_frame());
    let mut retry_input = None;
    let content = match result {
        Ok(output) => output.codex_content(),
        Err(error) => {
            let output = if params["tool"] == gamebuddy_services::voice::reply::TOOL {
                gamebuddy_services::voice::reply::retry_context(
                    store,
                    game,
                    &error.to_string(),
                    &gamebuddy_services::voice::reply::request_id(evidence.utterance, request),
                    request,
                )
            } else {
                gamebuddy_services::harness::ToolOutput {
                    value: json!({"error":error.to_string(),"live_player_request":request}),
                    images: vec![],
                }
            };
            frame = output.screen_frame();
            if output.value["current_game_context"].is_object() {
                retry_input =
                    Some(output.codex_input(gamebuddy_services::voice::reply::RETRY_INSTRUCTIONS));
            }
            output.codex_content()
        }
    };
    if std::env::args().any(|a| a == "--voice-trace") {
        eprintln!(
            "GAMEBUDDY_VOICE_TOOL name={} success={} images={} elapsed_ms={} text_bytes={}",
            params["tool"].as_str().unwrap_or_default(),
            success,
            content
                .as_array()
                .map(|v| v.iter().filter(|i| i["type"] == "inputImage").count())
                .unwrap_or_default(),
            tool_started.elapsed().as_millis(),
            content
                .as_array()
                .map(|items| items
                    .iter()
                    .filter_map(|i| i["text"].as_str())
                    .map(str::len)
                    .sum::<usize>())
                .unwrap_or_default()
        );
    }
    client.send(json!({"id":event["id"],"result":{"success":success,"contentItems":content}}))?;
    if let Some(frame) = &frame {
        let _ = gamebuddy_services::screen::seen::record(store, frame, "assistant");
    }
    let state_changed = voice_reply
        .as_ref()
        .is_some_and(|reply| reply["step_id"].is_string())
        || success
            && matches!(
                params["tool"].as_str(),
                Some(
                    "report_world_state"
                        | "review_route"
                        | "report_step_complete"
                        | "report_location"
                        | "set_next_action"
                        | "edit_plan"
                        | "restore_plan"
                        | "show_widget"
                        | "remove_widget"
                )
            );
    Ok(ToolOutcome {
        state_changed,
        voice_reply,
        frame,
        retry_input,
    })
}
