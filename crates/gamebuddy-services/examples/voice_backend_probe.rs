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
    let mut client = CodexClient::start()?;
    let mut tools = harness::tool_definitions();
    for tool in tools.as_array_mut().unwrap() {
        tool["type"] = json!("function")
    }
    let deadline = Instant::now() + Duration::from_secs(90);
    let thread = client.voice_thread(
        &model,
        prompts::REASONER,
        tools,
        std::env::var_os("GAMEBUDDY_PROBE_FAST").is_some(),
        deadline,
    )?;
    let started = Instant::now();
    let mut input = vec![json!({"type":"text","text":question})];
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
        input.extend(current.codex_input("CURRENT_GAME_CONTEXT for this request. This is evidence from get_game_context, not another request."));
    }
    client.request(
        "turn/start",
        json!({"threadId":thread,"input":input,"effort":"low","environments":[]}),
        deadline,
    )?;
    let mut count = 0;
    loop {
        let event = client.receive(deadline)?;
        if event["method"] == "item/tool/call" {
            count += 1;
            ensure!(count <= 12, "Too many tool calls");
            if let Some((ticket, image)) = ticket.as_ref().zip(replay.as_ref()) {
                screen::publish(&store, ticket, image, "recorded_test_frame")?;
            }
            let name = event["params"]["tool"].as_str().context("Tool name")?;
            let output =
                harness::call_tool_with_images(&store, name, event["params"]["arguments"].clone());
            let success = output.is_ok();
            let content = match output {
                Ok(output) => {
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
