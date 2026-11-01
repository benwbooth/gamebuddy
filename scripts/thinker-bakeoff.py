#!/usr/bin/env python3
"""Thinker bake-off: replay the player's real questions through one provider
and score the answers. Every case starts from a copy-on-write copy of the
store with that moment's frame and location estimate, so answers don't lean
on each other.

  thinker-bakeoff.py cases DATA CASES.json        pick the frame and estimate of each case
  thinker-bakeoff.py run DATA CASES.json OUT NAME PROVIDER
  thinker-bakeoff.py report OUT

PROVIDER is talk_probe's: ollama:MODEL, claude:MODEL, openai:ENDPOINT|MODEL.
"""
import argparse, glob, json, os, re, shutil, subprocess, sys, tempfile, time
from pathlib import Path

GUIDE = "67044b49-9ce5-4c35-868e-846d1d37c59c"
# Hand-written from the transcripts: the session and the player's words, what
# a right answer must say (each group: one of these words) and must not say.
CASES = [
    ("20261027-183815", "All right, what do I do now?", [["forepaw"], ["wing boots"]], ["use your wing boots", "use the wing boots"]),
    ("20261027-183815", "I don't have a wing boots.", [["forepaw"]], ["use your wing boots", "use the wing boots"]),
    ("20261027-183815", "How do I get to mask on?", [["left", "right", "up", "down", "already"]], []),
    ("20261027-183815", "How do I get the four paw?", [["left", "right", "up", "down", "where are you"]], ["right here", "already in forepaw", "sorry"]),
    ("20261027-224909", "I don't have the boots. Where do I go?", [["forepaw", "left", "right", "down", "up"]], ["fly"]),
    ("20261027-224909", "Hello, where do I go? I don't have boots.", [["forepaw", "left", "right", "down", "up"]], ["fly"]),
    ("20261027-230142", "I'm not in the town.", [], ["you're at victim", "in victim", "leaving town", "leave town", "key shop"]),
    ("20261027-230142", "I just told you I'm not in town.", [], ["you're at victim", "leaving town", "leave town", "key shop"]),
    ("20261027-234554", "Okay, where do I get the boots?", [["forepaw"], ["2,800", "2800"]], ["item room"]),
    ("20261027-234554", "Yeah, you gotta tell me where to go.", [["left", "right", "up", "down"]], ["walkthrough"]),
    ("20261028-014045", "How do I get to four paw?", [["left", "right", "up", "down", "where are you"]], ["walkthrough"]),
    ("20261028-160847", "Alright, what do I do?", [["forepaw"], ["wing boots"]], []),
    ("20261028-160847", "I don't have wing boots.", [["forepaw"]], ["use your wing boots", "use the wing boots"]),
    ("20261028-160847", "You gotta tell me where to go. Right now it says facing left toward forepaw.", [["right", "down", "up"]], []),
    ("20261028-160847", "Oh, I can't go.", [["what", "which", "where", "try", "another", "instead", "dead end"]], ["cool", "have fun"]),
    ("20261028-160847", "Okay, where do I go now?", [["left", "right", "up", "down"]], ["through mascon to victim, then"]),
    ("20261028-160847", "You gotta tell me what direction to go.", [["left", "right", "up", "down"]], ["door"]),
    ("20261028-160847", "Where do I go? I don't even s- I don't see a door in this screen. There's no door. What do- d", [["left", "right", "up", "down"]], ["door"]),
    ("20261028-190435", "Alright, where do I go?", [["left", "right", "up", "down"]], []),
    ("20261028-190435", "Ah, just died.", [], ["cool", "have fun", "keep chatting", "key shop", "keys"]),
    ("20261028-190435", "Okay, where do I go now?", [["left", "right", "up", "down", "which exits", "where are you"]], []),
    ("20261028-210126", "There's no wing boots in there. You have to go to the town.", [["forepaw"]], []),
    ("20261028-210126", "Okay, go down again.", [["down", "ladder", "left", "right"]], []),
    ("20261026-205321", "Oh, that's lame. So the more rank you get, the less you get of flight? That sucks", [["flight", "wing boots", "time", "seconds"]], ["not sure", "don't know", "shop"]),
    ("20261027-022050", "Can you hear me now?", [["yes", "yep", "hear", "loud"]], []),
]
LINK = ["maps", "media", "models", "references", "vision", "vision-rocm", "walkthroughs", "worlds",
        "game-data", "retroarch"]


def timeline(data, session):
    path = glob.glob(f"{data}/sessions/{session}-*/timeline.jsonl")[0]
    rows = []
    for line in open(path):
        try:
            rows.append(json.loads(line))
        except ValueError:
            pass
    return Path(path).parent, rows


def build(args):
    out = []
    for i, (session, words, must, never) in enumerate(CASES):
        folder, rows = timeline(args.data, session)
        said = next(r for r in rows if r.get("kind") == "said" and r["value"]["role"] == "user"
                    and r["value"]["text"].strip().startswith(words.strip()[:40]))
        t = said["t"]
        frames = [r for r in rows if r.get("kind") == "frame" and r["t"] <= t]
        if not frames:
            frames = [r for r in rows if r.get("kind") == "frame"]
        estimates = [r for r in rows if r.get("kind") == "doc" and r.get("t", 0) <= t
                     and str(r.get("key", "")).startswith("location-estimate:")
                     and r.get("value")]
        out.append({"id": i, "session": session, "question": said["value"]["text"].strip(),
                    "frame": str(folder / "frames" / frames[-1]["value"]["file"]) if frames else None,
                    "estimate": estimates[-1]["value"] if estimates else None,
                    "must": must, "never": never})
    json.dump(out, open(args.cases, "w"), indent=1)
    print(len(out), "cases;", sum(c["frame"] is None for c in out), "without a frame;",
          sum(c["estimate"] is None for c in out), "without an estimate")


def run(args):
    cases = json.load(open(args.cases))
    out = Path(args.out) / args.name
    out.mkdir(parents=True, exist_ok=True)
    results = []
    for case in cases:
        if args.only and case["id"] not in args.only:
            continue
        with tempfile.TemporaryDirectory(dir=args.out) as tmp:
            for name in ["gamebuddy.sqlite"]:
                subprocess.run(["cp", "--reflink=always", f"{args.data}/{name}", tmp], check=True)
            for name in LINK:
                if os.path.exists(f"{args.data}/{name}"):
                    os.symlink(f"{args.data}/{name}", f"{tmp}/{name}")
            env = dict(os.environ, GAMEBUDDY_TALK_TRACE="1")
            if case["estimate"] is not None:
                estimate = f"{tmp}/estimate.json"
                json.dump(case["estimate"], open(estimate, "w"))
                env["GAMEBUDDY_PROBE_ESTIMATE"] = estimate
            started = time.time()
            done = subprocess.run(["target/debug/examples/talk_probe", tmp, GUIDE, args.provider,
                                   case["frame"], case["question"]], env=env, text=True,
                                  capture_output=True, errors="replace", timeout=300)
            line = next((l for l in done.stdout.splitlines() if l.startswith('{"')), None)
            row = json.loads(line) if line else {"answer": {"Err": done.stderr[-300:]}}
            answer = row.get("answer")
            if isinstance(answer, dict):
                answer = answer.get("Ok") or "ERROR " + str(answer.get("Err"))
            text = (answer or "").lower()
            ok = (all(any(w in text for w in group) for group in case["must"])
                  and not any(w in text for w in case["never"]) and not text.startswith("error"))
            results.append({"id": case["id"], "question": case["question"], "answer": answer,
                            "ok": ok, "first": row.get("first_sentence_seconds"),
                            "seconds": row.get("seconds"), "wall": time.time() - started,
                            "trace": done.stderr[-4000:]})
            print(case["id"], ok, row.get("first_sentence_seconds"), (answer or "")[:110],
                  flush=True)
    json.dump(results, open(out / "results.json", "w"), indent=1)


def report(args):
    for path in sorted(glob.glob(f"{args.out}/*/results.json")):
        rows = json.load(open(path))
        first = sorted(r["first"] for r in rows if r.get("first") is not None)
        print(Path(path).parent.name, f"right {sum(r['ok'] for r in rows)}/{len(rows)}",
              f"first sentence median {first[len(first) // 2]:.1f} s, worst {first[-1]:.1f} s"
              if first else "no answers")


def main():
    parser = argparse.ArgumentParser()
    sub = parser.add_subparsers(dest="command", required=True)
    c = sub.add_parser("cases")
    c.add_argument("data")
    c.add_argument("cases")
    r = sub.add_parser("run")
    r.add_argument("data")
    r.add_argument("cases")
    r.add_argument("out")
    r.add_argument("name")
    r.add_argument("provider")
    r.add_argument("--only", type=int, nargs="*")
    p = sub.add_parser("report")
    p.add_argument("out")
    args = parser.parse_args()
    {"cases": build, "run": run, "report": report}[args.command](args)


if __name__ == "__main__":
    sys.exit(main())
