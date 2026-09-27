# GameBuddy

An AI game companion built in Rust, with Qt/QML through CXX-Qt and HTML widgets. GameBuddy is a separate project; Lunchpail supplies the game's identity and launch context. The player speaks to a compact companion icon and can change goals, references, the walkthrough outline and widgets during play.

## Current implementation

- SQLite guide graphs with nested steps, prerequisites, routes, source timestamps and coverage notes.
- Automatic first-launch YouTube discovery, model-assisted selection, cached download, chapter import and visual reference indexing. English captions are requested when available.
- Text/Markdown, guide JSON, local video and YouTube imports. Model compilation of text and bounded video sections; sparse samples remain incomplete evidence.
- Independent reasoning/vision provider settings: Ollama, OpenAI-compatible chat, OpenAI Responses and Gemini. Default local model: `qwen3.5:4b` via Ollama.
- AI tools to add text/YouTube references, enable/disable sources, edit goals and steps, inspect history and restore outlines. Changes are atomic and revision checked. Confirmed progress survives unchanged steps; altered tasks need new confirmation.
- A stdio MCP server exposing the same game tools to external agents.
- Draggable compact companion, hold-to-talk via whisper.cpp, Qt speech output, and typed HTML text/checklist/table/image/YouTube widgets with saved drag/resize layouts.
- Local visual retrieval with separate reference/live crops, repeated-match gating, uncertainty and optional VLM comparison. Similarity never completes objectives.
- Lunchpail integration adds an optional executable setting and forwards game/title/platform/process/session arguments after a successful launch.

**Development prototype:** the current OSD uses Qt desktop windows. Gamescope composition and native graphics-hook overlays are not implemented. Windows/macOS runtime behavior, full-duplex voice, complete source-wide walkthrough compilation, automatic browser text discovery and independent-gameplay synchronization remain acceptance work. See [architecture](docs/architecture-and-research.md) and [overlay decision](docs/overlay-backends.md).

## Run

```sh
nix develop
cargo build --workspace
./target/debug/gamebuddy --game Faxanadu --platform NES
```

Click the icon to expand, drag to move, or hold to talk. **Share game** starts OS screen sharing. **Arrange** enables widget editing; **Lock** restores click-through mode. **Options** opens the development workbench; `--workbench` opens it at startup. Capturing the whole desktop can include GameBuddy itself; select the game surface where supported.

Ollama must be running with the selected model installed. FFmpeg, yt-dlp and whisper.cpp are included in the Nix shell. The first speech turn downloads the English whisper.cpp base model (~142 MB). Set `GAMEBUDDY_WHISPER_MODEL` to use an existing model and `GAMEBUDDY_WHISPER_CLI` to override its executable. Voice currently uses composed push-to-talk turns.

Data lives in the platform's local data directory under `GameBuddy` (Linux: `~/.local/share/GameBuddy`). Override with `--data-dir` or `GAMEBUDDY_DATA_DIR`. Configure models in Options; keys are read from named environment variables.

```sh
./target/debug/gamebuddy-cli doctor
./target/debug/gamebuddy-cli prepare Faxanadu --platform NES
./target/debug/gamebuddy-cli coach 'Add a goal to finish without optional collectibles'
./target/debug/gamebuddy-cli tool get_outline
./target/debug/gamebuddy-mcp
```

Point an MCP client's stdio configuration at the absolute `gamebuddy-mcp` executable and pass the app's `--data-dir`. Its environment needs the media tools for imports. Keep shell startup messages off protocol stdout.

For Lunchpail development, set **GameBuddy executable** to the absolute `scripts/gamebuddy-dev` path and enable **Launch with GameBuddy**. The script builds/runs this project in its Nix shell. It is a development launcher, not an installed package or Gamescope session launcher.

For non-Nix builds, install Rust plus Qt 6 Core/Gui/Qml/Quick/QuickControls2/Multimedia/TextToSpeech/WebEngineQuick, expose `qmake` and Qt runtime plugins, then build with Cargo. Windows requires the MSVC Qt build for WebEngine. Native Windows/macOS verification is outstanding.

## Validate

```sh
cargo test -p gamebuddy-core -p gamebuddy-services -p gamebuddy-cli
cargo clippy --workspace --all-targets -- -D warnings
./target/debug/gamebuddy --expanded --smoke-test /tmp/gamebuddy-companion.png
```

The initial live source test imported RetroNate's [Faxanadu walkthrough](https://www.youtube.com/watch?v=P05edoxsWQg): ten author chapters and 603 sampled reference frames. A held-out source frame matched its chapter. This is a same-video retrieval check, not proof of tracking a different player. The local 4B VLM abstained on the first comparison; accuracy and game coverage are not established.

The local 4B/9B planning trials also exposed unreliable tool use and goal interpretation. The validated tools reject malformed/stale edits, but the conversational agent is not yet a dependable autonomous guide. See [validation results and remaining gates](docs/validation.md).

The [initial research](docs/initial-research-2026-09-27.md) predates the Rust/CXX-Qt decision. [Provider contracts](docs/provider-contracts.md) describe the intended model boundaries.
