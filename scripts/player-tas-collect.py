#!/usr/bin/env python3
"""Collect TAS training replays for the player's inverse-dynamics model.

For every current TASVideos publication on the chosen systems made in FCEUX
(.fm2) or BizHawk (.bk2): download the movie, find the ROM it was made for
among the given ROM folders by the movie's ROM hash, and replay it with
tas_replay into OUT/<publication id>/ (video.mkv, pads.txt with one RetroPad
mask per frame). Only player 1 on a standard pad is kept; movies with mice,
light guns, analog sticks, PAL or the Famicom Disk System are skipped. A
movie that drifts out of sync on our core is still correct training data:
every frame shows exactly what the recorded presses did on it.

  player-tas-collect.py OUT TAS_REPLAY CORES_DIR ROM_DIR... [--systems NES,SNES,Genesis,GB,GBC,GBA]
                        [--limit N] [--jobs N]
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

API = "https://tasvideos.org/api/v1/Publications?Systems={}&PageSize=100&CurrentPage={}"

# libretro RetroPad bits.
PAD = {"B": 0, "Y": 1, "Select": 2, "Start": 3, "Up": 4, "Down": 5, "Left": 6, "Right": 7,
       "A": 8, "X": 9, "L": 10, "R": 11}
# Per system: core, file extension, ROM extensions, and console button ->
# RetroPad button as the libretro core maps them.
SYSTEMS = {
    "NES": ("fceumm_libretro.so", "nes", (".nes",), {}),
    "SNES": ("bsnes_libretro.so", "sfc", (".sfc", ".smc"), {}),
    # Genesis Plus GX: Y=A, B=B, A=C, L=X, X=Y, R=Z, Select=Mode.
    "Genesis": ("genesis_plus_gx_libretro.so", "md", (".md", ".gen", ".bin", ".smd"),
                {"A": "Y", "B": "B", "C": "A", "X": "L", "Y": "X", "Z": "R", "Mode": "Select"}),
    "GB": ("gambatte_libretro.so", "gb", (".gb", ".gbc"), {}),
    "GBC": ("gambatte_libretro.so", "gbc", (".gbc", ".gb"), {}),
    "GBA": ("mgba_libretro.so", "gba", (".gba",), {}),
}


def fetch(url):
    request = urllib.request.Request(url, headers={"User-Agent": "GameBuddy TAS collector"})
    with urllib.request.urlopen(request, timeout=60) as response:
        return response.read()


def publications(cache, systems):
    path = os.path.join(cache, "publications-" + "-".join(systems) + ".json")
    if os.path.exists(path):
        return json.load(open(path))
    pubs = []
    for system in systems:
        page = 1
        while True:
            batch = json.loads(fetch(API.format(system, page)))
            if not batch:
                break
            pubs += [dict(p, system=system) for p in batch]
            page += 1
    pubs = [p for p in pubs if p["obsoletedById"] is None
            and p["movieFileName"].rsplit(".", 1)[-1] in ("fm2", "bk2")]
    json.dump(pubs, open(path, "w"))
    return pubs


def hashes(data):
    """Every hash a movie may name this ROM by: FCEUX's base64 MD5 of the
    ROM without its iNES header, and hex SHA1/MD5 of the file with and
    without a copier or iNES header."""
    bodies = [data]
    if data[:4] == b"NES\x1a":
        bodies.append(data[16 + (512 if data[6] & 4 else 0):])
    elif len(data) % 1024 == 512:
        bodies.append(data[512:])
    keys = []
    for body in bodies:
        md5 = hashlib.md5(body)
        keys += [base64.b64encode(md5.digest()).decode(), md5.hexdigest().upper(),
                 hashlib.sha1(body).hexdigest().upper()]
    return keys


ROM_EXTENSIONS = tuple(sorted({e for _, _, exts, _ in SYSTEMS.values() for e in exts}))


def rom_index(folders, cache):
    path = os.path.join(cache, "rom-index.json")
    index = json.load(open(path)) if os.path.exists(path) else {"files": [], "keys": {}}
    seen = set(index["files"])
    for folder in folders:
        for f in glob.glob(os.path.join(folder, "**", "*"), recursive=True):
            if f in seen or not os.path.isfile(f) or os.path.getsize(f) == 0:
                continue
            seen.add(f)
            index["files"].append(f)
            try:
                if f.lower().endswith(ROM_EXTENSIONS):
                    for key in hashes(open(f, "rb").read()):
                        index["keys"].setdefault(key, f)
                elif f.lower().endswith(".zip"):
                    archive = zipfile.ZipFile(f)
                    for name in archive.namelist():
                        if name.lower().endswith(ROM_EXTENSIONS):
                            for key in hashes(archive.read(name)):
                                index["keys"].setdefault(key, f + "::" + name)
            except (zipfile.BadZipFile, OSError, NotImplementedError):
                pass
    json.dump(index, open(path, "w"))
    return index["keys"]


def movie(pub, cache):
    folder = os.path.join(cache, "movies", str(pub["id"]))
    kind = pub["movieFileName"].rsplit(".", 1)[-1]
    found = glob.glob(os.path.join(folder, "*." + kind))
    if found:
        return found[0]
    os.makedirs(folder, exist_ok=True)
    data = fetch(f"https://tasvideos.org/{pub['id']}M?handler=Download")
    if data[:2] == b"PK" and kind == "fm2":
        zipfile.ZipFile(io.BytesIO(data)).extractall(folder)
    elif data[:2] == b"PK":
        # A .bk2 is itself a zip; the download may wrap it in another one.
        archive = zipfile.ZipFile(io.BytesIO(data))
        inner = [n for n in archive.namelist() if n.lower().endswith(".bk2")]
        if inner:
            archive.extractall(folder)
        else:
            open(os.path.join(folder, "movie.bk2"), "wb").write(data)
    else:
        open(os.path.join(folder, "movie." + kind), "wb").write(data)
    found = glob.glob(os.path.join(folder, "*." + kind))
    return found[0] if found else None


def fm2(path):
    """FCEUX movie -> (rom keys, reason to skip or None). Replayed as is."""
    text = open(path, "rb").read().decode("utf-8", "replace")
    h = dict(re.findall(r"^(\w+) (.*)$", text.split("\n|", 1)[0], re.M))
    if (h.get("port0") != "1" or h.get("port1", "0") != "0" or h.get("fourscore") == "1"
            or h.get("palFlag") == "1" or h.get("FDS") == "1"):
        return [], "skipped controller/region"
    return [h.get("romChecksum", "").removeprefix("base64:")], None


def bk2(path, system, out):
    """BizHawk movie -> (rom keys, reason to skip or None); writes out.pads."""
    archive = zipfile.ZipFile(path)
    names = {n.rsplit(".", 1)[0]: n for n in archive.namelist()}
    header = archive.read(names["Header"]).decode("utf-8", "replace")
    h = dict(re.findall(r"^(\w+) (.*?)\s*$", header, re.M))
    if h.get("PAL") == "True" or h.get("BoardName", "").startswith("FDS"):
        return [], "skipped controller/region"
    log = archive.read(names["Input Log"]).decode("utf-8", "replace").splitlines()
    key = next((line for line in log if line.startswith("LogKey:")), None)
    if not key:
        return [], "no log key"
    # LogKey:#Power|Reset|#P1 Up|P1 Down|...  '#' starts each '|'-group.
    groups = [g.split("|") for g in key[len("LogKey:"):].split("#") if g]
    groups = [[b for b in g if b] for g in groups]
    console = SYSTEMS[system][3]
    # Which (group, position) is player 1's button, and its RetroPad bit.
    columns = {}
    for gi, group in enumerate(groups):
        for bi, name in enumerate(group):
            button = name.removeprefix("P1 ")
            if name.startswith("P1 ") or (not name.startswith("P") or " " not in name):
                button = console.get(button, button)
                if button in PAD:
                    columns[(gi, bi)] = PAD[button]
                elif button in ("Reset", "Power"):
                    columns[(gi, bi)] = button
    if not any(isinstance(v, int) for v in columns.values()):
        return [], "skipped controller/region"
    frames = []
    for line in log:
        if not line.startswith("|"):
            continue
        fields = line.strip().strip("|").split("|")
        mask, reset = 0, False
        for gi, field in enumerate(fields):
            if "," in field:
                return [], "skipped controller/region"  # analog sticks
            for bi, c in enumerate(field):
                column = columns.get((gi, bi))
                if column is None or c in ". ":
                    continue
                if column == "Reset" or (column == "Power" and frames):
                    reset = True
                elif isinstance(column, int):
                    mask |= 1 << column
        frames.append(("!" if reset else "") + f"{mask:04x}")
    with open(out, "w") as f:
        f.write("\n".join(frames) + "\n")
    hash_ = h.get("SHA1", "").upper()
    return [hash_], None


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("out")
    parser.add_argument("tas_replay")
    parser.add_argument("cores")
    parser.add_argument("roms", nargs="+")
    parser.add_argument("--systems", default="NES,SNES,Genesis,GB,GBC,GBA")
    parser.add_argument("--limit", type=int, default=0)
    parser.add_argument("--jobs", type=int, default=6)
    args = parser.parse_args()
    systems = args.systems.split(",")
    cache = os.path.join(args.out, "_cache")
    os.makedirs(cache, exist_ok=True)
    pubs = publications(cache, systems)
    index = rom_index(args.roms, cache)
    print(f"{len(pubs)} movies, {len(index)} ROM hashes", file=sys.stderr, flush=True)
    report = []
    jobs = []
    for pub in sorted(pubs, key=lambda p: p["frames"]):
        out = os.path.join(args.out, str(pub["id"]))
        if os.path.exists(os.path.join(out, "publication.json")):
            continue
        try:
            path = movie(pub, cache)
            if not path:
                report.append((pub["id"], "no movie file", pub["title"]))
                continue
            if path.endswith(".fm2"):
                keys, skip = fm2(path)
            else:
                os.makedirs(out, exist_ok=True)
                keys, skip = bk2(path, pub["system"], os.path.join(out, "movie.pads"))
                path = os.path.join(out, "movie.pads")
        except Exception as error:  # noqa: BLE001 - keep collecting
            report.append((pub["id"], "movie failed", f"{pub['title']}: {error}"))
            continue
        if skip:
            report.append((pub["id"], skip, pub["title"]))
            continue
        rom = next((index[k] for k in keys if k in index), None)
        if not rom:
            report.append((pub["id"], "no rom", pub["title"]))
            continue
        jobs.append((pub, rom, path, out))
        if args.limit and len(jobs) >= args.limit:
            break

    def replay(job):
        pub, rom, path, out = job
        core, ext, _, _ = SYSTEMS[pub["system"]]
        result = subprocess.run([args.tas_replay, os.path.join(args.cores, core), rom, path, out,
                                 "--ext", ext], capture_output=True, text=True,
                                errors="replace")
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
