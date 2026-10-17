//! Import transcribed walkthrough data cards for the active guide:
//! game_data_import <data directory> <cards.json>
fn main() -> anyhow::Result<()> {
    use anyhow::Context;
    let mut args = std::env::args().skip(1);
    let store = gamebuddy_services::Store::open(args.next().context("Data directory")?)?;
    let path = std::path::PathBuf::from(args.next().context("cards.json")?);
    let count = gamebuddy_services::game_data::import_file(&store, &path)?;
    println!("Imported {count} data cards");
    Ok(())
}
