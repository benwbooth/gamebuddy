#!/usr/bin/env python3
"""Practice runs of a game-playing policy against a hidden RetroArch, and the
player scorecard: five fixed-length runs from one save state, each scored by
how many screens of the room-graph route it covered and how often it died.
Input goes in through RetroArch's network gamepad; what the policy and the
score see is the screen, nothing is read from the emulator.

  player-practice.py run OUT --policy nitrogen|random|plain|gated [--runs 5] [--secs 120]
  player-practice.py score DATA TOWARD OUT

The practice RetroArch is ~/.cache/gamebuddy-practice/run.sh under the
gb-practice unit; a run starts with LOAD_STATE, slot 0 of its state folder.
"""
import argparse, glob, json, os, random, socket, struct, subprocess, sys, threading, time
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
                frame = Image.open(now[1]).convert("RGB")
                os.unlink(now[1])
                return frame
            except OSError:
                pass
    return None


def practice_running():
    """Start the hidden RetroArch if needed; it sometimes crashes starting
    under Xvfb, so a run only begins once it answers with a screenshot."""
    for _ in range(4):
        active = subprocess.run(["systemctl", "--user", "is-active", "-q", "gb-practice"])
        if active.returncode:
            subprocess.run(["systemctl", "--user", "reset-failed", "gb-practice"],
                           stderr=subprocess.DEVNULL)
            subprocess.run(["systemd-run", "--user", "--unit=gb-practice", str(PRACTICE / "run.sh")],
                           check=True)
            time.sleep(9)
        if screenshot() is not None:
            return
    raise SystemExit("The practice RetroArch would not start")


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
        return out[:int(os.environ.get("PLAYER_STEP_FRAMES", 12))]


class Random:
    """The press mix of NitroGen's Faxanadu run, ignoring the screen."""

    def act(self, frame):
        time.sleep(0.13)
        choice = random.choices([set(), {"Right"}, {"Left"}, {"A"}, {"B"}, {"Down"}, {"Up"}],
                                [449, 97, 72, 67, 63, 27, 20])[0]
        return [choice] * 12


class Router:
    """The route's next move, kept fresh by a background thread: placing a
    frame on the maps takes half a second, the player can't wait for it."""

    def __init__(self, data, toward):
        self.process = subprocess.Popen(
            ["target/debug/examples/player_route", data, toward], stdin=subprocess.PIPE,
            stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
        self.path = PRACTICE / "route-frame.png"
        self.latest, self.frame = {}, None
        threading.Thread(target=self.follow, daemon=True).start()

    def follow(self):
        while True:
            frame = self.frame
            if frame is None:
                time.sleep(0.05)
                continue
            self.frame = None
            frame.save(self.path)
            self.process.stdin.write(f"{self.path}\n")
            self.process.stdin.flush()
            self.latest = json.loads(self.process.stdout.readline())


class Gate:
    """Obedience in code. The route names a direction; the player model may
    jump, swing and time its moves, but never holds the opposite direction,
    and walking toward the goal is held when it has no opinion. A vertical
    move has no known ladder spot on the screen, so it sweeps the screen
    trying the ladder, the way a person looks for one."""

    OPPOSITE = {"Left": "Right", "Right": "Left", "Up": "Down", "Down": "Up"}

    def __init__(self, policy, router):
        self.policy, self.router = policy, router
        self.sweep, self.turned = "Right", time.time()
        self.want, self.wanted_at = None, 0
        self.bar, self.fight_until = None, 0

    def act(self, frame):
        self.router.frame = frame
        now = time.time()
        moves = self.router.latest.get("moves")
        if moves:
            self.want, self.wanted_at = moves[0][0], now
        # A transition or a room off the maps: the last order stands a moment,
        # then the screen has nothing to say and the model finds its way out.
        want = self.want if now - self.wanted_at < 2 else None
        # Losing health means something is on us: stop walking into it and
        # swing, turning round after a moment in case it is behind.
        bar = health(frame)
        if bar is not None and self.bar is not None and bar < self.bar - 0.02:
            self.fight_until = now + 1.5
        self.bar = bar if bar is not None else self.bar
        if now < self.fight_until:
            side = want if want in ("Left", "Right") else self.sweep
            if self.fight_until - now < 0.7:
                side = self.OPPOSITE[side]
            self.policy.act(frame)
            return [{side}, {side}, {"B"}, {"B"}, set(), set()]
        if want is None:
            return self.policy.act(frame)
        if want in ("Up", "Down") and now - self.turned > 3:
            self.sweep, self.turned = self.OPPOSITE[self.sweep], now
        out = []
        for i, held in enumerate(self.policy.act(frame)):
            if want in ("Left", "Right"):
                wanted = want in held
                opposed = self.OPPOSITE[want] in held
                held = held & {"A", "B"}
                # Against orders it stands and fights; with no opinion it walks.
                if wanted or not opposed:
                    held.add(want)
            else:
                held = held & {"A", "B"}
                held.add(want if i % 12 < 6 else self.sweep)
            out.append(held)
        return out


def run_policy(args):
    practice_running()
    policy = {"nitrogen": Nitrogen, "random": Random}.get(args.policy, lambda: None)()
    if args.policy == "gated":
        policy = Gate(Nitrogen(), Router(args.data, args.toward))
    for n in range(1, args.runs + 1):
        out = Path(args.out) / args.policy / f"run{n}"
        out.mkdir(parents=True, exist_ok=True)
        practice_running()
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
            if args.policy == "gated" and policy.router.latest.get("arrived"):
                hold(set())
                frame.save(out / f"{int((time.time() - started) * 1000):07d}.png")
                break
            frame.save(out / f"{int((time.time() - started) * 1000):07d}.png")
            for held in policy.act(frame):
                hold(held)
                time.sleep(1 / 60)
        hold(set())
        print(args.policy, "run", n, "done", flush=True)


def health(frame, row=27, x0=24, x1=105):
    """Share of the HUD's power bar that is lit, or None on a screen without
    the HUD (a transition): a game-specific scorecard setting, the bar sits
    on one row and its lit pixels are strongly red."""
    p = np.asarray(frame.convert("RGB"), dtype=int)
    if (p[22, x0:x1].min(axis=1) > 200).mean() < 0.8:
        return None
    return float(((p[row, x0:x1, 0] > 150) & (p[row, x0:x1, 1] < 110)).mean())


def deaths(bars):
    """A bar that reached empty and came back near full: the hero respawned."""
    count, empty = 0, False
    for bar in bars:
        if bar is None:
            continue
        if bar < 0.04:
            empty = True
        elif empty and bar > 0.7:
            count, empty = count + 1, False
    return count


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
        result = {"start": left[0] if left else None, "best": min(left) if left else None,
                  "covered": left[0] - min(left) if left else 0, "reached_s": reached,
                  "deaths": deaths(bars), "frames": len(bars)}
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
