#!/usr/bin/env python3
"""Delay one real tool result in the private native-follow-up fixture.

The signed-in Codex process, native voice, reasoning and tools remain real.
Holding the result keeps a backend turn alive long enough to reproduce a
player clarifying a request before it finishes. No production delay hook.
"""
import json
import os
from pathlib import Path
import subprocess
import sys
import threading
import time

assert os.environ.get("GAMEBUDDY_TEST_FOLLOWUPS")
root = Path(os.environ["GAMEBUDDY_TEST_DATA"]).parent
assert str(root).startswith("/tmp/gamebuddy-realtime-test-")
child = subprocess.Popen(
    [os.environ["GAMEBUDDY_TEST_REAL_CODEX"], *sys.argv[1:]],
    stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, bufsize=1,
)
lock = threading.Lock()
calls = {}
held = False


def record(kind, **values):
    with (root / "tool-delay.jsonl").open("a") as out:
        out.write(json.dumps({"kind": kind, "at_ms": int(time.time() * 1000), **values}) + "\n")


def send(line):
    with lock:
        try:
            child.stdin.write(line)
            child.stdin.flush()
        except (BrokenPipeError, ValueError):
            pass


def release(line):
    record("released")
    send(line)


def incoming():
    global held
    for line in sys.stdin:
        value = json.loads(line)
        key = str(value.get("id"))
        if not held and "method" not in value and key in calls:
            held = True
            record("held", tool=calls[key])
            timer = threading.Timer(20, release, args=(line,))
            timer.daemon = True
            timer.start()
        else:
            send(line)
    with lock:
        child.stdin.close()


threading.Thread(target=incoming, daemon=True).start()
try:
    for line in child.stdout:
        value = json.loads(line)
        if value.get("method") == "item/tool/call":
            calls[str(value["id"])] = value["params"]["tool"]
        sys.stdout.write(line)
        sys.stdout.flush()
finally:
    child.wait()
sys.exit(child.returncode)
