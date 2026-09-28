# Contextual maps and next actions

Atlas preparation runs **inside GameBuddy**, automatically after walkthrough preparation and when an imported/revised guide needs an atlas. It uses the configured reasoning provider, not a developer-authored content pack. The CLI exposes the same service for inspection and resumable preparation:

```sh
gamebuddy-cli prepare Faxanadu --platform NES
gamebuddy-cli prepare-atlas GUIDE_ID --platform NES
gamebuddy-cli tool get_atlas
gamebuddy-cli tool get_game_context
```

The service discovers map references, classifies enabled leaf actions in batches using their parent instructions and sampled reference images, and saves source-evidenced place bindings incrementally. Each supplied step must be classified or explicitly left unknown. It reuses place identifiers across visits; moving between rooms or a weakly described interval can remain unbound. Model outputs are validated before publication. Images and narration remain untrusted source material, not instructions to the harness.

The initial catalog adapter reads the matching game table from VGMaps for NES, SNES, Genesis, Game Boy and GBA platform names. Availability depends on that catalog; unknown platforms still get a source-derived atlas. `add_map` accepts a player-supplied HTTPS image, source page and credit for any game. `set_map_enabled` controls preferences. GameBuddy downloads bounded raster previews into its data directory, builds contact sheets and asks the reasoning provider to compare their visual landmarks with known places. Association evidence is retained in `map_links`. These are model estimates, not geometrically registered room bounds. The OSD displays the original HTTPS map and attribution; displaying it currently requires network access even if an inference preview was cached.

Preparation uses a per-game process lock and revision-checked publication. Changed source material, steps or ancestor instructions invalidate their location bindings. Goal-only edits retain unchanged evidence. Interrupted batches resume, and failures leave a visible partial state for retry. Player-added maps and enable/disable preferences are kept separately from compiler snapshots so an in-flight build cannot erase them. A subsequent launch reuses reviewed steps and map associations.

During play, candidates from the existing screen matcher are grouped by place. Repeated distinct observations can locate a room even when several reference steps depict the same place. Strong unmapped competitors prevent that inference. Stale observations, reload epochs and uncertain matches clear the live location. The action cursor remains stable within the same estimated room; a changing reference screenshot alone does not advance the task.

The planner checks the editable goal/route graph, completed tasks and inherited prerequisites. An unknown prerequisite remains outstanding. A disabled prerequisite blocks advice until its source or plan is repaired. The **Now** card shows the eligible action and why it was selected; two following actions are explicitly previews. The outline highlights this action. The map follows the estimated room, while manual browsing and zoom do not alter tracked location or progress. Planned-stop links reflect objective ordering, **not verified physical door connections**.

**I completed this** explicitly confirms the displayed task. Voice and MCP can use `report_step_complete` only for an explicit player statement; it requires the current guide revision and progress epoch. Reports are stored separately in `reported_completed`, never inferred from advice or room similarity. Both types inform planning; old capture results cannot overwrite a newer report. Goal changes and newly eligible actions flow into live voice context without waiting for a new question.

Current limits: no automatic inventory/event verification, no player-coordinate dot, no room rectangle registered to map pixels, no geometric route drawing, no stitched maps, and no demonstrated accuracy on an independent gameplay session. The atlas and map associations improve the context available to perception and reasoning; they do not make the existing RGB/VLM matcher a validated spatial tracking system.

For reproducible UI review, `cargo run -p gamebuddy-services --example navigation_preview -- GUIDE_ID OUTPUT_DIR` exports the actual production widgets against controlled inputs drawn from the app-built atlas. It does not modify live progress or generate game content. This is a UI check, not a localization benchmark.
