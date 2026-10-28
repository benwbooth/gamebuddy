//! A small local model as GameBuddy's live voice, done the way small models
//! are good at: it understands the player and calls GameBuddy's own lookups
//! (prices, routes, the next step, the walkthrough, the screen) instead of
//! answering from memory, which made it invent prices and places.
//!
//! One OpenAI-compatible chat with tools, a few rounds at most; the final
//! answer is spoken sentence by sentence.
use crate::{HttpProvider, Settings, Store};
use anyhow::{Context, Result};
use gamebuddy_core::Guide;
use serde_json::{Value, json};
use std::time::Duration;

const SYSTEM: &str = "You are GameBuddy, the player's upbeat friend on the couch who helps them play this game, talking out loud. Never guess game facts: call a tool for prices and items (item_info), directions to a place (route_to), what to do next (next_step), anything else about the game (search_walkthrough) and what is on screen (look_at_screen). When the player says they have or do not have an item, call remember_item. After the tools answer, reply in one or two short, natural spoken sentences with direct instructions (go left, buy it at the shop for its price). Never say \"the walkthrough\" or \"the tool\". If they are just chatting, chat back briefly without tools. Believe the player about where they are and what they have. Only say what the lookups returned: if a lookup says something is unknown or failed, say you are not sure and ask; never make up directions, places or prices. Do not repeat your previous answer; answer the new question.";

pub fn tools() -> Value {
    let tool = |name: &str, description: &str, properties: Value, required: &[&str]| {
        json!({"type": "function", "function": {"name": name, "description": description,
            "parameters": {"type": "object", "properties": properties, "required": required}}})
    };
    json!([
        tool(
            "item_info",
            "Price, where to buy or find it, and what an item, spell or piece of equipment does.",
            json!({"name": {"type": "string", "description": "The item's name"}}),
            &["name"]
        ),
        tool(
            "route_to",
            "Which way to go from where the player is now to a named town, shop or area.",
            json!({"place": {"type": "string"}}),
            &["place"]
        ),
        tool(
            "next_step",
            "What the player should do next, given where they are and what they have and lack.",
            json!({}),
            &[]
        ),
        tool(
            "search_walkthrough",
            "Look up anything else about the game: bosses, keys, secrets, strategies, charts.",
            json!({"query": {"type": "string"}}),
            &["query"]
        ),
        tool(
            "look_at_screen",
            "Read the player's screen right now: HUD numbers, on-screen text, what is shown.",
            json!({}),
            &[]
        ),
        tool(
            "remember_item",
            "Record that the player has, or does not have, an item.",
            json!({"item": {"type": "string"}, "has": {"type": "boolean"}}),
            &["item", "has"]
        ),
    ])
}

fn clip(text: &str, limit: usize) -> String {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    text.chars().take(limit).collect()
}

/// Run one tool and describe the result in a few plain lines.
pub fn run_tool(store: &Store, guide: &Guide, name: &str, args: &Value) -> String {
    let result = (|| -> Result<String> {
        Ok(match name {
            "item_info" => {
                let query = args["name"].as_str().unwrap_or_default();
                let found = crate::game_data::search(store, guide, query)?;
                let cards: Vec<String> = found["cards"]
                    .as_array()
                    .or_else(|| found.as_array())
                    .into_iter()
                    .flatten()
                    .take(2)
                    .map(|card| {
                        let costs: Vec<String> = card["costs"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|c| {
                                Some(format!(
                                    "{} gold in {}",
                                    c["gold"].as_u64()?,
                                    c["place"].as_str()?
                                ))
                            })
                            .collect();
                        format!(
                            "{}: {}. {}",
                            card["name"].as_str().unwrap_or_default(),
                            if costs.is_empty() {
                                "no price listed".into()
                            } else {
                                costs.join(", ")
                            },
                            clip(card["notes"].as_str().unwrap_or_default(), 200)
                        )
                    })
                    .collect();
                if cards.is_empty() {
                    let steps = super::talk::passages(store, &format!("buy get {query}"), 2);
                    if steps.is_empty() {
                        format!("Nothing is known about {query}.")
                    } else {
                        steps.join("\n")
                    }
                } else {
                    cards.join("\n")
                }
            }
            "route_to" => {
                let place = args["place"].as_str().unwrap_or_default();
                let atlas = crate::navigation::atlas(store, guide)?;
                let here = super::talk::current_place(store, guide);
                crate::navigation::routing::set_from_text(
                    store,
                    guide,
                    &atlas,
                    &format!("go to {place}"),
                    here.as_deref(),
                )?;
                let goal = crate::navigation::routing::goal(store, guide)?;
                match (here, goal) {
                    (Some(here), Some(goal)) => {
                        let route =
                            crate::navigation::routing::route_view(guide, &atlas, &here, &goal);
                        match route {
                            Some(route) => {
                                let areas: Vec<&str> = route["areas_to_cross"]
                                    .as_array()
                                    .into_iter()
                                    .flatten()
                                    .filter_map(Value::as_str)
                                    .collect();
                                let first = route["hops"][0]["head"].as_str();
                                format!(
                                    "From {} to {}: {}{}",
                                    route["from"].as_str().unwrap_or("here"),
                                    goal.label,
                                    if areas.len() > 1 {
                                        format!("go through {}", areas.join(", then "))
                                    } else {
                                        "it is in this area".into()
                                    },
                                    first
                                        .map(|d| format!(". First head {d}."))
                                        .unwrap_or_default()
                                )
                            }
                            None => format!("No known way from here to {}.", goal.label),
                        }
                    }
                    (None, Some(goal)) => {
                        // The room may be unknown while the area map is known.
                        let saved: Option<Value> = store
                            .get(&crate::navigation::estimate::key(guide))
                            .ok()
                            .flatten();
                        let estimate = saved.map(|s| s["estimate"].clone()).unwrap_or_default();
                        let map = estimate["map_id"].as_str().filter(|_| {
                            matches!(estimate["status"].as_str(), Some("confident" | "likely"))
                        });
                        let position = estimate["position"]["x"]
                            .as_f64()
                            .zip(estimate["position"]["y"].as_f64())
                            .map(|(x, y)| [x as f32, y as f32]);
                        map.and_then(|map| {
                            crate::navigation::routing::route_from_map(
                                store, guide, &atlas, map, position, &goal,
                            )
                        })
                        .unwrap_or_else(|| {
                            format!(
                                "Destination set to {}, but where the player is now is not known yet; ask them.",
                                goal.label
                            )
                        })
                    }
                    _ => format!("No place called {place} is known."),
                }
            }
            "next_step" => super::talk::situation_lines(store, guide)?.join("\n"),
            "search_walkthrough" => {
                let query = args["query"].as_str().unwrap_or_default();
                let found = super::talk::passages(store, query, 3);
                if found.is_empty() {
                    "Nothing found.".into()
                } else {
                    found.join("\n")
                }
            }
            "look_at_screen" => {
                let settings: Settings = store.get("settings")?.unwrap_or_default();
                let snapshot = crate::screen::snapshot(store)?;
                let reader = HttpProvider::new(settings.vision)?;
                let reading = crate::brain::read_screen(&reader, &[snapshot.jpeg])?;
                format!(
                    "HUD: {}. On-screen text: {}. Scene: {}",
                    reading.hud.join(", "),
                    if reading.text.is_empty() {
                        "none".into()
                    } else {
                        reading.text
                    },
                    reading.scene
                )
            }
            "remember_item" => {
                let item = args["item"].as_str().unwrap_or_default();
                let has = args["has"].as_bool().unwrap_or(false);
                if ["", "none", "nothing", "item", "items"]
                    .contains(&item.trim().to_lowercase().as_str())
                {
                    return Ok("Nothing to record: name the item.".into());
                }
                let statement = format!("I {} the {item}", if has { "have" } else { "don't have" });
                match super::talk::record_possession(store, guide, &statement)? {
                    Some(_) => format!(
                        "Noted: the player {} {item}.",
                        if has { "has" } else { "does not have" }
                    ),
                    None => "Could not record that.".into(),
                }
            }
            other => format!("Unknown tool {other}."),
        })
    })();
    result.unwrap_or_else(|error| format!("That lookup failed: {error}"))
}

/// Tool calls the model wrote as text (`<tool_call><function=route_to>
/// <parameter=place>Forepaw</parameter>…`) instead of the structured field:
/// Omni does this now and then. Parameters with an unexpected name go to
/// the tool's own single parameter.
fn calls_in_text(text: &str) -> Vec<(String, Value)> {
    let mut calls = vec![];
    for chunk in text.split("<function=").skip(1) {
        let Some((name, body)) = chunk.split_once('>') else {
            continue;
        };
        let name = name.trim().to_owned();
        let mut args = serde_json::Map::new();
        for part in body.split("<parameter=").skip(1) {
            let Some((key, rest)) = part.split_once('>') else {
                continue;
            };
            let value = rest.split("</parameter>").next().unwrap_or_default().trim();
            args.insert(key.trim().to_owned(), json!(value));
        }
        let expected = match name.as_str() {
            "item_info" => Some("name"),
            "route_to" => Some("place"),
            "search_walkthrough" => Some("query"),
            _ => None,
        };
        if let Some(expected) = expected
            && !args.contains_key(expected)
            && let Some(value) = args.values().next().cloned()
        {
            args.insert(expected.into(), value);
        }
        calls.push((name, Value::Object(args)));
    }
    calls
}

/// Never speak markup: drop any tool-call tags left in an answer.
fn spoken(text: &str) -> String {
    let cut = text
        .find("<tool_call>")
        .or_else(|| text.find("<function="))
        .unwrap_or(text.len());
    text[..cut].trim().to_owned()
}

/// Answer one spoken turn with tools. Returns the spoken text.
pub fn answer(
    store: &Store,
    guide: &Guide,
    provider: &crate::ProviderConfig,
    history: &[Value],
    question: &str,
) -> Result<String> {
    crate::liquid::ready_for(provider);
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()?;
    let url = format!(
        "{}/chat/completions",
        provider.endpoint.trim_end_matches('/')
    );
    let mut messages = vec![json!({"role": "system", "content": SYSTEM})];
    for turn in history {
        let role = if turn["role"] == "user" {
            "user"
        } else {
            "assistant"
        };
        messages.push(json!({"role": role, "content": turn["text"]}));
    }
    messages.push(json!({"role": "user", "content": question}));
    let mut findings: Vec<String> = vec![];
    for round in 0..4 {
        // Two rounds of lookups at most, then answer: the small model would
        // otherwise keep looking things up and never speak.
        let last = round == 3 || findings.len() >= 2;
        if last && !findings.is_empty() {
            messages.push(json!({"role": "user", "content":
                "(GameBuddy: you have what you need. Answer the player now in one or two short spoken sentences, using the lookups above. No more lookups.)"}));
        }
        let mut body = json!({"model": provider.model, "messages": messages,
            "temperature": 0.1, "top_k": 50, "repeat_penalty": 1.1, "max_tokens": 300});
        if !last {
            body["tools"] = tools();
        }
        let reply: Value = client
            .post(&url)
            .json(&body)
            .send()
            .context("Could not reach the local model")?
            .error_for_status()?
            .json()?;
        let message = reply["choices"][0]["message"].clone();
        if std::env::var_os("GAMEBUDDY_AGENT_TRACE").is_some() {
            eprintln!(
                "AGENT round {round}: {}",
                message.to_string().chars().take(600).collect::<String>()
            );
        }
        let content = message["content"].as_str().unwrap_or_default().to_owned();
        let structured: Vec<(String, String, Value)> = message["tool_calls"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|call| {
                (
                    call["id"].as_str().unwrap_or_default().to_owned(),
                    call["function"]["name"]
                        .as_str()
                        .unwrap_or_default()
                        .to_owned(),
                    call["function"]["arguments"]
                        .as_str()
                        .and_then(|a| serde_json::from_str(a).ok())
                        .unwrap_or(Value::Null),
                )
            })
            .collect();
        let calls: Vec<(String, String, Value)> = if structured.is_empty() {
            calls_in_text(&content)
                .into_iter()
                .enumerate()
                .map(|(i, (name, args))| (format!("text-{round}-{i}"), name, args))
                .collect()
        } else {
            structured
        };
        if calls.is_empty() || last {
            let text = spoken(&content);
            if !text.is_empty() {
                return Ok(text);
            }
            // Still no words: ask once more with the lookups as plain facts
            // and no tool history, which small models answer far more often.
            // Raw lookup output is never spoken; it reads as gibberish.
            anyhow::ensure!(!findings.is_empty(), "No answer");
            return plain_answer(&client, &url, provider, question, &findings);
        }
        // Keep the exchange in the shape the chat template expects.
        messages.push(json!({"role": "assistant", "content": spoken(&content),
            "tool_calls": calls.iter().map(|(id, name, args)| json!({"id": id, "type": "function",
                "function": {"name": name, "arguments": args.to_string()}})).collect::<Vec<_>>()}));
        for (id, name, args) in calls {
            let output = run_tool(store, guide, &name, &args);
            if std::env::var_os("GAMEBUDDY_AGENT_TRACE").is_some() {
                eprintln!("TOOL {name} {args} -> {}", clip(&output, 400));
            }
            crate::flight::event(
                "voice_tool",
                json!({"name": name, "args": args, "output": clip(&output, 300)}),
            );
            if !output.starts_with("That lookup failed") && !output.starts_with("Unknown tool") {
                findings.push(output.clone());
            }
            messages.push(json!({"role": "tool", "tool_call_id": id, "content": output}));
        }
    }
    anyhow::bail!("No answer after several lookups")
}

/// One plain request: the facts the lookups found, the question, and nothing
/// to call. Falls back to asking the player rather than reading facts aloud.
fn plain_answer(
    client: &reqwest::blocking::Client,
    url: &str,
    provider: &crate::ProviderConfig,
    question: &str,
    findings: &[String],
) -> Result<String> {
    let facts = findings
        .iter()
        .map(|f| format!("- {}", clip(f, 400)))
        .collect::<Vec<_>>()
        .join("\n");
    let body = json!({"model": provider.model, "messages": [
            {"role": "system", "content": "You are GameBuddy, the player's upbeat gaming friend. Reply in one or two short, natural spoken sentences. Use only the facts given. Never read out codes, numbers lists or HUD labels. If the facts say something is not known, ask the player about it."},
            {"role": "user", "content": format!("Facts:\n{facts}\n\nThe player said: {question}")}],
        "temperature": 0.1, "top_k": 50, "repeat_penalty": 1.1, "max_tokens": 120});
    let reply: Value = client
        .post(url)
        .json(&body)
        .send()
        .context("Could not reach the local model")?
        .error_for_status()?
        .json()?;
    let text = spoken(
        reply["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or_default(),
    );
    if text.is_empty() {
        return Ok("Sorry, I couldn't work that out. Where are you right now?".to_owned());
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    #[test]
    fn tool_calls_written_as_text_are_understood() {
        let calls = super::calls_in_text(
            "<tool_call>\n<function=search_walkthrough>\n<parameter=goal>\nwhat should I do next\n</parameter>\n</function>\n</tool_call>",
        );
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "search_walkthrough");
        assert_eq!(calls[0].1["query"], "what should I do next");
        assert_eq!(super::spoken("Sure! <tool_call><function=x>"), "Sure!");
    }
}
