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
    /// Which way to go from here, no place named ("which direction?", "I
    /// can't go that way").
    Move,
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

/// Asking which way to go, or saying the way they tried is blocked: these
/// once fell through to small talk ("Oh, I can't go") or a text search that
/// answered "go through the tower door" on a screen with no door.
const MOVING: &[&str] = &[
    "which way",
    "what way",
    "which direction",
    "what direction",
    "where do i go",
    "where should i go",
    "where to go",
    "where now",
    "left or right",
    "up or down",
    "cant go",
    "cannot go",
    "can not go",
    "dead end",
    "im stuck",
    "i am stuck",
    "no door",
    "way out",
    "get out of here",
];

fn moving(lower: &str) -> bool {
    MOVING.iter().any(|w| lower.contains(w))
}

fn blocked(lower: &str) -> bool {
    ["cant go", "cannot go", "can not go", "dead end", "stuck"]
        .iter()
        .any(|w| lower.contains(w))
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
        .or_else(|| short_item_name(store, guide, &said))
}

/// "The boots" for Wing Boots: the last word of a name, when one item is
/// meant: the only card ending in that word, or the only such item the
/// player lacks.
fn short_item_name(store: &Store, guide: &Guide, said: &[String]) -> Option<String> {
    let names: Vec<String> = crate::game_data::cards(store, guide)
        .unwrap_or_default()
        .iter()
        .filter_map(|card| card["name"].as_str().map(|n| n.trim().to_owned()))
        .filter(|name| {
            let last = name.to_lowercase();
            let last = last
                .split_whitespace()
                .last()
                .unwrap_or_default()
                .to_owned();
            name.contains(' ') && last.len() > 3 && routing::heard(said, &last)
        })
        .collect();
    if names.len() == 1 {
        return names.into_iter().next();
    }
    let statements = talk::player_statements(store, &guide.game);
    let lacking = talk::lacking_items(store, guide, &statements);
    let mut lacked = names
        .into_iter()
        .filter(|name| lacking.iter().any(|item| item.eq_ignore_ascii_case(name)));
    lacked.next().filter(|_| lacked.next().is_none())
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
    if moving(&lower) {
        return plain(Kind::Move);
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

const SORT: &str =
    "You sort what a video game player just said to their helper. Reply with JSON only.";

/// For a model strong enough to see the game and know it: it answers from
/// the picture and its own knowledge, not only from what was looked up.
const EXPERT: &str = "You are an expert at this game, sitting beside the player and watching them play, like a friend who has beaten it many times. The image is the screen right now. You also get a log of what just happened and notes GameBuddy looked up. Tell the player exactly what to do from this very screen, in one or two short, natural spoken sentences (under 35 words). Judge from what you see: believe the picture over the notes, and never ask the player what is on screen. Exact commands and solutions come only from the walkthrough steps given or from the player: what a menu word does differs from game to game, so never work one out from its name or from memory. If the steps do not make the command clear, name the goal and say you are not sure of the command; if they disagree, say so. Believe anything the player has told you about the game over the steps. If the player is reacting (they died, are stuck, are annoyed), say what happened and how to deal with it, as a friend who saw it too. Name the on-screen option to choose, not buttons, unless the player asks about controls. No lists, no markdown, no ids. Never say \"the walkthrough\", \"the notes\" or \"the log\", and never start with your own name.";

/// Have the provider's ready processes started before the first question.
pub fn warm(provider: &HttpProvider) {
    if provider.config.kind == crate::ProviderKind::ClaudeCode {
        for system in [EXPERT, crate::watch::LOOK] {
            crate::subscription::claude_warm(
                &provider.config.model,
                &provider.config.reasoning_effort,
                system,
            );
        }
    }
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
            "intent": {"type": "string", "enum": ["travel", "move", "item", "next", "location", "screen", "general", "chat"]},
            "place": {"type": "string"},
            "item": {"type": "string"}
        },
        "required": ["intent", "place", "item"]
    });
    let request = |json| ModelRequest {
        system: SORT.into(),
        prompt: format!(
            "The player said: \"{text}\"\nPick one intent: travel (how to get to a named place; put the place in \"place\"), move (which way to go from here, or the way they tried is blocked), item (where to get something, its price or what it does; put it in \"item\"), next (what to do now), location (where am I), screen (what is on screen), general (any other question about the game), chat (not about the game). Leave \"place\" and \"item\" empty when not used."
        ),
        images: vec![],
        json,
    };
    let reply = speaker.complete_structured(request(true), schema)?;
    let parsed: Value = serde_json::from_str(reply.trim())?;
    let kind = match parsed["intent"].as_str() {
        Some("travel") => Kind::Travel,
        Some("move") => Kind::Move,
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
        // Going somewhere without a place GameBuddy knows: which way from
        // here, toward the current goal.
        Kind::Travel if place.is_none() => Intent {
            kind: Kind::Move,
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
pub(crate) fn card_lines(cards: &[Value], acquiring: bool) -> Vec<(String, String)> {
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
/// `true` last: the plain sentence is the whole answer, said without a
/// model call.
pub fn facts(
    store: &Store,
    guide: &Guide,
    atlas: &Atlas,
    intent: &Intent,
    question: &str,
) -> Result<(Vec<String>, Option<String>, bool)> {
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
    let spoken_map = spoken_area.as_ref().and_then(|area| {
        atlas
            .maps
            .iter()
            .find(|m| m.title.eq_ignore_ascii_case(area))
            .map(|m| m.id.clone())
    });
    let way_to = |goal: &routing::Goal| match (&here, &spoken_map) {
        (None, Some(map)) => routing::route_from_map(store, guide, atlas, map, None, goal)
            .unwrap_or_else(|| format!("No known way from there to {}.", goal.label)),
        _ => routing::describe(store, guide, atlas, here.as_deref(), goal),
    };
    let mut facts = vec![];
    let mut template = None;
    match intent.kind {
        Kind::Move => {
            facts.push(here_line);
            let lower = question.to_lowercase().replace(['\'', '’'], "");
            let goal = routing::goal(store, guide).ok().flatten();
            let way = goal.as_ref().map(&way_to);
            let areas = way
                .as_deref()
                .and_then(|w| w.split_once("cross "))
                .and_then(|(_, rest)| rest.split(". ").next())
                .map(|areas| areas.trim_end_matches('.').to_owned());
            let dead_end = if blocked(&lower) {
                "Okay, that way's a dead end. "
            } else {
                ""
            };
            let next_area = areas
                .as_deref()
                .and_then(|a| a.split(", then ").next())
                .map(str::to_owned);
            let through = goal
                .as_ref()
                .zip(areas)
                .map(|(goal, areas)| format!("{} is through {areas}", goal.label));
            if let Some((_, screens)) = way
                .as_deref()
                .and_then(|w| w.split_once("From this screen: "))
            {
                // The screens to cross, from the walkthrough run's own path:
                // GameBuddy's words, not a model's guess at a direction. The
                // moves lead; the list of areas every time was a mouthful.
                let mut screens = screens.trim_end_matches('.').to_owned();
                if let Some(first) = screens.get(..1).map(str::to_uppercase) {
                    screens.replace_range(..1, &first);
                }
                let toward = next_area
                    .filter(|_| !screens.contains("ask me again"))
                    .map(|area| format!(" That takes you toward {area}."))
                    .unwrap_or_else(|| ".".into());
                let line = format!("{dead_end}{screens}{toward}");
                return Ok((vec![line.clone()], Some(line), true));
            }
            if let Some(way) = way.clone().filter(|w| w.contains("First head")) {
                // The walkthrough states which way to head from this room.
                template = Some(way.replace("The player is", "You are"));
                facts.push(way);
                facts.extend(lacking);
            } else {
                // No path through these rooms is known: say so in GameBuddy's
                // own words instead of letting a model guess a direction.
                let line = format!(
                    "{dead_end}{}I don't know the way through these rooms yet. Which exits can you see: a ladder, a door, or an opening?",
                    through.map(|t| format!("{t}, but ")).unwrap_or_default()
                );
                return Ok((vec![line.clone()], Some(line), true));
            }
        }
        Kind::Travel => {
            let goal = routing::goal_for(atlas, intent.place.as_deref().unwrap_or_default());
            facts.push(here_line);
            let way = way_to(&goal);
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
                // "Where do I get X" wants where it is bought or found. Listed
                // shops answer that; notes on the run's own pickups only
                // send the player somewhere the walkthrough happened to be.
                if !facts.iter().any(|f| f.contains(" gold in ")) {
                    for passage in talk::passages(store, &format!("buy get {name}"), 2) {
                        facts.push(format!("From the game notes: {}", clip(&passage, 220)));
                    }
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
            let mut lines = talk::situation_lines(store, guide)?;
            // On the way somewhere, that trip is what to do now; the
            // walkthrough's next step waits at the other end, and saying it
            // here sends the player the opposite way.
            let trip = (intent.kind == Kind::Next)
                .then(|| routing::goal(store, guide).ok().flatten())
                .flatten();
            if let Some(goal) = &trip {
                lines.retain(|l| {
                    !l.starts_with("The walkthrough's next task:")
                        && !l.starts_with("What the walkthrough video did")
                });
                facts.push(format!("What to do now: get to {}.", goal.label));
                facts.push(way_to(goal));
                template = Some(format!("Head for {}.", goal.label));
            }
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
            // Said by GameBuddy itself: a small model given the screen facts
            // read the HUD numbers aloud.
            let line = match talk::here_now(store, guide)["place"].as_str() {
                Some(place) => format!("You're at {place}."),
                None => "I can't tell where you are yet. What do you see around you?".into(),
            };
            return Ok((vec![here_line], Some(line), true));
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
    Ok((facts, template, false))
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
    // A model that sees the screen needs no help working out what was meant,
    // and sorting the sentence first cost it six seconds.
    let sees = !crate::providers::on_this_computer(&provider.config);
    let intent = classify(store, guide, &atlas, question, (!sees).then_some(provider));
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
    let (mut facts, template, settled) = facts(store, guide, &atlas, &intent, question)?;
    if sees {
        for line in talk::situation_lines(store, guide).unwrap_or_default() {
            if !facts.contains(&line) {
                facts.push(line);
            }
        }
    }
    let watched = crate::watch::lines(store, guide);
    if settled && let Some(line) = template {
        on_sentence(line.clone());
        crate::flight::event(
            "talk",
            json!({"ms": started.elapsed().as_millis() as u64, "first_sentence_ms": 0, "chars": line.len(),
                "intent": intent.kind, "grounded": true, "templated": true}),
        );
        return Ok(Turn {
            reply: line,
            intent,
            facts,
            grounded: true,
            templated: true,
            ms: started.elapsed().as_millis() as u64,
            first_sentence_ms: Some(0),
        });
    }
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
    let frame = sees
        .then(|| crate::screen::snapshot(store).ok())
        .flatten()
        .map(|snapshot| {
            let _ = crate::screen::seen::record(store, &snapshot, "assistant");
            snapshot.jpeg
        });
    let request = ModelRequest {
        system: if frame.is_some() { EXPERT } else { SPEAK }.into(),
        prompt: format!(
            "Game: {}\n\nWhat just happened, oldest first:\n{}\n\nWalkthrough steps for this moment:\n{}\n\nRecent conversation:\n{}\n\nFacts GameBuddy looked up:\n{}\n\nThe player said: \"{question}\"\n\nWhat GameBuddy says out loud:",
            guide.game,
            watched.join("\n"),
            crate::watch::steps_here(store, guide, question).join("\n"),
            talk::recent_turns(store, &guide.game)
                .iter()
                .rev()
                .take(6)
                .rev()
                .map(|t| format!(
                    "{}: {}",
                    t["role"].as_str().unwrap_or_default(),
                    t["text"].as_str().unwrap_or_default()
                ))
                .collect::<Vec<_>>()
                .join("\n"),
            facts
                .iter()
                .map(|f| format!("- {f}"))
                .collect::<Vec<_>>()
                .join("\n")
        ),
        images: frame.iter().cloned().collect(),
        json: false,
    };
    let checked = frame.is_none();
    // "You're at the elevated shop" came from a walkthrough passage while
    // where the player was stood unknown: a location is said only when the
    // facts hold one.
    let knows_place = facts.iter().any(|f| {
        f.starts_with("The player is at")
            || f.starts_with("The player says they are in")
            || (f.starts_with("Where the player is") && !f.contains("not known"))
    });
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
        if !checked
            || (grounded(&sentence, &facts, &names) && (knows_place || !claims_place(&sentence)))
        {
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

fn claims_place(sentence: &str) -> bool {
    let lower = sentence.to_lowercase().replace('’', "'");
    [
        "you're at ",
        "you are at ",
        "you're in ",
        "you are in ",
        "you're on ",
        "you are on ",
    ]
    .iter()
    .any(|p| lower.starts_with(p))
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
