# Development validation

September 27, 2026, NixOS/KDE Wayland, RX 7900 XTX. This is evidence for the development prototype, not cross-platform overlay or game-completion certification.

| Check | Result |
| --- | --- |
| Rust workspace build | Passed locally with Qt 6.11.2 |
| Rust tests | 26 passed: existing 21 domain/service cases plus overlay launch parsing, focus policy, pixel decoding and session command transport |
| Strict workspace Clippy | Passed from the final Rust tree |
| MCP | Real stdio handshake and all 11 tools listed; goal edit, text reference attachment, widget creation and outline restore exercised on a fixture |
| Qt UI | Actual compact icon, expanded companion and workbench rendered on Wayland; image captures inspected |
| HTML interaction | Production Qt WebEngine inside Gamescope: widget dragged from (600,80) to (750,276), resized from 350×260 to 430×330, and persisted to SQLite |
| Gamescope interaction | Live private-session fixture passed keyboard passthrough, Shift+Tab focus transfer, Escape release, Ctrl+Shift+Tab visibility, icon expansion/drag, and socket focus/hide/show commands |
| Capture isolation | Pixel-identical game-only frames with overlay closed/open; fullscreen RetroArch Flatpak capture excluded GameBuddy using XRes PID selection and XComposite readback |
| Real game | Faxanadu/FCEUmm rendered fullscreen beneath the overlay on KDE Wayland. A copied auto-save loaded successfully; test saves/config stayed in a separate temporary directory. Session exited cleanly |
| Local speech | whisper.cpp transcribed a synthesized request about the next tower; “Faxanadu” was transcribed as “Faksanadu” |
| Video import | RetroNate Faxanadu video downloaded; ten author chapters imported; 603 timestamped reference frames indexed |
| Captions | Live request was rate-limited by YouTube (HTTP 429); video/chapters retained with an explicit coverage note. Caption parsing tested with timed JSON3 data |
| Visual synchronization | Held-out frame from the reference video matched its chapter. No independent-player accuracy measurement yet; 4B VLM initially abstained |
| Lunchpail | Three focused unit tests and the opt-in live Gamescope integration test passed. The real launch function retained the original game executable, actual PID and controller pipes; the overlay fixture passed and the supervisor was reaped. Physical controller behavior and launching through Lunchpail's GUI remain unverified |

Local reasoning tests revealed both malformed widgets and incorrect goal interpretation. Typed schemas and a repair loop reject malformed results. The coach now binds edits to the revision actually shown to the model, and refuses to return a success response while a mutation has an unresolved error. This avoids delegating revision arithmetic to the model and prevents a failed edit from being reported as saved. Semantic planning quality still needs a representative evaluation.

The final 9B trial applied a structurally valid outline edit but represented the player's goal as unsourced steps, repeated edits, and exhausted its budget after returning an invalid widget. Its response was rejected. Neither tested local model is validated for reliable autonomous planning; use the MCP harness or configure a stronger reasoner and evaluate its behavior. The goal/edit tools passed deterministic and MCP tests, which does not establish natural-language agent reliability.

Downloaded models: Qwen3.5 4B and 9B through Ollama, and whisper.cpp base English. Ollama reported GPU allocation for both Qwen models. Larger local reasoning is an experiment, not a benchmark-backed default. Other existing Ollama models were not explicitly stopped.

Outstanding runtime gates: native Wayland game clients, Windows/macOS overlays and packaging, physical controller bindings/navigation, Steam/Mangoapp coexistence, display/DPI changes, broader game compatibility, actual microphone and spoken audio conversation, reliable progress adaptation, continuous perception while reasoning, and complete reference-wide walkthrough compilation. See the [overlay decision](overlay-backends.md).

The compositor tests used Gamescope **3.16.29** from the project flake, Qt **6.11.2**, a 1280×720 nested game canvas and the host's 5119×2160 fullscreen output. The outer compositor used Wayland; game and UI used private Xwayland. Keyboard events were generated only in dedicated private integration sessions. Screenshot artifacts and logs are kept locally under `artifacts/gamescope/` (ignored by Git). No independent gameplay-localization accuracy is implied by the rendering/capture test.

The GitHub workflow covers Rust core/services/CLI and overlay-library tests on Linux, Windows and macOS. It does not build Qt packages or establish runtime compatibility.
