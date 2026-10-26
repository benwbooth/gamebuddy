//! Data cards and charts a walkthrough shows: every item, weapon, armor,
//! magic, key and boss with its prices by town, stats and the narrator's
//! tips, plus tables such as a rank chart. `reference_scan` finds them while
//! the guide is prepared; `sync_widgets` keeps them on screen.
use crate::Store;
use anyhow::{Context, Result};
use gamebuddy_core::{Guide, Widget, WidgetContent, WidgetItemSources};
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
        if name.is_empty() {
            continue;
        }
        if let Some(existing) = kept.iter_mut().find(|k| {
            k["name"].as_str().unwrap_or_default().to_lowercase() == name
                && k["category"].as_str().unwrap_or_default().to_lowercase() == category
        }) {
            // A card shown again in a later town adds that town's price.
            let more: Vec<Value> = card["costs"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|c| {
                    !existing["costs"].as_array().is_some_and(|have| {
                        have.iter().any(|h| {
                            h["place"].as_str().map(str::to_lowercase)
                                == c["place"].as_str().map(str::to_lowercase)
                        })
                    })
                })
                .cloned()
                .collect();
            if let Some(costs) = existing["costs"].as_array_mut() {
                costs.extend(more);
            }
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

/// The guide step whose video interval covers a card's timestamp, so the
/// widget links back to where the walkthrough showed it.
fn step_at(guide: &Guide, card: &Value) -> Option<String> {
    let ms = card["video_ms"].as_u64()?;
    guide
        .steps
        .iter()
        .filter(|s| guide.step_enabled(s))
        .filter(|s| {
            s.anchors.iter().any(|a| {
                a.start_ms.is_some_and(|start| start <= ms) && a.end_ms.is_some_and(|end| ms <= end)
            })
        })
        // The narrowest interval is the most specific step.
        .min_by_key(|s| {
            s.anchors
                .iter()
                .filter_map(|a| Some(a.end_ms? - a.start_ms?))
                .min()
                .unwrap_or(u64::MAX)
        })
        .map(|s| s.id.clone())
}

fn chart(card: &Value) -> Option<(Vec<String>, Vec<Vec<String>>)> {
    let table = &card["table"];
    let cell = |v: &Value| {
        v.as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| v.to_string())
    };
    let columns: Vec<String> = table["columns"]
        .as_array()?
        .iter()
        .map(cell)
        .take(12)
        .collect();
    let rows: Vec<Vec<String>> = table["rows"]
        .as_array()?
        .iter()
        .filter_map(Value::as_array)
        .take(100)
        .map(|row| {
            let mut row: Vec<String> = row.iter().map(cell).take(columns.len()).collect();
            row.resize(columns.len(), String::new());
            row
        })
        .collect();
    (!columns.is_empty() && !rows.is_empty()).then_some((columns, rows))
}

fn summary(card: &Value, field: &str, part: impl Fn(&Value) -> Option<String>) -> String {
    card[field]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(part)
        .collect::<Vec<_>>()
        .join(" · ")
}

/// GameBuddy's own reference widgets: charts that apply all game long (a
/// rank or level table) stay up, and a data widget follows the player with
/// the prices, stats and tips for what the route around them names. The
/// player can hide either; hidden widgets stay hidden.
pub fn sync_widgets(store: &Store, guide: &Guide) -> Result<()> {
    let all = cards(store, guide)?;
    if all.is_empty() {
        return Ok(());
    }
    let mut wanted: Vec<Widget> = vec![];
    for card in all.iter().filter(|c| c["scope"] != "area") {
        let Some((columns, rows)) = chart(card) else {
            continue;
        };
        let name = card["name"].as_str().unwrap_or("Chart");
        let slug: String = name
            .to_lowercase()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .take(40)
            .collect();
        if wanted.iter().any(|w| w.id == format!("chart-{slug}")) {
            continue;
        }
        wanted.push(Widget {
            id: format!("chart-{slug}"),
            title: name.to_owned(),
            content: WidgetContent::Table { columns, rows },
            source_step_ids: step_at(guide, card).into_iter().collect(),
            item_sources: vec![],
        });
        if wanted.len() == 2 {
            break;
        }
    }
    let nearby: Vec<Value> = crate::navigation::planning::area_cards(store, guide)?
        .into_iter()
        .filter(|c| chart(c).is_none())
        .take(8)
        .collect();
    let mut managed = vec!["area-data".to_owned()];
    managed.extend(wanted.iter().map(|w| w.id.clone()));
    if !nearby.is_empty() {
        let rows: Vec<Vec<String>> = nearby
            .iter()
            .map(|c| {
                vec![
                    c["name"].as_str().unwrap_or_default().to_owned(),
                    summary(c, "costs", |p| {
                        let place = p["place"].as_str().unwrap_or_default();
                        match p["gold"].as_u64() {
                            Some(gold) => Some(format!("{place} {gold}").trim().to_owned()),
                            None => p["text"]
                                .as_str()
                                .filter(|t| !t.is_empty())
                                .map(str::to_owned),
                        }
                    }),
                    summary(c, "stats", |s| {
                        Some(format!(
                            "{} {}",
                            s["name"].as_str()?,
                            s["value"].as_str().unwrap_or_default()
                        ))
                    }),
                    c["notes"]
                        .as_str()
                        .unwrap_or_default()
                        .chars()
                        .take(220)
                        .collect(),
                ]
            })
            .collect();
        let item_sources = nearby
            .iter()
            .enumerate()
            .filter_map(|(i, c)| {
                Some(WidgetItemSources {
                    item_index: i,
                    source_step_ids: vec![step_at(guide, c)?],
                })
            })
            .collect();
        wanted.push(Widget {
            id: "area-data".into(),
            title: "Prices, stats and tips nearby".into(),
            content: WidgetContent::Table {
                columns: ["Name", "Cost", "Stats", "Tips"].map(String::from).to_vec(),
                rows,
            },
            source_step_ids: vec![],
            item_sources,
        });
    }
    let current = crate::harness::widgets(store, guide)?;
    let mut next: Vec<Widget> = current
        .iter()
        .filter(|w| !managed.contains(&w.id) || wanted.iter().any(|x| x.id == w.id))
        .cloned()
        .collect();
    for widget in wanted {
        if widget.validate_sources(guide).is_err() {
            continue;
        }
        match next.iter().position(|w| w.id == widget.id) {
            Some(i) => next[i] = widget,
            // Leave room for widgets the voice shows.
            None if next.len() < 10 => next.push(widget),
            None => {}
        }
    }
    if serde_json::to_value(&next)? != serde_json::to_value(&current)? {
        store.put(&format!("widgets:{}:{}", guide.id, guide.revision), &next)?;
    }
    Ok(())
}

pub fn import_file(store: &Store, path: &std::path::Path) -> Result<usize> {
    let id: String = store.get("active_guide")?.context("No active guide")?;
    let guide = store.guide(&id)?;
    let cards: Vec<Value> = serde_json::from_slice(&std::fs::read(path)?)?;
    import(store, &guide, &cards)
}
