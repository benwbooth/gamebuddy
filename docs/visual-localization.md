# Visual localization for 2D and 3D games

GameBuddy maintains two separate structures: source-backed objectives and visual places. A reference timestamp identifies evidence, not the player's progress. Recognizing a room never completes an objective or proves inventory.

## First launch and reference preparation

GameBuddy's preparation workflow discovers or uses a pinned walkthrough, downloads it with captions when available, compiles bounded video sections into the editable objective graph, and prepares visual references in a background worker. This runs in the application; developer-created game guides are not required. A failed section receives one immediate retry and remains visibly incomplete if both attempts fail. Preparation resumes from saved sections and video chunks.

The reasoning provider calibrates each source's gameplay viewport, a HUD/background crop, and 2D versus 3D camera behavior. Source calibration is separate from the player's live crop. The index decodes at 2 FPS, retaining scene changes, scrolling views and periodic stationary views. It stores source IDs, timestamps, color screenshots, feature identities, place clusters and observed scene changes. A video cut is **not** a verified door or traversable edge.

The player or agent can pin the exact video with `pin_walkthrough`; `prepare_world` resumes preparation. `get_spatial_context` returns structured evidence and actual matched reference/map images. Existing tools still control goals, references, outline edits and explicit progress reports.

## Local perception

The native Rust baseline uses corner/BRIEF features, spatial edge retrieval, affine RANSAC and distributed-inlier checks. It is useful for clean 2D captures but failed on the saved CRT screenshot used in development.

The optional local worker uses **DINOv2-small** for candidate retrieval and **DISK + LightGlue** for correspondences. The model identity hashes the actual weights; changing weights invalidates incompatible indexes. The worker caches features, persists across frames, has bounded calls, and runs independently of screen capture and voice. Rust owns ingestion, tracking, graph state, persistence and widgets. Python is limited to the learned inference and geometry bridge.

- **2D:** affine registration, conservative translation-based local mosaics, verified revisit merging, and matching against tiles of actual reference-map pixels. Revisit merging requires dense reference-to-reference geometry and compatible colors. The map highlight is the camera footprint, not a player dot. Off-map transforms are rejected before clipping; two distinct captures must agree on the map and nearby footprint. Large jumps, missing matches and progress/session resets revoke the highlight until reconfirmed. Prepared geometry takes precedence over legacy RGB and semantic marker estimates.
- **3D:** image retrieval followed by epipolar RANSAC. The output is a visual place/reference viewpoint with provenance. It does not assume a flat scrolling screen, project a 2D map marker, or invent metric position, orientation, camera intrinsics or a reconstructed 3D mesh.
- **Temporal tracking:** repeated distinct capture timestamps, competing-place margins and an explicit uncertain state. Session changes, save/progress epochs, crop changes and long gaps invalidate tracking. A crop/reference/profile change or stale guide revision prevents old observations from appearing current.
- **Objective selection:** competing return-visit timestamps must agree before visual evidence moves the cursor. Alternative related tasks remain available to the reasoning provider. Observed events and player reports remain separate evidence.

Qwen remains a separate semantic observer. It can interpret dialogue, HUD text and scene meaning; it is not the geometric coordinate engine. These components implement visual place recognition, not a learned dynamics/world simulator.

## Setup

```sh
nix develop
cargo build --workspace
scripts/setup-spatial-worker
```

The default optional environment uses CPU PyTorch. For an AMD GPU supported by ROCm, install a separate environment using the appropriate official wheel index, for example the index tested on this workstation:

```sh
GAMEBUDDY_VISION_DIR="$HOME/.local/share/GameBuddy/vision-rocm" \
GAMEBUDDY_TORCH_INDEX=https://download.pytorch.org/whl/rocm7.1 \
  scripts/setup-spatial-worker
```

This configures `spatial-worker` in GameBuddy's store. CUDA/ROCm and Apple MPS are selected when available, otherwise CPU. CUDA device naming also covers ROCm in PyTorch. First use downloads the model weights. A custom JSON worker configuration can pass `--device` or `--model`; compatible Transformers encoders are pluggable, but DINOv3 has not been validated here. GPU inference was exercised on this Linux AMD workstation; Windows/macOS inference and overlay operation still require native testing.

Useful diagnostics (replace `GUIDE` with the loaded guide ID):

```sh
gamebuddy-cli pin-reference Faxanadu https://www.youtube.com/watch?v=8vEyiAs-fbY --platform "Nintendo Entertainment System"
gamebuddy-cli prepare Faxanadu --platform "Nintendo Entertainment System"
gamebuddy-cli prepare-world GUIDE
gamebuddy-cli enhance-world GUIDE
gamebuddy-cli prepare-map-pixels GUIDE
gamebuddy-cli locate GUIDE /path/to/cropped-game.png
gamebuddy-cli locate-map GUIDE /path/to/cropped-game.png
gamebuddy-cli tool get_spatial_context
```

`locate` inspects one frame and does not alter live progress. A single frame normally produces a candidate rather than confirmed tracking. `spatial-profile GUIDE profile.json` overrides calibration. Preparation and map caching are incremental; errors are shown in the map widget and spatial tool response.

## Optional emulator evidence

The read-only RetroArch adapter checks a specific Faxanadu USA PRG payload SHA-256, verifies the process/content and loopback UDP endpoint, and strictly validates complete RAM replies. On Linux it checks socket ownership against the specified process. It exposes raw area/screen IDs, inventory, death, transition and quest-bit changes with disassembly provenance. Raw sprite coordinates are not atlas coordinates; menus can retain stale RAM. It never writes memory or completes objectives.

```sh
gamebuddy-cli read-retroarch /path/to/Faxanadu.nes --endpoint 127.0.0.1:55355
gamebuddy-cli attach-retroarch /path/to/Faxanadu.nes GAME_PID --endpoint 127.0.0.1:55355
```

RetroArch must already have its network command interface enabled. This adapter is optional and is **not automatically enabled by Lunchpail**. Unknown ROM revisions fail closed. Linux process binding is implemented; other platforms reject attachment until a platform-specific binding exists. The generic vision path has no emulator dependency.

The isolated `libretro_probe` example runs real gameplay and save/restore without touching player saves. The current Faxanadu RAM profile was checked against a matching disassembly payload and exercised through the FCEUmm core. Live UDP attachment remains unverified: separate RetroArch test launches crashed in this environment before a usable command session. Parser tests are not claimed as proof of that transport.

## Evidence and remaining limits

See [localization validation](localization-validation.md) for the measured checks. Full-source sampling and a large graph do not prove every instruction correct. A few independent frames do not establish general game accuracy. Repeated textures, identical rooms, moving cameras, occlusion, edits, altered palettes, HUDs and unseen views can still produce ambiguity. Image similarity is not a probability.

The 3D camera-geometry tests include nonplanar points, camera rotation/translation and outliers. An actual 3D game playthrough remains an acceptance test; no metric SLAM, navigation mesh, collision map or universal game understanding is claimed.

Primary implementation references: [DINOv2](https://github.com/facebookresearch/dinov2), [LightGlue/DISK support](https://github.com/cvg/LightGlue), [RetroArch command implementation](https://github.com/libretro/RetroArch/blob/master/command.c), and the [pinned Faxanadu disassembly](https://github.com/chipx86/faxanadu/tree/eb0b5277be1c43a3bfdeda2a73acbc4c33580f47).

## Live map delivery and repeated-frame latency

Prepared worlds and map descriptors use a bounded shared cache. SQLite document-version triggers invalidate only the affected reference, including writes from another connection or process; ordinary capture updates do not reparse the large indexes. Launched games request a capture each second. The matcher still consumes the latest queued frame, and cold model startup remains separate from warmed matching time.

Map registration runs even when video-place retrieval has no candidate. Live spatial results must match the current capture session, crop epoch, progress epoch and coarse scene signature, and may trail the newest capture by at most three seconds. A changed room hides the old visual location while the next result is pending. Explicit player reports retain their separate provenance. The scene signature is a freshness veto, not a second localization method or proof of exact player position.

`get_location_map` returns actual atlas pixels through the native image path, including for a selected map without tracking. A fresh geometric match produces a neighborhood crop with a red box around the visible camera area. `get_game_context` includes that image beside the current screenshot when a fresh marker exists, avoiding another tool round trip. Both the subscription backend and local coach retain image-tool outputs. The camera box is not the player's exact coordinate.
