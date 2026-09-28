# Spatial understanding from walkthroughs

Use two linked graphs: a map of **places and transitions**, and the **objective graph** represented by the guide. Revisiting the same room must not reset completed tasks. Video time is evidence provenance, not the player's progress.

During ingestion, decode the whole video incrementally, retain distinct views and denser samples around transitions, isolate the game viewport, and process background landmarks separately from HUD text and temporary sprites. Retrieve likely matching views, verify their geometric overlap, cluster revisits and record observed doors or transitions. Preserve uncertainty about edits and unseen routes. Link the resulting places to source-backed objectives with a reasoning model.

For Faxanadu, begin with a 2D background model: align scrolling views, build local mosaics and track doors and screen transitions. Multiple background landmarks and recent transitions should disambiguate repeated tile patterns. Full 3D reconstruction is unnecessary for this first game.

During play, retrieve several candidate places and update a temporal belief over them using verified landmarks, camera motion, recent transitions and separate HUD/OCR evidence. Allow staying still, backtracking, death/reload and an unknown state. Widen retrieval after a discontinuity. A short sequence can distinguish two rooms that are impossible to distinguish in a single frame. When it cannot, retain both hypotheses and avoid location-specific instructions.

Give Codex, Claude or another reasoning provider compact structured observations: likely places, confidence, visible events, eligible tasks, relevant source excerpts and user goals. Use the model for semantic clarification and guidance. Local perception should handle frequent frames. Location alone never establishes task completion.

Candidates to evaluate are [DINOv3](https://github.com/facebookresearch/dinov3) for global/dense descriptors and [LightGlue](https://github.com/cvg/LightGlue) for local feature matching. DINOv3 has its own license; LightGlue is Apache-2.0 but feature extractors have separate licenses. [Hierarchical Localization](https://github.com/cvg/Hierarchical-Localization) illustrates retrieval followed by geometric verification; its 3D pipeline is not a drop-in game map builder. [OpenSeqSLAM2](https://github.com/qcr/openseqslam2) is a MATLAB research reference for sequence-based place recognition, not a proposed application dependency.

Keep orchestration, graphs, tracking and UI in Rust. Prototype or export model inference separately if necessary, then benchmark runtime support on the RX 7900 XTX and each supported platform. ONNX export availability alone does not prove AMD acceleration or performance.

Current implementation: timestamped frame indexing, low-resolution RGB fingerprints, repeated-match selection, and conservative local VLM comparison. Still unimplemented: a persistent place graph, learned retrieval, geometric verification, scrolling mosaics and a probabilistic sequence tracker. The current step-based retrieval can confuse repeat visits and cannot recover a place it never proposed.

Validate on an independently recorded Faxanadu playthrough with backtracking, death/reload, repeated rooms and CRT-on/off capture. Measure room accuracy, confident false matches, abstention, recovery delay and latency. Hold out entire sessions rather than neighboring frames of the reference video. Optional emulator state can supply evaluation labels without becoming a dependency of the general vision pipeline.
