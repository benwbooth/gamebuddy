#!/usr/bin/env python3
"""Inverse-dynamics model for the player: from a few frames before and after
moment t, which NES buttons were held at t.

Trained on TAS replays (crates/gamebuddy-services/examples/tas_replay.rs
writes video.mkv + inputs.txt per movie), then used to label speedrun videos
that have no inputs.

  player-idm.py prepare REPLAY_DIR...          decode video.mkv to frames.u8
  player-idm.py train OUT.pt --train DIR... --test DIR... [--steps N]
  player-idm.py eval MODEL.pt DIR...

Frames are kept at 128x120 RGB (a quarter of the NES picture), which still
shows the player's sprite move a pixel or two between frames.
"""
import argparse
import os
import subprocess
import sys
import time

import numpy as np
import torch
import torch.nn as nn
import torch.nn.functional as F

W, H = 128, 120
BUTTONS = "RLDUTSBA"  # FCEUX order: Right Left Down Up sTart Select B A
# Frames around t the model sees: input at t shows up a frame or two later,
# and the frames before give the motion it changes.
BEFORE, AFTER = 2, 5
WINDOW = BEFORE + 1 + AFTER


CHUNK = 1200  # 20 s of play kept together, so windows never span a cut


def pads(directory):
    with open(os.path.join(directory, "inputs.txt")) as f:
        return [line.rstrip("\n") for line in f]


def prepare(directory, cap=24000):
    """Decode video.mkv to frames.u8 at 128x120. Long runs keep evenly spaced
    20 s stretches up to `cap` frames, so one long movie doesn't outweigh
    many short games. chunks.npy holds each kept stretch's source start."""
    out = os.path.join(directory, "frames.u8")
    if os.path.exists(out):
        return
    total = len(pads(directory))
    count = max(1, total // CHUNK)
    keep = min(count, max(1, cap // CHUNK))
    starts = sorted({int(i * count / keep) * CHUNK for i in range(keep)})
    wanted = np.zeros(total + CHUNK, dtype=bool)
    for start in starts:
        wanted[start:start + CHUNK] = True
    size = W * H * 3
    decoder = subprocess.Popen(
        ["ffmpeg", "-nostdin", "-loglevel", "error", "-i", os.path.join(directory, "video.mkv"),
         "-vf", f"scale={W}:{H}:flags=area", "-f", "rawvideo", "-pix_fmt", "rgb24", "-"],
        stdout=subprocess.PIPE)
    with open(out + ".tmp", "wb") as f:
        index = 0
        while index < total:
            frame = decoder.stdout.read(size)
            if len(frame) < size:
                break
            if wanted[index]:
                f.write(frame)
            index += 1
    decoder.wait()
    np.save(os.path.join(directory, "chunks.npy"), np.array(starts, dtype=np.int64))
    os.rename(out + ".tmp", out)


def load(directory):
    frames = np.memmap(os.path.join(directory, "frames.u8"), dtype=np.uint8, mode="r")
    frames = frames.reshape(-1, H, W, 3)
    labels = np.array([[c not in ". " for c in pad] for pad in pads(directory)], dtype=np.float32)
    chunks_path = os.path.join(directory, "chunks.npy")
    if os.path.exists(chunks_path):
        starts = np.load(chunks_path)
        labels = np.concatenate([labels[s:s + CHUNK] for s in starts])
        n = min(len(frames), len(labels))
        # Kept stretches, as (begin, end) in the kept arrays.
        spans = [(i * CHUNK, min(n, (i + 1) * CHUNK)) for i in range(len(starts))]
    else:
        n = min(len(frames), len(labels))
        spans = [(0, n)]
    spans = [(b, e) for b, e in spans if e - b > WINDOW]
    return frames[:n], labels[:n], spans


class Data:
    def __init__(self, directories):
        self.sets = [load(d) for d in directories]
        self.names = directories
        # Square root of size: long runs count for more, but not linearly.
        sizes = np.array([sum(e - b for b, e in spans) for _, _, spans in self.sets], dtype=float)
        self.weights = np.sqrt(sizes) / np.sqrt(sizes).sum()

    def moments(self, i, count, rng):
        _, _, spans = self.sets[i]
        lengths = np.array([e - b - WINDOW for b, e in spans], dtype=float)
        which = rng.choice(len(spans), size=count, p=lengths / lengths.sum())
        return np.array([rng.integers(spans[k][0] + BEFORE, spans[k][1] - AFTER) for k in which])

    def batch(self, size, rng):
        which = rng.choice(len(self.sets), size=size, p=self.weights)
        xs, ys = [], []
        for i in which:
            frames, labels, _ = self.sets[i]
            t = self.moments(i, 1, rng)[0]
            xs.append(frames[t - BEFORE:t + AFTER + 1])
            ys.append(labels[t])
        return np.stack(xs), np.stack(ys)


class IDM(nn.Module):
    """Frames stacked on channels plus their differences, a small conv net."""

    def __init__(self, width=1.0):
        super().__init__()
        c = 3 * WINDOW + 3 * (WINDOW - 1)
        a, b, d, e = (int(n * width) for n in (64, 128, 192, 256))

        def block(i, o, s):
            return nn.Sequential(nn.Conv2d(i, o, 3, s, 1), nn.BatchNorm2d(o), nn.GELU())

        self.net = nn.Sequential(
            block(c, a, 1), block(a, a, 2),  # 64x60
            block(a, b, 1), block(b, b, 2),  # 32x30
            block(b, d, 1), block(d, d, 2),  # 16x15
            block(d, e, 1), block(e, e, 2),  # 8x8
        )
        self.head = nn.Sequential(nn.Flatten(), nn.Linear(e * 8 * 8, 512), nn.GELU(),
                                  nn.Dropout(0.2), nn.Linear(512, len(BUTTONS)))

    def forward(self, x):
        # x: (B, WINDOW, H, W, 3) uint8
        x = x.float().div(255).permute(0, 1, 4, 2, 3)  # B T C H W
        diff = x[:, 1:] - x[:, :-1]
        x = torch.cat([x.flatten(1, 2), diff.flatten(1, 2) * 4], 1)
        return self.head(self.net(x))


def augment(x, generator):
    """Make clean emulator frames look more like captured video: brightness,
    contrast, colour balance, a little blur and a shift of a few pixels."""
    b = x.shape[0]
    x = x.float()
    gain = 1 + 0.25 * (torch.rand(b, 1, 1, 1, 3, device=x.device, generator=generator) - 0.5)
    bias = 20 * (torch.rand(b, 1, 1, 1, 1, device=x.device, generator=generator) - 0.5)
    x = (x * gain + bias).clamp(0, 255)
    if torch.rand(1, generator=generator, device=x.device).item() < 0.5:
        t = x.permute(0, 1, 4, 2, 3).flatten(0, 1)
        t = F.interpolate(F.interpolate(t, scale_factor=0.6, mode="bilinear"), size=(H, W),
                          mode="bilinear")
        x = t.unflatten(0, (b, WINDOW)).permute(0, 1, 3, 4, 2)
    dx, dy = (int(v) for v in torch.randint(-4, 5, (2,), generator=generator, device=x.device))
    x = torch.roll(x, shifts=(dy, dx), dims=(2, 3))
    return x.round().clamp(0, 255).to(torch.uint8)


@torch.no_grad()
def evaluate(model, data, device, samples=4000, seed=1):
    model.eval()
    rng = np.random.default_rng(seed)
    results = {}
    for i, (name, (frames, labels, _)) in enumerate(zip(data.names, data.sets)):
        ts = data.moments(i, samples, rng)
        preds = []
        for chunk in np.array_split(ts, max(1, len(ts) // 256)):
            x = torch.from_numpy(np.stack([frames[t - BEFORE:t + AFTER + 1] for t in chunk]))
            preds.append(torch.sigmoid(model(x.to(device))).cpu().numpy())
        p = np.concatenate(preds) > 0.5
        y = labels[ts] > 0.5
        f1 = {}
        for i, button in enumerate(BUTTONS):
            tp = (p[:, i] & y[:, i]).sum()
            fp = (p[:, i] & ~y[:, i]).sum()
            fn = (~p[:, i] & y[:, i]).sum()
            if y[:, i].sum() >= 20:
                f1[button] = round(2 * tp / max(1, 2 * tp + fp + fn), 3)
        exact = float((p == y).all(1).mean())
        results[os.path.basename(name.rstrip("/"))] = {"exact": round(exact, 3), "f1": f1}
    model.train()
    return results


def train(args):
    device = "cuda"
    for d in args.train + args.test:
        prepare(d)
    train_data, test_data = Data(args.train), Data(args.test)
    model = IDM(args.width).to(device)
    print(f"{sum(p.numel() for p in model.parameters()) / 1e6:.1f}M parameters", flush=True)
    positive = np.concatenate([lab for _, lab, _ in train_data.sets]).mean(0)
    pos_weight = torch.tensor(np.clip((1 - positive) / np.maximum(positive, 1e-3), 1, 10),
                              device=device, dtype=torch.float32)
    opt = torch.optim.AdamW(model.parameters(), lr=args.lr, weight_decay=0.05)
    sched = torch.optim.lr_scheduler.OneCycleLR(opt, args.lr, total_steps=args.steps, pct_start=0.05)
    rng = np.random.default_rng(0)
    gen = torch.Generator(device=device).manual_seed(0)
    started = time.time()
    for step in range(1, args.steps + 1):
        x, y = train_data.batch(args.batch, rng)
        x = augment(torch.from_numpy(x).to(device), gen)
        y = torch.from_numpy(y).to(device)
        loss = F.binary_cross_entropy_with_logits(model(x), y, pos_weight=pos_weight)
        opt.zero_grad(set_to_none=True)
        loss.backward()
        opt.step()
        sched.step()
        if step % 200 == 0:
            print(f"step {step} loss {loss.item():.4f} {time.time() - started:.0f}s", flush=True)
        if step % args.eval_every == 0 or step == args.steps:
            print("test", evaluate(model, test_data, device), flush=True)
            torch.save({"model": model.state_dict(), "before": BEFORE, "after": AFTER,
                        "size": [W, H], "buttons": BUTTONS, "width": args.width}, args.out)
    print("train", evaluate(model, train_data, device, samples=2000), flush=True)


def main():
    parser = argparse.ArgumentParser()
    sub = parser.add_subparsers(dest="command", required=True)
    p = sub.add_parser("prepare")
    p.add_argument("dirs", nargs="+")
    t = sub.add_parser("train")
    t.add_argument("out")
    t.add_argument("--train", nargs="+", required=True)
    t.add_argument("--test", nargs="+", required=True)
    t.add_argument("--steps", type=int, default=20000)
    t.add_argument("--batch", type=int, default=64)
    t.add_argument("--lr", type=float, default=1e-3)
    t.add_argument("--eval-every", type=int, default=2000)
    t.add_argument("--width", type=float, default=1.0)
    e = sub.add_parser("eval")
    e.add_argument("model")
    e.add_argument("dirs", nargs="+")
    args = parser.parse_args()
    if args.command == "prepare":
        import concurrent.futures
        with concurrent.futures.ThreadPoolExecutor(8) as pool:
            list(pool.map(prepare, args.dirs))
    elif args.command == "train":
        train(args)
    else:
        saved = torch.load(args.model)
        model = IDM(saved.get("width", 1.0)).cuda()
        model.load_state_dict(saved["model"])
        for d in args.dirs:
            prepare(d)
        print(evaluate(model, Data(args.dirs), "cuda"))


if __name__ == "__main__":
    sys.exit(main())
