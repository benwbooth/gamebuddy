//! Press buttons on RetroArch's network gamepad for player 1, in order:
//! gamepad_probe start:200 none:3000 right+a:500 ...
//! Each step holds the buttons named (joined by +, or none) for that many ms.
fn main() -> anyhow::Result<()> {
    use anyhow::Context;
    use gamebuddy_services::retroarch::{Button, hold};
    for step in std::env::args().skip(1) {
        let (names, ms) = step.split_once(':').context("button:ms")?;
        let held: Vec<Button> = names
            .split('+')
            .filter(|n| *n != "none")
            .map(|n| serde_json::from_value(serde_json::json!(n)).context("Unknown button"))
            .collect::<anyhow::Result<_>>()?;
        hold(&held)?;
        std::thread::sleep(std::time::Duration::from_millis(ms.parse()?));
    }
    hold(&[])?;
    Ok(())
}
