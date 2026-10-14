# SoundCraft-AI import matrix

**Task:** P0-T03 in the [execution plan](execution-plan.md) · **Feeds:** PRD G01, FR-11, §4.2
**Version:** 0.1 · **Updated:** 14 October 2026 · **Status:** Measured on the M4 inspection host, **not the M1 reference host**. Repeat on the reference host before citing as G01 evidence.

## Method

- **Fixtures:** `tools/import-matrix/gen_fixtures.py` writes 32 two-second files at 44.1 and 48 kHz,
  mono and stereo. Each is silence plus one 48-sample click whose first sample is at **frame
  24000**. The stereo click is on the **left channel only**, so a swapped or folded channel map
  shows. Fixtures are generated, not committed.
- **Encoders:** WAV (16 and 24-bit PCM, 32-bit float); MP3 by LAME 128 kbps CBR and V2 VBR (both
  write a LAME/Xing info tag with delay and padding); MP3 by ffmpeg with the tag disabled; AAC-LC in
  M4A by macOS `afconvert` (writes an iTunSMPB entry and an edit list) and by ffmpeg's AAC encoder.
- **Decode path under test:** `soundcraft-cli convert IN OUT --bit-depth 32f` (release build,
  Rust 1.97.0, commit `8166be9`, code identical to pinned `6cf896e`). This is the same
  `soundcraft-audio-io` decoder the engine's import uses. The matrix does **not** yet cover the
  engine's placement of the decoded clip on the timeline (see "Not covered").
- **Measurement:** `tools/import-matrix/measure.py`: first frame where |sample| ≥ 0.1 FS, compared
  with frame 24000, plus frame count and which channel holds the click.
- **Cross-check:** the AAC and untagged-MP3 mono fixtures were decoded with ffmpeg, an independent
  decoder that applies edit lists and gapless tags, and measured the same way.
- **Smear:** lossy codecs spread the click by a few samples, so offsets of ±2 are codec smear, not
  timing error.

## Results

| Format | Fixtures | Rate and channels kept | Duration | Onset offset | Channel map | Verdict |
|---|---|---|---|---|---|---|
| WAV PCM 16, PCM 24, float 32 | 12 | ✅ | exact | 0 | ✅ | **Pass** |
| MP3, LAME tag, CBR 128 | 4 | ✅ | exact (0) | −1 to −2 (smear) | ✅ | **Pass** |
| MP3, LAME tag, VBR V2 | 4 | ✅ | exact (0) | 0 to −1 (smear) | ✅ | **Pass** |
| MP3, **no** tag (CBR 128) | 4 | ✅ | +1656 (44.1 kHz) / +1920 (48 kHz) frames | **+1103 to +1104** | ✅ | **Inherent**: see below |
| AAC-LC M4A, macOS `afconvert` | 4 | ✅ | +2936 (44.1 kHz) / +2304 (48 kHz) frames | **+2111 to +2112** | ✅ | **Fail** |
| AAC-LC M4A, ffmpeg encoder | 4 | ✅ | +1912 (44.1 kHz) / +1280 (48 kHz) frames | **+1023 to +1024** | ✅ | **Fail** |

Findings:

1. **AAC priming is not removed.** The offsets equal the encoder's priming samples (2112 for
   `afconvert`, 1024 for ffmpeg). ffmpeg decodes both files with the click at **exactly 24000**
   (offset 0), so the files carry the information. symphonia 0.5.5's MP4 reader parses the edit
   list (`atoms/elst.rs`), but the decode here shows no start trim. A clip imported from an AAC
   backing would sit 23 to 48 ms **late** at 44.1/48 kHz, which fails G01's "timeline alignment"
   for AAC. A repair is needed; see P1-E07.
2. **MP3 with a LAME tag is correct.** `enable_gapless` is working: durations are exact and onsets
   are within smear. VBR is handled.
3. **MP3 without a tag is +1104 samples, in ffmpeg too.** That is the standard LAME encoder delay
   (576) plus decoder delay (528 or 529), and nothing in the file declares it. The default
   behaviour (don't guess) is defensible. Whether to apply a fixed assumed offset to untagged MP3
   is a product decision (D9 in the plan); the import report should state which case applied.
4. **Channel mapping is right everywhere.** The left-only click stays left in all 16 stereo
   fixtures; rate and channel count are preserved in all 32.
5. **No decode failures** across the 32 files.

## Save and reopen

`tools/import-matrix/roundtrip.py`: a session with one 24-bit WAV stereo source and one LAME MP3
stereo source (48 kHz) was saved, then opened and saved to a new folder 100 times in a row.

- **Result:** 100 cycles, 0 mismatches. After every cycle the session JSON equals cycle 0 (all
  fields) and the SHA-256 of both files in `Audio Files/` is unchanged.
- **Compressed sources are decoded once into project PCM.** On save, the MP3 source was written as
  `Audio Files/<name>.wav` and recorded in the session with `"format": "Wav"`. This matches the
  PRD's "decode once into project-managed PCM" (§4.2). The original MP3's identity is **not** kept
  in the session (the source entry holds the new `.wav` path only), so PRD §4.2's "retain its source
  identity" is **not met**: another repair item.
- **Harness check:** a first attempt flagged a difference because my script renamed the file and
  the session name follows the file name. That shows the JSON comparison detects changes. The
  audio-hash comparison was not separately shown to detect a change.

## Not covered (still open for G01)

- **Engine placement on the timeline:** only the decoder was measured. Place an imported MP3 and
  AAC clip at a known position, bounce, and measure the onset in the bounce.
- **Trim, sample-rate conversion and session-rate mismatch** (for example 44.1 kHz file into a
  48 kHz session) with onset measurement.
- **Other AAC profiles and containers:** HE-AAC, `.aac` ADTS, `.mp4`, and `afconvert`/ffmpeg files
  with the edit list or iTunSMPB stripped.
- **Corrupt, truncated and zero-length files.** `crates/audio-io/tests/malformed.rs` exists, but
  its assertions weren't checked here.
- **WAV edge cases:** extensible headers, odd chunk layouts, RF64, 8-bit.
- **Mono-to-stereo and multichannel mapping** beyond mono and stereo (PRD limits to 8 tracks).
- **Reference host.** All of the above ran on an M4, macOS 27.0.1, not the M1.

## Repairs named by this task

| Item | Finding | Proposed task | Rough size (guess, to confirm) |
|---|---|---|---|
| AAC start trim | Priming samples kept: +1024 / +2112 | P1-E07: honour the MP4 edit list and iTunSMPB when decoding, with fixtures; first look at whether a symphonia update or a small local patch is simpler | 6–12 h |
| Source identity | Original compressed file's identity is lost on save | P1-E07: keep original name, size and hash in the session's source entry or the sidecar | 3–6 h |
| Untagged MP3 | Inherent +1104 | Decision D9, then a fixed offset or a user-visible warning | 2–4 h |

## Full measured table

Generated by `measure.py` on 14 October 2026. "Frames" is the decoded length; "Offset vs click"
is the detected onset minus frame 24000.

| Fixture | Rate | Ch | Frames | Expected frames | Onset (L) | Offset vs click | Channel map |
|---|---|---|---|---|---|---|---|
| aac_afconvert_mono_44100.m4a | 44100 | 1 | 91136 | 88200 (+2936) | 26112 | +2112 | mono |
| aac_afconvert_mono_48000.m4a | 48000 | 1 | 98304 | 96000 (+2304) | 26112 | +2112 | mono |
| aac_afconvert_stereo_44100.m4a | 44100 | 2 | 91136 | 88200 (+2936) | 26111 | +2111 | L only (correct) |
| aac_afconvert_stereo_48000.m4a | 48000 | 2 | 98304 | 96000 (+2304) | 26111 | +2111 | L only (correct) |
| aac_ffmpeg_mono_44100.m4a | 44100 | 1 | 90112 | 88200 (+1912) | 25024 | +1024 | mono |
| aac_ffmpeg_mono_48000.m4a | 48000 | 1 | 97280 | 96000 (+1280) | 25024 | +1024 | mono |
| aac_ffmpeg_stereo_44100.m4a | 44100 | 2 | 90112 | 88200 (+1912) | 25023 | +1023 | L only (correct) |
| aac_ffmpeg_stereo_48000.m4a | 48000 | 2 | 97280 | 96000 (+1280) | 25023 | +1023 | L only (correct) |
| mp3_cbr128_lame_mono_44100.mp3 | 44100 | 1 | 88200 | 88200 (+0) | 23999 | -1 | mono |
| mp3_cbr128_lame_mono_48000.mp3 | 48000 | 1 | 96000 | 96000 (+0) | 23999 | -1 | mono |
| mp3_cbr128_lame_stereo_44100.mp3 | 44100 | 2 | 88200 | 88200 (+0) | 23998 | -2 | L only (correct) |
| mp3_cbr128_lame_stereo_48000.mp3 | 48000 | 2 | 96000 | 96000 (+0) | 23998 | -2 | L only (correct) |
| mp3_cbr128_notag_mono_44100.mp3 | 44100 | 1 | 89856 | 88200 (+1656) | 25104 | +1104 | mono |
| mp3_cbr128_notag_mono_48000.mp3 | 48000 | 1 | 97920 | 96000 (+1920) | 25104 | +1104 | mono |
| mp3_cbr128_notag_stereo_44100.mp3 | 44100 | 2 | 89856 | 88200 (+1656) | 25103 | +1103 | L only (correct) |
| mp3_cbr128_notag_stereo_48000.mp3 | 48000 | 2 | 97920 | 96000 (+1920) | 25103 | +1103 | L only (correct) |
| mp3_vbrV2_lame_mono_44100.mp3 | 44100 | 1 | 88200 | 88200 (+0) | 24000 | +0 | mono |
| mp3_vbrV2_lame_mono_48000.mp3 | 48000 | 1 | 96000 | 96000 (+0) | 23999 | -1 | mono |
| mp3_vbrV2_lame_stereo_44100.mp3 | 44100 | 2 | 88200 | 88200 (+0) | 24000 | +0 | L only (correct) |
| mp3_vbrV2_lame_stereo_48000.mp3 | 48000 | 2 | 96000 | 96000 (+0) | 23999 | -1 | L only (correct) |
| wav_f32_mono_44100.wav | 44100 | 1 | 88200 | 88200 (+0) | 24000 | +0 | mono |
| wav_f32_mono_48000.wav | 48000 | 1 | 96000 | 96000 (+0) | 24000 | +0 | mono |
| wav_f32_stereo_44100.wav | 44100 | 2 | 88200 | 88200 (+0) | 24000 | +0 | L only (correct) |
| wav_f32_stereo_48000.wav | 48000 | 2 | 96000 | 96000 (+0) | 24000 | +0 | L only (correct) |
| wav_pcm16_mono_44100.wav | 44100 | 1 | 88200 | 88200 (+0) | 24000 | +0 | mono |
| wav_pcm16_mono_48000.wav | 48000 | 1 | 96000 | 96000 (+0) | 24000 | +0 | mono |
| wav_pcm16_stereo_44100.wav | 44100 | 2 | 88200 | 88200 (+0) | 24000 | +0 | L only (correct) |
| wav_pcm16_stereo_48000.wav | 48000 | 2 | 96000 | 96000 (+0) | 24000 | +0 | L only (correct) |
| wav_pcm24_mono_44100.wav | 44100 | 1 | 88200 | 88200 (+0) | 24000 | +0 | mono |
| wav_pcm24_mono_48000.wav | 48000 | 1 | 96000 | 96000 (+0) | 24000 | +0 | mono |
| wav_pcm24_stereo_44100.wav | 44100 | 2 | 88200 | 88200 (+0) | 24000 | +0 | L only (correct) |
| wav_pcm24_stereo_48000.wav | 48000 | 2 | 96000 | 96000 (+0) | 24000 | +0 | L only (correct) |

## Revision history

| Version | Date | Change |
|---|---|---|
| 0.1 | 14 October 2026 | First measurement (P0-T03) on the M4 inspection host. |
