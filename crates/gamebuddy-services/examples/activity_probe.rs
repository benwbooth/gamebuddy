//! Replay ordered gameplay through the configured reasoning backend.
//! All progress and evidence are written to a new private directory.
//! Append --scheduled to exercise the production background scheduler.
use anyhow::{Context, Result, ensure};
use gamebuddy_core::Progress;
use gamebuddy_services::{HttpProvider, Settings, Store, activity, screen};
use gamebuddy_services::{ModelRequest, ReasoningProvider};
use serde_json::json;
use std::{path::PathBuf, time::Duration};

struct RecordedProvider {
    provider: HttpProvider,
    output: PathBuf,
}
impl ReasoningProvider for RecordedProvider {
    fn complete(&self, request: ModelRequest) -> Result<String> {
        self.provider.complete(request)
    }
    fn complete_structured(
        &self,
        request: ModelRequest,
        schema: serde_json::Value,
    ) -> Result<String> {
        use base64::Engine;
        std::fs::write(
            self.output.join("request.json"),
            serde_json::to_vec_pretty(
                &json!({"system":request.system,"prompt":request.prompt,"schema":schema,"images":request.images.len()}),
            )?,
        )?;
        for (i, image) in request.images.iter().enumerate() {
            std::fs::write(
                self.output.join(format!("input-{i}.jpg")),
                base64::engine::general_purpose::STANDARD.decode(image)?,
            )?;
        }
        self.provider.complete_structured(request, schema)
    }
}

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let source = Store::open(args.next().context("source store")?)?;
    let output = PathBuf::from(args.next().context("new private output directory")?);
    ensure!(!output.exists(), "Use a new private directory");
    let step = args.next().context("step to observe")?;
    let args: Vec<_> = args.collect();
    let scheduled = args.iter().any(|a| a == "--scheduled");
    let spatial = args.iter().any(|a| a == "--spatial");
    let frames: Vec<_> = args
        .iter()
        .filter(|a| *a != "--scheduled" && *a != "--spatial")
        .map(PathBuf::from)
        .collect();
    ensure!(
        frames.len() >= 2,
        "Supply chronological before/after gameplay images"
    );
    let guide = source.guide(
        &source
            .get::<String>("active_guide")?
            .context("active guide")?,
    )?;
    let store = Store::open(&output)?;
    store.save_guide(&guide)?;
    store.put("active_guide", &guide.id)?;
    store.save_progress(
        &guide,
        &Progress {
            current_step: Some(step),
            ..if spatial {
                source.progress(&guide)?
            } else {
                Progress::default()
            }
        },
        "replay_fixture",
    )?;
    if spatial {
        for key in [
            "spatial-worker".into(),
            gamebuddy_services::spatial::key(&guide),
            format!("world-profile:{}", guide.id),
            format!("world-status:{}", guide.id),
            format!("spatial-maps:{}", guide.id),
            format!("atlas:{}", guide.id),
        ] {
            if let Some(value) = source.get::<serde_json::Value>(&key)? {
                store.put(&key, &value)?;
            }
        }
    }
    let ticket = screen::start(&store, &guide.game, true)?;
    if spatial
        && let Some(mut advice) =
            source.get::<serde_json::Value>(&format!("next-action:{}", guide.id))?
    {
        advice["session"] = json!(ticket.session);
        advice.as_object_mut().unwrap().remove("movement");
        store.put(&format!("next-action:{}", guide.id), &advice)?;
    }
    for path in frames {
        screen::publish(&store, &ticket, &path, "recorded_gameplay_replay")?;
        if spatial {
            let frame = screen::snapshot(&store)?;
            gamebuddy_services::spatial::observe(
                &store,
                &guide,
                &frame,
                store.progress(&guide)?.epoch,
            )?;
        }
        std::thread::sleep(Duration::from_millis(510));
    }
    let frame = screen::snapshot(&store)?;
    let settings: Settings = source.get("settings")?.unwrap_or_default();
    let start = std::time::Instant::now();
    let (review, completed) = if scheduled {
        store.put("settings", &settings)?;
        activity::schedule(&store, &guide, frame);
        let status = loop {
            if let Some(status) = store.get::<serde_json::Value>("activity-status")? {
                break status;
            }
            ensure!(
                start.elapsed().as_secs() < 60,
                "Background review timed out"
            );
            std::thread::sleep(Duration::from_millis(100));
        };
        ensure!(
            status["status"] == "watching",
            "Review was discarded: {status}"
        );
        (None, status["completed"].as_u64().unwrap_or(0) as usize)
    } else {
        let observation = activity::prepare(&store, &guide, &frame)?;
        let review = activity::assess(
            &RecordedProvider {
                provider: HttpProvider::new(settings.reasoning)?,
                output: output.clone(),
            },
            &observation,
        )?;
        let completed = activity::save_review(&store, &observation, &review)?;
        (Some(review), completed)
    };
    let progress = store.progress(&guide)?;
    let navigation =
        gamebuddy_services::navigation::context(&store, &guide, &progress, &Default::default())?;
    let result = json!({"elapsed_ms":start.elapsed().as_millis(),"review":review,"completed":completed,"progress":progress,
        "guidance":navigation["guidance"],"coaching":navigation["coaching"],"observed_progress":store.observed_progress(&guide,&progress)?,"activity":store.get::<serde_json::Value>("activity-status")?});
    std::fs::write(
        output.join("result.json"),
        serde_json::to_vec_pretty(&result)?,
    )?;
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}
