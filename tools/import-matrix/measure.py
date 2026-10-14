#!/usr/bin/env python3
"""Measure the P0-T03 fixtures through `soundcraft-cli convert` (decode to 32-bit float WAV).

For each fixture this reports the decoded sample rate, channel count, frame count, the first
frame where the click appears (threshold 0.1 full scale) and the offset from the known click
position, and whether a left-only click stayed on the left channel.

Usage: measure.py CLI_BINARY FIXTURE_DIR WORK_DIR
Output: a markdown table on stdout; progress and failures on stderr.
"""
import os
import struct
import subprocess
import sys

CLICK_AT = 24000
THRESHOLD = 0.1


def read_float_wav(path):
    with open(path, "rb") as f:
        b = f.read()
    if b[:4] != b"RIFF" or b[8:12] != b"WAVE":
        raise ValueError("not a RIFF/WAVE file")
    pos, fmt, data = 12, None, None
    while pos + 8 <= len(b):
        cid, size = b[pos:pos + 4], struct.unpack_from("<I", b, pos + 4)[0]
        body = b[pos + 8:pos + 8 + size]
        if cid == b"fmt ":
            fmt = struct.unpack_from("<HHIIHH", body, 0)
        elif cid == b"data":
            data = body
        pos += 8 + size + (size & 1)
    if fmt is None or data is None:
        raise ValueError("missing fmt or data chunk")
    tag, ch, rate, _, _, bits = fmt
    if bits != 32 or tag not in (3, 0xFFFE):
        raise ValueError(f"expected 32-bit float, got tag={tag} bits={bits}")
    n = len(data) // 4
    samples = struct.unpack("<%df" % n, data[:n * 4])
    return rate, ch, samples


def onset(samples, ch, channel):
    frames = len(samples) // ch
    for i in range(frames):
        if abs(samples[i * ch + channel]) >= THRESHOLD:
            return i
    return None


def main(cli, fixtures, work):
    os.makedirs(work, exist_ok=True)
    print("| Fixture | Rate | Ch | Frames | Expected frames | Onset (L) | Offset vs click | Channel map |")
    print("|---|---|---|---|---|---|---|---|")
    for name in sorted(os.listdir(fixtures)):
        src = os.path.join(fixtures, name)
        out = os.path.join(work, name + ".f32.wav")
        want_rate = int(name.rsplit("_", 1)[1].split(".")[0])
        want_ch = 1 if "_mono_" in name else 2
        r = subprocess.run([cli, "convert", src, out, "--bit-depth", "32f"], capture_output=True, text=True)
        if r.returncode != 0:
            print(f"| {name} | – | – | – | – | – | – | **decode failed**: {r.stderr.strip()[:60]} |")
            print(f"FAILED {name}: {r.stderr.strip()}", file=sys.stderr)
            continue
        try:
            rate, ch, samples = read_float_wav(out)
        except ValueError as e:
            print(f"| {name} | – | – | – | – | – | – | **bad output**: {e} |")
            continue
        frames = len(samples) // ch
        expected = 2 * want_rate
        on_l = onset(samples, ch, 0)
        off = "none" if on_l is None else f"{on_l - CLICK_AT:+d}"
        if ch == 2 and want_ch == 2:
            on_r = onset(samples, ch, 1)
            cmap = "L only (correct)" if on_r is None else f"click also on R at {on_r}"
        elif ch != want_ch:
            cmap = f"**channels {want_ch} -> {ch}**"
        else:
            cmap = "mono"
        flag = "" if rate == want_rate else " **rate changed**"
        print(f"| {name} | {rate}{flag} | {ch} | {frames} | {expected} ({frames - expected:+d}) | {on_l} | {off} | {cmap} |")


if __name__ == "__main__":
    if len(sys.argv) != 4:
        sys.exit(__doc__)
    main(*sys.argv[1:])
