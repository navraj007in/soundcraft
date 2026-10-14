#!/usr/bin/env python3
"""Generate the P0-T03 import fixtures.

Every fixture is a short synthetic signal: silence with one 1 ms full-scale click burst whose
first sample is at a known position (CLICK_AT), so a decoder's added or removed samples show up
as an onset offset. Fixtures are generated into the directory given on the command line and are
never committed (see docs/soundcraft-ai/import-matrix.md for the results).

Usage: gen_fixtures.py OUT_DIR
Needs: python3 (stdlib only), ffmpeg, lame, afconvert (macOS).
"""
import math
import os
import struct
import subprocess
import sys
import wave

CLICK_AT = 24000  # sample index of the first click sample
LENGTHS = {44100: 2 * 44100, 48000: 2 * 48000}  # two seconds
CLICK_LEN = 48  # samples; about 1 ms


def click_frames(rate, channels, kind):
    """Return interleaved float samples in [-1, 1]. `kind` is 'mono', 'stereo' or 'lr' (left and
    right differ, so a swapped or folded channel map is detectable)."""
    n = LENGTHS[rate]
    out = []
    for i in range(n):
        v = 0.0
        if CLICK_AT <= i < CLICK_AT + CLICK_LEN:
            # decaying 2 kHz burst starting at full scale
            t = i - CLICK_AT
            v = math.cos(2 * math.pi * 2000 * t / rate) * math.exp(-t / 24.0) * 0.9
        if channels == 1:
            out.append(v)
        elif kind == "lr":
            out.extend([v, 0.0])  # click on the left channel only
        else:
            out.extend([v, v])
    return out


def write_wav(path, rate, channels, kind, subtype):
    data = click_frames(rate, channels, kind)
    if subtype == "f32":
        # wave can't write float; emit a WAVE_FORMAT_IEEE_FLOAT file by hand
        payload = struct.pack("<%df" % len(data), *data)
        fmt = struct.pack("<HHIIHHH", 3, channels, rate, rate * channels * 4, channels * 4, 32, 0)
        with open(path, "wb") as f:
            f.write(b"RIFF" + struct.pack("<I", 4 + 8 + len(fmt) + 8 + len(payload)) + b"WAVE")
            f.write(b"fmt " + struct.pack("<I", len(fmt)) + fmt)
            f.write(b"data" + struct.pack("<I", len(payload)) + payload)
        return
    width = {"pcm16": 2, "pcm24": 3}[subtype]
    scale = float(2 ** (8 * width - 1) - 1)
    raw = bytearray()
    for v in data:
        q = int(round(v * scale))
        raw += q.to_bytes(width, "little", signed=True)
    with wave.open(path, "wb") as w:
        w.setnchannels(channels)
        w.setsampwidth(width)
        w.setframerate(rate)
        w.writeframes(bytes(raw))


def run(cmd):
    subprocess.run(cmd, check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def main(out):
    os.makedirs(out, exist_ok=True)
    for rate in (44100, 48000):
        for ch, kind, tag in ((1, "mono", "mono"), (2, "lr", "stereo")):
            for sub in ("pcm16", "pcm24", "f32"):
                write_wav(os.path.join(out, f"wav_{sub}_{tag}_{rate}.wav"), rate, ch, kind, sub)
    # compressed fixtures start from the 16-bit WAV of the same shape
    for rate in (44100, 48000):
        for ch, tag in ((1, "mono"), (2, "stereo")):
            src = os.path.join(out, f"wav_pcm16_{tag}_{rate}.wav")
            base = os.path.join(out, f"{{}}_{tag}_{rate}")
            # MP3 with LAME: CBR 128 and VBR V2 (LAME writes an Xing/Info tag with delay/padding)
            run(["lame", "--silent", "-b", "128", "--cbr", src, base.format("mp3_cbr128_lame") + ".mp3"])
            run(["lame", "--silent", "-V2", src, base.format("mp3_vbrV2_lame") + ".mp3"])
            # MP3 from ffmpeg without the LAME info tag: no declared delay or padding
            run(["ffmpeg", "-y", "-loglevel", "quiet", "-i", src, "-c:a", "libmp3lame", "-b:a", "128k",
                 "-write_xing", "0", base.format("mp3_cbr128_notag") + ".mp3"])
            # AAC-LC in M4A: macOS encoder (writes iTunSMPB / edit list), and ffmpeg's encoder
            run(["afconvert", "-f", "m4af", "-d", "aac", "-b", "128000", src, base.format("aac_afconvert") + ".m4a"])
            run(["ffmpeg", "-y", "-loglevel", "quiet", "-i", src, "-c:a", "aac", "-b:a", "128k",
                 base.format("aac_ffmpeg") + ".m4a"])
    print("fixtures written to", out)


if __name__ == "__main__":
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    main(sys.argv[1])
