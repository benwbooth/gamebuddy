#!/usr/bin/env python3
"""Compare local map retrievers using the production Rust geometry verifier.

Inputs and labels remain local. A SQLite backup isolates all application writes.
Ground truth must be independently assigned before examining these predictions.
"""
import argparse
import hashlib
from importlib.metadata import version
import json
from pathlib import Path
import sqlite3
import subprocess
import time

from map_retrieval import EmbeddingRetriever, hybrid_candidates

METHODS = ("dino", "gemma", "hybrid", "dino24", "union24")


def validate_manifest(manifest):
    if not manifest.get("capture_origin") or not manifest.get("label_provenance"):
        raise ValueError("Document independent capture and label provenance")
    if not manifest.get("frames") or any("expected" not in f for f in manifest["frames"]):
        raise ValueError("Every frame needs an explicit expected label (null for negatives)")
    for frame in manifest["frames"]:
        expected = frame["expected"]
        if expected is not None:
            if not expected.get("map_id"):
                raise ValueError("Positive labels need a map ID")
            bounds = expected.get("center_bounds")
            if bounds is not None and (len(bounds) != 4 or not (
                0 <= bounds[0] < bounds[2] <= 1 and 0 <= bounds[1] < bounds[3] <= 1
            )):
                raise ValueError("Invalid normalized center bounds")


def score_prediction(prediction, expected):
    if prediction is None:
        return "correct_abstention" if expected is None else "miss"
    if expected is None or prediction["map_id"] != expected["map_id"]:
        return "wrong"
    bounds = expected.get("center_bounds")
    if bounds is not None:
        polygon = prediction["polygon"]
        x = sum(p[0] for p in polygon) / len(polygon)
        y = sum(p[1] for p in polygon) / len(polygon)
        if not (bounds[0] <= x <= bounds[2] and bounds[1] <= y <= bounds[3]):
            return "wrong"
    return "correct"


def shortlist_hit(ids, tiles, expected):
    if expected is None:
        return None
    for identifier in ids:
        tile = tiles[identifier]
        if tile["map_id"] != expected["map_id"]:
            continue
        bounds = expected.get("center_bounds")
        if bounds is None:
            return True
        x, y, w, h = tile["rect"]
        mw, mh = tile["map_size"]
        if x/mw <= bounds[2] and (x+w)/mw >= bounds[0] and y/mh <= bounds[3] and (y+h)/mh >= bounds[1]:
            return True
    return False


def summarize(frames):
    result = {}
    for method in METHODS:
        rows = [f["methods"][method] for f in frames]
        result[method] = {
            "counts": {outcome: sum(r["outcome"] == outcome for r in rows)
                       for outcome in ("correct", "wrong", "miss", "correct_abstention")},
            "shortlist_hits": sum(r["shortlist_hit"] is True for r in rows),
            "verified_candidate_hits": sum(r["verified_candidate_hit"] is True for r in rows),
        }
    return result


def unique_image_summary(frames):
    unique = {}
    for frame in frames:
        previous = unique.setdefault(frame["sha256"], frame)
        if previous["expected"] != frame["expected"]:
            raise ValueError("Identical image files have conflicting location labels")
    return {"images": len(unique), "selection": "First occurrence of each image-file SHA256",
            "summary": summarize(list(unique.values()))}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--store", type=Path, required=True)
    parser.add_argument("--guide", required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--cache", type=Path, required=True)
    parser.add_argument("--manifest", type=Path)
    parser.add_argument("--index-only", action="store_true")
    parser.add_argument("--probe", type=Path, default=Path(__file__).resolve().parents[1] / "target/debug/examples/map_retrieval_probe")
    parser.add_argument("--device", default="cpu")
    parser.add_argument("--batch-size", type=int, default=8)
    args = parser.parse_args()
    if not args.index_only and not args.manifest:
        parser.error("Provide an independently labelled --manifest or --index-only")
    manifest_bytes = args.manifest.read_bytes() if args.manifest else None
    manifest = json.loads(manifest_bytes) if manifest_bytes is not None else None
    if args.batch_size < 1:
        parser.error("--batch-size must be positive")
    if manifest:
        validate_manifest(manifest)
    args.output.mkdir(parents=True, exist_ok=False)
    if manifest:
        (args.output / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    private = args.output / "store"
    private.mkdir()
    with sqlite3.connect(f"file:{args.store.resolve()}/gamebuddy.sqlite?mode=ro", uri=True) as source:
        with sqlite3.connect(private / "gamebuddy.sqlite") as target:
            source.backup(target)
    with sqlite3.connect(private / "gamebuddy.sqlite") as target:
        row = target.execute("SELECT body FROM documents WHERE key='spatial-worker'").fetchone()
        if row is None:
            raise ValueError("Configure the learned spatial worker first")
        worker = json.loads(row[0])
        if not any(Path(arg).name == "spatial-worker.py" for arg in worker["arguments"]):
            raise ValueError("Fair extraction timing requires the bundled spatial-worker.py")
        if "--recompute-features" not in worker["arguments"]:
            worker["arguments"].append("--recompute-features")
        target.execute("UPDATE documents SET body=? WHERE key='spatial-worker'", (json.dumps(worker),))
    (private / "maps").symlink_to((args.store / "maps").resolve(), target_is_directory=True)
    worker_log = (args.output / "worker.log").open("w")
    process = subprocess.Popen([str(args.probe.resolve()), str(private), args.guide],
                               stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                               stderr=worker_log, text=True, bufsize=1)

    def receive():
        line = process.stdout.readline()
        if not line:
            raise RuntimeError(f"Map probe exited; inspect {worker_log.name}")
        value = json.loads(line)
        if "error" in value:
            raise RuntimeError(value["error"])
        return value

    def call(request):
        process.stdin.write(json.dumps(request) + "\n")
        process.stdin.flush()
        return receive()

    try:
        ready = receive()
        export = call({"op": "index"})["result"]
        (args.output / "index.json").write_text(json.dumps(export))
        if manifest:
            known_maps = {m["id"] for m in export["maps"]}
            reference_hashes = {hashlib.sha256(Path(path).read_bytes()).hexdigest()
                                for path in export.get("reference_paths", []) if Path(path).is_file()}
            reference_hashes.update(hashlib.sha256(Path(m["path"]).read_bytes()).hexdigest()
                                    for m in export["maps"])
            labels_by_hash = {}
            for frame in manifest["frames"]:
                path = Path(frame["path"])
                if not path.is_absolute():
                    path = args.manifest.parent / path
                digest = hashlib.sha256(path.read_bytes()).hexdigest()
                if digest in reference_hashes:
                    raise ValueError("Scored query duplicates an indexed reference or map")
                if digest in labels_by_hash and labels_by_hash[digest] != frame["expected"]:
                    raise ValueError("Identical image files have conflicting location labels")
                labels_by_hash[digest] = frame["expected"]
                if frame["expected"] is not None and frame["expected"]["map_id"] not in known_maps:
                    raise ValueError("Expected map is not enabled in the corpus")
        started = time.monotonic()
        retriever = EmbeddingRetriever(args.cache, args.device)
        model_load_ms = (time.monotonic() - started) * 1000
        started = time.monotonic()
        matrix = retriever.index(export, args.batch_size)
        index_ms = (time.monotonic() - started) * 1000
        print(f"Ready: {len(matrix)} tiles; model load {model_load_ms:.0f} ms; index {index_ms:.0f} ms", flush=True)
        if args.index_only:
            return
        import numpy as np
        from PIL import Image
        tiles = {t["id"]: t for t in export["tiles"]}
        ids = np.array(list(tiles))
        report = {"capture_origin": manifest["capture_origin"],
                  "label_provenance": manifest["label_provenance"],
                  "manifest_sha256": hashlib.sha256(manifest_bytes).hexdigest(),
                  "model": retriever.identity, "device": args.device,
                  "batch_consistency_min_cosine": retriever.consistency,
                  "torch": retriever.torch.__version__,
                  "transformers": version("transformers"),
                  "sentence_transformers": version("sentence-transformers"),
                  "hardware": (retriever.torch.cuda.get_device_name(args.device)
                               if args.device.startswith("cuda") else "CPU"),
                  "dino_encoder": export["encoder"], "tiles": len(tiles),
                  "maps": len(export["maps"]),
                  "candidate_budgets": {name: (24 if name.endswith("24") else 12) for name in METHODS},
                  "fusion": "equal-budget rank interleaving; DINO first",
                  "query_feature_cache": "disabled for both encoders; DINO reference descriptors remain cached",
                  "startup_ms": {"dino": ready["startup_ms"], "gemma": model_load_ms},
                  "index_ms": index_ms, "index_work": retriever.index_stats, "frames": []}
        for number, frame in enumerate(manifest["frames"]):
            path = Path(frame["path"])
            if not path.is_absolute():
                path = args.manifest.parent / path
            query = call({"op": "query", "path": str(path.resolve()),
                          "background": str((args.output / "background.png").resolve())})
            dino = [entry[0] for entry in query["result"]["ranking"]]
            with Image.open(args.output / "background.png") as image:
                background = image.convert("RGB")
            started = time.monotonic()
            vector = retriever.encode([background])[0]
            scores = matrix @ vector
            gemma = ids[np.argsort(-scores, kind="stable")].tolist()
            gemma_ms = (time.monotonic() - started) * 1000
            candidates = {"dino": dino[:12], "gemma": gemma[:12],
                          "hybrid": hybrid_candidates(dino, gemma, 12),
                          "dino24": dino[:24],
                          "union24": list(dict.fromkeys(dino[:12] + gemma[:12]))}
            # All methods use exactly the same deterministic registrations. Time
            # the deduplicated union once: immutable worker-cache hits must not
            # make a method run later appear artificially faster.
            timings, singles = {}, {}
            for identifier in dict.fromkeys(i for values in candidates.values() for i in values):
                result = call({"op": "verify", "ids": [identifier]})
                timings[identifier] = result["elapsed_ms"]
                singles[identifier] = result["result"]
            item = {"image": path.name, "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
                    "expected": frame["expected"], "dino_query_ms": query["elapsed_ms"],
                    "gemma_query_ms": gemma_ms, "methods": {}}
            for method, chosen in candidates.items():
                result = call({"op": "verify", "ids": chosen})
                prediction = result["result"]
                item["methods"][method] = {
                    "candidate_ids": chosen,
                    "shortlist_hit": shortlist_hit(chosen, tiles, frame["expected"]),
                    "verified_candidate_hit": (any(score_prediction(singles[i], frame["expected"]) == "correct" for i in chosen)
                                               if frame["expected"] is not None else None),
                    "prediction": prediction, "outcome": score_prediction(prediction, frame["expected"]),
                    "registration_ms_sum": sum(timings[i] for i in chosen),
                }
            report["frames"].append(item)
            (args.output / "report.json").write_text(json.dumps(report, indent=2) + "\n")
            print(f"Frame {number+1}/{len(manifest['frames'])}: " + ", ".join(
                f"{name}={value['outcome']}" for name, value in item["methods"].items()), flush=True)
        report["summary"] = summarize(report["frames"])
        report["unique_images"] = unique_image_summary(report["frames"])
        report["latency_note"] = "Query times exclude capture, UI and startup. Registration sums reuse one measured call per unique candidate; not end-to-end timing. Cross-frame cache hits remain possible."
        report["scope"] = "Single-frame viewport-map matching. Coarse manually labelled center bounds, not subpixel accuracy or player coordinates. No live temporal tracker or UI; sequence frames are correlated."
        (args.output / "report.json").write_text(json.dumps(report, indent=2) + "\n")
        print(json.dumps(report["summary"], indent=2), flush=True)
    finally:
        process.stdin.close()
        try:
            process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            process.terminate()
            process.wait(timeout=10)
        worker_log.close()


if __name__ == "__main__":
    main()
