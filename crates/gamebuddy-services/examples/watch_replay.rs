//! Play a session's saved frames through the watcher on a copy of the data,
//! asking the given questions along the way:
//! watch_replay DATA_COPY GUIDE FRAMES_DIR [INDEX=QUESTION ...]
use anyhow::{Context, Result};
use gamebuddy_services::{HttpProvider, Settings, Store, screen, voice::grounded, watch};

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let store = Store::open(args.get(1).context("Data directory copy")?)?;
    let guide = store.guide(args.get(2).context("Guide id")?)?;
    let mut frames: Vec<_> = std::fs::read_dir(args.get(3).context("Frames")?)?
        .filter_map(|f| Some(f.ok()?.path()))
        .collect();
    frames.sort();
    let settings: Settings = store.get("settings")?.unwrap_or_default();
    let provider = HttpProvider::new(settings.reasoning)?;
    grounded::warm(&provider);
    store.put("active_guide", &guide.id)?;
    gamebuddy_services::recover_captions(&store, &guide)?;
    let ticket = screen::start(&store, &guide.game, true)?;
    for (index, frame) in frames.iter().enumerate() {
        screen::publish(&store, &ticket, frame, "replay")?;
        let snapshot = screen::snapshot(&store)?;
        let started = std::time::Instant::now();
        let looked = watch::watch(&store, &guide, &snapshot, true, true)?;
        println!(
            "FRAME {index} {:.1}s looked={looked}",
            started.elapsed().as_secs_f32()
        );
        if let Some(line) = watch::lines(&store, &guide).last() {
            println!("  {line}");
        }
        if let Ok(Some(cue)) = store.get::<serde_json::Value>(&format!("coaching:{}", guide.id)) {
            println!("  CUE {} {}", cue["kind"], cue["text"]);
        }
        for ask in args.iter().skip(4) {
            if let Some((at, question)) = ask.split_once('=')
                && at == index.to_string()
            {
                let started = std::time::Instant::now();
                let mut first = None;
                let turn = grounded::answer(&store, &guide, &provider, question, &mut |_| {
                    first.get_or_insert(started.elapsed());
                })?;
                println!("  ASK {question}");
                println!(
                    "  SAID ({:.1}s) {}",
                    first.unwrap_or_default().as_secs_f32(),
                    turn.reply
                );
            }
        }
    }
    Ok(())
}
