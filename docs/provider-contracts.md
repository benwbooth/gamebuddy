# Model boundaries and video synchronization

Design contracts, September 27, 2026. The Rust prototype implements provider adapters, ingestion, revisioned plans, MCP, a visual retrieval baseline and composed speech. The named interfaces below are architectural responsibilities, not a claim that every component or learned model is implemented. See the [current architecture](architecture-and-research.md) and README for exact status.

The app accepts a YouTube URL. A media adapter handles source access and decoding; models receive supported media inputs. Local models generally consume decoded frames or clips through their processor/runtime. [Gemini also documents direct public YouTube URL input](https://ai.google.dev/gemini-api/docs/video-understanding), which can be an optional ingestion provider. Neither route makes a public transcript or the underlying video bytes universally available.

| Interface | Input | Output | Responsibility |
| --- | --- | --- | --- |
| `VideoSourceAdapter` | YouTube URL or supplied media, source-access configuration | Source metadata, available audio/transcript/video tracks, explicit access failures | Resolve media separately from model inference; retain source timestamps |
| `GuideCompiler` | Segmented text, transcripts and supported video/frames | Versioned chapter/step/substep graph, source anchors, visual reference associations, coverage report | Compile an inspectable guide; permit cloud or local multimodal providers |
| `VisualReferenceEncoder` | Timestamped guide frames/clips and associated task text | Versioned embeddings and visual evidence index | Run once during import and cache results; use the same compatible encoder for live queries |
| `VisualSynchronizer` | Live frame/crop sequence, reference index, guide graph and previous estimates | Candidate locations/steps, temporal alignment, evidence IDs and ambiguity status | Run locally and remain independent of the conversational LLM |
| `ProgressStore` | Observations, sync estimates, completion evidence and user corrections | Durable progress and event history | Distinguish likely location from verified completion; handle reloads and revisions |
| `ReasoningProvider` | Current progress, retrieved guide material, player question, optional recent images | Proposed instructions, tool requests and typed widget content | Swappable cloud or local reasoning; the application validates proposed state changes |
| `VoiceProvider` | Microphone audio and coach context | Speech, transcript and interruption events | Swappable realtime or composed speech pipeline |

Generic OpenAI-compatible endpoints are useful for some local/cloud providers, but do not establish identical video, tool-calling, structured-output or streaming behavior. Each adapter should advertise capabilities and normalize its outputs to these contracts.

The local synchronizer needs the walkthrough's visual evidence. For every guide segment, retain source ID, start/end time, representative frame IDs, embeddings, OCR, associated graph node IDs, and relevant HUD/minimap crops. Preserve sequence order and multiple views of a landmark. Treat menus, cutscenes and repeated footage as explicit segment types. Keep graph and encoder versions with the index so a guide edit or model change cannot silently invalidate matches.

Import long videos hierarchically. Make a coarse chapter pass, process overlapping clips for individual actions, reconcile the resulting graph, and audit source coverage. Reference encoding can stream in bounded batches; the entire video does not need to remain in GPU context. This is normally indexing and inference over pretrained models, without fine-tuning a model for every game.

The first local implementation should combine three existing capabilities:

1. **Retrieve plausible reference segments.** Use the same visual encoder on guide frames and live observations. [Qwen3-VL-Embedding-2B](https://huggingface.co/Qwen/Qwen3-VL-Embedding-2B) is a candidate because its documented inputs include text, screenshots and video. Benchmark it against a cheaper image-matching baseline.
2. **Align over time and graph structure.** Score candidate sequences using appearance, OCR, previous matches, transitions and prerequisites. Permit skips, backtracking, optional branches and global re-localization. The player's clock does not progress at the walkthrough author's speed.
3. **Resolve ambiguous evidence.** Ask a compact local VLM such as [Qwen3.5-4B](https://huggingface.co/Qwen/Qwen3.5-4B) to compare a bounded set of live/reference crops or clips with candidate task descriptions. Require observable evidence for completion, and expose uncertainty when the distinguishing event was not visible.

Embeddings produce retrieval scores, not calibrated probabilities or proof that a task was completed. A character standing next to a chest may correspond to approaching it, opening it, or revisiting it after it was looted. The tracker must use temporal evidence, visible item acquisition, UI text, or explicit player confirmation to distinguish these states.

A useful synchronization result contains:

```json
{
  "guide_revision": "example-guide-r3",
  "session_epoch": 2,
  "observed_at_ms": 172000,
  "status": "ambiguous",
  "candidates": [
    {
      "step_id": "chapter-2.find-tower-key",
      "source_id": "walkthrough-video-1",
      "reference_interval_ms": [420000, 437000],
      "evidence_ids": ["live-frame-88", "reference-frame-114"],
      "support": ["tower-door-landmark", "quest-label-match"],
      "unresolved": ["key-possession"]
    }
  ],
  "completion_evidence": []
}
```

This is illustrative data, not an observation from an actual game. Source video times and live session times have different meanings and must remain separate. Results carry their observation time so older inference cannot overwrite newer state. Multiple guide sources may support the same task, and a single clip may support several substeps.

Store source-access status explicitly. A source can be `outline_ready`, `visual_index_ready`, or `needs_media`, with a coverage report for each stage. If the app has a text outline but cannot obtain visual references, it can still answer questions and guide manually, while clearly reporting that reference-based synchronization is unavailable. Never silently replace missing visual references with a claim that the full video was understood.

The decisive first experiment is to import a walkthrough, build its visual index, and locate a different player's recorded session within the graph while the reasoning LLM is disconnected. Then add the conversational provider and verify it can explain the current step, show supporting evidence, and follow corrections without changing the synchronizer. This demonstrates that the intended separation works before adding more widgets.
