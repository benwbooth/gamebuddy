# Subscription providers

GameBuddy can use the installed, signed-in **Codex CLI** and **Claude Code CLI**. It does not extract credentials from either desktop application. Model calls count against the signed-in account's applicable usage limits.

In **Options → Models**, choose **Use Codex subscription + live voice** or **Use Claude subscription + Codex voice**. These preserve the independent local vision configuration. An empty Codex model uses the CLI's configured model; the Claude preset uses `sonnet`. The installed Claude default may require usage credits, even when another model is available through the subscription.

```json
{
  "reasoning": {"kind":"codex", "endpoint":"", "model":"", "api_key_env":""},
  "vision": {"kind":"ollama", "endpoint":"http://127.0.0.1:11434", "model":"qwen3.5:4b", "api_key_env":""},
  "speech": "codex_realtime"
}
```

Use `claude_code` for Claude reasoning. `speech: "local"` selects the existing Whisper/Qt speech path. API-key providers remain separate settings. Credentials are owned and refreshed by the official CLIs; the subscription adapters remove API-key environment variables and never silently fall back to paid API requests.

## Realtime voice

The Codex adapter uses its experimental app-server **WebRTC v3** interface. Signaling travels over a private stdio connection; audio travels directly over WebRTC. The WebSocket transport currently requires API-key authentication and is deliberately not used. Claude's CLI supplies reasoning and image interpretation, not an embedded Claude Desktop speech service. Choosing Claude reasoning therefore still uses Codex for the voice session and its native tool handoffs.

Mute/unmute or choose **Listen** to connect. If the microphone is already unmuted at launch, choose Listen. Muting ends capture and the voice session. Voice remains independent of the overlay's keyboard focus. Only an explicit overlay voice command opens/focuses controls. GameBuddy tools expose guide context, reference excerpts, outline edits and widgets; they do not confirm objective completion or execute operating-system commands.

The voice web page is bundled with the application and served on a private, ephemeral localhost port. Only that exact origin receives microphone permission while voice is active. An unsupported CLI, lost connection or rejected account is reported in the companion; it does not silently enable robotic speech or switch billing providers.

## Walkthrough preparation and screen awareness

Preparation downloads/cache-reuses video and available captions, builds a visual index, divides uncompiled chapters into two-minute intervals, and asks the selected reasoning provider to interpret twelve ordered frames plus timed transcript evidence per interval. If captions are missing, local whisper.cpp transcribes that interval. Each successful section is saved immediately; failed sections can be retried on the next preparation. Player-created steps and existing compiled children are preserved.

The built-in HTML outline is always available, supports nested collapse/expand, opens the current step's ancestors, and follows location updates. Clicking a step only inspects it; it does not confirm progress. The preparation indicator distinguishes pending sections and failed reviews. Reviewing every interval is not proof of a complete or correct walkthrough: sparse frame sampling and transcription can miss events.

Game-only screen capture and fingerprint retrieval run independently of conversational requests. Repeated, sufficiently distinct matches select a location estimate and update the next-action widget without a question from the player. When retrieval is ambiguous, the local Ollama VLM compares the live frame with likely references, at most every 15 seconds; two agreeing, recent observations are required to select a VLM location estimate. Stale results after a guide revision or progress reset are discarded. Live voice receives changed game context. Remote VLMs require the explicit Compare control. General independent-gameplay accuracy, especially with shaders, HUD differences and similar rooms, still needs evaluation. No visual match automatically marks an objective complete.

## Development and checks

`./dev.sh` enters the GameBuddy Nix shell and runs an initial build, then rebuilds on Rust, C++, QML, HTML and Cargo changes using watchexec. `./dev.sh --once` builds once. It does not restart a live game. Lunchpail's launch wrapper continues to execute the last built binary without invoking Cargo.

- `cargo run -p gamebuddy-services --example subscription_probe -- codex [image]`
- `GAMEBUDDY_PROBE_MODEL=sonnet cargo run -p gamebuddy-services --example subscription_probe -- claude [image]`
- `scripts/test-realtime` uses a subscription call and isolated virtual audio devices to check the real microphone-to-WebRTC-to-widget path.
- `scripts/test-gamescope` verifies the compositor and HTML widget interactions.

Protocol reference: [Codex app-server](https://developers.openai.com/codex/app-server), version-matched schemas from `codex app-server generate-json-schema --experimental`. Subscription policy: [Claude Agent SDK and Claude plans](https://support.claude.com/en/articles/15036540-use-the-claude-agent-sdk-with-your-claude-plan). These interfaces and account policies can change.
