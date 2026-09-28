# GameBuddy

An AI game companion built in Rust, with Qt/QML through CXX-Qt and HTML widgets. GameBuddy is a separate project; Lunchpail supplies the game's identity and launch context. The player speaks to a compact companion icon and can change goals, references, the walkthrough outline and widgets during play.

## Current implementation

- SQLite guide graphs with nested steps, prerequisites, routes, source timestamps and coverage notes.
- Automatic first-launch YouTube discovery, model-assisted selection, cached download, chapter import and visual reference indexing. English captions are requested when available.
- Text/Markdown, guide JSON, local video and YouTube imports. Model compilation of text and bounded video sections; sparse samples remain incomplete evidence.
- Source-wide task/substep compilation with screenshot evidence, visible checks and ordering links. [Illustrated walkthroughs](docs/illustrated-walkthroughs.md) provide a searchable chapter reader, filmstrip, task graph and JSON exports.
- Independent reasoning/vision providers: signed-in Codex and Claude Code subscriptions, Ollama, OpenAI-compatible chat, OpenAI Responses and Gemini. Default local model: `qwen3.5:4b` via Ollama. [Subscription and live voice setup](docs/subscription-providers.md).
- AI tools to add text/YouTube references, enable/disable sources, edit goals and steps, inspect history and restore outlines. Changes are atomic and revision checked. Confirmed progress survives unchanged steps; altered tasks need new confirmation.
- A stdio MCP server exposing the same game tools to external agents.
- Draggable compact companion, Linux mic-unmute activation, local speech-turn detection and whisper.cpp transcription, Qt speech output, and typed HTML text/checklist/table/image/YouTube widgets with saved drag/resize layouts.
- Local visual retrieval with separate reference/live crops, repeated-match gating, uncertainty and conservative local VLM comparison. Similarity never completes objectives. [Spatial localization design and remaining work](docs/visual-localization.md).
- Automatic, resumable location-atlas preparation using the selected reasoning provider, walkthrough text and screenshots. GameBuddy discovers reference maps, compares their images with known places, and caches the associations; no game-specific room list is built into the application. [Maps and next actions](docs/contextual-navigation.md).
- A following/zoomable map OSD and prerequisite-aware next-action card, with goal changes, source preferences, explicit completion controls and separately recorded player reports through voice/MCP.
- Linux Gamescope session launcher with a compositor overlay, session shortcuts, HTML interaction and game-only XComposite capture.
- Lunchpail prepares the compositor before the emulator and retains its real game process/controller pipes; other platforms use the desktop companion.

**Development prototype:** the Linux compositor backend works with games on Gamescope’s private Xwayland server. Native Wayland game clients and Windows/macOS graphics-hook overlays are not implemented. Subscription-backed WebRTC voice and resumable source-wide compilation are implemented; their limits and checks are described in the [provider guide](docs/subscription-providers.md). Windows/macOS runtime behavior, automatic browser text discovery and independent-gameplay synchronization remain acceptance work. See [architecture](docs/architecture-and-research.md) and [overlay decision](docs/overlay-backends.md).

## Run

```sh
nix develop
cargo build --workspace
./target/debug/gamebuddy --launch --game Faxanadu --platform NES -- /path/to/emulator /path/to/game.nes
# Desktop companion / development workbench:
./target/debug/gamebuddy --workbench
```

On Linux, `--launch` starts Gamescope fullscreen at the host display's physical resolution, including ultrawide aspect ratios and fractional desktop scaling. Gamescope uses nearest filtering if additional scaling is needed; the game's own shaders and aspect-ratio settings still apply. `--windowed` defaults to 1280×720. `--width` and `--height` override the game resolution; specifying only one preserves the display aspect ratio. Click the icon to expand, drag to move, or hold to talk. **Shift+Tab** toggles interaction, **Ctrl+Shift+Tab** hides/restores everything, and **Escape** returns input to the game. Visible widgets can be dragged by their headers and resized from their bottom-right corners, even while the companion is collapsed. Hovering a widget captures only the pointer; keyboard input stays with the game. Click **×** to return a widget to the companion, then use **Widgets…** to show it again. **Hide widgets** / **Show widgets** toggle the whole set without losing layouts. **Options** opens inside the same overlay. Game-only synchronization is enabled automatically when references are available. `--no-prepare` skips automatic reference preparation for offline tests; `--quiet` starts with spoken output disabled.

Without `--launch`, the desktop companion uses **Share game** for OS screen sharing and opens Options in a separate workbench. Whole-desktop capture can include GameBuddy itself. Physical gamepad bindings remain to be connected; the session command interface is documented in [overlay backends](docs/overlay-backends.md).

With Codex live voice selected, GameBuddy connects automatically at game launch, introduces itself, and offers brief cues when the next action is stable. The microphone stays closed until activated. **Voice off** disables spoken output; **Stop** pauses the voice session until you resume. With **On unmute** enabled, mute/unmute your system microphone to start listening without taking game focus. If it is already unmuted, choose **Listen**. Pause to submit a speech turn; mute to stop capture. Say **“Open the guide”** or **“Arrange widgets”** to interact with the overlay. Hold-to-talk remains available. See [voice controls and platform coverage](docs/voice-input.md).

For local inference, Ollama must be running with the selected model installed. FFmpeg, yt-dlp and whisper.cpp are included in the Nix shell. Local transcription downloads the English whisper.cpp base model (~142 MB) on first use. Set `GAMEBUDDY_WHISPER_MODEL` to use an existing model and `GAMEBUDDY_WHISPER_CLI` to override its executable. The local speech option uses composed turns and pauses capture during spoken replies. The Codex live-voice option streams audio over WebRTC instead.

Data lives in the platform's local data directory under `GameBuddy` (Linux: `~/.local/share/GameBuddy`). Override with `--data-dir` or `GAMEBUDDY_DATA_DIR`. Configure models in Options; keys are read from named environment variables.

```sh
./target/debug/gamebuddy-cli doctor
./target/debug/gamebuddy-cli prepare Faxanadu --platform NES
./target/debug/gamebuddy-cli coach 'Add a goal to finish without optional collectibles'
./target/debug/gamebuddy-cli tool get_outline
./target/debug/gamebuddy-mcp
```

Point an MCP client's stdio configuration at the absolute `gamebuddy-mcp` executable and pass the app's `--data-dir`. Its environment needs the media tools for imports. Keep shell startup messages off protocol stdout.

For Lunchpail development, run `./dev.sh` to build initially and rebuild on source changes, then set **GameBuddy executable** to the absolute `scripts/gamebuddy-dev` path and enable **Launch with GameBuddy**. The launch script runs the existing `target/debug/gamebuddy` in this project's Nix runtime environment; it never runs Cargo during game startup. The watcher does not restart a running game. A missing binary fails immediately with the build command. On Linux, Lunchpail uses the Gamescope session handshake automatically when enabled. The script supplies the development runtime; it is not an installed package. The game command, working directory and controller pipes remain owned by Lunchpail.

For non-Nix builds, install Rust plus Qt 6 Core/Gui/Qml/Quick/QuickControls2/Multimedia/TextToSpeech/WebEngineQuick, expose `qmake` and Qt runtime plugins, then build with Cargo. Windows requires the MSVC Qt build for WebEngine. Native Windows/macOS verification is outstanding.

## Validate

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
./target/debug/gamebuddy --expanded --smoke-test /tmp/gamebuddy-companion.png
scripts/test-gamescope
scripts/test-resolution
scripts/test-voice
scripts/test-realtime
node scripts/test-voice-lifecycle.mjs
```

The initial live source test imported RetroNate's [Faxanadu walkthrough](https://www.youtube.com/watch?v=P05edoxsWQg): ten author chapters and 603 sampled reference frames. A held-out source frame matched its chapter. This is a same-video retrieval check, not proof of tracking a different player. The local 4B VLM abstained on the first comparison; accuracy and game coverage are not established.

The local 4B/9B planning trials also exposed unreliable tool use and goal interpretation. The validated tools reject malformed/stale edits, but the conversational agent is not yet a dependable autonomous guide. See [validation results and remaining gates](docs/validation.md).

The [initial research](docs/initial-research-2026-09-27.md) predates the Rust/CXX-Qt decision. [Provider contracts](docs/provider-contracts.md) describe the intended model boundaries.
