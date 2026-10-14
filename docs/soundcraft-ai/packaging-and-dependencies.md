# Packaging, dependencies and resource budget

**Task:** P0-T10 in the [execution plan](execution-plan.md) · **Feeds:** PRD §10.3, §11.3, G10, G13; plan decisions D1, D2 · **Version:** 0.1 · **Updated:** 14 October 2026
**Status:** Runtime choice made; licence inventory done for the Rust dependencies; model-weight licences **not** settled; neural-model resource numbers **not** measured (they need P0-T08).
**Measured on:** the M4 inspection host (macOS 27.0.1, 16 GB), release build of `soundcraft-cli` on Rust 1.97.0. **Not the M1 reference host.**

## 1. Decision: write the companion in Rust

| Reason | Evidence |
|---|---|
| The policy, DSP, audio decode and session code are already Rust and reusable by path dependency | `soundcraft-assist-policy` (P0-T09) has 13 external packages, all MIT or Apache style; `soundcraft-dsp` holds the YIN detector and meters |
| No Python environment to package (PRD §10.3) | The existing app is one Rust binary, signed with the hardened runtime |
| Fits the never-crash rules | `unsafe_code = "forbid"`, panic lints; a Python sidecar would sit outside them |
| The analysis workload is small on the existing code | §2 below |
| A neural model, if one is chosen, can run without a native library | `tract` is pure Rust (§4) |

Rejected: a **Python/PyTorch sidecar** (a large, hard-to-sign runtime, and the PRD says users must not
install Python). **Not decided:** whether any neural pitch model is needed at all. That depends on how
a classical tracker scores on the P0-T08 corpus (decision D2).

## 2. Measured resources (existing Rust code)

All figures are one run each, so treat them as order of magnitude, not p95. The workload is a 48 kHz
16-bit WAV: a 60 s synthetic voice-like take (vibrato, pauses) and the same take tiled into a 10 min
stereo programme.

| Workload | Wall time | Peak footprint | Max RSS |
|---|---|---|---|
| `soundcraft-cli --version` (process baseline) | < 0.01 s | 2 MiB | 7 MiB |
| Import 60 s mono vocal | 0.01 s | 20 MiB | 26 MiB |
| **Existing YIN note detection over the 60 s vocal** | **2.8 s** | **42 MiB** | **48 MiB** |
| Import 10 min stereo programme | 0.13 s | 350 MiB | 357 MiB |
| Import it, then bounce the whole mix | 1.4 s | 625 MiB | 632 MiB |
| **8 stereo tracks × 10 min, import** | 1.2 s | **2019 MiB** | 1535 MiB |
| **8 stereo tracks × 10 min, import and bounce** | 4.5 s | **2295 MiB** | 2135 MiB |

What they say:

- **Pitch job budget.** PRD §11.3 wants a 60 s pitch job in 30 s at p95 and 2 GiB for the companion
  and worker together. The existing YIN path used 2.8 s and 48 MiB on a 60 s clip, with ample
  room. This says nothing about accuracy (synthetic tones don't count toward G06) and nothing about
  a neural model, which is the open cost.
- **A 5 minute mono analysis snapshot** (the PRD's per-job limit) is about 58 MB as 32-bit float,
  so passing the dry vocal to the companion is cheap. The companion never needs the whole programme.
- **The app is the memory-heavy part, not the companion.** The PRD's maximum session (8 tracks of
  10 minutes) held as decoded float costs about 2.0 GiB of footprint in the engine process and about
  2.3 GiB when bouncing. That is not inside the 2 GiB companion budget, but it is in the G10 8-track
  workload and it is the `docs/gaps.md` G-06 "memory-bound media" gap showing up. It fits a 16 GB host
  comfortably; it would not fit a small one. Record it in the G10 evidence.
- **Binary size:** `soundcraft-cli` is 7.7 MiB (arm64 release). A companion that reuses these crates
  should be of similar size before any model.
- **Cold start of a model** (PRD: ≤ 20 s) and **neural inference time** are not measured here.

## 3. Dependency licence inventory (Rust)

Method: `cargo tree -e normal --target aarch64-apple-darwin` on the shipping binaries, with each
package's declared SPDX licence; `cargo metadata` for the whole lockfile.

| Build graph | Unique external packages | Permissive or dual with a permissive choice | MPL-2.0 | Copyleft-only |
|---|---|---|---|---|
| macOS app `soundcraft` | 223 | 207 | 16 | 0 |
| `soundcraft-cli` | 84 | 68 | 16 | 0 |
| `soundcraft-assist-policy` | 13 | 13 | 0 | 0 |

The whole lockfile (all platforms, dev tools included) has 524 packages and every one declares a
licence. Items that need an action or a decision:

1. **symphonia 0.5.5, 16 crates, MPL-2.0.** It decodes MP3, AAC and the other compressed formats. MPL-2.0
   is copyleft per file: using it unmodified needs only the licence notice and a pointer to its
   source. **Modifying any of its files obliges us to publish those modified files under MPL-2.0.**
   That matters for P1-E07 (AAC start trim): see decision D10.
2. **`self_cell`: Apache-2.0 OR GPL-2.0-only.** Choose Apache-2.0 and record the choice in the notices.
3. **`epaint_default_fonts`: (MIT OR Apache-2.0) AND OFL-1.1 AND Ubuntu-font-1.0.** The fonts are compiled into
   the binary, so the OFL and Ubuntu Font Licence texts have to ship in the notices. They are not rows
   in `ATTRIBUTION.md` today (they are a crate's, not repo assets); P3-D02 should add them to the
   notices bundle.
4. **Dual licences with an LGPL branch** (`MIT OR Apache-2.0 OR LGPL-2.1-or-later`, two crates) are in the
   lockfile but not in the macOS app graph. Take the MIT choice if they ever enter it.
5. **Codec patents are not a copyright-licence question and are not covered here.** MP3 and AAC
   decode in a commercial product should get a legal check in P3-D02. This document states no position.
6. **Bundled assets and model weights** are separate from the crate list: weights need their own
   inventory (§4) and every shipped file needs an `ATTRIBUTION.md` row (AGENTS.md).

`cargo-deny` or an equivalent should run in CI so a new copyleft-only dependency fails the build; not
set up yet (add to P3-D02, or earlier if a new dependency is added in Phase 1).

## 4. Pitch model and runtime options

Code licences below were read from each repository's own `LICENSE` file (14 October 2026, fetched
through a summarising tool, so quotes are second-hand). **Weights and training-data terms were not
verified for any of them.**

| Option | What it is | Code licence | Weights / data | Packaging cost |
|---|---|---|---|---|
| **Classical: YIN plus smoothing (pYIN-style)** | Existing Rust YIN, plus our own probabilistic smoothing and a confidence value | Ours (MIT OR Apache-2.0) | None | None |
| **CREPE** (marl/crepe) | Convolutional F0 model on the waveform | MIT (Jong Wook Kim, 2018) | Weights are ports and conversions of the original; no primary statement of the weights' licence was found | Needs an ONNX export; model file |
| **PENN** (interactiveaudiolab/penn) | Pitch and periodicity, FCNF0++ | MIT (Interactive Audio Lab, 2022) | Pretrained on MDB-stem-synth and PTDB per secondary sources; those datasets' terms not checked | Needs an ONNX export; model file |
| **FCPE** (CNChTu/FCPE) | Fast context-based pitch estimator | MIT (CN_ChiTu, 2023) | Weights come via a pip package; licence text not retrieved | Needs an ONNX export; model file |
| **RMVPE** (Dream-High/RMVPE) | Vocal pitch for singing voice | Apache-2.0 | Not checked | Needs an ONNX export; model file |

Runtimes (for any neural option):

| Runtime | Licence (primary file) | Native library to bundle | Note |
|---|---|---|---|
| **`tract`** (Sonos) | MIT, also Apache-2.0 | None, pure Rust | Preferred: nothing extra to sign; supports ONNX; speed on our model unmeasured |
| **`ort`** (pykeio) | MIT (pyke.io, 2023–2026; Nicolas Bigaouette, 2020) | Yes, the ONNX Runtime C++ library | Faster on large models; adds a dylib to bundle and sign. The wrapped runtime's own licence was not checked |

Not options (copyleft or unknown, per the project rule against GPL/AGPL/LGPL code): aubio, Essentia,
Praat, the Vamp pYIN plugin and Tony. I did not verify their current licences in this pass; they are
excluded by the rule and by prior knowledge, which should be re-checked if one is ever reconsidered.

PRD §12.2 also requires disclosing unknown overlap between a pretrained model's training data and the
evaluation singers, so any neural choice carries a data-provenance line item in the G06 report.

**Recommended order of work:** (1) build the classical tracker first; (2) score it on the P0-T08
corpus; (3) only if it misses G06/G07 by a margin, export one neural candidate to ONNX, run it on
`tract`, and measure cold load (≤ 20 s), 60 s inference (≤ 30 s at p95) and memory (≤ 2 GiB with the
companion) before spending effort on licences for the weights.

## 5. Packaging the companion in the macOS app

Current state (`packaging/macos/`): `package.sh` builds the binaries for the target architectures,
joins them with `lipo`, makes `SoundCraft.app` with the single executable
`Contents/MacOS/SoundCraft`, signs with `--options runtime` and `entitlements.plist` (audio input only;
the app is not sandboxed), notarises the zip, staples, builds the DMG and signs `soundcraft-cli`
separately. Nothing may touch the bundle after signing (`codesign --verify --strict --deep`).

What a companion adds:

| Change | Where | Why |
|---|---|---|
| Build `soundcraft-assist` with the others and `lipo` it | `package.sh` (the `-p …` list and the `for bin in` loop) | Same build path |
| Put it at `Contents/MacOS/soundcraft-assist` (or `Contents/Helpers/`) | `package.sh`, before signing | The app launches it as a child |
| Sign it **before** the outer app, with `--options runtime` | `package.sh` signing lines | Nested code must be signed inside-out for notarisation |
| Entitlements: **none new** | `entitlements.plist` | Not sandboxed, so outbound network (cloud text adapter) needs no entitlement. The companion does not capture audio, so it doesn't get `device.audio-input`. Add nothing without a comment, as that file requires |
| Models under `Contents/Resources/models/` with a checksum manifest | `package.sh`, `Info`/notices | PRD §10.3 records checksums and licences; resources are sealed by the signature |
| Notices bundle (licences, fonts, MPL source pointer, model terms) | New file under `Contents/Resources/` | G13 |
| API-key storage in the macOS Keychain | Companion code (P1-M03) | PRD §11.1. The Keychain ties access to the code signature, so ad-hoc local builds behave differently from signed releases; test both |
| Token file for the control channel in a private folder | Companion code (P1-E03) | See [control-channel-protection.md](control-channel-protection.md) |

If `ort` is chosen later: add the ONNX Runtime dylib under `Contents/Frameworks/`, sign it, and check
that library validation still passes. `tract` avoids all of that.

The Apple Silicon-only envelope (PRD §4.2) means the companion can be built arm64-only; the existing
script builds universal binaries, so either keep that or limit the companion's target list. A decision
for P3-D01, with no effect before then.

## 6. Decisions and open items

| # | Item | Position | Where it lands |
|---|---|---|---|
| D1 | Companion language | **Resolved: Rust.** ONNX engine, if needed: `tract` first | This note |
| D2 | Which F0 tracker | Classical first; neural only if the P0-T08 scores need it | P0-T08 |
| D10 | Fixing AAC start trim without modifying MPL-2.0 files | Prefer to read the container's gapless data ourselves in `soundcraft-audio-io` (the edit list and iTunSMPB) and trim after decode, which leaves symphonia unmodified. If a symphonia patch turns out simpler, publish the changed files under MPL-2.0 and record that in the notices | P1-E07 |
| – | Model-weight and training-data licences | Not verified for any model; required before any weights ship | Only if a neural option is chosen (P0-T08), then P3-D02 |
| – | Codec patents for MP3 and AAC decode | Out of scope for this note | Legal check in P3-D02 |
| – | `cargo-deny` in CI | Not set up | Before the first new dependency in Phase 1, or by P3-D02 |
| – | Memory of the 8-track, 10-minute envelope in the engine process | About 2.0 to 2.3 GiB measured | G10 evidence; gaps.md G-06 |
| – | Cold model load and neural inference time | Unmeasured | P0-T08 |

## Revision history

| Version | Date | Change |
|---|---|---|
| 0.1 | 14 October 2026 | First pass (P0-T10): runtime decision, measurements, licence inventory, model and runtime options, packaging plan. |
