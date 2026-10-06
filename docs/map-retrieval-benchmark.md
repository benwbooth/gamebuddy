# Comparing map candidate retrievers

`scripts/benchmark-map-retrieval.py` compares DINOv2 with local EmbeddingGemma 2
before changing live localization. Each method feeds candidate atlas tiles into
the same Rust crop, DISK/LightGlue registration, projection validation, and
ambiguity rejection used by `spatial_maps::locate`. It does not publish a live
location, change progress, or enable EmbeddingGemma in the application.

The methods are fixed before scoring labels:

| Method | Candidate selection |
| --- | --- |
| `dino` | Current DINO top 12 |
| `gemma` | EmbeddingGemma top 12 |
| `hybrid` | Alternate the two rankings, deduplicate, stop at 12 |
| `dino24` | DINO top 24, controlling for a larger search budget |
| `union24` | DINO top 12 plus Gemma top 12, deduplicated; at most 24 |

Gemma uses the pinned Google checkpoint
`914f7f89142e33e77833254d9c9b90c3cef7303b`, text and vision only, and normalized
768-dimensional image embeddings. It uses BF16 on the selected GPU or float32
on CPU. Float16 is unsuitable for this model's activation range. The float32
vectors are normalized again after BF16 inference. The harness uses eager
attention and checks single-image versus batched embeddings on four atlas
tiles before accepting or building the index (minimum cosine must be 0.999).
An initial run with the default SDPA backend on the tested ROCm runtime failed
this consistency check when examined afterward, so its scores were discarded.
Eager attention passed the same check; the attention choice is part of the
cache identity. This is a measured runtime issue, not a general quality result
for the model.

Map descriptors are cached
by model identity, map content hash, tile rectangle and preprocessing version.
The current experiment uses image-to-image retrieval; labels, walkthrough text,
room names and expected locations never enter the query or ranking.

## Run

Build the Rust probe:

```sh
nix develop -c cargo build -p gamebuddy-services --example map_retrieval_probe
```

Use a **separate** Python environment with PyTorch/torchvision appropriate to the
device, Pillow, `sentence-transformers==6.1.0` and `transformers==5.19.0`. The
existing spatial worker keeps its own environment. CPU is the default;
`--device cuda:0` also selects a ROCm device when using ROCm PyTorch. Select a
device explicitly after inspecting `torch.cuda.get_device_properties`.

```sh
/path/to/benchmark/python scripts/benchmark-map-retrieval.py \
  --store /path/to/prepared/GameBuddy \
  --guide GUIDE_ID \
  --manifest /path/to/independent-map-labels.json \
  --output /path/to/new-run \
  --cache /path/to/reusable-embedding-cache \
  --device cuda:0 --batch-size 8
```

The output directory must be new. The tool reads the source SQLite database in
read-only mode and backs it up to a private store. Cached map images are linked
for reading. The configured DINO worker may populate its immutable feature
cache, but the source database, capture session and progress are not written.
Model downloads happen during initial setup; subsequent inference is local.
Use `--index-only` without a manifest to build/resume the Gemma map cache first.

Supply already-cropped gameplay viewports, including the HUD. The shared
production calibration excludes the HUD during feature extraction. A manifest
looks like:

```json
{
  "capture_origin": "Independent emulator captures, not guide-video frames",
  "label_provenance": "Atlas regions inspected and labels frozen before predictions",
  "frames": [
    {
      "path": "gameplay.png",
      "expected": {
        "map_id": "map-id-from-index",
        "center_bounds": [0.1, 0.2, 0.3, 0.7]
      }
    },
    {"path": "title-screen.png", "expected": null}
  ]
}
```

Paths can be absolute or relative to the manifest. `center_bounds` are
normalized `[left, top, right, bottom]` bounds for the **viewport center**.
Omitting bounds scores only map identity. Negative examples explicitly use
`null`. The harness rejects exact copies of indexed reference/map files, but
this cannot detect all transformed source-video leakage. Independent provenance
and frozen labels remain necessary. The manifest and its hash are retained.

## Interpret the report

`shortlist_hit` says a candidate belongs to the expected map and overlaps the
labelled center region. That does not prove it can be registered.
`verified_candidate_hit` requires at least one individually verified candidate
to place the viewport in the expected region. Final `outcome` applies the
production ambiguity checks across the whole shortlist. It distinguishes a
correct location, wrong accepted location, missed location, and correct
abstention on a negative frame.

The private worker uses `--recompute-features` so both encoders actually process
each query, even if the captured frame already has cached features. This flag
does not change the normal live worker. Reference descriptors remain cached.
Query latency includes normalization/extraction/ranking on the DINO side and
image inference/ranking on the Gemma side. Startup and corpus indexing are
reported separately. The DINO query also extracts the DISK landmarks that all
methods need, so these are pipeline components, not isolated encoder timings.
Every distinct candidate registration is timed once per
frame, then reused across methods. These sums are diagnostic estimates, not
independent end-to-end timing: cross-frame worker-cache hits can remain, and
capture, temporal confirmation, UI and speech are excluded. Gemma still needs
the existing local-feature extractor for geometric verification.

The report includes both all-frame counts and `unique_images`, which scores
only the first occurrence of each image-file hash. This prevents repeated
identical frames from inflating the apparent amount of accuracy evidence.
Conflicting labels for identical files are rejected. Pixel-identical images
saved with different encodings require a separate pixel-level check.

Frames from one short gameplay sequence are correlated. A successful result
does not establish full-game accuracy, exact player coordinates, or a speed
improvement in the running UI. Retain the current live retriever unless an
independent comparison justifies changing it.

## Recorded comparison — 2026-10-06

**Keep the current DINO retriever.** EmbeddingGemma did not improve this replay,
either at the same candidate budget or when added alongside all twelve DINO
candidates. The machine-readable [evidence report](evidence/map-retrieval-2026-10-06.json)
retains model/runtime identities, input hashes, frozen labels, candidate IDs,
predictions, timings, and both all-frame and deduplicated summaries. Only the
new duplicate-aware summary was computed after the inference run; predictions
and labels were not changed.

The corpus contained 13,292 tiles from all 29 enabled Faxanadu maps. Queries were
55 frames from an independent FCEUmm replay and three separately saved CRT views. Exact-file
and RGB-pixel checks both found only **24 distinct positive images and two
distinct negative images** among those 58 frames. The positives cover three
maps (Eolis, Forepaw, and Tower of Trunk) and six coarse viewport-center regions.
Even the distinct images are correlated; this is a narrow convenience sample.

| Method | Correct distinct gameplay images / 24 | Misses / 24 | Correct across all 54 positive frames |
| --- | ---: | ---: | ---: |
| DINO top 12 (current) | 23 | 1 | 53 |
| Gemma top 12 | 4 | 20 | 4 |
| Mixed top 12 | 20 | 4 | 34 |
| DINO top 24 | 24 | 0 | 54 |
| DINO 12 + Gemma 12, deduplicated | 23 | 1 | 53 |

Every method abstained on both distinct negatives (four frames), and none
accepted a wrong location in this set. Gemma's shortlist overlapped the labelled
region in 16 of 24 distinct gameplay images, but only four produced a verified
location. Coarse retrieval success therefore did not translate into useful map
placement. The mixed top 12 displaced useful DINO candidates; retaining all
twelve DINO candidates avoided that loss but recovered no additional locations.
The one baseline miss, `frame-006360.png`, was recovered by DINO top 24 alone.

The valid run used BF16 eager attention on the local AMD GPU with
PyTorch `2.13.0+rocm7.1`, Transformers `5.19.0`, and Sentence Transformers `6.1.0`.
Minimum single-image/batch consistency cosine was **0.9999776**. Results from the
earlier inconsistent SDPA run are excluded.

| Query component, across all 58 frames | Median | 95th percentile |
| --- | ---: | ---: |
| Existing DINO query preparation, DISK extraction, and ranking | 37.5 ms | 77.0 ms |
| Additional Gemma inference and ranking | 134.6 ms | 153.1 ms |

The first DINO query took 3.44 seconds; the maximum Gemma query took 189.6 ms.
These components exclude registration, capture, temporal confirmation, UI and
speech. Among first occurrences of the 26 distinct images, median diagnostic
registration sums were 169 ms for DINO top 12, 299 ms for DINO top 24, and 304 ms
for the combined union. They are not independent end-to-end measurements.

The resumed index build computed 11,452 tiles in 22.46 minutes with batch size
eight; 1,840 valid eager-attention tiles were already cached. That timing is not
a complete cold-index build. Loading the cached model took 19.73 seconds.
Batch size sixteen encountered GPU memory pressure with the other local models
resident, so the completed run used eight.

Searching more DINO candidates is the more promising follow-up from this
sample, but one recovered image does not establish a safe general tuning
change. Live DINO top 12, geometry thresholds, and temporal tracking remain
unchanged. This experiment does not evaluate text-assisted retrieval, other
games, or live end-to-end performance.
