# Illustrated walkthroughs

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
