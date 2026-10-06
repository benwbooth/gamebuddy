//! Replay one missing prerequisite against real prepared source rules. This
//! changes only a private fixture, never the player's inventory or location.
use anyhow::{Context, Result, ensure};
use gamebuddy_core::outcomes::{Condition, KnownFact};
use gamebuddy_services::{Store, navigation, outcomes, screen};
use serde_json::json;

fn facts(condition: &Condition, progress: &mut gamebuddy_core::Progress) {
    match condition {
        Condition::Fact { id, value } => {
            progress.world.facts.insert(
                id.clone(),
                KnownFact {
                    value: *value,
                    evidence: "Private prerequisite simulation".into(),
                    source: "fixture".into(),
                    step_id: None,
                    at_ms: screen::now_ms(),
                },
            );
        }
        Condition::All { conditions } | Condition::Any { conditions } => {
            for c in conditions {
                facts(c, progress)
            }
        }
        _ => {}
    }
}
fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let root = std::path::PathBuf::from(args.next().context("Private compiled store")?);
    ensure!(root.starts_with("/tmp"), "Use a private fixture");
    let store = Store::open(root)?;
    let id: String = store.get("active_guide")?.context("Active guide")?;
    let guide = store.guide(&id)?;
    let model = outcomes::prepared(&store, &guide)?.context("Prepared outcomes")?;
    let fact = args.next().context("Required possession fact")?;
    let original = store.progress(&guide)?;
    let ticket = screen::start(&store, &guide.game, true)?;
    let path = store.root.join("readiness-frame.png");
    image::RgbImage::from_pixel(64, 48, image::Rgb([60, 100, 40])).save(&path)?;
    screen::publish(&store, &ticket, &path, "private_fixture")?;
    for target in args {
        let rule = model
            .model
            .actions
            .iter()
            .find(|r| r.step_id == target)
            .context("No prepared consumer rule")?;
        let condition = rule
            .requires
            .as_ref()
            .context("Consumer missing requirement")?;
        let mut progress = original.clone();
        progress.current_step = Some(target.clone());
        facts(condition, &mut progress);
        let item = progress
            .world
            .facts
            .get_mut(&fact)
            .context("Rule does not require the item")?;
        item.value = false;
        store.save_progress(&guide, &progress, "private_fixture")?;
        store.put(&format!("next-action:{id}"),&json!({"revision":guide.revision,"epoch":progress.epoch,"session":ticket.session,"updated_at_ms":screen::now_ms(),"step_id":target,"instructions":"Old recommendation to proceed.","location_evidence":{"kind":"fixture"}}))?;
        let action = || navigation::advice::NextAction {
            expected_revision: guide.revision,
            epoch: progress.epoch,
            step_id: target.clone(),
            instructions: "Proceed now.".into(),
            screen_direction: None,
        };
        let started = std::time::Instant::now();
        let rejected = navigation::advice::publish(&store, &guide, action())
            .err()
            .context("Missing item allowed")?;
        let nav = navigation::context(&store, &guide, &progress, &Default::default())?;
        ensure!(
            nav["guidance"]["now"]["id"] != target,
            "Cached advice bypassed requirement"
        );
        let elapsed = started.elapsed().as_millis();
        progress.world.facts.get_mut(&fact).unwrap().value = true;
        store.save_progress(&guide, &progress, "private_fixture")?;
        navigation::advice::publish(&store, &guide, action())?;
        println!(
            "{}",
            json!({"target":target,"missing_item_rejected":rejected.to_string(),"redirected_to":nav["guidance"]["now"]["id"],"warning":nav["guidance"]["warning"],"owned_item_accepted":true,"local_ms":elapsed,"source":rule.evidence_step_ids})
        );
    }
    store.save_progress(&guide, &original, "restore_private_fixture")?;
    Ok(())
}
