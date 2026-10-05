//! Model discovery used by the native settings page. Listing is not an inference test.
use crate::{HttpProvider, ModelRequest, ProviderConfig, ProviderKind, ReasoningProvider};
use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

pub fn discover(config: &ProviderConfig) -> Result<Value> {
    // Model discovery also works before a model has been selected.
    let mut checked = config.clone();
    if checked.model.is_empty() {
        checked.model = "discovery".into();
    }
    checked.validate()?;
    match config.kind {
        ProviderKind::Codex => {
            let mut client = crate::subscription::CodexClient::start()?;
            let deadline = Instant::now() + Duration::from_secs(20);
            let mut models = vec![];
            let mut cursor = Value::Null;
            loop {
                let page =
                    client.request("model/list", json!({"limit":100,"cursor":cursor}), deadline)?;
                for model in page["data"]
                    .as_array()
                    .context("Codex returned no model catalog")?
                {
                    let efforts: Vec<_> = model["supportedReasoningEfforts"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|e| e["reasoningEffort"].as_str())
                        .collect();
                    models.push(json!({"id":model["model"],"label":model["displayName"],"efforts":efforts,
                        "vision":model["inputModalities"].as_array().is_none_or(|a|a.iter().any(|v|v=="image"))}));
                }
                cursor = page["nextCursor"].clone();
                if cursor.is_null() {
                    break;
                }
            }
            client.thread(
                "",
                "Model configuration check. Do not start a turn.",
                deadline,
            )?;
            Ok(
                json!({"models":models,"default_model":client.resolved_model,"note":"GameBuddy defaults to GPT-6 Luna with Low thinking. Choose another model to override it; Test connection checks access with your subscription."}),
            )
        }
        ProviderKind::ClaudeCode => {
            // Stable CLI aliases avoid pinning a stale model generation.
            // Haiku has no effort control. Explicit IDs can still be entered.
            Ok(json!({"models":[
                {"id":"sonnet","label":"Sonnet","efforts":["low","medium","high","max"],"vision":true},
                {"id":"opus","label":"Opus","efforts":["low","medium","high","xhigh","max"],"vision":true},
                {"id":"haiku","label":"Haiku","efforts":[],"vision":true}],
                "note":"Uses your Claude Code account login. Aliases follow the installed client; Test connection checks access. Custom model IDs are also accepted."}))
        }
        ProviderKind::Ollama => {
            let client = reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(15))
                .build()?;
            let base = config.endpoint.trim_end_matches('/');
            let tags: Value = client
                .get(format!("{base}/api/tags"))
                .send()?
                .error_for_status()?
                .json()?;
            let mut models = vec![];
            for tag in tags["models"]
                .as_array()
                .context("Ollama returned no models")?
            {
                let name = tag["name"].as_str().context("Missing Ollama model name")?;
                let details: Value = client
                    .post(format!("{base}/api/show"))
                    .json(&json!({"model":name}))
                    .send()?
                    .error_for_status()?
                    .json()?;
                models.push(ollama_model(name, &details));
            }
            Ok(
                json!({"models":models,"note":"Installed models at this Ollama address. Screen interpretation requires a vision model."}),
            )
        }
        _ => Ok(
            json!({"models":[],"note":"Enter the model ID provided by your API service. Thinking defaults to the provider setting."}),
        ),
    }
}

fn ollama_model(name: &str, details: &Value) -> Value {
    let capabilities = details["capabilities"].as_array();
    let thinking = capabilities.is_some_and(|a| a.iter().any(|v| v == "thinking"));
    let efforts: Vec<String> = if let Some(values) = details["thinking"]["values"].as_array() {
        values
            .iter()
            .filter_map(|value| match value {
                Value::Bool(true) => Some("on".into()),
                Value::Bool(false) => Some("off".into()),
                Value::String(s) => Some(s.clone()),
                _ => None,
            })
            .collect()
    } else if thinking {
        // Older Ollama versions expose capabilities but no thinking metadata.
        if name
            .split(':')
            .next()
            .unwrap_or(name)
            .starts_with("gpt-oss")
        {
            vec!["low".into(), "medium".into(), "high".into()]
        } else {
            vec!["off".into(), "on".into()]
        }
    } else {
        vec![]
    };
    json!({"id":name,"label":name,"efforts":efforts,
        "vision":capabilities.map(|a|a.iter().any(|v|v=="vision"))})
}

pub fn test(config: ProviderConfig) -> Result<Value> {
    let started = Instant::now();
    let answer = HttpProvider::new(config)?.complete(ModelRequest {
        system: "This is a connection check. Reply with the single word READY. Use no tools."
            .into(),
        prompt: "Reply READY.".into(),
        images: vec![],
        json: false,
    })?;
    Ok(
        json!({"elapsed_ms":started.elapsed().as_millis(),"reply":answer.chars().take(100).collect::<String>()}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn thinking_options_follow_model_capabilities() {
        assert_eq!(
            ollama_model("qwen", &json!({"capabilities":["vision","thinking"]}))["efforts"],
            json!(["off", "on"])
        );
        assert_eq!(
            ollama_model("gpt-oss:20b", &json!({"capabilities":["thinking"]}))["efforts"],
            json!(["low", "medium", "high"])
        );
        assert_eq!(
            ollama_model(
                "custom",
                &json!({"thinking":{"values":[false,"low","high"]}})
            )["efforts"],
            json!(["off", "low", "high"])
        );
        assert_eq!(
            ollama_model("ocr", &json!({"capabilities":["vision"]}))["efforts"],
            json!([])
        );
    }
}
