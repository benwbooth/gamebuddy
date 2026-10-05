#[cxx_qt::bridge]
pub mod ffi {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
        include!("gamebuddy_frame.h");
        #[namespace = "gamebuddy"]
        unsafe fn save_video_frame(sink: *mut QObject, path: &QString) -> bool;
    }
    unsafe extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qproperty(QString, state_json)]
        #[qproperty(QString, settings_json)]
        #[qproperty(QString, startup_json)]
        #[qproperty(QString, message)]
        #[qproperty(QString, answer)]
        #[qproperty(QString, chat_error)]
        #[qproperty(bool, busy)]
        #[qproperty(bool, preparing)]
        #[qproperty(bool, matching)]
        #[qproperty(QString, transcript)]
        type Buddy = super::BuddyRust;
        #[qinvokable]
        fn initialize(self: Pin<&mut Buddy>);
        #[qinvokable]
        fn poll_startup(self: Pin<&mut Buddy>);
        #[qinvokable]
        fn launch_arguments(self: &Buddy) -> QString;
        #[qinvokable]
        fn prepare_game(self: Pin<&mut Buddy>);
        #[qinvokable]
        fn select_guide(self: Pin<&mut Buddy>, id: QString);
        #[qinvokable]
        fn select_step(self: Pin<&mut Buddy>, id: QString);
        #[qinvokable]
        fn import_text(
            self: Pin<&mut Buddy>,
            game: QString,
            title: QString,
            text: QString,
            compile: bool,
        );
        #[qinvokable]
        fn import_file(self: Pin<&mut Buddy>, game: QString, path: QString);
        #[qinvokable]
        fn import_youtube(self: Pin<&mut Buddy>, game: QString, url: QString);
        #[qinvokable]
        fn build_index(self: Pin<&mut Buddy>, interval: i32, crop_json: QString);
        #[qinvokable]
        fn compile_selected(self: Pin<&mut Buddy>);
        #[qinvokable]
        fn export_illustrated(self: Pin<&mut Buddy>);
        #[qinvokable]
        fn match_frame(self: Pin<&mut Buddy>, path: QString);
        #[qinvokable]
        fn match_game_frame(self: Pin<&mut Buddy>);
        #[qinvokable]
        fn set_screen_capture(self: Pin<&mut Buddy>, enabled: bool);
        #[qinvokable]
        fn crop_preview(self: &Buddy) -> QString;
        #[qinvokable]
        fn save_game_crop(self: Pin<&mut Buddy>, edit: QString) -> QString;
        #[qinvokable]
        fn compare_vision(self: Pin<&mut Buddy>);
        #[qinvokable]
        fn ask(self: Pin<&mut Buddy>, question: QString) -> bool;
        #[qinvokable]
        fn confirm_location(self: Pin<&mut Buddy>);
        #[qinvokable]
        fn complete_step(self: Pin<&mut Buddy>);
        #[qinvokable]
        fn reset_progress(self: Pin<&mut Buddy>);
        #[qinvokable]
        fn save_settings(self: Pin<&mut Buddy>, json: QString);
        #[qinvokable]
        fn provider_request(
            self: Pin<&mut Buddy>,
            id: QString,
            config: QString,
            operation: QString,
        );
        #[qsignal]
        fn provider_result(self: Pin<&mut Buddy>, id: QString, result: QString);
        #[qinvokable]
        fn capture_path(self: &Buddy) -> QString;
        #[qinvokable]
        fn source_url(self: &Buddy) -> QString;
        #[qinvokable]
        fn widget_html(self: &Buddy) -> QString;
        #[qinvokable]
        fn save_widget_layout(self: Pin<&mut Buddy>, json: QString);
        #[qinvokable]
        fn set_widget_visible(self: Pin<&mut Buddy>, id: QString, visible: bool);
        #[qinvokable]
        fn poll_widgets(self: Pin<&mut Buddy>);
        #[qinvokable]
        fn voice_path(self: &Buddy) -> QString;
        #[qinvokable]
        fn voice_directory(self: &Buddy) -> QString;
        #[qinvokable]
        fn voice_turn(self: Pin<&mut Buddy>, path: QString);
        #[qinvokable]
        fn realtime_transcript(self: Pin<&mut Buddy>, text: QString);
        #[qinvokable]
        fn map_preview(self: Pin<&mut Buddy>, id: QString);
        #[qsignal]
        fn map_preview_ready(self: Pin<&mut Buddy>, id: QString, preview: QString);
        #[qinvokable]
        fn guide_browser_request(self: Pin<&mut Buddy>, request: QString);
        #[qsignal]
        fn guide_browser_response(self: Pin<&mut Buddy>, response: QString);
        #[qinvokable]
        fn discard_voice(self: &Buddy, path: QString);
        #[qsignal]
        fn voice_command(self: Pin<&mut Buddy>, action: i32);
        #[qinvokable]
        unsafe fn match_sink(self: Pin<&mut Buddy>, sink: *mut QObject);
    }
    impl cxx_qt::Threading for Buddy {}
}

use anyhow::{Context, Result};
use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::QString;
use gamebuddy_core::*;
use gamebuddy_services::{self as services, HttpProvider, Settings, Store};
use serde_json::json;
use std::{path::PathBuf, pin::Pin};

pub struct BuddyRust {
    state_json: QString,
    settings_json: QString,
    startup_json: QString,
    startup_worker: Option<services::spatial_worker::WorkerConfig>,
    message: QString,
    answer: QString,
    chat_error: QString,
    busy: bool,
    preparing: bool,
    matching: bool,
    vision_inflight: bool,
    last_vision_at: u64,
    vision_votes: Option<(String, u32)>,
    vision_evidence: Option<services::VisionVerdict>,
    transcript: QString,
    root: PathBuf,
    guide: Option<Guide>,
    selected: String,
    sync: Synchronizer,
    estimate: SyncEstimate,
    last_frame: Option<PathBuf>,
    game: String,
    screen_session: String,
    scene_inflight: bool,
    last_scene_at: u64,
    last_scene_image: String,
    map_inflight: bool,
    refresh_inflight: bool,
    refresh_generation: u64,
    poll_inflight: bool,
}
impl Default for BuddyRust {
    fn default() -> Self {
        Self {
            state_json: QString::from("{}"),
            settings_json: QString::from("{}"),
            startup_json: QString::from("null"),
            startup_worker: None,
            message: QString::from("Ready to start"),
            answer: QString::default(),
            chat_error: QString::default(),
            busy: false,
            preparing: false,
            matching: false,
            vision_inflight: false,
            last_vision_at: 0,
            vision_votes: None,
            vision_evidence: None,
            transcript: QString::default(),
            root: PathBuf::new(),
            guide: None,
            selected: String::new(),
            sync: Synchronizer::default(),
            estimate: SyncEstimate::default(),
            last_frame: None,
            game: "Faxanadu".into(),
            screen_session: String::new(),
            scene_inflight: false,
            last_scene_at: 0,
            last_scene_image: String::new(),
            map_inflight: false,
            refresh_inflight: false,
            refresh_generation: 0,
            poll_inflight: false,
        }
    }
}
impl Drop for BuddyRust {
    fn drop(&mut self) {
        if !self.screen_session.is_empty()
            && let Ok(store) = Store::open(&self.root)
        {
            let _ = services::screen::set_enabled(&store, &self.screen_session, &self.game, false);
        }
    }
}
fn q(s: impl AsRef<str>) -> QString {
    QString::from(s.as_ref())
}
fn argument(name: &str) -> Option<String> {
    let args: Vec<_> = std::env::args().collect();
    args.windows(2)
        .find(|pair| pair[0] == name)
        .map(|pair| pair[1].clone())
}

enum Outcome {
    Guide(Guide),
    Message(String),
    Answer(Result<String>),
    Vision(String, services::screen::Ticket),
    Voice(String, String),
    VoiceCommand(String, i32),
}
// Own only plain Rust data: database reads, navigation construction and JSON
// serialization must never run on Qt's event/render thread.
struct ViewSnapshot {
    root: PathBuf,
    game: String,
    guide: Option<Guide>,
    selected: String,
    sync: Synchronizer,
    estimate: SyncEstimate,
    vision_evidence: Option<services::VisionVerdict>,
    last_frame: Option<PathBuf>,
}
impl ffi::Buddy {
    pub fn launch_arguments(&self) -> QString {
        q(serde_json::to_string(
            &std::env::args_os()
                .map(|arg| arg.to_string_lossy().into_owned())
                .collect::<Vec<_>>(),
        )
        .unwrap_or_else(|_| "[]".into()))
    }
    pub fn initialize(mut self: Pin<&mut Self>) {
        let result = (|| -> Result<_> {
            let root = argument("--data-dir")
                .map(PathBuf::from)
                .map(Ok)
                .unwrap_or_else(services::data_dir)?;
            let store = Store::open(&root)?;
            let settings = store.get::<Settings>("settings")?.unwrap_or_default();
            let game = argument("--game").unwrap_or_else(|| "Faxanadu".into());
            let active = store.get::<String>("active_guide")?;
            let guides = store.guides()?;
            let guide = guides
                .iter()
                .find(|g| Some(&g.id) == active.as_ref() && g.game.eq_ignore_ascii_case(&game))
                .or_else(|| guides.iter().find(|g| g.game.eq_ignore_ascii_case(&game)))
                .cloned();
            let capture = services::screen::start(&store, &game, argument("--game-pid").is_some())?;
            Ok((root, settings, guide, game, capture.session))
        })();
        match result {
            Ok((root, settings, guide, game, capture_session)) => {
                self.as_mut().rust_mut().root = root;
                self.as_mut().rust_mut().game = game;
                self.as_mut().rust_mut().screen_session = capture_session;
                self.as_mut()
                    .set_settings_json(q(serde_json::to_string(&settings).unwrap()));
                if let Some(g) = guide {
                    self.as_mut().use_guide(g);
                } else {
                    self.as_mut().refresh();
                }
                self.as_mut().set_message(q(
                    "Ready. Load a game to prepare its walkthrough automatically.",
                ));
                if (argument("--game").is_some() || argument("--game-pid").is_some())
                    && !std::env::args().any(|arg| arg == "--no-prepare")
                {
                    self.prepare_game();
                }
            }
            Err(e) => self.set_message(q(format!("Startup failed: {e:#}"))),
        }
    }
    pub fn prepare_game(mut self: Pin<&mut Self>) {
        if *self.preparing() {
            return;
        }
        let game = self.rust().game.clone();
        let platform = argument("--platform").unwrap_or_default();
        let progress_thread = self.qt_thread();
        let result_thread = self.qt_thread();
        let root = self.rust().root.clone();
        self.as_mut().set_preparing(true);
        self.set_message(q("Preparing this game's walkthrough…"));
        std::thread::spawn(move || {
            let result = Store::open(root).and_then(|store| {
                services::prepare_game(&store, &game, &platform, &|message| {
                    let _ = progress_thread
                        .queue(move |mut object| object.as_mut().set_message(q(message)));
                })
            });
            let _ = result_thread.queue(move |mut object| {
                object.as_mut().set_preparing(false);
                match result {
                    Ok(guide) => {
                        object.as_mut().use_guide(guide);
                        object.as_mut().set_message(q(
                            "Reference preparation finished. Watching the game for location matches.",
                        ));
                    }
                    Err(e) => object.as_mut().set_message(q(format!(
                        "Guide setup: {e:#}. Retry from the companion controls."
                    ))),
                }
            });
        });
    }
    fn refresh(mut self: Pin<&mut Self>) {
        self.as_mut().rust_mut().refresh_generation += 1;
        if self.rust().refresh_inflight {
            return;
        }
        self.as_mut().rust_mut().refresh_inflight = true;
        let generation = self.rust().refresh_generation;
        let data = self.rust();
        let data = ViewSnapshot {
            root: data.root.clone(),
            game: data.game.clone(),
            guide: data.guide.clone(),
            selected: data.selected.clone(),
            sync: data.sync.clone(),
            estimate: data.estimate.clone(),
            vision_evidence: data.vision_evidence.clone(),
            last_frame: data.last_frame.clone(),
        };
        let thread = self.qt_thread();
        std::thread::spawn(move || {
            let started = std::time::Instant::now();
            let result = (|| -> Result<String> {
                let store = Store::open(&data.root)?;
                let guides = store
                    .guides()?
                    .into_iter()
                    .map(|g| json!({"id":g.id,"game":g.game,"title":g.title,"steps":g.steps.len()}))
                    .collect::<Vec<_>>();
                let count = if let Some(g) = &data.guide {
                    store.reference_count(g)?
                } else {
                    0
                };
                let selected = data.guide.as_ref().and_then(|g| g.step(&data.selected));
                let navigation = data
                    .guide
                    .as_ref()
                    .map(|g| {
                        let mut navigation = services::navigation::context(
                            &store,
                            g,
                            &data.sync.progress,
                            &data.estimate,
                        )?;
                        // The full tree belongs to the reader, not every voice/MCP context.
                        navigation["player_steps"] = json!(gamebuddy_core::player_outline(g));
                        Ok::<_, anyhow::Error>(navigation)
                    })
                    .transpose()?
                    .unwrap_or_else(|| json!({}));
                let current = &navigation["guidance"]["now"];
                let widgets = data
                    .guide
                    .as_ref()
                    .map(|g| services::harness::widgets(&store, g))
                    .transpose()?
                    .unwrap_or_default();
                let layouts = store
                    .get::<serde_json::Value>("widget-layout")?
                    .unwrap_or_else(|| json!({}));
                let widget_visibility = store
                    .get::<serde_json::Value>(&format!(
                        "widget-visibility:{}",
                        data.guide
                            .as_ref()
                            .map(|g| g.id.as_str())
                            .unwrap_or_default()
                    ))?
                    .unwrap_or_else(|| json!({}));
                if let Some(guide) = &data.guide {
                    store.put_active_session(guide, &json!({"guide_id":guide.id,"guide_revision":guide.revision,"game":data.game,"estimate":data.estimate,"vision":data.vision_evidence,"last_frame":data.last_frame}))?;
                }
                let frame_url = data
                    .last_frame
                    .as_ref()
                    .and_then(|p| url::Url::from_file_path(p).ok())
                    .map(|u| u.to_string());
                let preparation = data
                    .guide
                    .as_ref()
                    .map(|g| store.get::<serde_json::Value>(&format!("preparation:{}", g.id)))
                    .transpose()?
                    .flatten();
                let book_url = data
                    .guide
                    .as_ref()
                    .and_then(|g| {
                        store
                            .get::<String>(&format!("book:{}", g.id))
                            .ok()
                            .flatten()
                    })
                    .and_then(|p| url::Url::from_file_path(p).ok())
                    .map(|u| u.to_string());
                Ok(json!({"game":data.game,"guides":guides,"guide":data.guide,"selected":selected,"current":current,"navigation":navigation,"screen":services::screen::status(&store)?,"progress":data.sync.progress,"estimate":data.estimate,"reference_count":count,"frame_url":frame_url,"data_dir":data.root,"widgets":widgets,"layouts":layouts,"widget_visibility":widget_visibility,"preparation":preparation,"book_url":book_url,"launch_context":{"game_id":argument("--game-id"),"platform":argument("--platform"),"game_pid":argument("--game-pid"),"session_id":argument("--session-id")}}).to_string())
            })();
            if std::env::args().any(|a| a == "--ui-trace") {
                eprintln!(
                    "GAMEBUDDY_UI_SNAPSHOT worker_ms={}",
                    started.elapsed().as_millis()
                );
            }
            let _ = thread.queue(move |mut object| {
                object.as_mut().rust_mut().refresh_inflight = false;
                if object.rust().refresh_generation != generation {
                    object.refresh();
                    return;
                }
                match result {
                    Ok(state) => {
                        // Refresh the OSD without turning every new instruction into
                        // a spoken answer. The narrator controls unsolicited speech.
                        object.set_state_json(q(state));
                    }
                    Err(e) => object.set_message(q(format!("Could not read library: {e:#}"))),
                }
            });
        });
    }
    fn use_guide(mut self: Pin<&mut Self>, guide: Guide) {
        self.as_mut().rust_mut().startup_worker = None;
        if let Ok(store) = Store::open(&self.rust().root) {
            self.as_mut().rust_mut().startup_worker =
                services::spatial::startup_config(&store, &guide)
                    .ok()
                    .flatten();
            services::spatial::prewarm(&store, &guide);
        }
        self.as_mut().poll_startup();
        if self.rust().game != guide.game {
            if let Ok(store) = Store::open(&self.rust().root) {
                let enabled = services::screen::ticket(&store, &self.rust().screen_session).is_ok();
                let _ = services::screen::set_enabled(
                    &store,
                    &self.rust().screen_session,
                    &guide.game,
                    enabled,
                );
            }
            self.as_mut().rust_mut().last_scene_at = 0;
            self.as_mut().rust_mut().last_scene_image.clear();
        }
        let progress = Store::open(&self.rust().root)
            .and_then(|s| {
                s.put("active_guide", &guide.id)?;
                s.progress(&guide)
            })
            .unwrap_or_default();
        self.as_mut().rust_mut().selected = progress.current_step.clone().unwrap_or_else(|| {
            guide
                .steps
                .first()
                .map(|s| s.id.clone())
                .unwrap_or_default()
        });
        self.as_mut().rust_mut().sync = Synchronizer::new(progress);
        self.as_mut().rust_mut().game = guide.game.clone();
        self.as_mut().rust_mut().guide = Some(guide);
        self.as_mut().rust_mut().estimate = SyncEstimate::default();
        self.as_mut().rust_mut().last_frame = None;
        self.as_mut().rust_mut().vision_votes = None;
        self.as_mut().rust_mut().vision_evidence = None;
        self.refresh();
    }
    pub fn poll_startup(mut self: Pin<&mut Self>) {
        let status = self
            .rust()
            .startup_worker
            .as_ref()
            .and_then(|config| services::spatial_worker::startup(config, "live"));
        let value = serde_json::to_string(&status).unwrap_or_else(|_| "null".into());
        if self.startup_json().to_string() != value {
            if std::env::args().any(|arg| arg == "--ui-trace") {
                eprintln!("GAMEBUDDY_STARTUP {value}");
            }
            self.as_mut().set_startup_json(q(value));
            self.refresh();
        }
    }
    fn background<F>(mut self: Pin<&mut Self>, label: &str, job: F)
    where
        F: FnOnce(PathBuf) -> Result<Outcome> + Send + 'static,
    {
        if *self.busy() {
            return;
        }
        let root = self.rust().root.clone();
        let thread = self.qt_thread();
        self.as_mut().set_busy(true);
        self.set_message(q(label));
        std::thread::spawn(move || {
            let result = job(root);
            let _=thread.queue(move|mut object|{
            object.as_mut().set_busy(false);
            match result {
                Ok(Outcome::Guide(g))=>{let count=g.steps.len();object.as_mut().use_guide(g);object.as_mut().set_message(q(format!("Imported {count} guide nodes. Source coverage is shown in the guide.")));},
                Ok(Outcome::Message(m))=>object.as_mut().set_message(q(m)),
                Ok(Outcome::Answer(result))=>{match result {
                    Ok(a)=>{object.as_mut().set_answer(q(a));object.as_mut().set_message(q("Answer ready"));},
                    Err(e)=>{let error=format!("{e:#}");object.as_mut().set_chat_error(q(&error));object.as_mut().set_message(q(error));},
                }},
                Ok(Outcome::Vision(a,ticket))=>{
                    if Store::open(&object.rust().root).and_then(|s|services::screen::capture_ticket(&s,&ticket.session)).is_ok_and(|current|current==ticket){
                        object.as_mut().set_answer(q(a));object.as_mut().set_message(q("Visual comparison ready. Confirm a location when the evidence is sufficient."));
                    }else{object.as_mut().set_message(q("Screen settings changed; discarded the earlier comparison."));}
                },
                Ok(Outcome::Voice(transcript,answer))=>{object.as_mut().set_transcript(q(transcript));object.as_mut().set_answer(q(answer));object.as_mut().set_message(q("Ready to listen"));},
                Ok(Outcome::VoiceCommand(transcript,action))=>{object.as_mut().set_transcript(q(transcript));object.as_mut().voice_command(action);object.as_mut().set_message(q("Voice command applied"));},
                Err(e)=>object.as_mut().set_message(q(format!("{e:#}"))),
            }
            object.poll_widgets();
        });
        });
    }
    pub fn select_guide(mut self: Pin<&mut Self>, id: QString) {
        if *self.busy() {
            return;
        }
        match Store::open(&self.rust().root).and_then(|s| s.guide(&id.to_string())) {
            Ok(g) => self.use_guide(g),
            Err(e) => self.as_mut().set_message(q(e.to_string())),
        }
    }
    pub fn select_step(mut self: Pin<&mut Self>, id: QString) {
        if *self.busy() {
            return;
        }
        self.as_mut().rust_mut().selected = id.to_string();
        self.refresh();
    }
    pub fn import_text(
        self: Pin<&mut Self>,
        game: QString,
        title: QString,
        text: QString,
        compile: bool,
    ) {
        self.background("Importing walkthrough text…", move |root| {
            let store = Store::open(root)?;
            let g = if compile {
                let settings = store.get::<Settings>("settings")?.unwrap_or_default();
                services::compile_text(
                    &HttpProvider::new(settings.reasoning)?,
                    &game.to_string(),
                    &title.to_string(),
                    &text.to_string(),
                )?
            } else {
                import_markdown(&game.to_string(), &title.to_string(), &text.to_string())?
            };
            store.save_guide(&g)?;
            Ok(Outcome::Guide(g))
        });
    }
    pub fn import_file(self: Pin<&mut Self>, game: QString, path: QString) {
        self.background("Importing local source…", move |root| {
            let store = Store::open(root)?;
            let path = services::local_path(&path.to_string())?;
            let extension = path
                .extension()
                .unwrap_or_default()
                .to_string_lossy()
                .to_lowercase();
            let g = if extension == "json" {
                serde_json::from_slice(&std::fs::read(&path)?)?
            } else if ["md", "txt"].contains(&extension.as_str()) {
                import_markdown(
                    &game.to_string(),
                    &path.file_stem().unwrap_or_default().to_string_lossy(),
                    &std::fs::read_to_string(&path)?,
                )?
            } else {
                services::import_video(&game.to_string(), &path, None)?
            };
            store.save_guide(&g)?;
            Ok(Outcome::Guide(g))
        });
    }
    pub fn import_youtube(self: Pin<&mut Self>, game: QString, url: QString) {
        self.background(
            "Importing YouTube walkthrough. This can take a few minutes…",
            move |root| {
                let store = Store::open(root)?;
                let g =
                    services::import_youtube(&store, &game.to_string(), &url.to_string(), &|m| {
                        eprintln!("{m}")
                    })?;
                Ok(Outcome::Guide(g))
            },
        );
    }
    pub fn build_index(mut self: Pin<&mut Self>, interval: i32, crop_json: QString) {
        let Some(guide) = self.rust().guide.clone() else {
            return;
        };
        let crop = match serde_json::from_str::<Crop>(&crop_json.to_string()) {
            Ok(c) => c,
            Err(e) => {
                self.as_mut().set_message(q(e.to_string()));
                return;
            }
        };
        self.background("Building a local visual index…", move |root| {
            let store = Store::open(root)?;
            let index =
                services::index_video(&store, &guide, crop, interval.max(1) as u32, &|m| {
                    eprintln!("{m}")
                })?;
            Ok(Outcome::Message(format!(
                "{} reference frames indexed locally. Ready to match gameplay.",
                index.frames.len()
            )))
        });
    }
    pub fn export_illustrated(self: Pin<&mut Self>) {
        let Some(guide) = self.rust().guide.clone() else {
            return;
        };
        self.background("Building illustrated guide…", move |root| {
            let store = Store::open(&root)?;
            services::walkthrough::export_book(
                &store,
                &guide,
                &root.join("walkthroughs").join(&guide.id),
            )?;
            Ok(Outcome::Guide(guide))
        });
    }
    pub fn compile_selected(self: Pin<&mut Self>) {
        let Some(guide) = self.rust().guide.clone() else {
            return;
        };
        let selected = self.rust().selected.clone();
        self.background(
            "Examining video frames and compiling substeps…",
            move |root| {
                let store = Store::open(&root)?;
                let settings = store.get::<Settings>("settings")?.unwrap_or_default();
                let g = services::compile_segment(
                    &HttpProvider::new(settings.reasoning)?,
                    &guide,
                    &selected,
                    &root.join("work"),
                )?;
                store.revise_guide(
                    &g,
                    guide.revision,
                    "Compile video section into action steps",
                )?;
                Ok(Outcome::Guide(g))
            },
        );
    }
    pub fn set_screen_capture(mut self: Pin<&mut Self>, enabled: bool) {
        if self.rust().screen_session.is_empty() {
            return;
        }
        let result = Store::open(&self.rust().root).and_then(|store| {
            services::screen::set_enabled(
                &store,
                &self.rust().screen_session,
                &self.rust().game,
                enabled,
            )
        });
        if let Err(e) = result {
            self.as_mut()
                .set_message(q(format!("Screen watching: {e:#}")));
        }
        self.refresh();
    }
    pub fn crop_preview(&self) -> QString {
        let result = (|| -> Result<serde_json::Value> {
            let data = self.rust();
            let store = Store::open(&data.root)?;
            let ticket = services::screen::capture_ticket(&store, &data.screen_session)?;
            let path = data
                .last_frame
                .as_ref()
                .context("Wait for a game frame, or share the game first")?;
            let image = image::open(path)?;
            let (width, height) = (image.width(), image.height());
            let preview = image.thumbnail(width.min(1600), height.min(1000)).to_rgb8();
            std::fs::create_dir_all(data.root.join("captures"))?;
            let path = data.root.join("captures/crop-preview.jpg");
            preview.save(&path)?;
            Ok(
                json!({"ticket":ticket,"width":width,"height":height,"preview_width":preview.width(),"preview_height":preview.height(),"url":format!("{}?v={}",url::Url::from_file_path(path).map_err(|_|anyhow::anyhow!("Invalid preview path"))?,services::screen::now_ms())}),
            )
        })();
        q(result
            .unwrap_or_else(|e| json!({"error":format!("{e:#}")}))
            .to_string())
    }
    pub fn save_game_crop(mut self: Pin<&mut Self>, edit: QString) -> QString {
        let result = (|| -> Result<()> {
            let edit: serde_json::Value = serde_json::from_str(&edit.to_string())?;
            let ticket: services::screen::Ticket = serde_json::from_value(edit["ticket"].clone())?;
            anyhow::ensure!(
                ticket.session == self.rust().screen_session && ticket.game == self.rust().game,
                "Game changed; reopen the crop editor"
            );
            let crop: Crop = serde_json::from_value(edit["crop"].clone())?;
            services::screen::set_crop(&Store::open(&self.rust().root)?, &ticket, crop)
        })();
        if let Err(e) = result {
            return q(format!("{e:#}"));
        }
        let progress = self.rust().sync.progress.clone();
        self.as_mut().rust_mut().sync = Synchronizer::new(progress);
        self.as_mut().rust_mut().estimate = SyncEstimate::default();
        self.as_mut().rust_mut().vision_votes = None;
        self.as_mut().rust_mut().vision_evidence = None;
        self.as_mut().rust_mut().last_vision_at = 0;
        self.as_mut().rust_mut().last_scene_at = 0;
        self.as_mut().rust_mut().last_scene_image.clear();
        self.as_mut().set_message(q(
            "Game crop saved. The next captured frame uses your selection.",
        ));
        self.refresh();
        QString::default()
    }
    pub fn match_frame(mut self: Pin<&mut Self>, path: QString) {
        self.as_mut().set_screen_capture(true);
        self.match_input(path, None);
    }
    pub fn match_game_frame(self: Pin<&mut Self>) {
        let pid = argument("--game-pid").and_then(|value| value.parse::<u32>().ok());
        if let Some(pid) = pid {
            self.match_input(QString::default(), Some(pid));
        }
    }
    fn match_input(mut self: Pin<&mut Self>, path: QString, game_pid: Option<u32>) {
        if *self.matching() {
            return;
        }
        let guide = self.rust().guide.clone();
        let mut sync = self.rust().sync.clone();
        let initial = sync.progress.clone();
        let root = self.rust().root.clone();
        let Ok(ticket) = Store::open(&root)
            .and_then(|store| services::screen::ticket(&store, &self.rust().screen_session))
        else {
            return;
        };
        let thread = self.qt_thread();
        self.as_mut().set_matching(true);
        let spatial_thread = self.qt_thread();
        let spatial_guide = guide.as_ref().map(|g| (g.id.clone(), g.revision));
        let spatial_session = ticket.session.clone();
        let spatial_notify = std::sync::Arc::new(move || {
            let identity = spatial_guide.clone();
            let session = spatial_session.clone();
            let _ = spatial_thread.queue(move |object| {
                if object.rust().screen_session == session
                    && object
                        .rust()
                        .guide
                        .as_ref()
                        .map(|g| (g.id.clone(), g.revision))
                        == identity
                {
                    object.refresh();
                }
            });
        });
        std::thread::spawn(move || {
            let result = (|| -> Result<_> {
                let store = Store::open(&root)?;
                let captured_at = services::screen::now_ms();
                let captured = (|| -> Result<PathBuf> {
                    if let Some(pid) = game_pid {
                        let path = root.join("captures").join(format!("game-{pid}.png"));
                        #[cfg(target_os = "linux")]
                        {
                            let pending = path.with_extension("pending.png");
                            gamebuddy_overlay::x11::capture_game(pid, &pending)?;
                            std::fs::rename(pending, &path)?;
                        }
                        #[cfg(not(target_os = "linux"))]
                        anyhow::bail!("Gamescope capture is only available on Linux");
                        Ok(path)
                    } else {
                        services::local_path(&path.to_string())
                    }
                })();
                if let Err(error) = &captured {
                    let _ = services::screen::capture_error(&store, &ticket, &error.to_string());
                }
                let path = captured?;
                let captured_done = services::screen::now_ms();
                // Publish the screen even before any guide or reference index exists.
                services::screen::publish_at(
                    &store,
                    &ticket,
                    &path,
                    if game_pid.is_some() {
                        "game_window"
                    } else {
                        "shared_frame"
                    },
                    captured_at,
                )?;
                if std::env::args().any(|a| a == "--ui-trace") {
                    eprintln!(
                        "GAMEBUDDY_CAPTURE capture_ms={} publish_ms={}",
                        captured_done.saturating_sub(captured_at),
                        services::screen::now_ms().saturating_sub(captured_done)
                    );
                }
                if let Some(guide) = &guide
                    && let Ok(snapshot) = services::screen::snapshot(&store)
                {
                    services::activity::schedule(&store, guide, snapshot.clone());
                    services::spatial::schedule_notifying(
                        &store,
                        guide,
                        snapshot,
                        sync.progress.epoch,
                        spatial_notify,
                    );
                }
                let matching = if let Some(guide) = &guide {
                    if services::spatial::load_cached(&store, guide)?
                        .is_some_and(|w| w.status == "ready")
                    {
                        None
                    } else if let Some(index) = store.index(guide)? {
                        let vector = fingerprint(&image::open(&path)?, ticket.crop)?;
                        let estimate = sync.observe(
                            guide,
                            &index,
                            &vector,
                            services::screen::now_ms(),
                            sync.progress.epoch,
                        )?;
                        Some((sync, estimate))
                    } else {
                        None
                    }
                } else {
                    None
                };
                Ok((matching, path))
            })();
            let _ = thread.queue(move |mut object| {
                object.as_mut().set_matching(false);
                match result {
                    Ok((matching, path)) => {
                        let valid = Store::open(&root)
                            .and_then(|s| services::screen::ticket(&s, &ticket.session))
                            .is_ok_and(|current| current == ticket);
                        if !valid {
                            return;
                        }
                        object.as_mut().rust_mut().last_frame = Some(path);
                        if let (Some(guide), Some((sync, estimate))) = (guide, matching) {
                            let current = &object.rust().sync.progress;
                            if object
                                .rust()
                                .guide
                                .as_ref()
                                .is_some_and(|g| g.id == guide.id && g.revision == guide.revision)
                                && current.epoch == initial.epoch
                                && current.current_step == initial.current_step
                                && current.completed == initial.completed
                            {
                                if let Some(step) = sync
                                    .progress
                                    .current_step
                                    .as_deref()
                                    .and_then(|id| guide.step(id))
                                {
                                    object.as_mut().rust_mut().selected = step.id.clone();
                                }
                                if let Err(e) = Store::open(&root).and_then(|s| {
                                    s.save_progress(&guide, &sync.progress, "visual_observation")
                                }) {
                                    object.as_mut().set_message(q(format!(
                                        "Could not save observation: {e:#}"
                                    )));
                                    object.as_mut().poll_widgets();
                                    object.observe_screen();
                                    return;
                                }
                                object.as_mut().rust_mut().sync = sync;
                                object.as_mut().rust_mut().estimate = estimate;
                            }
                        }
                        object.as_mut().refresh();
                        object.as_mut().observe_screen();
                        object.verify_location();
                    }
                    Err(e) => {
                        object
                            .as_mut()
                            .set_message(q(format!("Screen capture/matching: {e:#}")));
                        object.as_mut().refresh();
                        object.observe_screen();
                    }
                }
            });
        });
    }
    fn observe_screen(mut self: Pin<&mut Self>) {
        if self.rust().scene_inflight {
            return;
        }
        let now = services::screen::now_ms();
        if now.saturating_sub(self.rust().last_scene_at) < 3_000 {
            return;
        }
        let root = self.rust().root.clone();
        let Ok(store) = Store::open(&root) else {
            return;
        };
        let Ok(snapshot) = services::screen::snapshot(&store) else {
            return;
        };
        if snapshot.ticket.session != self.rust().screen_session {
            return;
        }
        if snapshot.jpeg == self.rust().last_scene_image
            && now.saturating_sub(self.rust().last_scene_at) < 4_000
        {
            return;
        }
        let Ok(settings) = store.get::<Settings>("settings") else {
            return;
        };
        let config = settings.unwrap_or_default().vision;
        let guide = self.rust().guide.clone();
        let epoch = self.rust().sync.progress.epoch;
        let location_context = guide
            .as_ref()
            .and_then(|g| {
                let atlas = services::navigation::atlas(&store, g).ok()?;
                let tracker = store
                    .get::<gamebuddy_core::PlaceTracker>(&services::navigation::tracker_key(g))
                    .ok()?;
                let spatial = services::spatial::context(&store, g).ok()?;
                // Geometric tracking owns location once prepared. Keep this
                // request focused on scene meaning instead of reclassifying
                // every atlas place on each frame.
                let places = if spatial["preparation"]["status"] == "ready" {
                    Vec::new()
                } else {
                    atlas.places
                };
                Some(json!({"places":places,"last_location":tracker,
                    "geometric_location":spatial["observation"]["tracker"]}))
            })
            .unwrap_or_else(|| json!({"places":[]}));
        self.as_mut().rust_mut().scene_inflight = true;
        self.as_mut().rust_mut().last_scene_at = now;
        self.as_mut().rust_mut().last_scene_image = snapshot.jpeg.clone();
        let thread = self.qt_thread();
        std::thread::spawn(move || {
            let result = HttpProvider::new(config).and_then(|provider| {
                services::screen::observe(
                    &provider,
                    &snapshot,
                    snapshot
                        .metadata
                        .get("scene")
                        .filter(|scene| scene["fresh"] == true),
                    &location_context,
                )
            });
            let saved = Store::open(&root)
                .and_then(|store| services::screen::save_observation(&store, &snapshot, &result));
            if saved.is_ok()
                && let (Some(guide), Ok(scene)) = (&guide, &result)
            {
                let _ = Store::open(&root).and_then(|store| {
                    services::navigation::tracking::observe_place(
                        &store,
                        guide,
                        &snapshot,
                        scene.location.as_ref(),
                        epoch,
                    )
                });
            }
            if std::env::args().any(|a| a == "--voice-trace") {
                eprintln!(
                    "GAMEBUDDY_SCREEN_OBSERVATION success={} saved={} elapsed_ms={}",
                    result.is_ok(),
                    saved.is_ok(),
                    services::screen::now_ms().saturating_sub(snapshot.captured_at_ms)
                );
            }
            let _ = thread.queue(move |mut object| {
                object.as_mut().rust_mut().scene_inflight = false;
                object.as_mut().refresh();
                object.verify_location();
            });
        });
    }
    fn verify_location(mut self: Pin<&mut Self>) {
        let at = self.rust().estimate.observed_at_ms;
        if self.rust().vision_inflight
            || self.rust().scene_inflight
            || self.rust().estimate.status == "located"
            || at.saturating_sub(self.rust().last_vision_at) < 15_000
        {
            return;
        }
        let Some(guide) = self.rust().guide.clone() else {
            return;
        };
        if Store::open(&self.rust().root)
            .and_then(|store| services::spatial::context(&store, &guide))
            .is_ok_and(|state| state["preparation"]["status"] == "ready")
        {
            return;
        }
        let candidates = self.rust().estimate.candidates.clone();
        if candidates.is_empty() {
            return;
        }
        let root = self.rust().root.clone();
        let Ok(capture_ticket) = Store::open(&root)
            .and_then(|store| services::screen::ticket(&store, &self.rust().screen_session))
        else {
            return;
        };
        let Ok(settings) =
            Store::open(&root).and_then(|s| Ok(s.get::<Settings>("settings")?.unwrap_or_default()))
        else {
            return;
        };
        // Automatic frame interpretation stays local by default. Remote visual
        // providers remain available through the explicit Compare control.
        if settings.vision.kind != services::ProviderKind::Ollama {
            return;
        }
        let initial = self.rust().sync.progress.clone();
        let Ok(snapshot) = Store::open(&root).and_then(|s| services::screen::snapshot(&s)) else {
            return;
        };
        if snapshot.ticket != capture_ticket {
            return;
        }
        let live_image = snapshot.jpeg;
        self.as_mut().rust_mut().vision_inflight = true;
        self.as_mut().rust_mut().last_vision_at = at;
        let thread = self.qt_thread();
        std::thread::spawn(move || {
            let result = HttpProvider::new(settings.vision).and_then(|provider| {
                services::compare_candidate_images(&provider, &guide, live_image, &candidates)
            });
            let _=thread.queue(move|mut object|{
                object.as_mut().rust_mut().vision_inflight=false;
                if !Store::open(&root).and_then(|s|services::screen::ticket(&s,&capture_ticket.session)).is_ok_and(|t|t==capture_ticket)
                    || object.rust().guide.as_ref().is_none_or(|g|g.id!=guide.id || g.revision!=guide.revision)
                    || object.rust().sync.progress.epoch!=initial.epoch
                    || object.rust().sync.progress.current_step!=initial.current_step
                    || object.rust().sync.progress.completed!=initial.completed
                    || object.rust().estimate.status=="located"
                    || object.rust().estimate.observed_at_ms.saturating_sub(at)>20_000 {
                    object.as_mut().rust_mut().vision_votes=None;
                    return;
                }
                let Ok(verdict)=result else{
                    object.as_mut().rust_mut().vision_votes=None;
                    return;
                };
                let id=verdict.step_id.clone().filter(|id|!verdict.visible_evidence.is_empty()
                    && object.rust().estimate.candidates.iter().any(|c|&c.step_id==id));
                if let Some(id)=id {
                    let votes=object.rust().vision_votes.as_ref().filter(|(last,_)|last==&id).map(|(_,n)|n+1).unwrap_or(1);
                    object.as_mut().rust_mut().vision_votes=Some((id.clone(),votes));
                    if votes>=2 {
                        let mut updated = object.rust().sync.clone();
                        if updated.confirm_step(&guide,&id).is_err() { return; }
                        if Store::open(&root).and_then(|s|s.save_progress(&guide,&updated.progress,"vision_location_estimate")).is_err() {
                            object.as_mut().rust_mut().vision_votes=None;
                            object.poll_widgets();
                            return;
                        }
                        object.as_mut().rust_mut().sync=updated;
                        object.as_mut().rust_mut().estimate.status="vision_estimate".into();
                        object.as_mut().rust_mut().estimate.observed_at_ms=at;
                        object.as_mut().rust_mut().estimate.located_step=Some(id.clone());
                        object.as_mut().rust_mut().estimate.detail="Repeated local vision match. Location is an estimate; objective completion is unverified.".into();
                        object.as_mut().rust_mut().selected=id.clone();
                    }
                } else {object.as_mut().rust_mut().vision_votes=None;}
                object.as_mut().rust_mut().vision_evidence=Some(verdict);
                object.refresh();
            });
        });
    }
    pub fn compare_vision(self: Pin<&mut Self>) {
        let Some(guide) = self.rust().guide.clone() else {
            return;
        };
        let candidates = self.rust().estimate.candidates.clone();
        self.background(
            "Comparing the live frame with the local vision model…",
            move |root| {
                let store = Store::open(root)?;
                let settings = store.get::<Settings>("settings")?.unwrap_or_default();
                let snapshot = services::screen::snapshot(&store)?;
                anyhow::ensure!(
                    snapshot.ticket.game.eq_ignore_ascii_case(&guide.game),
                    "Capture belongs to a different game"
                );
                let v = services::compare_candidate_images(
                    &HttpProvider::new(settings.vision)?,
                    &guide,
                    snapshot.jpeg,
                    &candidates,
                )?;
                let matched = v
                    .step_id
                    .as_deref()
                    .and_then(|id| guide.step(id))
                    .map(|s| s.title.as_str())
                    .unwrap_or("No verified match");
                Ok(Outcome::Vision(
                    format!(
                        "Visual comparison: {matched}\n\nVisible evidence:\n{}\n\nUncertain:\n{}",
                        v.visible_evidence.join("\n"),
                        v.uncertainties.join("\n")
                    ),
                    snapshot.ticket,
                ))
            },
        );
    }
    pub fn ask(mut self: Pin<&mut Self>, question: QString) -> bool {
        let question = question.to_string().trim().to_owned();
        if *self.busy() || question.is_empty() {
            return false;
        }
        let game = self.rust().game.clone();
        self.as_mut().set_transcript(q(&question));
        self.as_mut().set_answer(q(""));
        self.as_mut().set_chat_error(q(""));
        self.background(
            "Thinking with your selected reasoning model…",
            move |root| {
                Ok(Outcome::Answer((|| {
                    let store = Store::open(root)?;
                    let settings = store.get::<Settings>("settings")?.unwrap_or_default();
                    services::harness::chat(
                        &HttpProvider::new(settings.reasoning)?,
                        &store,
                        &game,
                        &question,
                    )
                })()))
            },
        );
        true
    }
    fn change_progress(mut self: Pin<&mut Self>, kind: &str) {
        if *self.busy() {
            return;
        }
        let Some(guide) = self.rust().guide.clone() else {
            return;
        };
        let selected = self.rust().selected.clone();
        let result = (|| -> Result<()> {
            match kind {
                "confirm" => self
                    .as_mut()
                    .rust_mut()
                    .sync
                    .confirm_step(&guide, &selected)?,
                "complete" => self
                    .as_mut()
                    .rust_mut()
                    .sync
                    .confirm_complete(&guide, &selected)?,
                _ => self.as_mut().rust_mut().sync.reset(),
            };
            Store::open(&self.rust().root)?.save_progress(
                &guide,
                &self.rust().sync.progress,
                kind,
            )?;
            Ok(())
        })();
        match result {
            Ok(()) => {
                if kind == "confirm" {
                    self.as_mut().rust_mut().estimate = SyncEstimate {
                        observed_at_ms: std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis() as u64,
                        epoch: self.rust().sync.progress.epoch,
                        status: "confirmed".into(),
                        located_step: Some(selected),
                        ..Default::default()
                    };
                }
                self.as_mut().set_message(q("Progress saved"));
                self.refresh();
            }
            Err(e) => self.set_message(q(e.to_string())),
        }
    }
    pub fn confirm_location(self: Pin<&mut Self>) {
        self.change_progress("confirm");
    }
    pub fn complete_step(self: Pin<&mut Self>) {
        self.change_progress("complete");
    }
    pub fn reset_progress(self: Pin<&mut Self>) {
        self.change_progress("reload");
    }
    pub fn save_settings(mut self: Pin<&mut Self>, json: QString) {
        if *self.busy() {
            return;
        }
        let result = (|| -> Result<Settings> {
            let s: Settings = serde_json::from_str(&json.to_string())?;
            s.reasoning.validate()?;
            s.vision.validate()?;
            Store::open(&self.rust().root)?.put("settings", &s)?;
            Ok(s)
        })();
        match result {
            Ok(s) => {
                self.as_mut()
                    .set_settings_json(q(serde_json::to_string(&s).unwrap()));
                self.set_message(q(
                    "Model settings saved. Credentials are read from environment variables.",
                ));
            }
            Err(e) => self.set_message(q(format!("Invalid settings: {e:#}"))),
        }
    }
    pub fn provider_request(
        self: Pin<&mut Self>,
        id: QString,
        config: QString,
        operation: QString,
    ) {
        let thread = self.qt_thread();
        let (id, config, operation) = (id.to_string(), config.to_string(), operation.to_string());
        std::thread::spawn(move || {
            let result = (|| -> Result<serde_json::Value> {
                let config: services::ProviderConfig = serde_json::from_str(&config)?;
                match operation.as_str() {
                    "models" => services::provider_catalog::discover(&config),
                    "test" => services::provider_catalog::test(config),
                    _ => anyhow::bail!("Unknown provider operation"),
                }
            })()
            .unwrap_or_else(|error| json!({"error":format!("{error:#}")}));
            let _ = thread.queue(move |mut object| {
                object
                    .as_mut()
                    .provider_result(q(id), q(result.to_string()))
            });
        });
    }
    pub fn capture_path(&self) -> QString {
        let dir = self.rust().root.join("captures");
        if std::fs::create_dir_all(&dir).is_err() {
            return QString::default();
        }
        q(dir.join("live.png").to_string_lossy())
    }
    /// Qt passes a live QObject from VideoOutput.videoSink for this call.
    /// The C++ helper validates its runtime type and retains no pointer.
    pub unsafe fn match_sink(self: Pin<&mut Self>, sink: *mut cxx_qt::QObject) {
        if *self.matching() {
            return;
        }
        let path = self.capture_path();
        // SAFETY: the sink is borrowed for the duration of this QML invocation.
        if unsafe { ffi::save_video_frame(sink, &path) } {
            self.match_frame(path);
        }
    }
    pub fn widget_html(&self) -> QString {
        q(include_str!("../assets/widgets.html")
            .replace(
                "/* WIDGET_GESTURES_SCRIPT */",
                include_str!("../assets/widget-gestures.js"),
            )
            .replace(
                "/* GUIDE_LIBRARY_SCRIPT */",
                include_str!("../assets/guide-library.js"),
            ))
    }
    pub fn voice_path(&self) -> QString {
        let directory = self.rust().root.join("voice");
        if std::fs::create_dir_all(&directory).is_err() {
            return QString::default();
        }
        let name = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        q(
            url::Url::from_file_path(directory.join(format!("utterance-{name}.wav")))
                .map(String::from)
                .unwrap_or_default(),
        )
    }
    pub fn voice_directory(&self) -> QString {
        q(self.rust().root.join("voice").to_string_lossy())
    }
    pub fn discard_voice(&self, path: QString) {
        if let Ok(path) = services::local_path(&path.to_string())
            && path.parent() == Some(self.rust().root.join("voice").as_path())
        {
            let _ = std::fs::remove_file(path);
        }
    }
    pub fn voice_turn(mut self: Pin<&mut Self>, path: QString) {
        if *self.busy() {
            return;
        }
        self.as_mut().set_chat_error(q(""));
        let game = self.rust().game.clone();
        self.background(
            "Listening to your recording… (first use downloads the local speech model)",
            move |root| {
                let path = services::local_path(&path.to_string())?;
                let transcript = services::speech::transcribe(&root, &path);
                // Captured audio is ephemeral, including failed recognition or
                // provider calls. Never delete a user-supplied external file.
                if path.parent() == Some(root.join("voice").as_path()) {
                    let _ = std::fs::remove_file(&path);
                }
                let transcript = transcript?;
                if let Some(action) = services::voice::overlay_command(&transcript) {
                    return Ok(Outcome::VoiceCommand(transcript, action));
                }
                let store = Store::open(root)?;
                let settings = store.get::<Settings>("settings")?.unwrap_or_default();
                let provider = HttpProvider::new(settings.reasoning)?;
                let answer = services::harness::chat(&provider, &store, &game, &transcript)?;
                Ok(Outcome::Voice(transcript, answer))
            },
        );
    }
    pub fn realtime_transcript(mut self: Pin<&mut Self>, text: QString) {
        self.as_mut().set_chat_error(q(""));
        if let Some(action) = services::voice::overlay_command(&text.to_string()) {
            self.as_mut().voice_command(action);
        }
        self.set_transcript(text);
    }
    pub fn map_preview(self: Pin<&mut Self>, id: QString) {
        let Some(guide) = self.rust().guide.clone() else {
            return;
        };
        let root = self.rust().root.clone();
        let thread = self.qt_thread();
        let id = id.to_string();
        std::thread::spawn(move || {
            let preview = Store::open(root)
                .and_then(|s| {
                    if let Some(room) = id.strip_prefix("spatial:") {
                        services::spatial::preview(&s, &guide, room)
                    } else {
                        services::navigation::cached_preview(&s, &guide, &id)
                    }
                })
                .ok();
            let _ = thread.queue(move |object| {
                if object
                    .rust()
                    .guide
                    .as_ref()
                    .is_some_and(|g| g.id == guide.id && g.revision == guide.revision)
                    && let Some(preview) = preview
                {
                    object.map_preview_ready(q(id), q(preview));
                }
            });
        });
    }
    pub fn poll_widgets(mut self: Pin<&mut Self>) {
        if self.rust().root.as_os_str().is_empty() || self.rust().poll_inflight {
            return;
        }
        self.as_mut().rust_mut().poll_inflight = true;
        let root = self.rust().root.clone();
        let game = self.rust().game.clone();
        let identity = self
            .rust()
            .guide
            .as_ref()
            .map(|g| (g.id.clone(), g.revision));
        let state = self.state_json().to_string();
        let prepare = !std::env::args().any(|a| a == "--no-prepare");
        let thread = self.qt_thread();
        std::thread::spawn(move || {
            let result = (|| -> Result<_> {
                let store = Store::open(root)?;
                let id = identity
                    .as_ref()
                    .map(|(id, _)| id.clone())
                    .or(store.get::<String>("active_guide")?);
                let guide = id
                    .and_then(|id| store.guide(&id).ok())
                    .filter(|g| g.game.eq_ignore_ascii_case(&game));
                let progress = guide.as_ref().map(|g| store.progress(g)).transpose()?;
                let mut place = None;
                let mut atlas_pending = false;
                if prepare && let Some(guide) = &guide {
                    let state: serde_json::Value = serde_json::from_str(&state)?;
                    place = state["navigation"]["location"]["place_id"]
                        .as_str()
                        .filter(|p| services::navigation::placement::needed(&store, guide, p))
                        .map(str::to_owned);
                    atlas_pending = services::navigation::atlas(&store, guide)?.status == "pending";
                }
                Ok((guide, progress, place, atlas_pending))
            })();
            let _ = thread.queue(move |mut object| {
                object.as_mut().rust_mut().poll_inflight = false;
                if object.rust().game != game
                    || object
                        .rust()
                        .guide
                        .as_ref()
                        .map(|g| (g.id.clone(), g.revision))
                        != identity
                {
                    return;
                }
                if let Ok((guide, progress, place, atlas_pending)) = result {
                    if let Some(guide) = guide.filter(|g| {
                        identity
                            .as_ref()
                            .is_none_or(|(_, revision)| *revision != g.revision)
                    }) {
                        object.as_mut().use_guide(guide);
                    } else if let Some(progress) =
                        progress.filter(|p| p.epoch > object.rust().sync.progress.epoch)
                    {
                        object.as_mut().rust_mut().sync = Synchronizer::new(progress);
                        object.as_mut().rust_mut().estimate = SyncEstimate::default();
                    }
                    object
                        .as_mut()
                        .prepare_background_maps(place, atlas_pending);
                }
                object.refresh();
            });
        });
    }
    fn prepare_background_maps(
        mut self: Pin<&mut Self>,
        place: Option<String>,
        atlas_pending: bool,
    ) {
        if !self.rust().map_inflight
            && let Some(guide) = self.rust().guide.clone()
            && let Some(place) = place
        {
            self.as_mut().rust_mut().map_inflight = true;
            let root = self.rust().root.clone();
            let thread = self.qt_thread();
            std::thread::spawn(move || {
                let _ = Store::open(root)
                    .and_then(|s| services::navigation::placement::prepare(&s, &guide, &place));
                let _ = thread.queue(|mut object| {
                    object.as_mut().rust_mut().map_inflight = false;
                    object.refresh()
                });
            });
        }
        // Atlas preparation belongs to the application lifecycle, including
        // imported guides, changed goals/references, and newly supplied maps.
        if !*self.preparing()
            && atlas_pending
            && let Some(guide) = self.rust().guide.clone()
        {
            self.as_mut().set_preparing(true);
            let root = self.rust().root.clone();
            let platform = argument("--platform").unwrap_or_default();
            let updates = self.qt_thread();
            let thread = self.qt_thread();
            std::thread::spawn(move || {
                let result = Store::open(root).and_then(|store| {
                    services::navigation::prepare_atlas(&store, &guide, &platform, &|message| {
                        let _ = updates
                            .queue(move |mut object| object.as_mut().set_message(q(message)));
                    })
                });
                let _ = thread.queue(move |mut object| {
                    object.as_mut().set_preparing(false);
                    if let Err(error) = result {
                        object.as_mut().set_message(q(format!(
                            "Location atlas: {error:#}. Retry preparation to resume."
                        )));
                    }
                    object.refresh();
                });
            });
        }
    }
    pub fn set_widget_visible(mut self: Pin<&mut Self>, id: QString, visible: bool) {
        let result = (|| -> Result<()> {
            let guide = self.rust().guide.as_ref().context("No game guide loaded")?;
            let id = id.to_string();
            let store = Store::open(&self.rust().root)?;
            anyhow::ensure!(
                [
                    "current-step",
                    "walkthrough-outline",
                    "location-map",
                    "guide-library",
                    "navigation-arrow"
                ]
                .contains(&id.as_str())
                    || services::harness::widgets(&store, guide)?
                        .iter()
                        .any(|w| w.id == id),
                "Unknown widget"
            );
            let key = format!("widget-visibility:{}", guide.id);
            let mut prefs = store
                .get::<serde_json::Value>(&key)?
                .unwrap_or_else(|| json!({}));
            prefs[&id] = json!(visible);
            store.put(&key, &prefs)
        })();
        if let Err(e) = result {
            self.as_mut().set_message(q(e.to_string()));
        }
        self.refresh();
    }
    pub fn guide_browser_request(mut self: Pin<&mut Self>, request: QString) {
        let parsed = serde_json::from_str::<services::guide_browser::Request>(&request.to_string());
        let Ok(request) = parsed else {
            self.as_mut()
                .guide_browser_response(q(json!({"error":"Invalid guide request"}).to_string()));
            return;
        };
        if !self
            .rust()
            .guide
            .as_ref()
            .is_some_and(|g| g.id == request.guide_id)
        {
            self.as_mut().guide_browser_response(q(json!({"request_id":request.request_id,"guide_id":request.guide_id,"error":"The active guide changed"}).to_string()));
            return;
        }
        let root = self.rust().root.clone();
        let thread = self.qt_thread();
        std::thread::spawn(move || {
            let result = Store::open(root)
                .and_then(|store| services::guide_browser::respond(&store, &request));
            let response = match result {
                Ok(data) => {
                    json!({"request_id":request.request_id,"guide_id":request.guide_id,"revision":request.revision,"data":data})
                }
                Err(e) => {
                    json!({"request_id":request.request_id,"guide_id":request.guide_id,"revision":request.revision,"error":format!("{e:#}")})
                }
            };
            let _ = thread.queue(move |mut object| {
                object
                    .as_mut()
                    .guide_browser_response(q(response.to_string()));
                object.poll_widgets();
            });
        });
    }
    pub fn save_widget_layout(mut self: Pin<&mut Self>, json: QString) {
        let result = (|| -> Result<()> {
            let value: serde_json::Value = serde_json::from_str(&json.to_string())?;
            let object = value.as_object().context("Invalid widget layout")?;
            anyhow::ensure!(object.len() <= 100, "Too many saved widget layouts");
            for bounds in object.values() {
                for name in ["x", "y", "width", "height"] {
                    let number = bounds[name].as_f64().context("Invalid widget bounds")?;
                    anyhow::ensure!(
                        number.is_finite() && (0.0..=32768.0).contains(&number),
                        "Widget bounds are out of range"
                    );
                }
            }
            Store::open(&self.rust().root)?.put("widget-layout", &value)
        })();
        if let Err(e) = result {
            self.as_mut().set_message(q(e.to_string()));
        }
    }
    pub fn source_url(&self) -> QString {
        self.rust()
            .guide
            .as_ref()
            .and_then(|g| {
                g.step(&self.rust().selected)
                    .and_then(|s| services::open_source_url(g, s).ok())
            })
            .map(q)
            .unwrap_or_default()
    }
}
