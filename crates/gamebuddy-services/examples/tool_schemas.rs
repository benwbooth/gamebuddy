//! Print GameBuddy's agent tools and live-voice setup as JSON, for wiring and
//! testing external reasoners.
fn main() {
    use gamebuddy_services::voice::prompts;
    println!(
        "{}",
        serde_json::json!({
            "tools": gamebuddy_services::harness::tool_definitions(),
            "reasoner": prompts::REASONER,
            "gemini_tools": prompts::gemini_tools(),
            "gemini_prompt": prompts::gemini_live(),
        })
    );
}
