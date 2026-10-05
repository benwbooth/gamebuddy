# Localization validation — 2026-10-03

This records a narrow prototype check, not universal game or physical-room accuracy. The machine-readable [evidence report](evidence/localization-2026-10-03.json) retains per-frame results and source reference IDs.

## Source compilation

The selected source is U Can Beat Video Games' [Faxanadu Ultimate Guide](https://www.youtube.com/watch?v=8vEyiAs-fbY), duration 5,770,879 ms. GameBuddy itself downloaded and compiled it through its normal preparation services. One section initially failed dependency validation; retry produced a valid graph. The final coverage export reports:

| Item | Result |
|---|---:|
| Reviewed video sections | 53/53 |
| Unreviewed video intervals | 0 |
| Outline nodes / leaf actions | 824 / 549 |
| Illustrated-guide screenshots | 578 |
| Dense visual reference views | 3,112 |
| Visual clusters after revisit verification | 201 |

Source review uses sampled frames and available captions. It does not establish that every instruction is correct, that all optional game content is covered, or that all 201 clusters represent distinct physical rooms. The old RetroNate guide is retained separately.

## Independent gameplay and CRT evidence

The Rust `libretro_probe` example ran the user's matching Faxanadu USA ROM through FCEUmm, with private software rendering and in-memory save/restore. It produced 273 sampled frames spanning three RAM screen IDs and two save/progress epochs. The player's actual saves and running game were not used or changed.

A separate small calibration set was inspected to assign source cluster labels. A 55-frame held-out replay then used those frozen labels: title screen versus the broad Eolis corridor cluster. The corridor cluster spans multiple RAM screens, so this measures **visual-place** recognition rather than distinguishing every room or doorway.

| Result | Count |
|---|---:|
| Correct accepted assignments | 45 |
| Incorrect confident assignments | 0 |
| Abstentions | 10 |
| Median / maximum localization time | 176 / 931 ms |

Timing covers warmed feature extraction/retrieval/registration/tracking on ROCm; it excludes model startup, capture, map registration and semantic reasoning. It is not a voice response-time measurement. Consecutive frames from one short session are correlated; these figures are not a general accuracy estimate.

The saved CRT screenshot was independently aligned to the real VGMaps Eolis image with **308 inlier landmarks and 0.883 px mean residual** in normalized matching coordinates. The inspected map crop agrees with the street/buildings in that screenshot. The initial check used the Eolis map alone. A follow-up search across all 13,096 indexed tiles from 29 maps selected the same Eolis region with the same 308 inliers. One successful capture still does not establish accuracy across the game. Repeating that same saved image exercised the two-observation gate but is not evidence of real temporal stability. The native Rust-only matcher abstained on that image; the learned matcher supplied the useful correspondences.

## 3D, state and UI checks

- Three Python geometry tests cover nonplanar 3D points under camera rotation/translation with outliers, rejection of unrelated 3D correspondences, and 2D scrolling versus HUD-only evidence.
- Rust checks cover geometrically unrelated/blank images, scrolling with changing foreground, competing place hypotheses, session/epoch reset, return-visit objective ambiguity, and source/calibration invalidation.
- The ROM payload was compared with the pinned disassembly: the PRG payload matched; four iNES header bytes differed. The different USA revision was rejected. RAM decoding was exercised through the real core. Strict UDP reply parsing passed, but live RetroArch UDP attachment is **not verified** because isolated RetroArch frontends crashed before a usable command session.
- The Gamescope regression test passed keyboard passthrough, focus shortcuts, pointer expansion, widget dragging, dismissal/restoration and clean game-only capture.
- An opt-in integration test published the independent CRT capture through the real screen service, localized it, passed its camera footprint into navigation, delivered both reference/map images through the MCP/voice tool path, and verified that changing the crop immediately hid stale localization. It uses an isolated store and the Eolis map fixture.
- The actual HTML widget rendered the measured map highlight and reference image in a browser fixture. The map picker opened with Eolis selected and both images loaded. This is UI/data-contract evidence, not a live gameplay demonstration.
- Workspace tests, Clippy with warnings denied, workspace build, and widget JavaScript syntax checks passed. Existing C++ `_FORTIFY_SOURCE` build warnings remain.

No actual 3D game session, full-game Faxanadu replay, death recovery sequence, native Windows/macOS runtime, metric 3D reconstruction or autonomous objective completion has passed acceptance testing in this change.

## October 4: map continuity and arrival corrections

The saved vendor-area session exposed two inconsistent reads: the atlas marker used the newest observation while its named-place tracker could still belong to the previous capture, and a scrolling screen could invalidate a match before asynchronous registration finished. Navigation now derives the place directly from the map's accepted observation. A bounded translation check tolerates small camera scrolls during the existing three-second frame-lag budget; completed non-matches, capture changes and expired observations still invalidate visual location.

The companion receives the matched scenery's source actions and place IDs alongside the native map image. Saved advice is labelled as previous advice, and the live speaker no longer receives that instruction as background context. Final transcript corrections reach an already-running reasoning turn. A place-only player report invalidates previous advice without selecting an older task associated with the same street or claiming task completion.

The captured CRT vendor screen matched Eolis with 301 map inliers. Thirty sampled refreshes while translating that saved image retained the same map, named place and native map image in the agent context. The translations are a continuity regression, not new independent gameplay or a full-game accuracy measurement. The isolated Gamescope/WebRTC voice fixture also heard an arrival correction, recorded the new location, replaced the old walk-left action and audibly acknowledged that the player was already at the vendors. Enable that regression with `GAMEBUDDY_TEST_LOCATION=1 scripts/test-realtime`.

## Reproduce with your own game files

```sh
nix develop
cargo run -p gamebuddy-services --example libretro_probe -- \
  /path/to/fceumm_libretro.so /path/to/Faxanadu.nes /tmp/private-faxanadu-capture
# Labels must come from inspection/independent evidence, not predictions on the scored frames.
gamebuddy-cli evaluate-world GUIDE /path/to/labelled-replay.json
/path/to/vision/venv/bin/python scripts/test-spatial-geometry.py
scripts/test-gamescope
# Optional full local-model integration check; the source store is copied into a temporary test store.
GAMEBUDDY_TEST_SPATIAL_STORE=/path/to/prepared/fixture \
GAMEBUDDY_TEST_SPATIAL_FRAME=/path/to/independent/cropped-frame.jpg \
  cargo test -p gamebuddy-services live_capture_to_map_and_mcp_images -- --ignored
```

A replay has `capture_origin` and `frames`; each frame includes `path`, `at_ms`, `session`, `epoch`, `expected_room` (null means no accepted place is expected), and optional `event`. Exact indexed reference-file hashes are rejected. This catches a basic leakage mistake, not every possible source-video transformation; independent capture provenance still matters. ROMs, model weights, source videos and private captured frames are not included in the repository.
