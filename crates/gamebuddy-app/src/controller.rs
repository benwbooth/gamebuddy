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
        #[qproperty(QString, message)]
        #[qproperty(QString, answer)]
        #[qproperty(bool, busy)]
        #[qproperty(bool, preparing)]
        #[qproperty(bool, matching)]
        #[qproperty(QString, transcript)]
        type Buddy = super::BuddyRust;
        #[qinvokable]
        fn initialize(self: Pin<&mut Buddy>);
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
        fn ask(self: Pin<&mut Buddy>, question: QString);
        #[qinvokable]
        fn confirm_location(self: Pin<&mut Buddy>);
        #[qinvokable]
        fn complete_step(self: Pin<&mut Buddy>);
        #[qinvokable]
        fn reset_progress(self: Pin<&mut Buddy>);
        #[qinvokable]
        fn save_settings(self: Pin<&mut Buddy>, json: QString);
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
        fn map_preview(self: &Buddy, id: QString) -> QString;
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
    message: QString,
    answer: QString,
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
    automatic_answer: String,
    screen_session: String,
    scene_inflight: bool,
    last_scene_at: u64,
    last_scene_image: String,
    map_inflight: bool,
}
impl Default for BuddyRust {
    fn default() -> Self {
        Self {
            state_json: QString::from("{}"),
            settings_json: QString::from("{}"),
            message: QString::from("Ready to start"),
            answer: QString::default(),
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
            automatic_answer: String::new(),
            screen_session: String::new(),
            scene_inflight: false,
            last_scene_at: 0,
            last_scene_image: String::new(),
            map_inflight: false,
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
    Answer(String),
    Vision(String, services::screen::Ticket),
    Voice(String, String),
    VoiceCommand(String, i32),
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
        let result = (|| -> Result<(String, String)> {
            let data = self.rust();
            let store = Store::open(&data.root)?;
            let guides = store
                .guides()?
                .into_iter()
                .map(|g| json!({"id":g.id,"game":g.game,"title":g.title,"steps":g.steps.len()}))
                .collect::<Vec<_>>();
            let count = if let Some(g) = &data.guide {
                store.index(g)?.map(|i| i.frames.len()).unwrap_or(0)
            } else {
                0
            };
            let selected = data.guide.as_ref().and_then(|g| g.step(&data.selected));
            let navigation = data
                .guide
                .as_ref()
                .map(|g| {
                    services::navigation::context(&store, g, &data.sync.progress, &data.estimate)
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
                store.put("session", &json!({"guide_id":guide.id,"guide_revision":guide.revision,"game":data.game,"estimate":data.estimate,"vision":data.vision_evidence,"last_frame":data.last_frame}))?;
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
            let automatic = if navigation["guidance"]["status"] == "ready" {
                format!(
                    "{}: {}",
                    current["title"].as_str().unwrap_or_default(),
                    current["instructions"].as_str().unwrap_or_default()
                )
            } else {
                String::new()
            };
            Ok((json!({"game":data.game,"guides":guides,"guide":data.guide,"selected":selected,"current":current,"navigation":navigation,"screen":services::screen::status(&store)?,"progress":data.sync.progress,"estimate":data.estimate,"reference_count":count,"frame_url":frame_url,"data_dir":data.root,"widgets":widgets,"layouts":layouts,"widget_visibility":widget_visibility,"preparation":preparation,"book_url":book_url,"launch_context":{"game_id":argument("--game-id"),"platform":argument("--platform"),"game_pid":argument("--game-pid"),"session_id":argument("--session-id")}}).to_string(), automatic))
        })();
        match result {
            Ok((state, automatic)) => {
                if automatic != self.rust().automatic_answer {
                    if !automatic.is_empty()
                        || self.answer().to_string() == self.rust().automatic_answer
                    {
                        self.as_mut().set_answer(q(&automatic));
                    }
                    self.as_mut().rust_mut().automatic_answer = automatic;
                }
                self.set_state_json(q(state));
            }
            Err(e) => self.set_message(q(format!("Could not read library: {e:#}"))),
        }
    }
    fn use_guide(mut self: Pin<&mut Self>, guide: Guide) {
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
                Ok(Outcome::Answer(a))=>{object.as_mut().set_answer(q(a));object.as_mut().set_message(q("Answer ready"));},
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
        std::thread::spawn(move || {
            let result = (|| -> Result<_> {
                let store = Store::open(&root)?;
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
                // Publish the screen even before any guide or reference index exists.
                services::screen::publish(
                    &store,
                    &ticket,
                    &path,
                    if game_pid.is_some() {
                        "game_window"
                    } else {
                        "shared_frame"
                    },
                )?;
                let matching = if let Some(guide) = &guide {
                    if let Some(index) = store.index(guide)? {
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
        if self.rust().scene_inflight || self.rust().vision_inflight {
            return;
        }
        let now = services::screen::now_ms();
        if now.saturating_sub(self.rust().last_scene_at) < 6_000 {
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
            && now.saturating_sub(self.rust().last_scene_at) < 15_000
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
                Some(json!({"places":atlas.places,"last_location":tracker}))
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
                    snapshot.metadata.get("scene"),
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
    pub fn ask(self: Pin<&mut Self>, question: QString) {
        self.background(
            "Thinking with your selected reasoning model…",
            move |root| {
                let store = Store::open(root)?;
                let settings = store.get::<Settings>("settings")?.unwrap_or_default();
                Ok(Outcome::Answer(services::harness::coach(
                    &HttpProvider::new(settings.reasoning)?,
                    &store,
                    &question.to_string(),
                )?))
            },
        );
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
        q(include_str!("../assets/widgets.html"))
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
    pub fn voice_turn(self: Pin<&mut Self>, path: QString) {
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
                let answer = if store.get::<String>("active_guide")?.is_some() {
                    services::harness::coach(&provider, &store, &transcript)?
                } else {
                    services::ask(&provider, None, &Progress::default(), &transcript)?
                };
                Ok(Outcome::Voice(transcript, answer))
            },
        );
    }
    pub fn realtime_transcript(mut self: Pin<&mut Self>, text: QString) {
        if let Some(action) = services::voice::overlay_command(&text.to_string()) {
            self.as_mut().voice_command(action);
        }
        self.set_transcript(text);
    }
    pub fn map_preview(&self, id: QString) -> QString {
        let preview = self.rust().guide.as_ref().and_then(|guide| {
            Store::open(&self.rust().root)
                .and_then(|s| services::navigation::cached_preview(&s, guide, &id.to_string()))
                .ok()
        });
        q(preview.unwrap_or_default())
    }
    pub fn poll_widgets(mut self: Pin<&mut Self>) {
        if self.rust().root.as_os_str().is_empty() {
            return;
        }
        if let Some(guide) = &self.rust().guide
            && let Ok(progress) = Store::open(&self.rust().root).and_then(|s| s.progress(guide))
            && progress.epoch > self.rust().sync.progress.epoch
        {
            self.as_mut().rust_mut().sync = Synchronizer::new(progress);
            self.as_mut().rust_mut().estimate = SyncEstimate::default();
        }
        let latest = self.rust().guide.as_ref().and_then(|g| {
            Store::open(&self.rust().root)
                .and_then(|s| s.guide(&g.id))
                .ok()
                .filter(|latest| latest.revision != g.revision)
        });
        if let Some(guide) = latest {
            self.as_mut().use_guide(guide);
        } else if self.rust().guide.is_none() {
            let prepared = Store::open(&self.rust().root)
                .ok()
                .and_then(|s| {
                    s.get::<String>("active_guide")
                        .ok()
                        .flatten()
                        .and_then(|id| s.guide(&id).ok())
                })
                .filter(|g| g.game.eq_ignore_ascii_case(&self.rust().game));
            if let Some(guide) = prepared {
                self.as_mut().use_guide(guide);
            }
        }
        self.as_mut().refresh();
        if !self.rust().map_inflight
            && !std::env::args().any(|a| a == "--no-prepare")
            && let Some(guide) = self.rust().guide.clone()
            && let Ok(state) =
                serde_json::from_str::<serde_json::Value>(&self.state_json().to_string())
            && let Some(place) = state["navigation"]["location"]["place_id"]
                .as_str()
                .map(str::to_owned)
            && let Ok(store) = Store::open(&self.rust().root)
            && services::navigation::placement::needed(&store, &guide, &place)
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
            && !std::env::args().any(|a| a == "--no-prepare")
            && let Some(guide) = self.rust().guide.clone()
            && let Ok(atlas) =
                Store::open(&self.rust().root).and_then(|s| services::navigation::atlas(&s, &guide))
            && atlas.status == "pending"
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
                ["current-step", "walkthrough-outline", "location-map"].contains(&id.as_str())
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
