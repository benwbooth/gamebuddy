# Adaptive game companion architecture

Current design, September 27, 2026. Implementation status is in the README; this document includes requirements still to build.

## The play loop

1. Identify the game, edition/platform and session. Prefer Lunchpail's launch metadata. Process/window inspection and visual confirmation are fallbacks to implement; the player can correct uncertain identity.
2. Collect internet references as needed: walkthrough videos, captions, text guides, maps and strategy material. Retain source identity, timestamps, access failures and coverage. Cache work between sessions.
3. Maintain an outline graph around player goals, with nested tasks, optional objectives, prerequisites, alternative routes, backtracking and recovery after loading a save.
4. Show relevant HTML widgets over the game. A single draggable icon is the resting state; speech is the primary input.
5. Discuss goals and progress continuously. Observe the game, update location hypotheses, answer questions and adapt the plan. Advice, estimated location and confirmed completion remain distinct.

The player can ask to use a different video, ignore a guide, skip collectibles, add an item goal, break down a step or undo an outline change. Reference and goal edits go through the tool harness. Explicit progress correction currently remains in the workbench; speech-to-confirm-progress needs a separate evidence-aware tool.

## Implementation boundaries

| Layer | Code | Responsibility |
| --- | --- | --- |
| Domain | `gamebuddy-core` | Graph, source anchors, goals, source preferences, progress, visual retrieval, widgets |
| Services | `gamebuddy-services` | SQLite, media, captions, provider adapters, agent tools, speech, atomic revisions |
| Agent access | `gamebuddy-mcp` | Stdio MCP tools shared with the in-app coach |
| App | `gamebuddy-app` | CXX-Qt controller, compact QML companion, capture, HTML widgets, workbench |
| Launcher | Lunchpail | Game identity and process/session context; future pre-launch overlay handshake |
| Native overlay | Linux Gamescope adapter implemented | Private Xwayland game/overlay composition, focus routing and game-only capture; native Wayland and Windows/macOS graphics hooks remain |

The Rust host owns models, internet access, speech, compilation and state. A future injected shim handles only frame presentation/capture and input forwarding over bounded IPC. Models cannot execute OS commands through the game tools.

## Living plan

Guides have stable step IDs, increasing revisions, source preferences and player goals. Disabling a reference excludes its steps from normal retrieval and its frames from synchronization while retaining provenance and history.

`get_outline` returns the current graph. `edit_plan` applies step upserts/removals, goals and reference preferences against an expected revision. Parent/prerequisite cycles, dangling references, goals and source anchors are validated before an atomic SQLite commit. `restore_plan` restores a historical outline as a new revision.

Confirmed completion is copied only for unchanged steps. Changed/deleted steps need new evidence, while earlier revisions remain available. Observation epochs advance on edits. Visual references carry forward only where their anchors still identify the same content. Widgets are revision scoped so obsolete instructions disappear after editing.

`add_reference` attaches player-selected text or YouTube material, remapping IDs. The model can reconcile the added route in a subsequent edit. Import does not establish that conflicting sources agree or every interval was understood. Automatic web-page discovery and a complete multi-source compiler remain work.

## Models and evidence

Reasoning and visual inference are independently configurable. Frontier models can reconcile long references, interpret scenes, plan around goals and generate tool calls. Local perception handles frequent samples; expensive reasoning runs on questions, changes or unresolved evidence. Provider benchmarks are not GameBuddy accuracy measurements.

The implemented baseline uses spatial RGB fingerprints, source anchors, a continuity prior and repeated-match gating. Learned embeddings, OCR, temporal events and calibrated VLM reranking need evaluation on independent gameplay. The first target is Faxanadu on the development host's RX 7900 XTX.

Video compilation samples six frames plus available captions from a section of at most ten minutes. Author chapters alone are navigation. A complete compiler must chunk long sections, account for source intervals, reconcile overlapping tasks and revisit gaps at finer resolution. Captions cannot describe every silent action.

Voice currently composes local whisper.cpp, the configured reasoner and Qt speech. Duplex streaming, interruption while tools are working and a hotkey independent of game focus remain work.

## Acceptance gates

- Overlay: fullscreen game, draggable icon/widgets, input passthrough, restored focus, DPI/resolution changes, game-only capture and teardown; verify separately on Linux/Windows/macOS.
- Compilation: preserved source anchors, explicit missing media/captions and audited coverage before calling an outline complete.
- Adaptation: live source/goal changes, graph edit/undo, retained valid progress and rejection of stale model results.
- Synchronization: independent player footage, backtracking, death/load, repeated rooms, alternate routes and unseen scenes; report accuracy, abstention and latency.
- Conversation: microphone-to-answer latency, interruption, model switching, tool use and stable game controls.

The window prototype and focused tests do not establish untested platform or game-completion gates.
