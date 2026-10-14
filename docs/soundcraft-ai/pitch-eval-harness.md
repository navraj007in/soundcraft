# Pitch evaluation harness

**Task:** P0-T08 (pitch benchmark) in the [execution plan](execution-plan.md) · **Spec:** PRD v0.3 §12.2, §12.3 · **Feeds:** G06, G07; plan tasks P1-A03, P2-Q01 to P2-Q03
**Version:** 0.2 · **Updated:** 14 October 2026 · **Status:** The **scoring harness and the classical probabilistic tracker (pYIN-style) are built and tested**. **The benchmark itself is not done:** it needs annotated natural-vocal recordings, which don't exist yet, and the neural candidate is not built (and only needed if the classical tracker falls short).
**Code:** the harness is crate `crates/pitch-eval` (`soundcraft-pitch-eval`, layer 1); the tracker is `crates/dsp/src/f0.rs` (branch `spike/pyin`).

## 1. What exists

`soundcraft-pitch-eval MANIFEST.json` scores a tracker on annotated vocal excerpts exactly as PRD
§12.2 defines scoring, and prints a report.

| PRD §12.2 rule | Where it is |
|---|---|
| Common 10 ms grid | `HOP_S`; every tracker returns one frame per grid step |
| No octave folding | `fifty_cents_is_the_line`, `octave_errors_are_counted_and_fail_the_score_without_folding` |
| An abstention on a reference-voiced frame fails the 50-cent score | `abstaining_on_a_voiced_frame_fails_it` |
| Octave errors: an estimate within 50 cents of an erroneous octave multiple; denominator is reference-voiced frames | `is_octave_error`, same tests |
| Primary scores include all adjudicated frames, transitions included | `Scores.primary` |
| Secondary interior score drops a fixed 20 ms either side of each voiced/unvoiced boundary and never replaces the primary | `Scores.interior`, `BOUNDARY_GUARD_S`; printed under "secondary" |
| Disputed annotations excluded before scoring, with their duration reported | manifest `excluded` ranges; "N s excluded as disputed" |
| Voicing precision and recall | `Metrics` |
| High-confidence coverage over all reference-voiced frames; error rate over high-confidence estimates on reference-voiced frames | `hc_coverage`, `hc_error` |
| Frame-pooled, per-singer, per-condition and singer-macro scores | `aggregate`, `singer_macro`, the report |
| 95% intervals from at least 2000 bootstrap resamples of singers, each sampled singer's excerpts kept together; method and seed named | `bootstrap`, `CI_METHOD`; the report prints both; fewer than 2000 prints a note |
| At least 10 excerpts and 3 singers per condition; each condition at least 85% | printed per condition with flags |
| Corpus minimums (≥160 excerpts, ≥16 singers, ≥30 min; holdout ≥80 excerpts from ≥8 singers) | a checklist at the end of the report |
| Holdout singers must not occur in threshold selection; holdout results must not tune thresholds | A singer in both splits is refused. Scoring the holdout (`--split holdout` or `all`) is refused unless `--frozen-thresholds LABEL` names the frozen, versioned thresholds, and the label is printed |
| Singers in a small corpus give wide intervals and no population claim | A warning prints when fewer than 8 singers are scored |

**Inputs.** A manifest (`corpus_version`, and per excerpt: `id`, `singer`, `split`, `audio`, `reference`,
`conditions`, `excluded` ranges) and, per excerpt, a reference file of `time_seconds,f0_hz` rows with
`f0_hz <= 0` meaning unvoiced. A value holds until the next row; the last row holds for one hop, so a
file should end with an unvoiced row. Malformed rows stop the run and name the file and line.
Audio is decoded with `soundcraft-audio-io` and folded to mono.

**Trackers.** A `Tracker` trait returns, per grid frame, an optional F0 and an optional confidence.
Implemented: **YIN** (the existing detector, 65 to 1100 Hz, threshold 0.15, 43 ms window) and the
**probabilistic tracker** (§6). YIN reports no confidence, so the high-confidence statistics read 0%
coverage and "n/a" error for it, which is correct and not a defect: G07's confidence requirements
need a tracker that outputs one. Choose with `--tracker yin|pyin` (default `yin`).

## 2. How it was tested

17 tests: 13 unit tests on the scoring and the bootstrap, and 4 end-to-end runs of the command line on
a generated corpus (dev run with seed and JSON output, the holdout guard, the singer-overlap refusal,
and bad input named by file and line).

**Mutation checks.** Six deliberate breaks of the scoring code, each caught by a test: the 50-cent
line moved, abstention not counted as a miss, octave folding, no boundary guard, the reference
shifted by 20 ms on the grid, and a bootstrap that does not resample.

**Two real defects found while testing the harness itself:**

1. A first line like `-1,100` (numeric but invalid) was accepted as a "header" by the reference
   parser. Only a non-numeric first line is a header now.
2. The first version of the 50-cent test used +60 cents, which f32 rounding put so close to a
   mutated limit of 60 that moving the limit changed nothing. The tests now straddle the line at
   49 and 51 cents (and the octave tolerance the same way), and they fail with the limit at 40 or 60.

## 3. A smoke run on synthetic audio (implementation check only)

The 60 s synthetic voice-like take used for the resource measurements (harmonic tone, 5.5 Hz vibrato,
phrases and pauses, notes changing every 2 s) was scored with ground truth taken from its generator;
the amplitude ramps at phrase edges and the note jumps were marked as excluded (6.56 s). YIN scored
100% within 50 cents, median error 2.7 cents, 0% octave errors, 99.1% voicing precision, 100% recall.

**This says nothing about natural voices.** A clean harmonic tone is the easiest case there is, and
PRD §12.2 says synthetic tones test the implementation and must not raise natural-vocal scores. It
shows that the manifest, decoding, scoring, bootstrap and report work together.

## 4. What is still needed to finish P0-T08

| Need | Why | Who |
|---|---|---|
| **20 to 30 natural vocal excerpts** with reference F0 and voiced boundaries, from several singers, including low and high registers, breathiness, slides and ornamentation. For this small Phase 0 benchmark, development data only | Without natural audio there is nothing to score | You (recordings you have permission to use, or a licensed dataset) |
| **Reference annotations** checked independently, disputed frames marked before scoring | PRD §12.2 | A second annotator; a tool-assisted first pass is fine but must be reviewed |
| ~~A pYIN-style tracker~~ | **Built (§6)**; it still has to be scored on natural recordings | Done; scoring needs the recordings |
| **One neural candidate exported to ONNX and run on `tract`**, only if the classical tracker misses the targets | See [packaging-and-dependencies.md](packaging-and-dependencies.md) §4; weights licences still unverified | Later |
| Licence and consent terms for any recording or dataset | Commercial-use, redistribution and participant consent are separate (PRD §12.2) | You |

Datasets exist for this purpose, but I did not download or evaluate any: their licences differ, some
exclude commercial use, and the choice belongs with the corpus decision above.

## 5. Running it

```
cargo run --release -p soundcraft-pitch-eval -- path/to/manifest.json \
    [--split dev|holdout|all] [--frozen-thresholds LABEL] \
    [--tracker yin|pyin] [--bootstrap 2000] [--seed 20261014] [--hc-threshold 0.8] [--json out.json]
```

## 6. The probabilistic tracker (`soundcraft_dsp::f0`)

An original implementation of the pYIN idea (Mauch and Dixon, 2014) from its published description:

- **Candidates.** For each 10 ms frame, YIN's difference function is evaluated once; 100 thresholds drawn
  from a Beta(2, 18) prior (mean 0.1) each pick a pitch candidate, so a frame gets a set of weighted
  candidates and a total "voiced mass".
- **Model.** A hidden Markov model with a **voiced and an unvoiced state for every 10-cent pitch bin**
  (about 491 bins from 65 to 1100 Hz). Pitch drifts by a triangular step of at most ±100 cents per frame,
  or **jumps** to any pitch with probability 0.02; voiced and unvoiced switch with probability 0.02.
- **Output per frame:** a decoded pitch (Viterbi), a `voicing_score` and an `f0_score` (the share of the model's
  belief within 50 cents of the decoded pitch). **Both scores are uncalibrated rankings, not probabilities**
  (PRD §8.3); thresholds on them are to be fixed on development data before any holdout is scored. The
  harness uses their product as its single "confidence".
- **Safety.** Non-finite samples count as zero; unsupported rates, empty input, bad settings and
  signals over 600 s return nothing or an error; nothing panics. Memory is about 35 MB per minute of audio
  (about 180 MB at the 5-minute product limit).

**Two real defects found and fixed while building it, both by tests that failed:**

1. **A single shared "unvoiced" state** (my first design) let the voiced path pay the cost of pitch drift each
   frame while the unvoiced path paid none. In moderate noise unvoiced won every frame, so a clear tone at about 14 dB SNR
   scored 0%. The published design gives unvoiced a state per pitch bin too; the rebuild fixed it.
2. **Note changes were ramped.** With drift alone, a jump of a fifth was followed over 6 frames through pitches nobody sang
   (212, 224, 237 Hz and so on), costing about 60 ms of wrong frames per large note change, and PRD §12.2 scores transitions
   in the primary result. The small jump probability above removes it: the contour now moves to the new pitch at once
   (0 frames in between), and the pitch score dips exactly at the change and recovers.

**Verification.** 20 unit tests (steady, glissando, vibrato, silence, onsets and offsets, low and high registers,
noise, tone in noise, a pitch step landing on the right frame, pitch refined to under 2 cents between bin centres, hostile
input, bad settings, determinism) and one **exact check**: on a tiny model, every possible state sequence is enumerated and
the tracker's voicing score and decoded path are compared with the exact posterior, to within 0.002. Thirteen deliberate
breaks of the model were tried and twelve make a test fail. The thirteenth (keeping candidates outside the pitch
range) cannot happen, because the lag search already bounds them. **Five breaks survived the first rounds of tests**
(no voiced mass, a window not centred on the frame, pitch not refined, a pitch-score window far too wide, a backward pass
that ignored jumps); each gap got a test, and all five are now caught.

**Where it helps, on synthetic tones (an implementation check, not accuracy evidence):**

| Case | YIN within 50 cents | pYIN within 50 cents |
|---|---|---|
| clean steady 220 Hz | 99.6% | 100.0% |
| steady, 10 dB SNR | 99.2% | 99.6% |
| **steady, 5 dB SNR** | **0.0%** | **99.2%** |
| steady, 0 dB SNR | 0.0% | 0.0% |
| vibrato ±40 cents, 10 dB | 99.2% | 99.6% |
| glissando 1.5 octaves, 10 dB | 98.8% (0.4% octave errors) | 99.6% (0%) |
| phrases with pauses, 10 dB | 93.8% | 97.9% |
| low register 72 Hz, 10 dB | 86.9% | 89.2% |
| white noise only | 0 frames voiced | 0 frames voiced |

Limits to state plainly: **neither tracker does anything useful at 0 dB SNR**, and **low registers are weak for both**
(the 43 ms window holds only about three periods at 72 Hz; a longer window for low voices is the obvious thing to
try). On the 60 s synthetic take the CLI run (decode, track, score, 2000-resample bootstrap) took 2.5 s and 53 MiB for pYIN
and 4.2 s and 30 MiB for YIN.

**What this does not show:** how either tracker behaves on real voices (breath, vocal fry, ornaments, mixed
registers, room noise), which is what G06 and G07 test.

## Revision history

| Version | Date | Change |
|---|---|---|
| 0.1 | 14 October 2026 | Harness, tests and synthetic smoke run (P0-T08, scoring half). |
| 0.2 | 14 October 2026 | Probabilistic tracker, its two fixed defects, verification and the synthetic comparison. |
