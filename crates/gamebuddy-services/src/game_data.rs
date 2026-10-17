//! Data cards transcribed from a walkthrough video's side panels: every
//! item, weapon, armor, magic, key and boss with its prices by town, stats
//! and the narrator's tips, plus tables such as the rank chart. The video
//! crop used for screen matching keeps only the game area, so these panels
//! were otherwise lost.
use crate::Store;
use anyhow::{Context, Result};
use gamebuddy_core::Guide;
use serde_json::{Value, json};

fn key(guide: &Guide) -> String {
    format!("game-data:{}", guide.id)
}

/// Replace the guide's cards with transcribed ones (non-cards are dropped;
/// repeated cards keep their first appearance).
pub fn import(store: &Store, guide: &Guide, cards: &[Value]) -> Result<usize> {
    let mut kept: Vec<Value> = vec![];
    for card in cards.iter().filter(|c| c["is_card"] == true) {
        let name = card["name"]
            .as_str()
            .unwrap_or_default()
            .trim()
            .to_lowercase();
        let category = card["category"]
            .as_str()
            .unwrap_or_default()
            .trim()
            .to_lowercase();
        if name.is_empty()
            || kept.iter().any(|k| {
                k["name"].as_str().unwrap_or_default().to_lowercase() == name
                    && k["category"].as_str().unwrap_or_default().to_lowercase() == category
            })
        {
            continue;
        }
        let mut card = card.clone();
        if let Some(object) = card.as_object_mut() {
            object.remove("is_card");
            object.remove("model");
            let at = object.get("t").and_then(Value::as_u64).unwrap_or(0);
            object.insert("video_ms".into(), json!(at * 1000));
            object.remove("t");
        }
        kept.push(card);
    }
    store.put(&key(guide), &kept)?;
    Ok(kept.len())
}

pub fn cards(store: &Store, guide: &Guide) -> Result<Vec<Value>> {
    Ok(store.get::<Vec<Value>>(&key(guide))?.unwrap_or_default())
}

/// Cards matching a query (name, category or text), best matches first.
pub fn search(store: &Store, guide: &Guide, query: &str) -> Result<Value> {
    let all = cards(store, guide)?;
    let words: Vec<String> = query
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() > 2)
        .map(str::to_owned)
        .collect();
    let mut scored: Vec<(usize, &Value)> = all
        .iter()
        .map(|card| {
            let name = card["name"].as_str().unwrap_or_default().to_lowercase();
            let text = card.to_string().to_lowercase();
            let score = words
                .iter()
                .map(|w| {
                    usize::from(text.contains(w.as_str()))
                        + 3 * usize::from(name.contains(w.as_str()))
                })
                .sum();
            (score, card)
        })
        .filter(|(score, _)| *score > 0 || words.is_empty())
        .collect();
    scored.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
    Ok(json!({
        "cards": scored.into_iter().take(8).map(|(_, c)| c).collect::<Vec<_>>(),
        "all_names": all.iter().map(|c| format!("{} ({})", c["name"].as_str().unwrap_or_default(), c["category"].as_str().unwrap_or_default())).collect::<Vec<_>>(),
        "instruction": "Data cards from the walkthrough video's side panels: prices by town, stats and tips. To show one, or a table such as Ranks, use show_widget with a table (columns and rows) or text."
    }))
}

/// Cards whose names appear in the given text, for area notes.
pub fn mentioned(store: &Store, guide: &Guide, text: &str) -> Result<Vec<Value>> {
    let lower = text.to_lowercase();
    Ok(cards(store, guide)?
        .into_iter()
        .filter(|card| {
            card["name"]
                .as_str()
                .map(|n| n.trim().to_lowercase())
                // "Gold" is everywhere and says nothing about a card.
                .is_some_and(|n| n.len() > 3 && n != "gold" && lower.contains(&n))
        })
        .take(12)
        .collect())
}

pub fn import_file(store: &Store, path: &std::path::Path) -> Result<usize> {
    let id: String = store.get("active_guide")?.context("No active guide")?;
    let guide = store.guide(&id)?;
    let cards: Vec<Value> = serde_json::from_slice(&std::fs::read(path)?)?;
    import(store, &guide, &cards)
}
