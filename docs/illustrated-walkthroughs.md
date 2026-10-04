# Illustrated walkthroughs

## In-game guide library

Expand the GameBuddy companion and choose **Browse guide**, or enable **Guide library** in **Widgets…**. This reader uses the current saved outline and updates during preparation and AI plan edits. It is independent of the exported HTML book.

- **Outline:** searchable, expandable chapters/tasks; inspect instructions, completion checks, route links, prerequisites, source timestamps, related maps and paginated screenshots. **Current step** returns to the live cursor. Browsing does not change the player's location or task cursor.
- **Bookmarks:** save useful steps across launches. Completion checkboxes update actual player progress and can be unchecked. Checking a chapter covers its descendants; unchecking a child preserves completed siblings. Old in-flight screen observations cannot overwrite a checkbox edit.
- **My goals:** **Make this a goal** adds the selected task to the same revisioned plan the AI uses. Voice-created goals appear here too; checkboxes track their linked tasks. Goals without linked tasks are labelled rather than assigned invented completion criteria.
- **All pictures / All maps:** inspect all indexed source images (eight per page, with a larger picture view and video timestamps) and attached map images with credits, source links, associated places and outline bindings. Images load on demand, with a bounded in-memory cache. The map picker and picture viewer stay open through live updates.
- **Sources / How it was built:** inspect reference preferences, channel metadata where retained, source coverage and gaps, the compilation method, and actual revision summaries. Older revisions do not record the exact model/prompt; the reader explicitly says so.

Every widget has **−** to minimize to its header, **+** to restore, and **×** to dismiss into the companion tray. The companion's **Hide widgets / Show widgets** controls the full group. Layouts, minimized state, individual visibility, bookmarks, goals and progress persist. The library starts hidden so first launch does not cover gameplay with the full reader.

The Rust service resolves image IDs only against the current guide index. Reader mutations check guide revision; completion changes also check progress epoch. Pictures and source links retain provenance without granting the HTML page arbitrary local file access.

Run the isolated real Qt/SQLite integration test with:

```sh
nix develop --command scripts/test-guide-library
```

It requires Xvfb and Node 22 or newer. It creates generated test material under `/tmp`, checks the real Rust/Qt reader bridge, captures screenshots, and leaves live game data untouched. `scripts/test-gamescope` separately verifies compositor input routing and widget dragging/dismissal.

## Exported illustrated guide

The Rust compiler reviews source intervals with twelve ordered frames and the full timed caption/transcription excerpt. It asks the configured reasoning provider for tasks, nested executable substeps, visible completion checks and explicit dependencies. It validates frame spans, IDs, hierarchy and dependency ordering before saving a revision. Automatic first-load preparation uses this compiler and exports an illustrated guide when a visual index is available.

An existing coarse guide can be preserved while producing a separate detailed edition:

```sh
nix develop --command cargo build --workspace
nix develop --command ./target/debug/gamebuddy-cli walkthrough GUIDE_ID artifacts/faxanadu --jobs 2
# Resume the edition ID printed by the first command:
./target/debug/gamebuddy-cli walkthrough EDITION_ID artifacts/faxanadu --resume --jobs 2
./target/debug/gamebuddy-cli export-book EDITION_ID artifacts/faxanadu
```

Requests run in bounded batches; revision writes are serialized. Each completed section is persisted and the reader is refreshed on disk after every batch. Failed sections remain pending for a later resume. The original guide, goals and progress are preserved in its own edition.

Open `artifacts/faxanadu/index.html` to inspect the reader, or use **Open illustrated guide** on the app's guide page. The export includes chapter navigation, search, expandable sections, task/substep instructions, screenshot galleries, a full filmstrip, video timestamp links, a task ordering graph, and JSON exports. All code and images are local; no third-party JavaScript is needed. Screenshot previews preserve source evidence. Route edges describe the demonstrated ordering; prerequisite edges are separate, model-inferred constraints.

Reviewing all source intervals is a coverage milestone, not a verified independent completion run. A single video's route may omit optional content; sparse images and transcription can miss details. Section-specific evidence gaps remain visible in the reader. The source video and screenshots are retained so those gaps can be investigated without accepting invented instructions.
