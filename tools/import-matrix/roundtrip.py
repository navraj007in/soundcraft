#!/usr/bin/env python3
"""P0-T03 save/reopen loop: open a session, save it to a new folder, repeat N times.

Each cycle runs `soundcraft-cli run --in PREV --save NEXT` in a fresh folder. After every cycle
the session JSON and the SHA-256 of every file in `Audio Files/` are compared with cycle 0. Any
difference is reported with the first differing JSON path. Nothing is deleted: each cycle gets
its own new folder under WORK_DIR, which must not exist yet.

Usage: roundtrip.py CLI_BINARY START_SCRAFT WORK_DIR [CYCLES=100]
Exit code 0 only if all cycles match cycle 0 exactly.
"""
import hashlib
import json
import os
import shutil
import subprocess
import sys


def sha(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def snapshot(scraft):
    folder = os.path.dirname(scraft)
    with open(scraft, encoding="utf-8") as f:
        doc = json.load(f)
    audio = {}
    adir = os.path.join(folder, "Audio Files")
    if os.path.isdir(adir):
        for name in sorted(os.listdir(adir)):
            audio[name] = sha(os.path.join(adir, name))
    return doc, audio


def first_diff(a, b, path="$"):
    if type(a) is not type(b):
        return f"{path}: type {type(a).__name__} != {type(b).__name__}"
    if isinstance(a, dict):
        for k in sorted(set(a) | set(b)):
            if k not in a or k not in b:
                return f"{path}.{k}: present in only one"
            d = first_diff(a[k], b[k], f"{path}.{k}")
            if d:
                return d
    elif isinstance(a, list):
        if len(a) != len(b):
            return f"{path}: length {len(a)} != {len(b)}"
        for i, (x, y) in enumerate(zip(a, b)):
            d = first_diff(x, y, f"{path}[{i}]")
            if d:
                return d
    elif a != b:
        return f"{path}: {a!r} != {b!r}"
    return None


def main(cli, start, work, cycles):
    if os.path.exists(work):
        sys.exit(f"{work} already exists; give a new folder")
    os.makedirs(work)
    c0 = os.path.join(work, "cycle_000")
    os.makedirs(c0)
    shutil.copy(start, os.path.join(c0, "s0.scraft"))
    src_audio = os.path.join(os.path.dirname(start), "Audio Files")
    shutil.copytree(src_audio, os.path.join(c0, "Audio Files"))
    base_doc, base_audio = snapshot(os.path.join(c0, "s0.scraft"))
    prev, bad = os.path.join(c0, "s0.scraft"), 0
    for n in range(1, cycles + 1):
        folder = os.path.join(work, f"cycle_{n:03d}")
        os.makedirs(folder)
        nxt = os.path.join(folder, "s0.scraft")
        r = subprocess.run([cli, "run", "--in", prev, "--save", nxt], capture_output=True, text=True)
        if r.returncode != 0:
            print(f"cycle {n}: CLI failed: {r.stderr.strip()[:200]}")
            bad += 1
            break
        doc, audio = snapshot(nxt)
        diff = first_diff(base_doc, doc)
        if diff or audio != base_audio:
            print(f"cycle {n}: MISMATCH {diff or 'audio file hashes differ: ' + str(sorted(set(audio.items()) ^ set(base_audio.items()))[:2])}")
            bad += 1
            break
        prev = nxt
    print(f"cycles run: {n}, mismatches: {bad}, audio files tracked: {len(base_audio)}")
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    if len(sys.argv) not in (4, 5):
        sys.exit(__doc__)
    main(sys.argv[1], sys.argv[2], sys.argv[3], int(sys.argv[4]) if len(sys.argv) == 5 else 100)
