//! Facts first, words last: the spoken turn for models on this computer.
//!
//! GameBuddy's own code works out what the player asked (the intent), looks
//! the answer up (routes, prices, the next step, the screen, where the
//! player is) and writes it as a few plain fact lines. The language model is
//! called once, to say those facts as a friend would. Every number and place
//! name it says must be in the facts; a sentence that fails that check is
//! replaced by a plain sentence built from the facts. Small models invented
//! prices, copied old answers and jumbled routes when they did the deciding;
//! here they cannot.
use crate::{HttpProvider, ModelRequest, Settings, Store};
use anyhow::Result;
use gamebuddy_core::{Guide, navigation::Atlas};
use serde_json::{Value, json};

use super::talk;
use crate::navigation::routing;

const SPEAK: &str = "You are GameBuddy, the player's upbeat friend on the couch, talking out loud while they play a game. You are given facts that GameBuddy already looked up. Say the answer to the player in one or two short, natural spoken sentences (under 35 words), as direct instructions where they apply. If the player is reacting rather than asking (they died, they are stuck, they are annoyed), answer like a friend first in a few words, then help. Use only the facts: every price, number and place name you say must come from them, and say nothing the facts do not support. If a fact says something is not known, say so in a few words and ask the player. Do not read HUD readings or codes aloud unless the player asked about them. No lists, no markdown, no ids, no written sounds like \"Mm\" or \"Hmm\". Never say \"the walkthrough\", \"the facts\" or \"the lookup\", and never start with your own name.";

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// How to get somewhere.
    Travel,
    /// Where to get, or what something costs or does.
    Item,
    /// What to do now.
    Next,
    /// Why the next step.
    Why,
    /// Say the last answer again.
    Repeat,
    /// Where am I.
    Location,
    /// What is on the screen.
    Screen,
    /// The player reports what they have or lack, asking nothing.
    Statement,
    /// Any other question about the game.
    General,
    /// Not about the game.
    Chat,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct Intent {
    pub kind: Kind,
    /// A place id, for travel.
    pub place: Option<String>,
    /// A data card's name, for items.
    pub item: Option<String>,
}

/// One answered turn, with what went into it.
#[derive(Clone, Debug, serde::Serialize)]
pub struct Turn {
    pub reply: String,
    pub intent: Intent,
    pub facts: Vec<String>,
    /// Every sentence spoken passed the fact check.
    pub grounded: bool,
    /// The plain sentence stood in for the model's words.
    pub templated: bool,
    pub ms: u64,
    pub first_sentence_ms: Option<u64>,
}

fn clip(text: &str, limit: usize) -> String {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().count() <= limit {
        return text;
    }
    let cut: String = text.chars().take(limit).collect();
    format!(
        "{}…",
        cut.rsplit_once(' ').map_or(cut.as_str(), |(head, _)| head)
    )
}

fn asks(lower: &str) -> bool {
    lower.contains('?')
        || [
            "where", "how", "what", "which", "can i", "do i", "should i", "is there", "tell me",
            "any idea", "when", "who",
        ]
        .iter()
        .any(|w| lower.contains(w))
}

/// The data card the sentence names, matched on its words as heard (a name
/// split or misheard by a letter or two still counts).
fn named_item(store: &Store, guide: &Guide, text: &str) -> Option<String> {
    let said = routing::heard_words(text);
    crate::game_data::cards(store, guide)
        .unwrap_or_default()
        .iter()
        .filter_map(|card| {
            let name = card["name"].as_str()?.trim();
            let words: Vec<String> = name
                .to_lowercase()
                .split(|c: char| !c.is_alphanumeric())
                .filter(|w| w.len() > 1)
                .map(str::to_owned)
                .collect();
            if words.is_empty() || name.eq_ignore_ascii_case("gold") {
                return None;
            }
            let hit = words.iter().filter(|w| routing::heard(&said, w)).count();
            // Every word of a short name, or most of a long one.
            (hit == words.len() || (words.len() >= 3 && hit + 1 >= words.len()))
                .then(|| (hit, name.len(), name.to_owned()))
        })
        .max()
        .map(|(_, _, name)| name)
}

/// What the player asked, by rules first (instant, never wrong about a
/// phrase they match), then by the local model with a fixed set of answers
/// for the rest.
pub fn classify(
    store: &Store,
    guide: &Guide,
    atlas: &Atlas,
    text: &str,
    speaker: Option<&HttpProvider>,
) -> Intent {
    let lower = text.to_lowercase().replace(['\'', '’'], "");
    let plain = |kind| Intent {
        kind,
        place: None,
        item: None,
    };
    if let Some(kind) = super::prepared::intent(text) {
        return plain(match kind {
            "next" => Kind::Next,
            "why" => Kind::Why,
            "repeat" => Kind::Repeat,
            _ => Kind::Location,
        });
    }
    if let Some(place) = routing::destination(guide, atlas, text) {
        return Intent {
            kind: Kind::Travel,
            place: Some(place),
            item: None,
        };
    }
    let item = named_item(store, guide, text);
    let statement = talk::possession_statement(text);
    if statement && !asks(&lower) {
        return Intent {
            kind: Kind::Statement,
            place: None,
            item,
        };
    }
    if lower.contains("where am i") || lower.contains("what room") || lower.contains("what area") {
        return plain(Kind::Location);
    }
    if [
        "on screen",
        "on the screen",
        "do you see",
        "what is this",
        "whats this",
        "looking at",
        "look at the",
        "whats happening",
        "what is happening",
        "whats going on",
    ]
    .iter()
    .any(|w| lower.contains(w))
    {
        return plain(Kind::Screen);
    }
    if let Some(item) = item {
        return Intent {
            kind: Kind::Item,
            place: None,
            item: Some(item),
        };
    }
    let content_words = lower
        .split_whitespace()
        .filter(|w| w.trim_matches(|c: char| !c.is_alphanumeric()).len() > 3)
        .count();
    if content_words < 2 && !asks(&lower) {
        return plain(Kind::Chat);
    }
    speaker
        .and_then(|speaker| {
            model_intent(store, guide, atlas, speaker, text)
                .ok()
                .flatten()
        })
        .unwrap_or_else(|| plain(Kind::General))
}

/// The local model's reading of an unclear sentence, held to a fixed set of
/// intents by a JSON schema so it can only pick, never wander.
fn model_intent(
    store: &Store,
    guide: &Guide,
    atlas: &Atlas,
    speaker: &HttpProvider,
    text: &str,
) -> Result<Option<Intent>> {
    use crate::providers::ReasoningProvider;
    let schema = json!({
        "type": "object",
        "properties": {
            "intent": {"type": "string", "enum": ["travel", "item", "next", "location", "screen", "general", "chat"]},
            "place": {"type": "string"},
            "item": {"type": "string"}
        },
        "required": ["intent", "place", "item"]
    });
    let reply = speaker.complete_structured(
        ModelRequest {
            system: "You sort what a video game player just said to their helper. Reply with JSON only.".into(),
            prompt: format!(
                "The player said: \"{text}\"\nPick one intent: travel (how to get somewhere; put the place in \"place\"), item (where to get something, its price or what it does; put it in \"item\"), next (what to do now), location (where am I), screen (what is on screen), general (any other question about the game), chat (not about the game). Leave \"place\" and \"item\" empty when not used."
            ),
            images: vec![],
            json: true,
        },
        schema,
    )?;
    let parsed: Value = serde_json::from_str(reply.trim())?;
    let kind = match parsed["intent"].as_str() {
        Some("travel") => Kind::Travel,
        Some("item") => Kind::Item,
        Some("next") => Kind::Next,
        Some("location") => Kind::Location,
        Some("screen") => Kind::Screen,
        Some("chat") => Kind::Chat,
        _ => Kind::General,
    };
    let place = parsed["place"]
        .as_str()
        .filter(|p| !p.trim().is_empty())
        .and_then(|p| routing::named_place(atlas, p));
    let item = parsed["item"]
        .as_str()
        .filter(|i| !i.trim().is_empty())
        .and_then(|i| named_item(store, guide, i));
    crate::flight::event(
        "voice_intent",
        json!({"by": "model", "intent": kind, "place": place, "item": item}),
    );
    Ok(Some(match kind {
        // The model's place or item must be one GameBuddy knows, else the
        // question is treated as a general one.
        Kind::Travel if place.is_none() => Intent {
            kind: Kind::General,
            place: None,
            item,
        },
        Kind::Item if item.is_none() => Intent {
            kind: Kind::General,
            place: None,
            item: None,
        },
        kind => Intent { kind, place, item },
    }))
}

/// The screen as the setup's screen reader sees it right now, as fact lines.
fn screen_facts(store: &Store) -> Vec<String> {
    let settings: Settings = store.get("settings").ok().flatten().unwrap_or_default();
    let snapshot = crate::screen::snapshot(store).ok();
    let reader = HttpProvider::new(settings.vision).ok();
    let Some((snapshot, reader)) = snapshot.zip(reader) else {
        return vec!["The screen could not be read just now.".into()];
    };
    match crate::brain::read_screen(&reader, &[snapshot.jpeg]) {
        Ok(reading) => {
            let mut lines = vec![];
            if !reading.scene.trim().is_empty() {
                lines.push(format!("On screen: {}", clip(&reading.scene, 200)));
            }
            // Dialogue or a message, not HUD digits the reader copied here.
            let wordy = reading
                .text
                .split_whitespace()
                .filter(|w| w.len() >= 3 && w.chars().all(char::is_alphabetic))
                .count()
                >= 2;
            if wordy {
                lines.push(format!("Text on screen: \"{}\".", clip(&reading.text, 160)));
            }
            if !reading.gear.is_empty() {
                lines.push(format!("Visible equipment: {}.", reading.gear.join(", ")));
            }
            if !reading.hud.is_empty() {
                lines.push(format!("HUD readings: {}.", reading.hud.join(", ")));
            }
            lines
        }
        Err(_) => vec!["The screen could not be read just now.".into()],
    }
}

/// Rows of a card's table (a rank or price chart) that mention any of the
/// words, as "name: column value, column value" lines.
fn table_lines(card: &Value, words: &[String]) -> Vec<String> {
    let name = card["name"].as_str().unwrap_or_default();
    let columns: Vec<String> = card["table"]["columns"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|c| {
            c.as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| c.to_string())
        })
        .collect();
    card["table"]["rows"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_array)
        .filter(|row| {
            row.iter().any(|cell| {
                let cell = cell.as_str().unwrap_or_default().to_lowercase();
                words.contains(&cell)
            })
        })
        .take(3)
        .map(|row| {
            format!(
                "{name}: {}",
                columns
                    .iter()
                    .zip(row)
                    .map(|(c, v)| format!("{c} {}", v.as_str().unwrap_or_default()))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })
        .collect()
}

/// Item facts and the plain sentence for each: where it is sold and for how
/// much, and (unless `acquiring`, when the player asked where to get it)
/// what it does.
fn card_lines(cards: &[Value], acquiring: bool) -> Vec<(String, String)> {
    let mut seen = vec![];
    cards
        .iter()
        .filter_map(|card| {
            let name = card["name"].as_str()?;
            if seen.contains(&name) {
                return None;
            }
            seen.push(name);
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
            let notes = if acquiring {
                String::new()
            } else {
                clip(card["notes"].as_str().unwrap_or_default(), 180)
            };
            let fact = format!(
                "{name}: {}{}",
                if costs.is_empty() {
                    "no price is listed".to_owned()
                } else {
                    format!("sold for {}", costs.join(", "))
                },
                if notes.is_empty() {
                    String::new()
                } else {
                    format!(". {notes}")
                }
            );
            let template = if costs.is_empty() {
                if notes.is_empty() {
                    format!("I don't have a price for {name}.")
                } else {
                    format!("{name}: {notes}")
                }
            } else {
                format!("{name} costs {}.", costs.join(", "))
            };
            Some((fact, template))
        })
        .collect()
}

/// The facts for this intent, and the plain sentence that can stand in for
/// the model's words.
pub fn facts(
    store: &Store,
    guide: &Guide,
    atlas: &Atlas,
    intent: &Intent,
    question: &str,
) -> Result<(Vec<String>, Option<String>)> {
    let here = talk::current_place(store, guide);
    // Where the player just said they are, before the estimate catches up.
    let spoken_area = crate::navigation::tracking::spoken_area(atlas, question);
    let here_line = match (talk::here_now(store, guide)["place"].as_str(), &spoken_area) {
        (_, Some(area)) => format!("The player says they are in {area}; believe them."),
        (Some(place), None) => format!("The player is at: {place}."),
        (None, None) => "Where the player is right now is not known.".to_owned(),
    };
    let said = talk::player_statements(store, &guide.game);
    // Only the missing items this question is about: the save may list many.
    let topic = format!("{question} {}", intent.item.as_deref().unwrap_or_default()).to_lowercase();
    let lacking: Vec<String> = talk::lacking_items(store, guide, &said)
        .into_iter()
        .filter(|item| topic.contains(item.as_str()) || topic.contains(item.trim_end_matches('s')))
        .map(|item| format!("The player does not have {item}."))
        .collect();
    let mut facts = vec![];
    let mut template = None;
    match intent.kind {
        Kind::Travel => {
            let goal = routing::goal_for(atlas, intent.place.as_deref().unwrap_or_default());
            facts.push(here_line);
            let spoken_map = spoken_area.as_ref().and_then(|area| {
                atlas
                    .maps
                    .iter()
                    .find(|m| m.title.eq_ignore_ascii_case(area))
                    .map(|m| m.id.clone())
            });
            let way = match (&here, spoken_map) {
                (None, Some(map)) => {
                    routing::route_from_map(store, guide, atlas, &map, None, &goal)
                        .unwrap_or_else(|| format!("No known way from there to {}.", goal.label))
                }
                _ => routing::describe(store, guide, atlas, here.as_deref(), &goal),
            };
            template = Some(if way.starts_with("The way to") {
                format!(
                    "I'm not sure where you are yet, so I can't route you to {}. Where are you right now?",
                    goal.label
                )
            } else if way.starts_with("No known way") {
                format!("I don't know a way from here to {} yet.", goal.label)
            } else if way.starts_with("The player is already") {
                format!("You're already at {}.", goal.label)
            } else {
                way.replace("The player is", "You are")
            });
            facts.push(way);
            facts.extend(lacking);
        }
        Kind::Item => {
            let name = intent.item.clone().unwrap_or_default();
            let lower = question.to_lowercase();
            let acquiring = [
                "where", "get ", "buy", "find", "obtain", "sell", "cost", "price", "much",
            ]
            .iter()
            .any(|w| lower.contains(w));
            let found = crate::game_data::search(store, guide, &name)?;
            let cards: Vec<Value> = found["cards"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|c| {
                    c["name"]
                        .as_str()
                        .is_some_and(|n| n.eq_ignore_ascii_case(&name))
                })
                .chain(found["cards"].as_array().into_iter().flatten())
                .take(2)
                .cloned()
                .collect();
            let lines = card_lines(&cards, acquiring);
            template = lines.first().map(|(_, t)| t.clone());
            facts.extend(lines.into_iter().map(|(f, _)| f));
            if facts.is_empty() {
                facts.push(format!("Nothing is listed about {name}."));
                template = Some(format!("I don't have anything on {name}, sorry."));
            }
            if acquiring {
                facts.push(format!(
                    "The player asked where to get {name}: answer with the place and price, not how to use it."
                ));
                // "Where do I get X" wants where it is bought or found.
                for passage in talk::passages(store, &format!("buy get {name}"), 2) {
                    facts.push(format!("From the game notes: {}", clip(&passage, 220)));
                }
            }
            facts.extend(lacking);
            facts.push(here_line);
        }
        Kind::Next | Kind::Why | Kind::Statement => {
            if intent.kind == Kind::Statement {
                facts.push(format!(
                    "The player just told you: \"{}\".",
                    clip(question, 120)
                ));
                template = Some("Got it.".into());
            }
            let lines = talk::situation_lines(store, guide)?;
            if let Some(next) = lines
                .iter()
                .find(|l| l.starts_with("The walkthrough's next task:"))
            {
                let task = next
                    .trim_start_matches("The walkthrough's next task:")
                    .trim();
                if intent.kind != Kind::Statement {
                    template = Some(format!("Next up: {}", clip(task, 160)));
                }
            }
            if intent.kind != Kind::Statement
                && let Some(missing) = lines
                    .iter()
                    .find(|l| l.starts_with("The player does NOT have"))
            {
                template = Some(clip(missing, 160));
            }
            facts.extend(lines);
            if intent.kind == Kind::Why {
                for passage in talk::passages(store, question, 2) {
                    facts.push(format!("From the game notes: {}", clip(&passage, 220)));
                }
            }
        }
        Kind::Repeat => {}
        Kind::Location => {
            template = Some(match talk::here_now(store, guide)["place"].as_str() {
                Some(place) => format!("You're at {place}."),
                None => "I can't tell where you are yet.".into(),
            });
            facts.push(here_line);
            facts.extend(screen_facts(store));
        }
        Kind::Screen => {
            let lines = screen_facts(store);
            template = lines
                .iter()
                .find(|l| l.starts_with("On screen:"))
                .map(|l| l.trim_start_matches("On screen:").trim().to_owned());
            facts.extend(lines);
            facts.push(here_line);
        }
        Kind::General => {
            let words: Vec<String> = question
                .to_lowercase()
                .split(|c: char| !c.is_alphanumeric())
                .filter(|w| w.len() > 3)
                .map(str::to_owned)
                .collect();
            let cards = crate::game_data::mentioned(store, guide, question)?;
            facts.extend(
                card_lines(&cards, false)
                    .into_iter()
                    .take(2)
                    .map(|(f, _)| f),
            );
            // A chart row about what they asked (a rank's threshold).
            let found = crate::game_data::search(store, guide, question)?;
            for card in found["cards"].as_array().into_iter().flatten().take(3) {
                facts.extend(table_lines(card, &words));
            }
            for passage in talk::passages(store, question, 3) {
                facts.push(format!("From the game notes: {}", clip(&passage, 240)));
            }
            if facts.is_empty() {
                facts.push("Nothing in the game notes matches this; say you are not sure.".into());
            }
            template = Some("I'm not sure about that one, sorry.".into());
            facts.push(here_line);
            facts.extend(lacking);
        }
        Kind::Chat => {
            facts.push(
                "The player is chatting or reacting, not asking about the game: reply as a friend in one short sentence."
                    .into(),
            );
            template = Some("I'm here, go ahead.".into());
        }
    }
    Ok((facts, template))
}

fn numbers(text: &str) -> Vec<String> {
    let joined = text.replace(",", "");
    joined
        .split(|c: char| !c.is_ascii_digit())
        .filter(|n| !n.is_empty())
        .map(|n| n.trim_start_matches('0').to_owned())
        .collect()
}

/// Every number and known name in the sentence appears in the facts.
pub fn grounded(sentence: &str, facts: &[String], names: &[String]) -> bool {
    let pool = facts.join("\n").to_lowercase();
    let pool_numbers = numbers(&pool);
    if numbers(sentence).iter().any(|n| !pool_numbers.contains(n)) {
        return false;
    }
    let lower = sentence.to_lowercase();
    let said: Vec<String> = lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_owned)
        .collect();
    names.iter().all(|name| {
        let name = name.to_lowercase();
        let spoken = name.split_whitespace().all(|w| said.iter().any(|s| s == w));
        !spoken || pool.contains(&name)
    })
}

/// Names a reply may only use when the facts do: places (the first word of
/// each label, which is the town or area) and data cards.
fn known_names(store: &Store, guide: &Guide, atlas: &Atlas) -> Vec<String> {
    let common = [
        "the", "of", "and", "room", "chamber", "interior", "exterior", "shop", "door", "town",
        "area", "outdoor", "upper", "lower", "raised", "elevated", "large", "small", "left",
        "right", "next", "gold",
    ];
    let mut names: Vec<String> = atlas
        .places
        .iter()
        .filter_map(|p| {
            p.label
                .split(|c: char| !c.is_alphanumeric())
                .next()
                .map(str::to_lowercase)
        })
        .filter(|w| w.len() >= 4 && !common.contains(&w.as_str()))
        .collect();
    names.extend(
        crate::game_data::cards(store, guide)
            .unwrap_or_default()
            .iter()
            .filter_map(|c| c["name"].as_str())
            .map(|n| n.trim().to_lowercase())
            .filter(|n| n.len() >= 4 && !common.contains(&n.as_str())),
    );
    names.sort();
    names.dedup();
    names
}

/// Answer the player's words: decide, look up, speak once, check each
/// sentence before it is spoken.
pub fn answer(
    store: &Store,
    guide: &Guide,
    provider: &HttpProvider,
    question: &str,
    on_sentence: &mut dyn FnMut(String),
) -> Result<Turn> {
    let started = std::time::Instant::now();
    let atlas = crate::navigation::atlas(store, guide)?;
    let intent = classify(store, guide, &atlas, question, Some(provider));
    crate::flight::event("voice_intent", json!(intent));
    if intent.kind == Kind::Repeat {
        let last = talk::recent_turns(store, &guide.game)
            .into_iter()
            .rev()
            .find(|t| t["role"] != "user")
            .and_then(|t| t["text"].as_str().map(str::to_owned))
            .unwrap_or_else(|| "I haven't said anything yet.".into());
        on_sentence(last.clone());
        return Ok(Turn {
            reply: last,
            intent,
            facts: vec![],
            grounded: true,
            templated: true,
            ms: started.elapsed().as_millis() as u64,
            first_sentence_ms: Some(0),
        });
    }
    let (facts, template) = facts(store, guide, &atlas, &intent, question)?;
    let names = known_names(store, guide, &atlas);
    let trace = std::env::var_os("GAMEBUDDY_TALK_TRACE").is_some();
    if trace {
        eprintln!(
            "INTENT {}",
            serde_json::to_string(&intent).unwrap_or_default()
        );
        for fact in &facts {
            eprintln!("FACT {fact}");
        }
    }
    let request = ModelRequest {
        system: SPEAK.into(),
        prompt: format!(
            "Facts GameBuddy looked up:\n{}\n\nThe player said: \"{question}\"\n\nWhat GameBuddy says out loud:",
            facts
                .iter()
                .map(|f| format!("- {f}"))
                .collect::<Vec<_>>()
                .join("\n")
        ),
        images: vec![],
        json: false,
    };
    let mut sentences = talk::Sentences::new();
    let mut spoken: Vec<String> = vec![];
    let mut all_grounded = true;
    let mut stopped = false;
    let mut first_ms = None;
    let fallback = template
        .clone()
        .unwrap_or_else(|| "Sorry, I'm not sure about that one. Where are you right now?".into());
    let mut take = |sentence: String,
                    spoken: &mut Vec<String>,
                    stopped: &mut bool,
                    all_grounded: &mut bool,
                    first_ms: &mut Option<u64>| {
        if *stopped {
            return;
        }
        let sentence = tidy(&sentence);
        if sentence.is_empty() {
            return;
        }
        if grounded(&sentence, &facts, &names) {
            first_ms.get_or_insert(started.elapsed().as_millis() as u64);
            on_sentence(sentence.clone());
            spoken.push(sentence);
        } else {
            if trace {
                eprintln!("UNGROUNDED {sentence}");
            }
            *all_grounded = false;
            *stopped = true;
            if spoken.is_empty() {
                first_ms.get_or_insert(started.elapsed().as_millis() as u64);
                on_sentence(fallback.clone());
                spoken.push(fallback.clone());
            }
        }
    };
    let result = provider.complete_streaming(request, &mut |piece| {
        for sentence in sentences.push(piece) {
            take(
                sentence,
                &mut spoken,
                &mut stopped,
                &mut all_grounded,
                &mut first_ms,
            );
        }
    });
    if let Some(rest) = sentences.finish() {
        take(
            rest,
            &mut spoken,
            &mut stopped,
            &mut all_grounded,
            &mut first_ms,
        );
    }
    let mut templated = !all_grounded && spoken.first() == Some(&fallback);
    if spoken.is_empty() {
        if let Err(error) = &result {
            eprintln!("Spoken answer: {error:#}");
        }
        // No usable words from the model: the plain sentence stands in.
        on_sentence(fallback.clone());
        spoken.push(fallback.clone());
        templated = true;
    }
    let reply = spoken.join(" ");
    let turn = Turn {
        reply,
        intent,
        facts,
        grounded: all_grounded,
        templated,
        ms: started.elapsed().as_millis() as u64,
        first_sentence_ms: first_ms,
    };
    if trace {
        eprintln!(
            "REPLY ({} ms, grounded {}, templated {}) {}",
            turn.ms, turn.grounded, turn.templated, turn.reply
        );
    }
    crate::flight::event(
        "talk",
        json!({"ms": turn.ms, "first_sentence_ms": turn.first_sentence_ms, "chars": turn.reply.len(),
            "intent": turn.intent.kind, "grounded": turn.grounded, "templated": turn.templated}),
    );
    Ok(turn)
}

/// Strip what a small model adds that a friend never says: its own name as
/// an address, quotes around the whole line, leftover markup.
fn tidy(sentence: &str) -> String {
    let trimmed = sentence.trim().trim_matches('"').trim();
    let trimmed = trimmed
        .strip_prefix("GameBuddy: ")
        .or_else(|| trimmed.strip_prefix("GameBuddy, "))
        .or_else(|| trimmed.strip_prefix("GameBuddy here, "))
        .unwrap_or(trimmed);
    if trimmed.starts_with('<') || trimmed.starts_with('{') {
        return String::new();
    }
    let mut chars = trimmed.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().collect::<String>() + chars.as_str())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::grounded;

    #[test]
    fn numbers_and_names_must_come_from_the_facts() {
        let facts = vec![
            "Wing Boots: 2800 gold in Forepaw. Lets you fly.".to_owned(),
            "The player is at: Mascon.".to_owned(),
        ];
        let names = vec![
            "forepaw".to_owned(),
            "wing boots".to_owned(),
            "eolis".to_owned(),
        ];
        assert!(grounded(
            "Head to Forepaw and buy the Wing Boots for 2,800 gold.",
            &facts,
            &names
        ));
        assert!(!grounded("Wing Boots cost 50 gold.", &facts, &names));
        assert!(!grounded("Buy them in Eolis.", &facts, &names));
        assert!(grounded("You're in Mascon right now.", &facts, &names));
    }
}
