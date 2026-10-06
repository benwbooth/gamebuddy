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
- Automatic conversation ducking on Linux/PipeWire: game audio fades down by 12 dB for speech and returns afterward. Saved game volume, mute, channel balance and other apps are preserved; toggle it in Options → Models.
- Local 2D/3D visual-place indexing with DINOv2/DISK/LightGlue, geometric verification, revisit merging, source timestamps and temporal uncertainty. 2D map-pixel registration highlights the camera footprint; 3D uses epipolar place matching. [Setup and limits](docs/visual-localization.md).
- Automatic, resumable location-atlas preparation using the selected reasoning provider, walkthrough text and screenshots. GameBuddy discovers reference maps, compares their images with known places, and caches the associations; no game-specific room list is built into the application. [Maps and next actions](docs/contextual-navigation.md).
- One passive walkthrough widget keeps the current instruction expanded among earlier and upcoming steps. Context adjusts to the available space, and long instructions scroll automatically without nested scrollbars or a duplicate next-action card.
- Continuous walkthrough recovery retires fulfilled temporary goals and repairs stalled routes from source evidence without clearing completed actions or dropping unfinished goals. Unsupported repairs surface a specific missing-information question. Movement arrows follow the current action and fresh view; stale or unsupported directions disappear.
- A persistent **Run journal** widget records observed milestones, player reports, source-backed plan repairs, notes, and retained game screenshots. Search and pin clues, codes and decisions; browse earlier runs without changing game progress. Voice/coach agents can save notes and recall entries or their images. Old screenshots remain historical evidence, not present inventory or location.
- A following/zoomable map OSD and prerequisite-aware next-action card, with goal changes, source preferences, explicit completion controls and separately recorded player reports through voice/MCP.
- Linux Gamescope session launcher with a compositor overlay, session shortcuts, HTML interaction and game-only XComposite capture.
- Background screen observation through the independent vision provider, including before a walkthrough/index exists. Native screenshot tools give voice, coach and MCP agents actual image access; meaningful repeated events can prompt brief spoken feedback.
- Source-backed danger warnings before nearby route hazards, shown in the Now card and spoken without waiting for scene inference. Warnings retain source cautions such as attacks that bypass protection.
- Automatic action progress from short screenshot sequences, checked by the selected reasoning provider. Visible changes such as paying for treatment and restoring a meter can complete an action and advance the next-action card; accepted events retain their before/after evidence.
- **Crop…** beside the companion's screen status opens a visual selector: draw, move or resize the play area, preview it, and save it per game. AI screenshots and location matching use that region. Cancel keeps the previous crop; Reset to full frame restores the whole capture when saved.
- Lunchpail prepares the compositor before the emulator and retains its real game process/controller pipes; other platforms use the desktop companion.

**Development prototype:** the Linux compositor backend works with games on Gamescope’s private Xwayland server. Native Wayland game clients and Windows/macOS graphics-hook overlays are not implemented. Subscription-backed WebRTC voice and resumable source-wide compilation are implemented; their limits and checks are described in the [provider guide](docs/subscription-providers.md). Windows/macOS runtime behavior, automatic browser text discovery and broad independent-gameplay synchronization remain acceptance work. See [architecture](docs/architecture-and-research.md) and [overlay decision](docs/overlay-backends.md).

## Run

```sh
nix develop
cargo build --workspace
./target/debug/gamebuddy --launch --game Faxanadu --platform NES -- /path/to/emulator /path/to/game.nes
# Desktop companion / development workbench:
./target/debug/gamebuddy --workbench
```

The Nix environment supplies Gamescope with continuous trackpad scroll and native pinch forwarding. Use `nix develop` or `scripts/gamebuddy-dev` for that input support; an external `GAMEBUDDY_GAMESCOPE_BIN` override must provide the same protocols. Image/map viewers accept two-finger pan and pinch zoom. The expanded companion can also be moved by dragging its title.

On Linux, `--launch` starts Gamescope fullscreen at the host display's physical resolution, including ultrawide aspect ratios and fractional desktop scaling. Gamescope uses nearest filtering if additional scaling is needed; the game's own shaders and aspect-ratio settings still apply. `--windowed` defaults to 1280×720. `--width` and `--height` override the game resolution; specifying only one preserves the display aspect ratio. Click the icon to expand, drag to move, or hold to talk. **Shift+Tab** toggles interaction, **Ctrl+Shift+Tab** hides/restores everything, and **Escape** returns input to the game. Visible widgets can be dragged by their headers and resized from their bottom-right corners, even while the companion is collapsed. Hovering a widget captures only the pointer; keyboard input stays with the game. Click **×** to return a widget to the companion, then use **Widgets…** to show it again. **Hide widgets** / **Show widgets** toggle the whole set without losing layouts. **Options** opens inside the same overlay. Game-only watching starts automatically, including while references are still preparing. The companion shows what the vision model observes; **Pause watching** stops capture and screen-tool access. Location matching also runs once references are available. `--no-prepare` skips automatic reference preparation for offline tests; `--quiet` starts with spoken output disabled.

Without `--launch`, the desktop companion uses **Share game** for OS screen sharing and opens Options in a separate workbench. Whole-desktop capture can include GameBuddy itself. Physical gamepad bindings remain to be connected; the session command interface is documented in [overlay backends](docs/overlay-backends.md).

With Codex live voice selected, GameBuddy connects automatically at game launch, introduces itself, and speaks the next instruction as goals advance, and offers source-backed coaching for visible obstacles and repeated failures. **Hands-free** is on by default: if your mic is unmuted, just speak—no click or mute toggle needed. A muted mic waits for unmute. The local speech backend also starts listening automatically. **Stop listening** stops capture until you resume; **Voice off** disables spoken output; **Stop** pauses the voice session. Disable **Hands-free** or launch with `--no-listen` for manual activation. Pause to submit a speech turn; mute to stop capture. Say **“Open the guide”** or **“Arrange widgets”** to interact with the overlay. Hold-to-talk remains available. See [voice controls and platform coverage](docs/voice-input.md).

Expand the companion and type into **Message GameBuddy…**, then press **Enter** or **Send**. Text uses your selected reasoning model, the current screen, and the same game conversation history and guide tools as composed voice. You can chat while a guide is preparing; guide edits become available once it is ready. Replies and any connection errors appear above the input. **Escape** returns keyboard input to the game.

Codex sessions are saved separately for each game and resumed on the next launch, including the reasoning thread's earlier tool results. Voice reconnects with the saved dialogue and a fresh audio connection; text coaching also resumes its own thread. Current screenshots and progress take precedence over old discussion after loading a save. GameBuddy stores session IDs in its data directory; the signed-in Codex CLI retains the thread history. If that history was deleted or the tool catalog changes incompatibly, GameBuddy starts a new thread using its existing dialogue archive. Other providers continue to receive the saved dialogue archive.

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

The native Codex voice handles ordinary conversation and stories directly as speech-to-speech. It delegates game questions and application actions to the tool backend and speaks its verified results naturally. The **Thinking…** indicator tracks that delegated work. Routine next-step requests can select current source-backed guidance locally. Screen movement and unrelated progress updates do not cut off an ongoing native reply. Optional prepared neural speech uses a different local voice and composed text replies; enable it deliberately with `scripts/setup-prepared-speech`. See [voice architecture, setup and measured limits](docs/responsiveness.md).

GameBuddy prepares outcome rules separately from the demonstrated walkthrough: having an item can satisfy a requirement obtained by several methods, making a redundant detour **Not needed** without marking it completed. The normal route prioritizes progress; optional rewards remain available by request. Confirmed acquisitions and consumption update saved world facts, while resetting progress clears them. Exact possession reports such as “I have the Silver Key” update locally when that item has a source-defined rule. Route corrections preserve the original walkthrough and invalidate conflicting advice immediately. Screenshot and map evidence still cannot establish hidden inventory by themselves.

**Models & speed** separates interactive advice, screen interpretation, and background route planning. Automatic planning uses GPT-6.1 Sol with medium thinking when advice uses Codex; other providers inherit the advice configuration. An optional planning override exposes its own provider, model, and thinking controls. Rules are cached by guide revision and source region, prepared ahead of the current task, and evaluated locally on progress updates. `--no-prepare` also disables automatic outcome compilation. Fresh urgent screen events can interrupt routine advice or pending reasoning once the player finishes speaking. Readable death messages and numeric low-health displays use a bounded local detector; other scenes retain the vision fallback.

For Lunchpail development, run `./dev.sh` to build initially and rebuild on source changes, then set **GameBuddy executable** to the absolute `scripts/gamebuddy-dev` path and enable **Launch with GameBuddy**. The launch script runs the existing `target/debug/gamebuddy` in this project's Nix runtime environment; it never runs Cargo during game startup. The watcher does not restart a running game. A missing binary fails immediately with the build command. On Linux, Lunchpail uses the Gamescope session handshake automatically when enabled. The script supplies the development runtime; it is not an installed package. The game command, working directory and controller pipes remain owned by Lunchpail.

For non-Nix builds, install Rust plus Qt 6 Core/Gui/Qml/Quick/QuickControls2/Multimedia/TextToSpeech/WebEngineQuick, expose `qmake` and Qt runtime plugins, then build with Cargo. Windows requires the MSVC Qt build for WebEngine. Native Windows/macOS verification is outstanding.

## Validate

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
./target/debug/gamebuddy --expanded --smoke-test /tmp/gamebuddy-companion.png
scripts/test-gamescope
bash scripts/test-native-input
GAMEBUDDY_TEST_POINTER_REPLAY=finger bash scripts/test-native-input
GAMEBUDDY_TEST_POINTER_REPLAY=pinch bash scripts/test-native-input
GAMEBUDDY_TEST_WORKING=1 scripts/test-realtime
scripts/test-resolution
scripts/test-voice
scripts/test-realtime
GAMEBUDDY_TEST_LOCATION=1 scripts/test-realtime
node scripts/test-voice-lifecycle.mjs
QT_QPA_PLATFORM=offscreen QT_QUICK_BACKEND=software qmltestrunner -input scripts/qml-tests
```

The selected Faxanadu source is now U Can Beat Video Games’ [Ultimate Guide](https://www.youtube.com/watch?v=8vEyiAs-fbY). GameBuddy reviewed all 53 video sections into 824 outline nodes (549 leaf actions), exported 578 screenshots, and indexed 3,112 dense visual references. The original RetroNate guide remains preserved. Source review is not proof of complete game correctness; [localization validation](docs/localization-validation.md) separates independent capture evidence from source coverage.

The [map retrieval benchmark](docs/map-retrieval-benchmark.md) compares local EmbeddingGemma 2 and DINO candidate selection through the same production geometry checks, using independently labelled gameplay and an isolated store.

The local 4B/9B planning trials also exposed unreliable tool use and goal interpretation. The validated tools reject malformed/stale edits, but the conversational agent is not yet a dependable autonomous guide. See [validation results and remaining gates](docs/validation.md).

The [initial research](docs/initial-research-2026-09-27.md) predates the Rust/CXX-Qt decision. [Provider contracts](docs/provider-contracts.md) describe the intended model boundaries.
