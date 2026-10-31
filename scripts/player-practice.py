#!/usr/bin/env python3
"""Practice runs of a game-playing policy against a hidden RetroArch, and the
player scorecard: five fixed-length runs from one save state, each scored by
how many screens of the room-graph route it covered and how often it died.
Input goes in through RetroArch's network gamepad; what the policy and the
score see is the screen, nothing is read from the emulator.

  player-practice.py run OUT --policy nitrogen|random|plain [--runs 5] [--secs 120]
  player-practice.py score DATA TOWARD OUT

The practice RetroArch is ~/.cache/gamebuddy-practice/run.sh under the
gb-practice unit; a run starts with LOAD_STATE, slot 0 of its state folder.
"""
import argparse, glob, json, os, random, socket, struct, subprocess, sys, time
from pathlib import Path

import numpy as np
from PIL import Image

PRACTICE = Path.home() / ".cache/gamebuddy-practice"
NITROGEN = Path(os.environ.get("NITROGEN_DIR", Path.home() / "src/nitrogen-spike"))
PAD = {"B": 0, "Y": 1, "Select": 2, "Start": 3, "Up": 4, "Down": 5, "Left": 6, "Right": 7,
       "A": 8, "X": 9}
send = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)


def command(text):
    send.sendto(text.encode(), ("127.0.0.1", 55394))


def hold(buttons):
    for name, i in PAD.items():
        send.sendto(struct.pack("=iiiiHxx", 0, 1, 0, i, name in buttons), ("127.0.0.1", 55410))


def screenshot():
    def newest():
        shots = [(os.path.getmtime(p), p) for p in glob.glob(str(PRACTICE / "shots/*.png"))]
        return max(shots) if shots else None
    before = newest()
    command("SCREENSHOT")
    for _ in range(40):
        time.sleep(0.02)
        now = newest()
        if now and now != before:
            try:
                return Image.open(now[1]).convert("RGB")
            except OSError:
                pass
    return None


def practice_running():
    active = subprocess.run(["systemctl", "--user", "is-active", "-q", "gb-practice"])
    if active.returncode:
        subprocess.run(["systemd-run", "--user", "--unit=gb-practice", str(PRACTICE / "run.sh")],
                       check=True)
        time.sleep(9)


class Nitrogen:
    """NitroGen zero-shot: gamepad players' habits mapped to a two-button pad."""

    def __init__(self):
        sys.path.insert(0, str(NITROGEN / "src"))
        from nitrogen.inference_session import InferenceSession
        from nitrogen.shared import BUTTON_ACTION_TOKENS
        self.tokens = BUTTON_ACTION_TOKENS
        real, sys.stdin = sys.stdin, open(os.devnull)
        self.session = InferenceSession.from_ckpt(str(NITROGEN / "ng.pt"), context_length=1)
        sys.stdin = real

    def act(self, frame):
        r = self.session.predict(np.array(frame.resize((256, 256))))
        out = []
        for row, (x, y) in zip(r["buttons"], r["j_left"]):
            on = lambda n: row[self.tokens.index(n)] > 0.5
            held = set()
            if on("DPAD_LEFT") or x < -0.5:
                held.add("Left")
            if on("DPAD_RIGHT") or x > 0.5:
                held.add("Right")
            if on("DPAD_UP") or y < -0.5:
                held.add("Up")
            if on("DPAD_DOWN") or y > 0.5:
                held.add("Down")
            if on("SOUTH"):
                held.add("A")
            if on("WEST") or on("EAST"):
                held.add("B")
            out.append(held)
        return out[:12]


class Random:
    """The press mix of NitroGen's Faxanadu run, ignoring the screen."""

    def act(self, frame):
        time.sleep(0.13)
        choice = random.choices([set(), {"Right"}, {"Left"}, {"A"}, {"B"}, {"Down"}, {"Up"}],
                                [449, 97, 72, 67, 63, 27, 20])[0]
        return [choice] * 12


def run_policy(args):
    practice_running()
    policy = {"nitrogen": Nitrogen, "random": Random}.get(args.policy, lambda: None)()
    for n in range(1, args.runs + 1):
        out = Path(args.out) / args.policy / f"run{n}"
        out.mkdir(parents=True, exist_ok=True)
        command("LOAD_STATE")
        time.sleep(1)
        if args.policy == "plain":
            env = dict(os.environ, AUTOPLAY_FRAMES=str(out))
            subprocess.run(["target/debug/examples/autoplay_probe", args.data,
                            str(PRACTICE / "shots"), args.toward, str(args.secs)],
                           env=env, stdout=open(out / "log.txt", "w"))
            continue
        started = time.time()
        while time.time() - started < args.secs:
            frame = screenshot()
            if frame is None:
                continue
            frame.save(out / f"{int((time.time() - started) * 1000):07d}.png")
            for held in policy.act(frame):
                hold(held)
                time.sleep(1 / 60)
        hold(set())
        print(args.policy, "run", n, "done", flush=True)


def health(frame, row=27, x0=24, x1=105):
    """Share of the HUD's power bar that is lit, a game-specific scorecard
    setting: the bar sits on one row, lit pixels are strongly red."""
    p = np.asarray(frame.convert("RGB"), dtype=int)[row, x0:x1]
    return float(((p[:, 0] > 150) & (p[:, 1] < 110)).mean())


def score(args):
    runs = sorted(glob.glob(f"{args.out}/*/run*"))
    placed = subprocess.run(["target/debug/examples/player_score", args.data, args.toward, "1",
                             *runs], capture_output=True, text=True, check=True).stdout
    rows = {}
    for line in placed.splitlines():
        try:
            row = json.loads(line)
        except ValueError:
            continue
        rows.setdefault(row["run"], []).append(row)
    table = {}
    for run in runs:
        left = [r["left"] for r in rows.get(run, []) if r.get("left") is not None]
        reached = next((r["ms"] / 1000 for r in rows.get(run, []) if r.get("left") == 0), None)
        bars = [health(Image.open(p)) for p in sorted(glob.glob(run + "/*.png"))]
        deaths = len([1 for i in range(1, len(bars)) if bars[i - 1] < 0.08 <= bars[i] and
                      max(bars[i:i + 20]) > 0.7])
        result = {"start": left[0] if left else None, "best": min(left) if left else None,
                  "covered": left[0] - min(left) if left else 0, "reached_s": reached,
                  "deaths": deaths, "frames": len(bars)}
        print(run, json.dumps(result), flush=True)
        table.setdefault(run.split("/")[-2], []).append(result)
    for policy, results in table.items():
        n = len(results)
        print(policy, f"{n} runs: screens covered {[r['covered'] for r in results]},",
              f"reached {sum(r['reached_s'] is not None for r in results)}/{n},",
              f"deaths {[r['deaths'] for r in results]}")


def main():
    parser = argparse.ArgumentParser()
    sub = parser.add_subparsers(dest="command", required=True)
    r = sub.add_parser("run")
    r.add_argument("out")
    r.add_argument("--policy", default="nitrogen")
    r.add_argument("--runs", type=int, default=5)
    r.add_argument("--secs", type=int, default=120)
    r.add_argument("--data", default=str(Path.home() / ".local/share/GameBuddy"))
    r.add_argument("--toward", default="Mascon")
    s = sub.add_parser("score")
    s.add_argument("data")
    s.add_argument("toward")
    s.add_argument("out")
    args = parser.parse_args()
    (run_policy if args.command == "run" else score)(args)


if __name__ == "__main__":
    main()
