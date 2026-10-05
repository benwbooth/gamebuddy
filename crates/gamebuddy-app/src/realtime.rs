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
        fn trace_event(self: Pin<&mut RealtimeVoice>, kind: QString, value: QString);
        #[qinvokable]
        fn next_narration(self: Pin<&mut RealtimeVoice>, context: QString) -> QString;
        #[qinvokable]
        fn detect_speech(self: Pin<&mut RealtimeVoice>, samples: QString) -> bool;
        #[qinvokable]
        fn reset_speech(self: Pin<&mut RealtimeVoice>);
        #[qinvokable]
        fn player_speech(self: Pin<&mut RealtimeVoice>, active: bool);
        #[qinvokable]
        fn recognize_input(self: Pin<&mut RealtimeVoice>, text: QString, complete: bool);
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
}
enum Command {
    Speech(String),
    Diagnostic(String, Value),
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
        self.as_mut().rust_mut().generation += 1;
        self.set_status(QString::from("Voice disconnected"));
    }
    pub fn speak(self: Pin<&mut Self>, text: QString) {
        if let Some(tx) = &self.rust().commands {
            let _ = tx.send(Command::Speech(text.to_string()));
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
                .next(&context, Instant::now())
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
                    "{}\n{}",
                    gamebuddy_services::voice::prompts::REASONER,
                    conversation::MEMORY_RULES
                );
                let mut client = CodexClient::start()?;
                let mut tools = gamebuddy_services::harness::tool_definitions();
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
                let thread = client.voice_thread(
                    model,
                    &instructions,
                    tools,
                    settings.fast_voice,
                    if settings.reasoning.kind == gamebuddy_services::ProviderKind::Codex {
                        &settings.reasoning.reasoning_effort
                    } else {
                        ""
                    },
                    deadline,
                )?;
                let mut memory = conversation::RealtimeMemory::new(&store, &game, &thread);
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
                    if event["params"]["role"] != "assistant" || approved_speech.get() {
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
                    ) {
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
                client.request_with_notifications("thread/realtime/start", json!({"threadId":thread,"outputModality":"audio","version":"v3","delegationAckFiller":false,"clientManagedHandoffs":true,"codexResponseHandoffMode":"bemTags","includeStartupContext":false,"prompt":gamebuddy_services::voice::prompts::SPEAKER,"initialItems":conversation::initial_items(&context,&history),"transport":{"type":"webrtc","sdp":offer.to_string()}}), deadline, &mut notification)?;
                let mut sequence = 1000;
                let mut context_requests = std::collections::HashSet::new();
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
                let mut reasoning_turn = None;
                let mut reasoning_input = None;
                let mut supplied_request = String::new();
                let mut final_answer = String::new();
                let mut supplied_frame = None;
                let mut last_frame_check = Instant::now();
                let mut refreshed_view = false;
                let mut delivery = gamebuddy_services::voice::delivery::Delivery::default();
                let mut pending_tools = std::collections::VecDeque::<(Value, bool)>::new();
                loop {
                    match rx.recv_timeout(Duration::from_millis(20)) {
                        Ok(command) => {
                            sequence += 1;
                            let (method, params) = match command {
                                Command::Diagnostic(kind, value) => {
                                    diagnostics.record(&format!("browser/{kind}"), value);
                                    continue;
                                }
                                Command::Speech(text) => {
                                    approved_speech.set(true);
                                    (
                                        "thread/realtime/appendSpeech",
                                        json!({"threadId":thread,"text":text}),
                                    )
                                }
                            };
                            client.send(json!({"id":sequence,"method":method,"params":params}))?;
                        }
                        Err(mpsc::RecvTimeoutError::Disconnected) => {
                            client.request_with_notifications(
                                "thread/realtime/stop",
                                json!({"threadId":thread}),
                                Instant::now() + Duration::from_secs(3),
                                &mut notification,
                            )?;
                            return Ok(());
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                    }
                    while let Some(event) = client.poll()? {
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
                            let utterance = current_input.utterance();
                            if event["params"]["turn"]["status"] == "completed"
                                && reasoning_turn.as_ref() == Some(&event["params"]["turn"]["id"])
                                && reasoning_input == Some(utterance)
                                && !final_answer.is_empty()
                            {
                                delivery.complete(utterance, &supplied_request, &final_answer);
                            } else if reasoning_input == Some(utterance)
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
                        if event["id"]
                            .as_u64()
                            .is_some_and(|id| context_requests.remove(&id))
                        {
                            if std::env::args().any(|a| a == "--voice-trace") {
                                eprintln!(
                                    "GAMEBUDDY_VOICE_CONTEXT_ACCEPTED success={}",
                                    event.get("error").is_none()
                                );
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
                                || (!readonly && !input.lock().unwrap().settled(Instant::now()));
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
                    let (quiet, request, utterance) = {
                        let input = input.lock().unwrap();
                        let now = Instant::now();
                        (input.settled(now), input.text(now), input.utterance())
                    };
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
                    if let Some(answer) = delivery.take_reply(utterance, &request, quiet) {
                        diagnostics.record(
                            "answer_ready",
                            json!({"utterance":utterance,"text_bytes":answer.len()}),
                        );
                        emit(json!({"method":"gamebuddy/speech","params":{"text":answer}}));
                    }
                    // A late handoff must not deliver the previous answer over
                    // a new question. Cancel as input begins, before its new
                    // delegation needs the backend; pauses inside one request
                    // keep the same utterance identity.
                    if reasoning_input.is_some_and(|prior| prior != utterance)
                        && let Some(turn_id) = &reasoning_turn
                    {
                        sequence += 1;
                        interrupt_requests.insert(sequence, turn_id.clone());
                        client.send(json!({"id":sequence,"method":"turn/interrupt","params":{"threadId":thread,"turnId":turn_id}}))?;
                        reasoning_input = None;
                        context_turn = None;
                    }
                    if quiet {
                        // Dispatch every recognized local utterance even when
                        // the live model neither delegates nor answers. Waiting
                        // for an assistant transcript creates a deadlock when
                        // the speaker correctly stays silent for backend approval.
                        if delivery.dispatch(utterance, &request, reasoning_since.is_some()) {
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
                                gamebuddy_services::outcomes::quick_report(&store, &game, &request)
                            {
                                emit(json!({"method":"gamebuddy/state-changed","params":{}}));
                                delivery.complete(utterance, &request, &answer);
                                diagnostics
                                    .record("local_world_report", json!({"utterance":utterance}));
                                continue;
                            }
                            let context_started = Instant::now();
                            let frame = gamebuddy_services::screen::snapshot(&store).ok();
                            let evidence = gamebuddy_services::harness::call_tool_with_images(
                                &store,
                                "get_game_context",
                                json!({}),
                            )
                            .ok()
                            .filter(|c| {
                                c.value["game"]
                                    .as_str()
                                    .is_some_and(|g| g.eq_ignore_ascii_case(&game))
                            });
                            let label = format!(
                                "CURRENT_GAME_CONTEXT for this request. Complete live player request: {request}. Inspect the supplied images directly; do not fetch the same context again."
                            );
                            let turn_input = evidence.as_ref().map(|c| c.codex_input(&label))
                                .unwrap_or_else(|| vec![json!({"type":"text","text":format!("Complete live player request: {request}")})]);
                            sequence += 1;
                            start_requests.insert(sequence, (utterance, request.clone(), frame));
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
                            && start_requests.is_empty()
                            && (request != supplied_request || changed_view)
                            && let Some(turn_id) = context_turn.as_ref()
                        {
                            refreshed_view |= changed_view;
                            if !supplied_request.is_empty() {
                                for (event, reject) in &mut pending_tools {
                                    let name = event["params"]["tool"].as_str().unwrap_or_default();
                                    if !name.starts_with("get_")
                                        && !matches!(name, "search_guide" | "read_reference")
                                    {
                                        *reject = true;
                                    }
                                }
                            }
                            let evidence = if supplied_request.is_empty() || changed_view {
                                supplied_frame = gamebuddy_services::screen::snapshot(&store).ok();
                                gamebuddy_services::harness::call_tool_with_images(
                                    &store,
                                    "get_game_context",
                                    json!({}),
                                )
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
                            reasoning_input = Some(utterance);
                            delivery.started(utterance);
                            context_requests.insert(sequence);
                            let label = format!(
                                "The complete live player request is: {request}. This supersedes any earlier partial delegation. Resolve their location correction before repeating previous advice. Carry out every requested action. If a newer image accompanies this update, the view changed while you were thinking: reassess obstacles, enemies and the immediate interaction before giving directions."
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
                            client.send(json!({"id":sequence,"method":"turn/steer","params":{"threadId":thread,"expectedTurnId":turn_id,"input":turn_input}}))?;
                            supplied_request = request.clone();
                        }
                        if let Some((event, reject)) = pending_tools.pop_front()
                            && answer_tool(&mut client, &store, &game, &event, &request, reject)?
                        {
                            emit(json!({"method":"gamebuddy/state-changed","params":{}}));
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

fn answer_tool(
    client: &mut CodexClient,
    store: &Store,
    game: &str,
    event: &Value,
    request: &str,
    rejected: bool,
) -> anyhow::Result<bool> {
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
        gamebuddy_services::harness::call_tool_with_images(
            store,
            params["tool"].as_str().unwrap_or(""),
            params["arguments"].clone(),
        )
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
    let content=result.map(|r|r.codex_content()).unwrap_or_else(|e|json!([{"type":"inputText","text":json!({"error":e.to_string(),"live_player_request":request}).to_string()}]));
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
    Ok(success
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
        ))
}
