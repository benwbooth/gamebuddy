//! What search_guide returns for a sentence: search_probe <data dir> <text>
fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let store = gamebuddy_services::Store::open(&args[1])?;
    let value = gamebuddy_services::harness::call_tool(
        &store,
        "search_guide",
        serde_json::json!({"query": args[2]}),
    )?;
    let text = serde_json::to_string(&value)?;
    println!(
        "keys {:?}",
        value.as_object().map(|o| o.keys().collect::<Vec<_>>())
    );
    for key in ["matches", "steps", "results"] {
        for m in value[key].as_array().into_iter().flatten().take(4) {
            println!(
                "{key}: {}",
                serde_json::to_string(m)?
                    .chars()
                    .take(300)
                    .collect::<String>()
            );
        }
    }
    Ok(())
}
