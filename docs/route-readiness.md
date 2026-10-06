# Route readiness and last-seen screenshots

Route preparation reads the complete enabled source route, so a requirement stated at an earlier pickup can guard a later action in another chapter. It compiles separate acquisition, departure and consumer rules. Local evaluation rejects an unmet world requirement even when the player is at the matching location or an older recommendation was accepted there. It resolves the accepted action's prerequisite before falling back to an older checklist goal. Nearby requirement warnings take priority over general combat warnings.

An explicit current player location report can support actions bound to that place. It does not establish possession or completion. A route review preserves facts from actions it did not review. Source instructions remain the evidence; the model may leave an unsupported requirement unknown.

The **Last seen screen** widget keeps the exact cropped JPEG shared with the answer model or processed by the scene watcher, with its capture age and reader. New raw captures do not advance it. Images travel through the existing native preview bridge; the page still cannot read arbitrary local files. Capture tickets invalidate the preview on pause, crop changes, restart and game changes. The image element stays mounted during unrelated UI refreshes.

Voice input ignores recognized breath/sigh annotations without dropping the surrounding words. Initial reasoning input includes a focused source search and its place IDs, reducing serial guide/atlas requests. A rejected final answer receives fresh context and an explicit native turn update. The rejection still publishes no advice; request identity, progress and current pixels are validated again.

## Validation, 2026-10-06

- 37 core and 180 service tests passed. One existing spatial integration test requires a prepared local index and was not run.
- `scripts/test-guide-library` passed in native Qt WebEngine, including the recorded 1245×960 game screenshot, stable image identity, age display, cleared paused/new-session views, and the existing walkthrough, map, gesture and journal checks.
- The configured native model compiled the actual 824-node Faxanadu guide into requirements for the separate Ruby Ring conversation, the Wyvern-area departure and the Elven fountain rock. Missing-ring replays rejected both departure and rock instructions and redirected to the pickup in 13–21 ms; simulated possession permitted both. Simulations did not claim player completion.
- The same missing-ring voice request went from eight calls and 22.7 seconds to two calls and 5.3 seconds until its final answer was ready. The new answer published the ring pickup and explained the later fountain use and Joker Key return cost.
- Changed-image replays rejected the old answer, then supplied replacement pixels through a native turn update. They finished with the new image's correct description in 5.7 and 8.6 seconds total, using two and three calls respectively; the latter still made one screenshot lookup. A tool-result-only retry had taken 20.9 seconds and five calls.

These are individual native backend replays, not microphone-to-audio latency guarantees or a complete-game routing validation. Source preparation remains a background model pass (about 59 seconds for this guide); local requirements and warnings do not wait on that pass once prepared.

Replays use private `/tmp` stores. `outcome_probe` prepares their source rules; `route_readiness_probe STORE FACT GATE...` checks a missing-item case and simulated possession. `voice_backend_probe` supports `GAMEBUDDY_PROBE_IMMEDIATE=1`, `GAMEBUDDY_PROBE_PREFETCH=1`, `GAMEBUDDY_PROBE_FRAME` and `GAMEBUDDY_PROBE_CHANGED_FRAME` for the native answer/replacement-image path. `seen_screen_probe STORE FRAME` exercises the same image storage and bridge used by the UI.
