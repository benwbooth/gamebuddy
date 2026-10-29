#!/usr/bin/env python3
"""Collect TAS training replays for the player's inverse-dynamics model.

For every current TASVideos NES publication made in FCEUX (.fm2): download the
movie, find the ROM it was made for among the given ROM folders (by FCEUX's
romChecksum: MD5 of the ROM without its iNES header), and replay it with
tas_replay into OUT/<publication id>/ (video.mkv + inputs.txt). Movies for
two players, the Four Score, PAL or the Famicom Disk System are skipped.

  player-tas-collect.py OUT TAS_REPLAY CORE ROM_DIR... [--limit N] [--jobs N]
"""
import argparse
import base64
import concurrent.futures
import glob
import hashlib
import io
import json
import os
import re
import subprocess
import sys
import urllib.request
import zipfile

API = "https://tasvideos.org/api/v1/Publications?Systems=NES&PageSize=100&CurrentPage={}"


def fetch(url):
    request = urllib.request.Request(url, headers={"User-Agent": "GameBuddy TAS collector"})
    with urllib.request.urlopen(request, timeout=60) as response:
        return response.read()


def publications(cache):
    path = os.path.join(cache, "publications.json")
    if os.path.exists(path):
        return json.load(open(path))
    pubs, page = [], 1
    while True:
        batch = json.loads(fetch(API.format(page)))
        if not batch:
            break
        pubs += batch
        page += 1
    pubs = [p for p in pubs if p["obsoletedById"] is None and p["movieFileName"].endswith(".fm2")]
    json.dump(pubs, open(path, "w"))
    return pubs


def checksum(data):
    if data[:4] == b"NES\x1a":
        data = data[16 + (512 if data[6] & 4 else 0):]
    return base64.b64encode(hashlib.md5(data).digest()).decode()


def rom_index(folders, cache):
    path = os.path.join(cache, "rom-index.json")
    index = json.load(open(path)) if os.path.exists(path) else {}
    seen = set(index.values())
    for folder in folders:
        for f in glob.glob(os.path.join(folder, "**", "*"), recursive=True):
            if not os.path.isfile(f) or os.path.getsize(f) == 0:
                continue
            if f.lower().endswith(".nes") and f not in seen:
                index[checksum(open(f, "rb").read())] = f
            elif f.lower().endswith(".zip"):
                try:
                    archive = zipfile.ZipFile(f)
                    for name in archive.namelist():
                        spec = f + "::" + name
                        if name.lower().endswith(".nes") and spec not in seen:
                            data = archive.read(name)
                            # Only headered ROMs load without a guessed header.
                            if data[:4] == b"NES\x1a":
                                index[checksum(data)] = spec
                except (zipfile.BadZipFile, OSError):
                    pass
    json.dump(index, open(path, "w"))
    return index


def movie(pub, cache):
    folder = os.path.join(cache, "movies", str(pub["id"]))
    found = glob.glob(os.path.join(folder, "*.fm2"))
    if found:
        return found[0]
    os.makedirs(folder, exist_ok=True)
    data = fetch(f"https://tasvideos.org/{pub['id']}M?handler=Download")
    if data[:2] == b"PK":
        zipfile.ZipFile(io.BytesIO(data)).extractall(folder)
    else:
        open(os.path.join(folder, "movie.fm2"), "wb").write(data)
    found = glob.glob(os.path.join(folder, "*.fm2"))
    return found[0] if found else None


def header(text):
    return dict(re.findall(r"^(\w+) (.*)$", text.split("\n|", 1)[0], re.M))


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("out")
    parser.add_argument("tas_replay")
    parser.add_argument("core")
    parser.add_argument("roms", nargs="+")
    parser.add_argument("--limit", type=int, default=0)
    parser.add_argument("--jobs", type=int, default=6)
    args = parser.parse_args()
    cache = os.path.join(args.out, "_cache")
    os.makedirs(cache, exist_ok=True)
    pubs = publications(cache)
    index = rom_index(args.roms, cache)
    report = []
    jobs = []
    for pub in sorted(pubs, key=lambda p: p["frames"]):
        out = os.path.join(args.out, str(pub["id"]))
        if os.path.exists(os.path.join(out, "inputs.txt")):
            continue
        try:
            path = movie(pub, cache)
        except Exception as error:  # noqa: BLE001 - keep collecting
            report.append((pub["id"], "download failed", str(error)))
            continue
        if not path:
            continue
        text = open(path, "rb").read().decode("utf-8", "replace")
        h = header(text)
        if (h.get("port0") != "1" or h.get("port1", "0") != "0" or h.get("fourscore") == "1"
                or h.get("palFlag") == "1" or h.get("FDS") == "1"):
            report.append((pub["id"], "skipped controller/region", pub["title"]))
            continue
        rom = index.get(h.get("romChecksum", "").removeprefix("base64:"))
        if not rom:
            report.append((pub["id"], "no rom", pub["title"]))
            continue
        jobs.append((pub, rom, path, out))
        if args.limit and len(jobs) >= args.limit:
            break

    def replay(job):
        pub, rom, path, out = job
        result = subprocess.run([args.tas_replay, args.core, rom, path, out],
                                capture_output=True, text=True)
        status = "ok" if result.returncode == 0 else "failed: " + result.stderr.strip()[-200:]
        print(pub["id"], status, pub["title"], flush=True)
        if result.returncode == 0:
            json.dump(pub, open(os.path.join(out, "publication.json"), "w"))
        return pub["id"], status, pub["title"]

    with concurrent.futures.ThreadPoolExecutor(args.jobs) as pool:
        report += list(pool.map(replay, jobs))
    done = sum(1 for _, status, _ in report if status == "ok")
    with open(os.path.join(args.out, "collect-report.tsv"), "a") as f:
        for row in report:
            f.write("\t".join(map(str, row)) + "\n")
    print(f"{done} replays", file=sys.stderr)


if __name__ == "__main__":
    main()
