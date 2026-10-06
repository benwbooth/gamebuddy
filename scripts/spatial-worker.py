#!/usr/bin/env python3
"""Optional learned image features. Rust owns ingestion, graph, tracking and UI.

JSON-lines protocol; stdout is protocol only. DINO retrieves candidate views,
DISK/LightGlue supplies correspondences, and RANSAC verifies geometry. 3D uses
epipolar geometry; it never invents a metric pose from an uncalibrated video.
"""
import argparse
import contextlib
import hashlib
import json
import os
import sys
from functools import lru_cache
from pathlib import Path

parser = argparse.ArgumentParser()
parser.add_argument("--cache", type=Path, required=True)
parser.add_argument("--model", default="facebook/dinov2-small")
parser.add_argument("--device", default="auto")
parser.add_argument("--recompute-features", action="store_true",
                    help="Benchmark extraction even when this image is already cached")
args = parser.parse_args()
args.cache.mkdir(parents=True, exist_ok=True)
os.environ.setdefault("HF_HOME", str(args.cache / "weights"))
if Path("/etc/ssl/certs/ca-certificates.crt").exists():
    os.environ.setdefault("SSL_CERT_FILE", "/etc/ssl/certs/ca-certificates.crt")


def startup(completed, message):
    # Keep protocol events on the original stdout even while libraries redirect
    # their own startup output to stderr. Stages report work, not estimated time.
    print(json.dumps({"startup": {"completed": completed, "total": 6, "message": message}}),
          file=sys.__stdout__, flush=True)


startup(0, "Loading the graphics runtime…")
with contextlib.redirect_stdout(sys.stderr):
    from spatial_geometry import verify
    import numpy as np
    import torch
    startup(1, "Loading visual tracking tools…")
    from PIL import Image
    from lightglue import DISK, LightGlue
    from lightglue.utils import rbd
    from transformers import AutoImageProcessor, AutoModel

    startup(2, "Preparing the graphics device…")
    torch.set_num_threads(4)
    torch.hub.set_dir(str(args.cache / "weights"))
    device = ("cuda" if torch.cuda.is_available() else "mps" if torch.backends.mps.is_available() else "cpu") if args.device == "auto" else args.device
    def cached_model(factory):
        # Reopening a prepared game must not wait for Hugging Face metadata
        # requests. First-time setup can still download missing model files.
        try:
            return factory.from_pretrained(args.model, cache_dir=str(args.cache / "weights"), local_files_only=True)
        except OSError:
            return factory.from_pretrained(args.model, cache_dir=str(args.cache / "weights"))

    processor = cached_model(AutoImageProcessor)
    startup(3, "Loading the screen recognition model…")
    encoder = cached_model(AutoModel).eval().to(device)
    startup(4, "Loading the map matching models…")
    extractor = DISK(max_num_keypoints=512).eval().to(device)
    matcher = LightGlue(features="disk", depth_confidence=-1, width_confidence=-1).eval().to(device)
    startup(5, "Checking model files…")
    digest = hashlib.sha256()
    for model in (encoder, extractor, matcher):
        for name, tensor in model.state_dict().items():
            digest.update(name.encode())
            digest.update(tensor.detach().cpu().contiguous().reshape(-1).view(torch.uint8).numpy().tobytes())
    identity = args.model + ":sha256:" + digest.hexdigest() + ":disk512-lightglue-v1"


@lru_cache(maxsize=128)
def features(key):
    with np.load(args.cache / (key + ".npz"), allow_pickle=False) as data:
        return {k: torch.from_numpy(data[k]).to(device) for k in ("keypoints", "descriptors", "image_size")}


@torch.inference_mode()
def extract(path):
    raw = Path(path).read_bytes()
    key = hashlib.sha256(identity.encode() + raw).hexdigest()
    dest = args.cache / (key + ".npz")
    if dest.exists() and not args.recompute_features:
        with np.load(dest, allow_pickle=False) as data:
            embedding = data["embedding"].tolist()
    else:
        image = Image.open(path).convert("RGB").resize((256, 192))
        inputs = {k: v.to(device) for k, v in processor(images=image, return_tensors="pt").items()}
        embedded = encoder(**inputs).last_hidden_state[:, 0]
        embedded = torch.nn.functional.normalize(embedded, dim=-1)
        embedding = embedded[0].cpu().tolist()
        tensor = torch.from_numpy(np.asarray(image).copy()).permute(2, 0, 1).float().to(device) / 255
        local = extractor.extract(tensor, resize=None)
        tmp = dest.with_suffix(f".{os.getpid()}.pending.npz")
        np.savez_compressed(tmp, embedding=embedding, **{k: local[k].cpu().numpy() for k in ("keypoints", "descriptors", "image_size")})
        tmp.replace(dest)
    return {"embedding": embedding, "learned_key": key, "model": identity, "device": device}


@lru_cache(maxsize=2048)
@torch.inference_mode()
def register(query, reference, dimension, atlas=False):
    # Keys include the model identity and exact image bytes. Repeated captures
    # still receive fresh timestamps and pass Rust's stability/freshness checks,
    # but need not rerun the same immutable landmark comparison on the GPU.
    a, b = features(query), features(reference)
    result = rbd(matcher({"image0": a, "image1": b}))
    pairs = result["matches"].cpu().numpy()
    if len(pairs) < 16:
        return None
    p = a["keypoints"][0].cpu().numpy()[pairs[:, 0]].astype(np.float32)
    q = b["keypoints"][0].cpu().numpy()[pairs[:, 1]].astype(np.float32)
    return verify(p, q, dimension, atlas=atlas)


for line in sys.stdin:
    try:
        request = json.loads(line)
        with contextlib.redirect_stdout(sys.stderr):
            if request["op"] == "info":
                answer = {"model": identity, "device": device}
            elif request["op"] == "extract":
                answer = extract(request["path"])
            elif request["op"] == "register":
                answer = register(request["query"], request["reference"], request["dimension"], request.get("atlas", False))
            else:
                raise ValueError("Unknown spatial operation")
        print(json.dumps({"result": answer}, allow_nan=False), flush=True)
    except Exception as error:
        print(json.dumps({"error": str(error)}), flush=True)
