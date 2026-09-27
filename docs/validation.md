# Development validation

September 27, 2026, NixOS/KDE Wayland, RX 7900 XTX. This is evidence for the development prototype, not cross-platform overlay or game-completion certification.

| Check | Result |
| --- | --- |
| Rust workspace build | Passed locally with Qt 6.11.2 |
| Domain/service tests | 21 passed: graph validation, URLs, reference filtering, revision/history/progress handling, model-output repair and provider protocol checks |
| Strict workspace Clippy | Passed from the final Rust tree |
| MCP | Real stdio handshake and all 11 tools listed; goal edit, text reference attachment, widget creation and outline restore exercised on a fixture |
| Qt UI | Actual compact icon, expanded companion and workbench rendered on Wayland; image captures inspected |
| HTML interaction | Browser fixture using the production renderer: drag changed position from (28,30) to (58,325); resize changed 350×260 to 470×315; layout events emitted |
| Local speech | whisper.cpp transcribed a synthesized request about the next tower; “Faxanadu” was transcribed as “Faksanadu” |
| Video import | RetroNate Faxanadu video downloaded; ten author chapters imported; 603 timestamped reference frames indexed |
| Captions | Live request was rate-limited by YouTube (HTTP 429); video/chapters retained with an explicit coverage note. Caption parsing tested with timed JSON3 data |
| Visual synchronization | Held-out frame from the reference video matched its chapter. No independent-player accuracy measurement yet; 4B VLM initially abstained |
| Lunchpail | Two focused integration tests passed: disabled defaults/preferences and literal argument forwarding. Launch UI/runtime sequence still needs interactive game verification |

Local reasoning tests revealed both malformed widgets and incorrect goal interpretation. Typed schemas and a repair loop reject malformed results. The coach now binds edits to the revision actually shown to the model, and refuses to return a success response while a mutation has an unresolved error. This avoids delegating revision arithmetic to the model and prevents a failed edit from being reported as saved. Semantic planning quality still needs a representative evaluation.

The final 9B trial applied a structurally valid outline edit but represented the player's goal as unsourced steps, repeated edits, and exhausted its budget after returning an invalid widget. Its response was rejected. Neither tested local model is validated for reliable autonomous planning; use the MCP harness or configure a stronger reasoner and evaluate its behavior. The goal/edit tools passed deterministic and MCP tests, which does not establish natural-language agent reliability.

Downloaded models: Qwen3.5 4B and 9B through Ollama, and whisper.cpp base English. Ollama reported GPU allocation for both Qwen models. Larger local reasoning is an experiment, not a benchmark-backed default. Other existing Ollama models were not explicitly stopped.

Outstanding runtime gates: native Linux Gamescope overlay, Windows/macOS overlays and packaging, game-only capture, interactive passthrough/focus behavior over fullscreen games, actual microphone and spoken audio conversation, reliable progress adaptation, continuous perception while reasoning, and complete reference-wide walkthrough compilation. See the [overlay decision](overlay-backends.md).

The GitHub workflow covers Rust core/services/CLI on Linux, Windows and macOS. It does not build Qt packages or establish runtime compatibility.
