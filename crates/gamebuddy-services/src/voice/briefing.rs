//! Facts the voice can state at once from the guide's own data cards, with
//! no model call: what an item costs and where to buy it.
use crate::Store;
use anyhow::Result;

/// The item a price question asks about, if the words are one.
fn price_subject(request: &str) -> Option<String> {
    let q = request
        .trim()
        .trim_end_matches(['?', '.', '!'])
        .to_lowercase()
        .replace('\u{2019}', "'");
    let patterns: [(&str, &str); 8] = [
        ("how much is ", ""),
        ("how much are ", ""),
        ("how much does ", " cost"),
        ("how much do ", " cost"),
        ("what does ", " cost"),
        ("where can i buy ", ""),
        ("where do i buy ", ""),
        ("what's the price of ", ""),
    ];
    for (prefix, suffix) in patterns {
        if let Some(rest) = q.strip_prefix(prefix) {
            let rest = rest.strip_suffix(suffix).unwrap_or(rest).trim();
            let rest = ["a ", "an ", "the ", "one "]
                .iter()
                .find_map(|a| rest.strip_prefix(a))
                .unwrap_or(rest)
                .trim();
            // A follow-on clause makes it a different question.
            if rest.is_empty() || rest.split_whitespace().count() > 4 {
                return None;
            }
            return Some(rest.to_owned());
        }
    }
    None
}

fn singular(word: &str) -> &str {
    word.strip_suffix('s')
        .filter(|w| w.len() > 2)
        .unwrap_or(word)
}

/// "How much is a red potion?" answered from the walkthrough's data cards.
pub fn price_reply(
    store: &Store,
    game: &str,
    request: &str,
) -> Result<Option<super::prepared::Response>> {
    let Some(subject) = price_subject(request) else {
        return Ok(None);
    };
    let Some(id) = store.get::<String>("active_guide")? else {
        return Ok(None);
    };
    let guide = store.guide(&id)?;
    if !guide.game.eq_ignore_ascii_case(game) {
        return Ok(None);
    }
    let wanted = singular(&subject).to_owned();
    let cards = crate::game_data::cards(store, &guide)?;
    let card = cards.iter().find(|c| {
        c["name"].as_str().is_some_and(|n| {
            let n = n.to_lowercase();
            singular(&n) == wanted || n.contains(&wanted) && wanted.len() >= 4
        })
    });
    let Some(card) = card else { return Ok(None) };
    let prices: Vec<String> = card["costs"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| {
            let place = c["place"].as_str().filter(|p| !p.trim().is_empty())?;
            let gold = c["gold"].as_u64()?;
            Some(format!("{gold} gold in {place}"))
        })
        .collect();
    if prices.is_empty() {
        return Ok(None);
    }
    let name = card["name"].as_str().unwrap_or(&subject);
    // Where the player is comes first: that is the price they can pay now.
    let atlas = crate::navigation::atlas(store, &guide)?;
    let graph = gamebuddy_core::planner::PlanGraph::build(&guide, &atlas, &Default::default());
    let here = crate::navigation::planning::player_area(store, &guide, &graph)
        .ok()
        .flatten()
        .map(|a| a.to_lowercase());
    if let Some(here) = &here
        && let Some(local) = prices
            .iter()
            .find(|p| p.to_lowercase().ends_with(&format!(" in {here}")))
    {
        let gold = local.split(" gold").next().unwrap_or_default();
        let others = prices.len() - 1;
        let text = if others == 0 {
            format!("{name} costs {gold} gold here.")
        } else {
            format!(
                "{name} costs {gold} gold here; {others} other shop{} sell{} it.",
                if others == 1 { "" } else { "s" },
                if others == 1 { "s" } else { "" }
            )
        };
        return Ok(Some(super::prepared::Response {
            text,
            kind: "price".into(),
            step_id: None,
        }));
    }
    let list = match prices.len() {
        1 => prices[0].clone(),
        2 => format!("{} and {}", prices[0], prices[1]),
        _ => format!(
            "{}, and {}",
            prices[..prices.len() - 1].join(", "),
            prices.last().unwrap()
        ),
    };
    Ok(Some(super::prepared::Response {
        text: format!("{name} costs {list}."),
        kind: "price".into(),
        step_id: None,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reads_the_item_out_of_a_price_question() {
        assert_eq!(
            price_subject("How much is a red potion?").as_deref(),
            Some("red potion")
        );
        assert_eq!(
            price_subject("how much does the long sword cost").as_deref(),
            Some("long sword")
        );
        assert_eq!(
            price_subject("Where can I buy elixirs?").as_deref(),
            Some("elixirs")
        );
        assert_eq!(
            price_subject("how much is it to get to the next town from here then"),
            None
        );
    }
}
