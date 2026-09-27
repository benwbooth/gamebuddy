use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use gamebuddy_core::*;
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
    CompileSegment {
        guide: String,
        step: String,
    },
    Ask {
        guide: String,
        question: String,
    },
    Settings,
}
fn main() -> Result<()> {
    let args = Args::parse();
    let root = args.data_dir.map(Ok).unwrap_or_else(data_dir)?;
    let store = Store::open(root)?;
    let settings: Settings = store.get("settings")?.unwrap_or_default();
    let notify = |text: String| eprintln!("{text}");
    match args.command {
        Action::Configure { path } => {
            let settings: Settings = serde_json::from_slice(&std::fs::read(path)?)?;
            settings.reasoning.validate()?;
            settings.vision.validate()?;
            store.put("settings", &settings)?;
            println!("Model configuration saved");
        }
        Action::Coach { question } => println!(
            "{}",
            harness::coach(&HttpProvider::new(settings.reasoning)?, &store, &question)?
        ),
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
