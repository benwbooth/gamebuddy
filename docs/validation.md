# Development validation

September 27, 2026, NixOS/KDE Wayland, RX 7900 XTX. This is evidence for the development prototype, not cross-platform overlay or game-completion certification.

| Check | Result |
| --- | --- |
| Rust workspace build | Passed locally with Qt 6.11.2 |
| Rust tests | 33 passed: existing 31 cases plus native ultrawide/aspect-preserving resolution overrides and rotated physical display dimensions |
| Strict workspace Clippy | Passed from the final Rust tree |
| MCP | Real stdio handshake and all 11 tools listed; goal edit, text reference attachment, widget creation and outline restore exercised on a fixture |
| Qt UI | Actual compact icon, expanded companion and workbench rendered on Wayland; image captures inspected |
| HTML interaction | Production Qt WebEngine inside Gamescope: widget dragged from (600,80) to (750,276), resized from 350×260 to 430×330, and persisted to SQLite |
| Gamescope interaction | Live private-session fixture passed keyboard passthrough, Shift+Tab focus transfer, Escape release, Ctrl+Shift+Tab visibility, icon expansion/drag, and socket focus/hide/show commands |
| Native fullscreen resolution | Fixed the hardcoded 1280×720 canvas. Live Wayland physical-mode discovery reported 5120×2160 at 130% desktop scale. The private game saw exactly 5120×2160; Gamescope's outer screenshot was 5119×2160 due to fractional surface rounding. All four corners were filled and 489 consecutive one-pixel transitions remained sharp without blended gray pixels. Windowed input/widget/capture regression also passed |
| Capture isolation | Pixel-identical game-only frames with overlay closed/open; fullscreen RetroArch Flatpak capture excluded GameBuddy using XRes PID selection and XComposite readback |
| Real game | Faxanadu/FCEUmm rendered fullscreen beneath the overlay on KDE Wayland. A copied auto-save loaded successfully; test saves/config stayed in a separate temporary directory. Session exited cleanly |
| Native game with Lunchpail display settings | Faxanadu/FCEUmm through the v1 bridge detected and rendered 5120×2160. A second test reused Lunchpail's generated NES launch-display config and `--set-shader` preset: the selected RetroTube/Koko-AIO CRT shader and Duimon NES night bezel rendered together beneath GameBuddy. The composed screenshot was visually inspected; this was a programmatic bridge launch with copied save state, not a GUI click-through test. Proof is under `artifacts/resolution/faxanadu-with-display/` |
| Local speech | whisper.cpp transcribed a synthesized request about the next tower; “Faxanadu” was transcribed as “Faksanadu” |
| Mic activation | Virtual-source runtime test passed mute/unmute capture, passive focus, silence rejection, a real VAD/Whisper “open the guide” command, a question through a delayed local provider fixture, resumption and ephemeral recording cleanup. Qt entered Speaking then Ready while capture stayed closed. Physical MV7+ mute-button behavior remains unverified |
| Voice UI | Expanded companion rendered and inspected with Qt's offscreen backend; live Gamescope captures show the listening icon and spoken guide command. The latest standalone Wayland `grabToImage` smoke timed out waiting for its callback; the smoke command now has a bounded timeout |
| Video import | RetroNate Faxanadu video downloaded; ten author chapters imported; 603 timestamped reference frames indexed |
| Captions | Live request was rate-limited by YouTube (HTTP 429); video/chapters retained with an explicit coverage note. Caption parsing tested with timed JSON3 data |
| Visual synchronization | Held-out frame from the reference video matched its chapter. No independent-player accuracy measurement yet; 4B VLM initially abstained |
| Lunchpail | Three focused unit tests and the opt-in live Gamescope integration test passed. The real launch function retained the original game executable, actual PID and controller pipes; the overlay fixture passed and the supervisor was reaped. Physical controller behavior and launching through Lunchpail's GUI remain unverified |

Local reasoning tests revealed both malformed widgets and incorrect goal interpretation. Typed schemas and a repair loop reject malformed results. The coach now binds edits to the revision actually shown to the model, and refuses to return a success response while a mutation has an unresolved error. This avoids delegating revision arithmetic to the model and prevents a failed edit from being reported as saved. Semantic planning quality still needs a representative evaluation.

The final 9B trial applied a structurally valid outline edit but represented the player's goal as unsourced steps, repeated edits, and exhausted its budget after returning an invalid widget. Its response was rejected. Neither tested local model is validated for reliable autonomous planning; use the MCP harness or configure a stronger reasoner and evaluate its behavior. The goal/edit tools passed deterministic and MCP tests, which does not establish natural-language agent reliability.

Downloaded models: Qwen3.5 4B and 9B through Ollama, and whisper.cpp base English. Ollama reported GPU allocation for both Qwen models. Larger local reasoning is an experiment, not a benchmark-backed default. Other existing Ollama models were not explicitly stopped.

Voice activation uses composed local speech turns. One fast synthesized command was misheard by Whisper as “open the knives” and correctly took the ordinary question path without stealing focus. The slower US-English fixture passed; this is not a recognition-accuracy benchmark. Exact focus commands intentionally avoid fuzzy matching. See [voice input](voice-input.md) for controls and platform boundaries.

Outstanding runtime gates: native Wayland game clients, Windows/macOS overlays and packaging, physical controller bindings/navigation, Steam/Mangoapp coexistence, display/DPI changes, broader game compatibility, actual microphone and spoken audio conversation, reliable progress adaptation, continuous perception while reasoning, and complete reference-wide walkthrough compilation. See the [overlay decision](overlay-backends.md).

The compositor tests used Gamescope **3.16.29** from the project flake and Qt **6.11.2**. Initial fullscreen tests used a 1280×720 nested game canvas, which caused blurry scaling and a narrower image on this ultrawide display. The corrected fullscreen path uses the physical 5120×2160 mode; windowed fixtures retain 1280×720. The outer compositor used Wayland; game and UI used private Xwayland. Keyboard events were generated only in dedicated private integration sessions. Screenshot artifacts and logs are kept locally under `artifacts/gamescope/` and `artifacts/resolution/` (ignored by Git). No independent gameplay-localization accuracy is implied by the rendering/capture test.

The GitHub workflow covers Rust core/services/CLI and overlay-library tests on Linux, Windows and macOS. It does not build Qt packages or establish runtime compatibility.
