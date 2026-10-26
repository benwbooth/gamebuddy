use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use gamebuddy_core::*;
use gamebuddy_services::spatial;
use gamebuddy_services::*;
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "gamebuddy-cli",
    version,
    about = "GameBuddy guide ingestion and synchronization tools"
)]
struct Args {
    #[arg(long, global = true)]
    data_dir: Option<PathBuf>,
    #[command(subcommand)]
    command: Action,
}
#[derive(Subcommand)]
enum Action {
    /// Build the replay set for a guide from moments the player reported where they were.
    EvalBootstrap {
        guide: String,
    },
    /// Score location, latency and reliability on the replay sets and recent sessions.
    Scorecard,
    /// Summarise a recorded play session (the newest when no directory is given).
    SessionReport {
        dir: Option<PathBuf>,
    },
    /// Index the pixels of cached 2D maps for geometric viewport registration.
    PrepareMapPixels {
        guide: String,
    },
    /// Upgrade retained video views with learned features and revisit matching.
    EnhanceWorld {
        guide: String,
    },
    /// Attach read-only memory evidence to the current capture session (Linux).
    AttachRetroarch {
        rom: PathBuf,
        pid: u32,
        #[arg(long, default_value = "127.0.0.1:55355")]
        endpoint: std::net::SocketAddr,
    },
    /// Read the optional verified Faxanadu memory layout for diagnostics.
    ReadRetroarch {
        rom: PathBuf,
        #[arg(long, default_value = "127.0.0.1:55355")]
        endpoint: std::net::SocketAddr,
    },
    /// Calibrate and build/resume the geometric place graph from enabled videos.
    PrepareWorld {
        guide: String,
    },
    /// Configure an optional local learned feature worker (JSON executable/arguments).
    SpatialWorker {
        path: PathBuf,
    },
    /// Set the video viewport/HUD calibration and 2D/3D camera type.
    SpatialProfile {
        guide: String,
        path: PathBuf,
    },
    /// Localize one already-cropped frame without altering live progress.
    Locate {
        guide: String,
        image: PathBuf,
    },
    /// Register an already-cropped frame against enabled 2D atlas pixels.
    LocateMap {
        guide: String,
        image: PathBuf,
    },
    /// Replay independently labelled gameplay frames, including session/epoch resets.
    EvaluateWorld {
        guide: String,
        manifest: PathBuf,
    },
    /// Select an exact walkthrough; future preparation cannot choose a substitute.
    PinReference {
        game: String,
        url: String,
        #[arg(long, default_value = "")]
        platform: String,
    },
    /// Build/resume a source-derived location atlas with the configured LLM.
    PrepareAtlas {
        guide: String,
        #[arg(long, default_value = "")]
        platform: String,
    },
    Configure {
        path: PathBuf,
    },
    Coach {
        question: String,
    },
    Transcribe {
        path: PathBuf,
    },
    Tool {
        name: String,
        #[arg(default_value = "{}")]
        arguments: String,
    },
    Prepare {
        game: String,
        #[arg(long, default_value = "")]
        platform: String,
    },
    Discover {
        game: String,
        #[arg(long, default_value = "")]
        platform: String,
    },
    Doctor,
    List,
    ImportText {
        path: PathBuf,
        #[arg(long, default_value = "Faxanadu")]
        game: String,
        #[arg(long)]
        compile: bool,
    },
    ImportGuide {
        path: PathBuf,
    },
    ImportVideo {
        path: PathBuf,
        #[arg(long, default_value = "Faxanadu")]
        game: String,
    },
    ImportYoutube {
        url: String,
        #[arg(long, default_value = "Faxanadu")]
        game: String,
    },
    Export {
        guide: String,
        path: PathBuf,
    },
    /// Create or resume an illustrated edition, preserving the original guide.
    Walkthrough {
        guide: String,
        output: PathBuf,
        #[arg(long)]
        resume: bool,
        #[arg(long, default_value_t = 2)]
        jobs: usize,
    },
    ExportBook {
        guide: String,
        output: PathBuf,
    },
    Index {
        guide: String,
        #[arg(long, default_value_t = 10)]
        interval: u32,
        #[arg(long, default_value = "{\"x\":0,\"y\":0,\"width\":1,\"height\":1}")]
        crop: String,
    },
    Match {
        guide: String,
        image: PathBuf,
        #[arg(long)]
        vision: bool,
    },
    /// Resume automatic compilation of unfinished video sections.
    CompileWalkthrough {
        guide: String,
    },
    CompileSegment {
        guide: String,
        step: String,
    },
    Ask {
        guide: String,
        question: String,
    },
    Settings,
    /// Choose the saved edition used by the companion and its next launch.
    UseGuide {
        guide: String,
    },
    /// Apply a revision-checked PlanEdit JSON file to a specific saved guide.
    EditPlan {
        guide: String,
        path: PathBuf,
    },
}
fn main() -> Result<()> {
    let args = Args::parse();
    let root = args.data_dir.map(Ok).unwrap_or_else(data_dir)?;
    let store = Store::open(root)?;
    let settings: Settings = store.get("settings")?.unwrap_or_default();
    let notify = |text: String| eprintln!("{text}");
    match args.command {
        Action::PrepareMapPixels { guide } => println!(
            "{} map tiles",
            spatial_maps::prepare(&store, &store.guide(&guide)?, &notify)?
        ),
        Action::EnhanceWorld { guide } => {
            let w = spatial::enhance(&store, &store.guide(&guide)?, &notify)?;
            println!(
                "{}",
                serde_json::json!({"places":w.rooms.len(),"views":w.views.len(),"encoder":w.encoder})
            );
        }
        Action::AttachRetroarch { rom, pid, endpoint } => println!(
            "{}",
            serde_json::to_string_pretty(&telemetry::attach(&store, &rom, pid, endpoint)?)?
        ),
        Action::ReadRetroarch { rom, endpoint } => {
            telemetry::identify(&rom)?;
            println!("{}", telemetry::read_state(&rom, endpoint)?);
        }
        Action::PrepareWorld { guide } => {
            let guide = store.guide(&guide)?;
            let world = gamebuddy_services::spatial::build(&store, &guide, &notify)?;
            println!(
                "{}",
                serde_json::json!({"rooms":world.rooms.len(),"views":world.views.len(),"transitions":world.transitions.len(),"status":world.status,"dimension":world.profile.dimension,"encoder":world.encoder})
            );
        }
        Action::SpatialWorker { path } => {
            let config: spatial_worker::WorkerConfig =
                serde_json::from_slice(&std::fs::read(path)?)?;
            store.put("spatial-worker", &config)?;
        }
        Action::SpatialProfile { guide, path } => spatial::set_profile(
            &store,
            &store.guide(&guide)?,
            serde_json::from_slice(&std::fs::read(path)?)?,
        )?,
        Action::LocateMap { guide, image } => {
            let guide = store.guide(&guide)?;
            let world = spatial::load(&store, &guide)?.context("Prepare the world first")?;
            println!(
                "{}",
                serde_json::to_string_pretty(&spatial_maps::locate(
                    &store,
                    &guide,
                    &world,
                    &image::open(image)?
                )?)?
            );
        }
        Action::Locate { guide, image } => {
            let guide = store.guide(&guide)?;
            let world = spatial::load(&store, &guide)?.context("Prepare the world first")?;
            let mut engine = spatial::Engine::for_world(&store, &world, "evaluation")?;
            let mut tracker = gamebuddy_core::spatial::Tracker::default();
            let result = spatial::locate(
                &world,
                &guide,
                &image::open(image)?,
                &mut tracker,
                spatial::FrameContext {
                    session: "diagnostic",
                    epoch: 0,
                    at_ms: 1,
                },
                &mut engine,
            )?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Action::EvaluateWorld { guide, manifest } => println!(
            "{}",
            serde_json::to_string_pretty(&spatial::evaluate(
                &store,
                &store.guide(&guide)?,
                &manifest
            )?)?
        ),
        Action::PinReference {
            game,
            url,
            platform,
        } => println!(
            "{}",
            serde_json::to_string_pretty(&pin_reference(&store, &game, &platform, &url)?)?
        ),
        Action::PrepareAtlas { guide, platform } => {
            let guide = store.guide(&guide)?;
            let atlas =
                gamebuddy_services::navigation::prepare_atlas(&store, &guide, &platform, &notify)?;
            println!("{}", serde_json::to_string_pretty(&atlas)?);
        }
        Action::EditPlan { guide, path } => {
            let edit = serde_json::from_slice(&std::fs::read(path)?)?;
            let result = plan::edit_plan(&store, &guide, edit)?;
            println!("Saved guide {} revision {}", result.id, result.revision);
        }
        Action::UseGuide { guide } => {
            let guide = store.guide(&guide)?;
            store.put("active_guide", &guide.id)?;
            println!("Selected {} ({})", guide.title, guide.id);
        }
        Action::Configure { path } => {
            let mut settings: Settings = serde_json::from_slice(&std::fs::read(path)?)?;
            settings.apply_provider_scope();
            settings.reasoning.validate()?;
            settings.vision.validate()?;
            if let Some(planning) = &settings.planning {
                planning.validate()?;
            }
            if let Some(speech) = &settings.prepared_speech {
                speech.validate()?;
            }
            store.put("settings", &settings)?;
            println!("Model configuration saved");
        }
        Action::Coach { question } => {
            let guide: String = store
                .get("active_guide")?
                .context("Select a guide before asking for advice")?;
            let game = store.guide(&guide)?.game;
            let provider = HttpProvider::for_conversation(settings.reasoning, &store, &game)?;
            println!("{}", harness::coach(&provider, &store, &question)?);
        }
        Action::Transcribe { path } => println!("{}", speech::transcribe(&store.root, &path)?),
        Action::Tool { name, arguments } => println!(
            "{}",
            serde_json::to_string_pretty(&harness::call_tool(
                &store,
                &name,
                serde_json::from_str(&arguments)?
            )?)?
        ),
        Action::Prepare { game, platform } => {
            let guide = prepare_game(&store, &game, &platform, &notify)?;
            println!("{}", guide.id);
        }
        Action::Discover { game, platform } => {
            let candidates = discover_walkthroughs(&game, &platform)?;
            let choice = choose_walkthrough(
                &HttpProvider::new(settings.reasoning)?,
                &game,
                &platform,
                &candidates,
            )?;
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &serde_json::json!({"candidates":candidates,"choice":choice})
                )?
            );
        }
        Action::EvalBootstrap { guide } => {
            let guide = store.guide(&guide)?;
            let added = evals::bootstrap(&store, &guide)?;
            println!(
                "Added {added} cases to {}",
                evals::dir(&store, &guide).display()
            );
        }
        Action::Scorecard => println!(
            "{}",
            serde_json::to_string_pretty(&evals::scorecard(&store)?)?
        ),
        Action::SessionReport { dir } => {
            let dir = match dir {
                Some(dir) => dir,
                None => flight::sessions(&store.root)
                    .into_iter()
                    .next()
                    .context("No recorded sessions yet")?,
            };
            println!("{}", serde_json::to_string_pretty(&flight::report(&dir)?)?);
        }
        Action::Doctor => {
            println!("Data: {}", store.root.display());
            for tool in ["ffmpeg", "ffprobe", "yt-dlp"] {
                let flag = if tool == "yt-dlp" {
                    "--version"
                } else {
                    "-version"
                };
                let status = std::process::Command::new(tool).arg(flag).output();
                println!(
                    "{tool}: {}",
                    status.as_ref().map(|o| o.status.success()).unwrap_or(false)
                );
            }
            println!(
                "Reasoning: {:?} {}",
                settings.reasoning.kind, settings.reasoning.model
            );
            println!(
                "Vision: {:?} {}",
                settings.vision.kind, settings.vision.model
            );
        }
        Action::List => println!("{}", serde_json::to_string_pretty(&store.guides()?)?),
        Action::Settings => println!("{}", serde_json::to_string_pretty(&settings)?),
        Action::ImportText {
            path,
            game,
            compile,
        } => {
            let text = std::fs::read_to_string(&path)?;
            let title = path.file_stem().unwrap_or_default().to_string_lossy();
            let g = if compile {
                compile_text(
                    &HttpProvider::new(settings.reasoning)?,
                    &game,
                    &title,
                    &text,
                )?
            } else {
                import_markdown(&game, &title, &text)?
            };
            store.save_guide(&g)?;
            println!("{}", g.id);
        }
        Action::ImportGuide { path } => {
            let g: Guide = serde_json::from_slice(&std::fs::read(path)?)?;
            store.save_guide(&g)?;
            println!("{}", g.id);
        }
        Action::ImportVideo { path, game } => {
            let g = import_video(&game, &path, None)?;
            store.save_guide(&g)?;
            println!("{}", g.id);
        }
        Action::ImportYoutube { url, game } => {
            let g = import_youtube(&store, &game, &url, &notify)?;
            println!("{}", g.id);
        }
        Action::Export { guide, path } => {
            std::fs::write(path, serde_json::to_string_pretty(&store.guide(&guide)?)?)?
        }
        Action::Walkthrough {
            guide,
            output,
            resume,
            jobs,
        } => {
            let guide = store.guide(&guide)?;
            let guide = if resume {
                guide
            } else {
                walkthrough::new_edition(&store, &guide)?
            };
            eprintln!("Illustrated edition: {}", guide.id);
            let guide = walkthrough::build_edition(&store, &guide.id, jobs, &output, &notify)?;
            println!("{}\n{}", guide.id, output.join("index.html").display());
        }
        Action::ExportBook { guide, output } => {
            walkthrough::export_book(&store, &store.guide(&guide)?, &output)?
        }
        Action::Index {
            guide,
            interval,
            crop,
        } => {
            let g = store.guide(&guide)?;
            let index = index_video(&store, &g, serde_json::from_str(&crop)?, interval, &notify)?;
            println!("Indexed {} reference frames", index.frames.len());
        }
        Action::Match {
            guide,
            image,
            vision,
        } => {
            let g = store.guide(&guide)?;
            let index = store.index(&g)?.context("Build the visual index first")?;
            let vector = fingerprint(&image::open(&image)?, index.live_crop)?;
            let mut sync = Synchronizer::new(store.progress(&g)?);
            let epoch = sync.progress.epoch;
            let estimate = sync.observe(&g, &index, &vector, 1, epoch)?;
            println!("{}", serde_json::to_string_pretty(&estimate)?);
            if vision {
                let verdict = compare_candidates(
                    &HttpProvider::new(settings.vision)?,
                    &g,
                    image,
                    &estimate.candidates,
                )?;
                println!("{}", serde_json::to_string_pretty(&verdict)?);
            }
        }
        Action::CompileWalkthrough { guide } => {
            let guide = preparation::compile_walkthrough(&store, &store.guide(&guide)?, &notify)?;
            walkthrough::export_book(
                &store,
                &guide,
                &store.root.join("walkthroughs").join(&guide.id),
            )?;
            println!("{}", guide.revision);
        }
        Action::CompileSegment { guide, step } => {
            let g = store.guide(&guide)?;
            let compiled = compile_segment(
                &HttpProvider::new(settings.reasoning)?,
                &g,
                &step,
                &store.root.join("work"),
            )?;
            store.revise_guide(
                &compiled,
                g.revision,
                "Compile video section into action steps",
            )?;
            println!(
                "Compiled revision {} (rebuild its visual index)",
                compiled.revision
            );
        }
        Action::Ask { guide, question } => {
            let g = store.guide(&guide)?;
            println!(
                "{}",
                ask(
                    &HttpProvider::new(settings.reasoning)?,
                    Some(&g),
                    &store.progress(&g)?,
                    &question
                )?
            );
        }
    }
    Ok(())
}
