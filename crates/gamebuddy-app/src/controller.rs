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
        fn match_frame(self: Pin<&mut Buddy>, path: QString);
        #[qinvokable]
        fn match_game_frame(self: Pin<&mut Buddy>);
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
        fn poll_widgets(self: Pin<&mut Buddy>);
        #[qinvokable]
        fn voice_path(self: &Buddy) -> QString;
        #[qinvokable]
        fn voice_directory(self: &Buddy) -> QString;
        #[qinvokable]
        fn voice_turn(self: Pin<&mut Buddy>, path: QString);
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
    transcript: QString,
    root: PathBuf,
    guide: Option<Guide>,
    selected: String,
    sync: Synchronizer,
    estimate: SyncEstimate,
    last_frame: Option<PathBuf>,
    game: String,
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
            transcript: QString::default(),
            root: PathBuf::new(),
            guide: None,
            selected: String::new(),
            sync: Synchronizer::default(),
            estimate: SyncEstimate::default(),
            last_frame: None,
            game: "Faxanadu".into(),
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
    Vision(String),
    Voice(String, String),
    VoiceCommand(String, i32),
    Match(Box<(Synchronizer, SyncEstimate, PathBuf)>),
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
        let result = (|| -> Result<(PathBuf, Settings, Option<Guide>, String)> {
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
            Ok((root, settings, guide, game))
        })();
        match result {
            Ok((root, settings, guide, game)) => {
                self.as_mut().rust_mut().root = root;
                self.as_mut().rust_mut().game = game;
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
                if argument("--game").is_some()
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
                            "Walkthrough ready. Share your game view to synchronize.",
                        ));
                    }
                    Err(e) => object.as_mut().set_message(q(format!(
                        "Guide setup: {e:#}. Retry from the companion controls."
                    ))),
                }
            });
        });
    }
    fn refresh(self: Pin<&mut Self>) {
        let result = (|| -> Result<String> {
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
            let current = data.guide.as_ref().and_then(|g| {
                data.sync
                    .progress
                    .current_step
                    .as_deref()
                    .and_then(|id| g.step(id))
            });
            let widgets = data
                .guide
                .as_ref()
                .map(|g| services::harness::widgets(&store, g))
                .transpose()?
                .unwrap_or_default();
            let layouts = store
                .get::<serde_json::Value>("widget-layout")?
                .unwrap_or_else(|| json!({}));
            if let Some(guide) = &data.guide {
                store.put("session", &json!({"guide_id":guide.id,"guide_revision":guide.revision,"game":data.game,"estimate":data.estimate,"last_frame":data.last_frame}))?;
            }
            let frame_url = data
                .last_frame
                .as_ref()
                .and_then(|p| url::Url::from_file_path(p).ok())
                .map(|u| u.to_string());
            Ok(json!({"game":data.game,"guides":guides,"guide":data.guide,"selected":selected,"current":current,"progress":data.sync.progress,"estimate":data.estimate,"reference_count":count,"frame_url":frame_url,"data_dir":data.root,"widgets":widgets,"layouts":layouts,"launch_context":{"game_id":argument("--game-id"),"platform":argument("--platform"),"game_pid":argument("--game-pid"),"session_id":argument("--session-id")}}).to_string())
        })();
        match result {
            Ok(s) => self.set_state_json(q(s)),
            Err(e) => self.set_message(q(format!("Could not read library: {e:#}"))),
        }
    }
    fn use_guide(mut self: Pin<&mut Self>, guide: Guide) {
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
                Ok(Outcome::Vision(a))=>{object.as_mut().set_answer(q(a));object.as_mut().set_message(q("Visual comparison ready. Confirm a location when the evidence is sufficient."));},
                Ok(Outcome::Voice(transcript,answer))=>{object.as_mut().set_transcript(q(transcript));object.as_mut().set_answer(q(answer));object.as_mut().set_message(q("Ready to listen"));},
                Ok(Outcome::VoiceCommand(transcript,action))=>{object.as_mut().set_transcript(q(transcript));object.as_mut().voice_command(action);object.as_mut().set_message(q("Voice command applied"));},
                Ok(Outcome::Match(data))=>{let (sync,estimate,path)=*data;object.as_mut().set_message(q(&estimate.detail));object.as_mut().rust_mut().sync=sync;object.as_mut().rust_mut().estimate=estimate;object.as_mut().rust_mut().last_frame=Some(path);},
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
    pub fn match_frame(self: Pin<&mut Self>, path: QString) {
        self.match_input(path, None);
    }
    pub fn match_game_frame(self: Pin<&mut Self>) {
        let pid = argument("--game-pid").and_then(|value| value.parse::<u32>().ok());
        if let Some(pid) = pid {
            self.match_input(QString::default(), Some(pid));
        }
    }
    fn match_input(self: Pin<&mut Self>, path: QString, game_pid: Option<u32>) {
        let Some(guide) = self.rust().guide.clone() else {
            return;
        };
        let mut sync = self.rust().sync.clone();
        self.background("Matching gameplay to guide references…", move |root| {
            let store = Store::open(&root)?;
            let index = store
                .index(&guide)?
                .context("Build the guide's visual index first")?;
            let path = if let Some(pid) = game_pid {
                let path = root.join("captures").join(format!("game-{pid}.png"));
                #[cfg(target_os = "linux")]
                gamebuddy_overlay::x11::capture_game(pid, &path)?;
                #[cfg(not(target_os = "linux"))]
                anyhow::bail!("Gamescope capture is only available on Linux");
                path
            } else {
                services::local_path(&path.to_string())?
            };
            let vector = fingerprint(&image::open(&path)?, index.live_crop)?;
            let time = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_millis() as u64;
            let estimate = sync.observe(&guide, &index, &vector, time, sync.progress.epoch)?;
            store.save_progress(&guide, &sync.progress, "visual_observation")?;
            Ok(Outcome::Match(Box::new((sync, estimate, path))))
        });
    }
    pub fn compare_vision(self: Pin<&mut Self>) {
        let (Some(guide), Some(path)) = (self.rust().guide.clone(), self.rust().last_frame.clone())
        else {
            return;
        };
        let candidates = self.rust().estimate.candidates.clone();
        self.background(
            "Comparing the live frame with the local vision model…",
            move |root| {
                let settings = Store::open(root)?
                    .get::<Settings>("settings")?
                    .unwrap_or_default();
                let v = services::compare_candidates(
                    &HttpProvider::new(settings.vision)?,
                    &guide,
                    path,
                    &candidates,
                )?;
                let matched = v
                    .step_id
                    .as_deref()
                    .and_then(|id| guide.step(id))
                    .map(|s| s.title.as_str())
                    .unwrap_or("No verified match");
                Ok(Outcome::Vision(format!(
                    "Visual comparison: {matched}\n\nVisible evidence:\n{}\n\nUncertain:\n{}",
                    v.visible_evidence.join("\n"),
                    v.uncertainties.join("\n")
                )))
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
        if *self.busy() || self.rust().guide.is_none() {
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
    pub fn poll_widgets(mut self: Pin<&mut Self>) {
        if self.rust().root.as_os_str().is_empty() || *self.busy() {
            return;
        }
        let latest = self.rust().guide.as_ref().and_then(|g| {
            Store::open(&self.rust().root)
                .and_then(|s| s.guide(&g.id))
                .ok()
                .filter(|latest| latest.revision != g.revision)
        });
        if let Some(guide) = latest {
            self.as_mut().use_guide(guide);
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
