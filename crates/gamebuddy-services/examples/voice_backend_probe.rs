//! Exercise the same native image/tool loop as live voice using an isolated
//! data directory. Optional recorded frame replay never touches a running game.
use anyhow::{Context, Result, ensure};
use gamebuddy_services::{Store, harness, screen, subscription::CodexClient, voice::prompts};
use serde_json::json;
use std::time::{Duration, Instant};

fn main() -> Result<()> {
    let directory = std::env::args()
        .nth(1)
        .context("Private data directory required")?;
    let question = std::env::args().nth(2).context("Question required")?;
    let store = Store::open(directory)?;
    let model = std::env::var("GAMEBUDDY_PROBE_MODEL").unwrap_or_default();
    let replay = std::env::var("GAMEBUDDY_PROBE_FRAME")
        .ok()
        .map(std::path::PathBuf::from);
    let ticket = if replay.is_some() {
        let id: String = store
            .get("active_guide")?
            .context("Active guide required")?;
        Some(screen::start(&store, &store.guide(&id)?.game, true)?)
    } else {
        None
    };
    if std::env::var_os("GAMEBUDDY_PROBE_TRACK").is_some() {
        let id: String = store
            .get("active_guide")?
            .context("Active guide required")?;
        let guide = store.guide(&id)?;
        let world = gamebuddy_services::spatial::load_cached(&store, &guide)?
            .context("Prepared world required")?;
        gamebuddy_services::spatial::Engine::for_world(&store, &world, "live")?;
        let (ticket, frame) = ticket
            .as_ref()
            .zip(replay.as_ref())
            .context("Recorded frame required")?;
        for _ in 0..2 {
            screen::publish(&store, ticket, frame, "recorded_test_frame")?;
            gamebuddy_services::spatial::observe(
                &store,
                &guide,
                &screen::snapshot(&store)?,
                store.progress(&guide)?.epoch,
            )?;
        }
    }
    if std::env::var_os("GAMEBUDDY_PROBE_RESTORE_ADVICE").is_some() {
        let id: String = store
            .get("active_guide")?
            .context("Active guide required")?;
        let key = format!("next-action:{id}");
        let mut advice = store
            .get::<serde_json::Value>(&key)?
            .context("Saved advice required")?;
        advice["session"] = json!(ticket.as_ref().context("Replay ticket required")?.session);
        store.put(&key, &advice)?;
    }
    // Keep the recorded screen and matcher live while the real backend thinks,
    // as the application does. Never manufacture newer observation timestamps.
    let _replay_guard = if let Some((ticket, frame)) = ticket.as_ref().zip(replay.as_ref()) {
        let alive = std::sync::Arc::new(());
        let weak = std::sync::Arc::downgrade(&alive);
        let root = store.root.clone();
        let ticket = ticket.clone();
        let frame = frame.clone();
        std::thread::spawn(move || {
            let run = || -> Result<()> {
                let store = Store::open(root)?;
                let id: String = store
                    .get("active_guide")?
                    .context("Active guide required")?;
                while weak.upgrade().is_some() {
                    screen::publish(&store, &ticket, &frame, "recorded_test_frame")?;
                    if std::env::var_os("GAMEBUDDY_PROBE_TRACK").is_some() {
                        let guide = store.guide(&id)?;
                        gamebuddy_services::spatial::schedule(
                            &store,
                            &guide,
                            screen::snapshot(&store)?,
                            store.progress(&guide)?.epoch,
                        );
                    }
                    std::thread::sleep(Duration::from_secs(1));
                }
                Ok(())
            };
            if let Err(error) = run() {
                eprintln!("Replay stopped: {error}");
            }
        });
        Some(alive)
    } else {
        None
    };
    let mut client = CodexClient::start()?;
    let immediate = std::env::var_os("GAMEBUDDY_PROBE_IMMEDIATE").is_some();
    let mut tools = if immediate {
        gamebuddy_services::voice::reply::tools()
    } else {
        harness::tool_definitions()
    };
    for tool in tools.as_array_mut().unwrap() {
        tool["type"] = json!("function")
    }
    let deadline = Instant::now() + Duration::from_secs(90);
    let thread = client.voice_thread(
        &model,
        &format!(
            "{}\n{}",
            prompts::REASONER,
            if immediate {
                gamebuddy_services::voice::reply::INSTRUCTIONS
            } else {
                ""
            }
        ),
        tools,
        std::env::var_os("GAMEBUDDY_PROBE_FAST").is_some(),
        &std::env::var("GAMEBUDDY_PROBE_EFFORT").unwrap_or_default(),
        deadline,
    )?;
    let started = Instant::now();
    let request_id = gamebuddy_services::voice::reply::request_id(1, &question);
    let mut input =
        vec![json!({"type":"text","text":format!("{question}\nrequest_id: {request_id}")})];
    let mut supplied_frame = screen::snapshot(&store).ok();
    if std::env::var_os("GAMEBUDDY_PROBE_PREFETCH").is_some() {
        if let Some((ticket, image)) = ticket.as_ref().zip(replay.as_ref()) {
            screen::publish(&store, ticket, image, "recorded_test_frame")?;
        }
        let current = harness::call_tool_with_images(&store, "get_game_context", json!({}))?;
        println!(
            "prefetched_context bytes={} images={}",
            current.value.to_string().len(),
            current.images.len()
        );
        if current.images.len() > 1 {
            use base64::Engine;
            std::fs::write(
                store.root.join("probe-map.jpg"),
                base64::engine::general_purpose::STANDARD.decode(&current.images[1])?,
            )?;
        }
        input.extend(current.codex_input("CURRENT_GAME_CONTEXT for this request. This is evidence from get_game_context, not another request."));
    }
    let effort = std::env::var("GAMEBUDDY_PROBE_EFFORT").unwrap_or_else(|_| "low".into());
    client.request(
        "turn/start",
        json!({"threadId":thread,"input":input,"effort":effort,"environments":[]}),
        deadline,
    )?;
    let mut count = 0;
    let mut first_output = true;
    loop {
        let event = client.receive(deadline)?;
        if event["method"] == "item/tool/call" {
            count += 1;
            ensure!(count <= 12, "Too many tool calls");
            if let Some((ticket, image)) = ticket.as_ref().zip(replay.as_ref()) {
                let changed =
                    std::env::var_os("GAMEBUDDY_PROBE_CHANGED_FRAME").map(std::path::PathBuf::from);
                let path = if event["params"]["tool"] == gamebuddy_services::voice::reply::TOOL {
                    changed.as_ref().unwrap_or(image)
                } else {
                    image
                };
                screen::publish(&store, ticket, path, "recorded_test_frame")?;
                if changed.is_some()
                    && event["params"]["tool"] == gamebuddy_services::voice::reply::TOOL
                {
                    ensure!(
                        supplied_frame
                            .as_ref()
                            .is_some_and(
                                |frame| !screen::still_current(&store, frame).unwrap_or(true)
                            ),
                        "Changed replay did not invalidate the supplied view"
                    );
                    println!("changed_screen_current=false");
                }
            }
            let name = event["params"]["tool"].as_str().context("Tool name")?;
            println!("request={name} arguments={}", event["params"]["arguments"]);
            if matches!(name, "get_game_context" | "get_game_screen") {
                supplied_frame = screen::snapshot(&store).ok();
            }
            let output = if name == gamebuddy_services::voice::reply::TOOL {
                gamebuddy_services::voice::reply::apply(
                    &store,
                    event["params"]["arguments"].clone(),
                    &request_id,
                    supplied_frame.as_ref(),
                )
            } else {
                harness::call_tool_with_images(&store, name, event["params"]["arguments"].clone())
            };
            let success = output.is_ok();
            let content = match output {
                Ok(output) => {
                    if let Some(reply) = output.value.get("voice_reply") {
                        println!(
                            "reply_ready_ms={} reply={}",
                            started.elapsed().as_millis(),
                            reply["text"]
                        );
                    }
                    println!(
                        "tool={name} elapsed_ms={} text_bytes={} images={}",
                        started.elapsed().as_millis(),
                        output.value.to_string().len(),
                        output.images.len()
                    );
                    output.codex_content()
                }
                Err(error) => {
                    println!("tool={name} error={error}");
                    json!([{"type":"inputText","text":error.to_string()}])
                }
            };
            client.send(
                json!({"id":event["id"],"result":{"success":success,"contentItems":content}}),
            )?;
        } else {
            client.reject(&event)?;
            if first_output && event["method"] == "item/agentMessage/delta" {
                first_output = false;
                println!("first_answer_ms={}", started.elapsed().as_millis());
            }
            if event["method"] == "item/completed"
                && event["params"]["item"]["type"] == "agentMessage"
            {
                println!("answer={}", event["params"]["item"]["text"]);
            }
            if event["method"] == "turn/completed" {
                ensure!(
                    event["params"]["turn"]["status"] == "completed",
                    "Turn failed: {}",
                    event["params"]["turn"]["error"]
                );
                println!(
                    "model={model} elapsed_ms={} tool_calls={count}",
                    started.elapsed().as_millis()
                );
                break;
            }
        }
    }
    Ok(())
}
