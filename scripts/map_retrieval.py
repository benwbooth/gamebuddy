"""Local EmbeddingGemma candidate retrieval for offline map experiments."""
import hashlib
import json
import os
from pathlib import Path

MODEL = "google/embeddinggemma-2"
REVISION = "914f7f89142e33e77833254d9c9b90c3cef7303b"


def hybrid_candidates(dino, gemma, limit=12):
    """Equal-budget rank interleaving, without comparing unrelated cosine scales.

    Keep the baseline as the first tie-breaker. The budget is fixed before scoring
    any labelled frames; this is deliberately not fitted to the evaluation set.
    """
    if limit < 1:
        raise ValueError("Candidate budget must be positive")
    result, seen = [], set()
    for i in range(max(len(dino), len(gemma))):
        for ranking in (dino, gemma):
            if i < len(ranking) and ranking[i] not in seen:
                seen.add(ranking[i])
                result.append(ranking[i])
                if len(result) == limit:
                    return result
    return result


class EmbeddingRetriever:
    def __init__(self, cache, device="cpu"):
        import numpy as np
        import torch
        from sentence_transformers import SentenceTransformer

        self.np = np
        self.torch = torch
        self.cache = Path(cache)
        self.cache.mkdir(parents=True, exist_ok=True)
        torch.set_num_threads(4)
        dtype = torch.bfloat16 if device.startswith("cuda") else torch.float32
        self.identity = {"model": MODEL, "revision": REVISION, "dtype": str(dtype),
                         "modalities": "text+image", "dimensions": 768,
                         "attention": "eager",
                         "tile_resize": "pillow-bilinear-256x192-v1"}
        self.model = SentenceTransformer(
            MODEL, revision=REVISION, device=device,
            cache_folder=str(self.cache / "weights"),
            config_kwargs={"audio_config": None},
            model_kwargs={"torch_dtype": dtype, "attn_implementation": "eager"},
        )

    def encode(self, images, batch_size=16):
        values = self.model.encode(
            [{"image": image} for image in images], batch_size=batch_size,
            normalize_embeddings=True, show_progress_bar=False,
        )
        values = self.np.asarray(values, dtype=self.np.float32)
        if values.shape != (len(images), 768) or not self.np.isfinite(values).all():
            raise ValueError("Invalid EmbeddingGemma vectors")
        # BF16 output normalization has visible rounding error. Normalize again
        # in float32 so dot products are cosine similarities in every batch.
        norms = self.np.linalg.norm(values, axis=1, keepdims=True)
        if (norms < 1e-8).any():
            raise ValueError("EmbeddingGemma returned zero vectors")
        return values / norms

    def index(self, export, batch_size=16):
        from PIL import Image
        from functools import lru_cache

        maps = {m["id"]: m for m in export["maps"]}
        # Content identity also protects against map-file replacement while IDs
        # and dimensions remain unchanged. No labels enter index construction.
        hashes = {key: hashlib.sha256(Path(m["path"]).read_bytes()).hexdigest()
                  for key, m in maps.items()}

        @lru_cache(maxsize=4)
        def source(map_id):
            with Image.open(maps[map_id]["path"]) as image:
                return image.convert("RGB")

        entries = []
        for tile in export["tiles"]:
            identity = json.dumps([self.identity, hashes[tile["map_id"]], tile["rect"]], sort_keys=True)
            path = self.cache / (hashlib.sha256(identity.encode()).hexdigest() + ".npy")
            entries.append((tile, path))
        if not entries:
            raise ValueError("The prepared atlas has no enabled tiles")
        missing = [(tile, path) for tile, path in entries if not path.exists()]
        self.index_stats = {"cached_tiles": len(entries) - len(missing),
                            "computed_tiles": len(missing), "batch_size": batch_size}
        # Check real image inputs before accepting a batch-built corpus. The
        # initial ROCm SDPA run changed vectors with batch composition (~.945
        # cosine); eager attention is stable in the same runtime.
        samples = []
        for i in sorted({0, len(entries)//3, 2*len(entries)//3, len(entries)-1}):
            tile = entries[i][0]
            x, y, w, h = tile["rect"]
            samples.append(source(tile["map_id"]).crop((x,y,x+w,y+h))
                           .resize((256,192), Image.Resampling.BILINEAR))
        singles = self.np.stack([self.encode([im])[0] for im in samples])
        batched = self.encode(samples)
        self.consistency = float(self.np.min(self.np.sum(singles * batched, axis=1)))
        if self.consistency < .999:
            raise ValueError(f"Image embeddings depend on batch composition: cosine={self.consistency}")
        print(f"Batch consistency minimum cosine: {self.consistency:.7f}", flush=True)
        for start in range(0, len(missing), batch_size):
            batch = missing[start:start + batch_size]
            images = []
            for tile, _ in batch:
                x, y, w, h = tile["rect"]
                images.append(source(tile["map_id"]).crop((x, y, x+w, y+h))
                              .resize((256, 192), Image.Resampling.BILINEAR))
            for (_, path), vector in zip(batch, self.encode(images, batch_size)):
                temporary = path.with_suffix(f".{os.getpid()}.pending")
                with temporary.open("wb") as output:
                    self.np.save(output, vector, allow_pickle=False)
                temporary.replace(path)
            print(f"Indexed {min(start+batch_size,len(missing))}/{len(missing)} uncached map tiles", flush=True)
        matrix = self.np.stack([self.np.load(path, allow_pickle=False) for _, path in entries])
        if matrix.shape != (len(entries), 768) or not self.np.isfinite(matrix).all():
            raise ValueError("Invalid cached map embeddings")
        return matrix
