//! Live restart proof using the installed, signed-in Codex CLI. The random
//! nonce exists only in a tool result, never in the saved transcript/bootstrap.
use anyhow::{Context, Result, ensure};
use gamebuddy_services::{
    HttpProvider, ModelRequest, ProviderConfig, ProviderKind, ReasoningProvider, Store,
    subscription::{
        CodexClient,
        session::{Options, Role},
    },
};
use serde_json::json;
use std::time::{Duration, Instant};

fn open(store: &Store, game: &str) -> Result<(CodexClient, String, bool)> {
    let mut client = CodexClient::start()?;
    let deadline = Instant::now() + Duration::from_secs(30);
    let tier = client.voice_tier("gpt-6.1-sol", true, deadline);
    let session = client.saved_thread(store, game, Role::Reasoner, Options {
        model: "gpt-6.1-sol", instructions: "Follow the player's current request. Retain tool results in conversation memory. Never repeat an earlier tool action unless explicitly requested anew.",
        tools: json!([{"name":"read_restart_token","description":"Read the token used to verify saved tool history.","inputSchema":{"type":"object","properties":{},"additionalProperties":false},"type":"function"}]),
        tier: Some(tier), effort: "low",
    }, deadline)?;
    Ok((client, session.id, session.resumed))
}

fn turn(
    client: &mut CodexClient,
    thread: &str,
    prompt: &str,
    token: Option<&str>,
) -> Result<String> {
    let deadline = Instant::now() + Duration::from_secs(120);
    client.request(
        "turn/start",
        json!({"threadId":thread,"input":[{"type":"text","text":prompt}],"effort":"low"}),
        deadline,
    )?;
    let mut answer = String::new();
    let mut calls = 0;
    loop {
        let event = client.receive(deadline)?;
        if event["method"] == "item/tool/call" {
            ensure!(
                event["params"]["threadId"] == thread,
                "Tool called on wrong thread"
            );
            ensure!(
                event["params"]["tool"] == "read_restart_token",
                "Unexpected tool"
            );
            let token =
                token.context("Old tool action was replayed instead of remembering its result")?;
            calls += 1;
            client.send(json!({"id":event["id"],"result":{"success":true,"contentItems":[{"type":"inputText","text":token}]}}))?;
        } else {
            client.reject(&event)?;
        }
        if event["params"]["threadId"] != thread {
            continue;
        }
        if event["method"] == "item/completed" && event["params"]["item"]["type"] == "agentMessage"
        {
            answer = event["params"]["item"]["text"]
                .as_str()
                .unwrap_or_default()
                .into();
        }
        if event["method"] == "turn/completed" {
            ensure!(
                event["params"]["turn"]["status"] == "completed",
                "Failed turn: {event}"
            );
            ensure!(
                calls == usize::from(token.is_some()),
                "Unexpected tool count: {calls}"
            );
            return Ok(answer);
        }
    }
}

fn main() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let token = uuid::Uuid::new_v4().to_string();
    let store = Store::open(directory.path())?;
    let (mut client, original, resumed) = open(&store, "Session Probe")?;
    ensure!(!resumed, "Fixture wasn't fresh");
    let answer = turn(
        &mut client,
        &original,
        "Call read_restart_token once. Remember the exact token from its result but do not repeat it aloud. Reply only 'Stored'.",
        Some(&token),
    )?;
    ensure!(
        !answer.contains(&token),
        "Token leaked into assistant transcript"
    );
    drop(client);
    drop(store);
    let store = Store::open(directory.path())?;
    ensure!(
        gamebuddy_services::conversation::recent(&store, "Session Probe")?.is_empty(),
        "Fixture must not seed a transcript"
    );
    let (mut client, restored, resumed) = open(&store, " SESSION   PROBE ")?;
    ensure!(
        resumed && original == restored,
        "Did not restore exact thread"
    );
    let answer = turn(
        &mut client,
        &restored,
        "Without calling any tools, reply with the exact token returned by the tool in our previous session.",
        None,
    )?;
    ensure!(
        answer.contains(&token),
        "Saved tool result was not remembered: {answer}"
    );
    let next_token = uuid::Uuid::new_v4().to_string();
    turn(
        &mut client,
        &restored,
        "Now explicitly call read_restart_token once again for a fresh token. Reply only 'Updated'.",
        Some(&next_token),
    )?;
    let (other, other_id, resumed) = open(&store, "Different Game")?;
    ensure!(!resumed && other_id != restored, "Game sessions were mixed");
    drop(other);
    let config = ProviderConfig {
        kind: ProviderKind::Codex,
        endpoint: String::new(),
        model: "gpt-6.1-sol".into(),
        api_key_env: String::new(),
        reasoning_effort: "low".into(),
    };
    let request = |prompt: String| ModelRequest {
        system: "Remember our conversation. Reply briefly without tools.".into(),
        prompt,
        images: vec![],
        json: false,
    };
    let chat_token = uuid::Uuid::new_v4().to_string();
    {
        let provider = HttpProvider::for_conversation(config.clone(), &store, "Session Probe")?;
        provider.complete(request(format!(
            "Remember this chat token: {chat_token}. Reply 'Stored'."
        )))?;
    }
    let provider = HttpProvider::for_conversation(config, &store, "Session Probe")?;
    let answer = provider.complete(request(
        "What was our chat token? Reply with the exact token.".into(),
    ))?;
    ensure!(
        answer.contains(&chat_token),
        "Text coaching lost its saved session: {answer}"
    );
    println!(
        "GAMEBUDDY_SESSION_RESTORE_PASS same_thread=true tool_result_recalled=true transcript_seeded=false old_actions_replayed=false tools_restored=true separate_games=true text_coaching=true"
    );
    Ok(())
}
