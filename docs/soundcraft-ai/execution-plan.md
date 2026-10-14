# SoundCraft-AI execution plan

**Implements:** [SoundCraft-AI Product Requirements v0.3](../../SoundCraft_AI_PRD_v0.3.docx) (14 October 2026)
**Plan version:** 0.15 · **Updated:** 14 October 2026 · **Status:** Draft. Phase 0 has not started.
**Baseline:** fork `navraj007in/soundcraft`, `main` at `6cf896e` (the PRD's pinned 0.6.1 rebuild).

This plan breaks the PRD into phases, work packages and tasks, and ties each one to the
requirements (FR-xx) and release gates (Gxx) it serves. The PRD is the source of truth. If this plan
and the PRD disagree, the PRD wins, and a scope change needs a versioned PRD amendment (PRD §13.5).

> **ID collision warning.** `docs/gaps.md` uses inherited parity-gap IDs written `G-01`, `G-02`…
> (with a hyphen). PRD release gates are written `G01`, `G02`… (no hyphen). They are unrelated:
> gaps.md `G-01` is "Audio interface support" and PRD `G01` is "Import and persistence". This plan
> always writes "gaps.md G-xx" for the inherited IDs.

---

## 1. Starting point: what the code already has

Source inspection of this repository on 14 October 2026. Every item is **source-inspected only**.
None is runtime-qualified yet; that is Phase 0 work (PRD §3.1).

| PRD capability | What the source shows | File | Consequence for the plan |
|---|---|---|---|
| Session revision | `Engine.revision: u64` starts at 1 per `Engine` instance, `wrapping_add` on every document change except zoom. **Measured (P0-T06, [spike-guarded-publish.md](spike-guarded-publish.md)): `edit.select`, `edit.select_none` and `transport.locate` also move it**, as do save/bounce (plugin state capture) and plugin-editor tweaks. | `crates/engine/src/lib.rs:113`, `:163-167` | Too sensitive to use directly: a playhead move would stale every plan. Use a content revision (P1-E01), plus a session-instance ID because the counter resets on restart (PRD §7.2). |
| Revision exposure | `session.inspect` does not return the revision. | `crates/ui-egui/src/control.rs:119`, `crates/engine/src/inspect.rs` | Add the revision to inspect and to execute replies (P1-E01). |
| Undo / copy-on-write | `Arc<Session>` snapshot stack (limit 200). `execute` rolls back a failed command. `push_undo(label, before)` records a manual undo point. `execute_merged` coalesces gestures. | `crates/engine/src/lib.rs:223-332` | **Confirmed by the P0-T06 spike:** several operations publish as one undo entry, all or nothing, on this history with about 100 lines (`crates/engine/src/group.rs`, branch `spike/guarded-publish`). The stack is capped, can't be looked up by revision and is cleared on reopen, so it is not a version store (P1-U06). |
| Batch | MCP `batch` runs commands one after another, with no transaction. | `crates/automation/src/mcp.rs:133` | Not atomic. A new guarded group command is needed (P1-E02). |
| Local control | TCP on `127.0.0.1`, no authentication, unbounded line reads. **Measured (P0-T07, [control-channel-protection.md](control-channel-protection.md)): an HTTP POST from a web page with a JSON body executes `engine.execute`**, so any page the user visits can drive the app. | `apps/soundcraft/src/control_server.rs:13`, `:36-66` | A token-protected assistant mode, an allowlist and an HTTP first-line rejection are required before any assistant mutation (P1-E03). |
| Execution outcomes | `Engine.journal` is an in-memory list of `(id, params)`. Nothing records which external request produced which change, and nothing survives a restart. | `crates/engine/src/lib.rs:108-109` | Receipts, a status query and saved-state evidence are needed for reconciliation (P1-C07 – P1-C09). |
| Autosave | Every 2 minutes, a session with unsaved changes is copied with `save_copy` to the autosave folder (skipped while recording). | `crates/ui-egui/src/lib.rs:513-532` | Autosaves are saved-state evidence; they must carry the sidecar too (P1-C05, P1-C09). |
| Disk recording | One WAV file per take, a lock-free 4-second ring, header rewritten every second, **default input device only**. | `crates/playback/src/record.rs:1-20` | Qualify it, don't rewrite it (P0-T04, P3-R*). |
| Devices / block size | `BLOCK = 512`; `open_device` uses `default_output_device`. Device selection is already tracked as gaps.md G-01 (≈25–40 h). | `crates/playback/src/lib.rs:36`, `:714-717`; `docs/gaps.md` | Reuse the gaps.md G-01 work for FR-11 (P3-R01). |
| Pitch | Frame-level YIN returns `Option<f32>` with no confidence. Notes are rounded to MIDI. | `crates/dsp/src/pitch_detect.rs` | A new continuous F0 tracker with confidence is needed. YIN is a baseline only (P0-T08, P1-A03). |
| Compressed import | symphonia 0.5.5 with `mp3`, `aac`, `isomp4` features, `enable_gapless: true`. **Measured (P0-T03, [import-matrix.md](import-matrix.md)):** LAME-tagged MP3 and VBR are correct; **AAC keeps its start priming (+1024 or +2112 samples)**; untagged MP3 is +1104; the original compressed file's identity isn't kept on save. | `Cargo.toml:39`, `crates/audio-io/src/symph.rs:41` | Repair AAC trimming and keep source identity (P1-E07); decide untagged MP3 handling (D9). |
| EQ / compressor | `Eq<S>` with `eq7_response` / `eq1_response`; `Compressor`. | `crates/dsp/src/plugins/eq.rs:129,306`, `dynamics.rs:59` | The response functions make EQ conformance tests cheap (P2-M01). |
| Loudness | `LoudnessMeter` exists. True-peak support not yet checked. | `crates/dsp/src/meter.rs:141` | Use it for audition matching. Confirm the method and version, and add true-peak if missing (P1-U04). |
| Offline render | `render_bounce(s: &Session, …)` renders from any `Session` value. | `crates/engine/src/io.rs:285` | Candidate and original can render from the same immutable snapshot (P1-U02). |
| Mixer commands | `mix.volume`, `mix.pan`, `mix.mute`, `mix.solo`, `track.rename`, `mix.insert`, `mix.insert_param(s)`, `mix.insert_bypass`, `mix.insert_remove`. **Not isolated:** `set_grouped` applies volume, mute and solo to active mix-group members; exclusive solo (`exclusive` param, or `solo_mode` containing `xor`) clears every other solo; with no `track`/`tracks` param, commands fall back to the selection. | `crates/engine/src/cmd/mix.rs:185`, `:213-261`; `crates/engine/src/cmd/mod.rs:236` | Candidate operations for the admitted manifest (FR-02), but only behind strict parameter validation and a scope diff (P1-E06). |
| Export | `file.bounce_mix {path, …}` writes straight to `path`, with no source-path protection and no replacement check. `normalize` defaults to false and `dither` to true. | `crates/engine/src/cmd/file.rs:87` | A guarded export command is needed (P2-X01). |
| Session file | `.scraft`, written atomically via a `.scraft.tmp` file; bundle save supported. | `crates/model/src/session.rs:10`, `crates/engine/src/io.rs:600,817` | AI provenance goes in a versioned sidecar, not the core file (P1-C05). |
| Branding | README carries ArtCraft branding; `docs/brand/` is under a separate licence. | `README.md`, `docs/brand/LICENSE-brand.txt` | Remove branding before external distribution (P3-D03). |

---

## 2. Plan at a glance

| Phase | Goal | Entry | Exit (PRD) | Gates evaluated | Indicative size* |
|---|---|---|---|---|---|
| **0. Discovery and foundation** | Test the product's value with 6–8 vocalists and qualify the foundation | Start date and weekly capacity recorded | Decision on G00; capability matrix; named repairs; separate go/repair/stop decisions for imported editing and for recording | G00 (decision); evidence toward G01, G02, G04, G05, G12 | ≤ 4 weeks, ≤ 40 eng h (PRD §13.1) |
| **1. Integrated imported workflow** | One end-to-end path: import, inspect, propose (one goal), audition, Keep, undo, save/reopen | G00 = go (or a bounded redesign); imported editing = go | Path works on real vocals, including stale, interrupted, duplicate and later-manual-edit cases | Applicable subsets of G01, G03, G04, G05, G08, G09, and the local-control and cloud-budget parts of G12 | ≈ 8–12 weeks |
| **2. Vocal quality and product qualification** | All three goals, selected-note practice, MIDI comparison, frozen corpus, product pilot | Phase 1 exit | Full imported workflow passes G03–G08 and G11 with no open integrity defect | G03, G04, G05, G06, G07, G08, G11 | ≈ 8–12 weeks (corpus lead time dominates) |
| **3. Recording and early access** | Device controls, recording qualification, privacy, packaging | Phase 2 exit; recording = go or repair | G01–G13 pass for the declared envelope | G01–G13; G02-SW only if software monitoring is advertised | ≈ 6–10 weeks |

\* Phases 1–3 are planning-grade guesses and are **re-estimated at the Phase 0 exit** from real
integration experiments (PRD §13.5). Only the Phase 0 timebox is a commitment.

```
Week (upper-bound estimates):
             1   2   3   4 | 5 ............ 16 | 17 ........... 28 | 29 ........ 38
Product   [recruit · G00 ] |   corpus licensing/recording ────────►| G11 pilot  |
Technical [qualify · spike]| Phase 1 build     | Phase 2 quality   | Phase 3 recording + release
Decision              ▲ P0 exit           ▲ P1 exit          ▲ P2 exit      ▲ G13
```

The pitch corpus (PRD §12.2: ≥160 excerpts, ≥16 singers, ≥80-excerpt holdout) has the longest lead
time of anything in the plan. Licensing and recording it **starts in Phase 1** so it doesn't block
the G06/G07 work in Phase 2.

---

## 3. Working conventions

**Branches and upstream.** `main` follows `upstream/main` (storytold). Product work goes on `ai/*`
branches merged into a long-lived `ai/main`. Generic repairs (revision exposure, guarded group
publish, control-channel auth, device selection, recorder fixes) go in small separate commits on
`fix/*` branches so they can be offered upstream as PRs. Upstream accepting them is never a delivery
dependency (PRD §10.1).

**Where code goes.** The engine stays thin and provider-free (PRD §10.1). The new crates must be
registered with `cargo xtask layers`:

| Piece | Location | Layer | Notes |
|---|---|---|---|
| Revision, instance ID, guarded group publish, scope diff, receipt table and status query, task-pinned snapshots, capability manifest, guarded export | `crates/engine` (new `cmd/assist.rs`) | L4 | Generic; can be offered upstream |
| Optional `Session.applied_tasks` lineage marker | `crates/model` | L2 | `#[serde(default)]`; older builds ignore it |
| Token-authenticated control endpoint | `apps/soundcraft/src/control_server.rs`, `crates/automation` | apps / L5 | Generic |
| Plan / approval / receipt / analysis contracts | new `crates/assist-contract` | L2–L4 (serde-only) | Versioned schemas shared by the app and the companion |
| Execution controller, journal, analysis broker, model adapter | new `apps/soundcraft-assist` (companion process) | apps | Single owner of task state (PRD §10.1) |
| Production policy: rule declarations, evaluator, fixtures | `crates/assist-policy` (`soundcraft-assist-policy`, branch `spike/policy`) | L0, standalone | Pure and deterministic; the companion depends on it. See [production-policy.md](production-policy.md) |
| F0 tracker with confidence, rule features | `crates/dsp` (new `f0.rs`, `features.rs`) | L1 | The existing `pitch_detect.rs` stays as is for Audio to MIDI |
| Assistant, vocal inspector, change review, audition, history panels | `crates/ui-egui` | L6 | UI commands go in `UI_COMMANDS` |

**House rules that apply to every task.** Everything user-visible is a command with tests. No
panics in non-test code. Input-derived numbers are treated as hostile. The audio callback never
blocks or allocates. `cargo xtask ci` passes before every commit. One topic per commit. Assets are
logged in `ATTRIBUTION.md`. All of this comes from `AGENTS.md`. Plugin and asset names stay our own
(clean-room).

**Evidence records.** Each capability is marked *documented*, *source-inspected* and
*runtime-tested* independently, with the commit, lockfile, device/OS, and an evidence link and
date (PRD §3.1, §15). They live in `docs/soundcraft-ai/capability-matrix.md`, which P0-T02 creates.

---

## 4. Phase 0: product discovery and foundation qualification

**Timebox:** 4 calendar weeks from a recorded start date, with a ceiling of 40 engineering hours.
Proposed split: ≈ 28 h technical, ≈ 12 h discovery preparation. At the deadline, issue go, repair,
stop or inconclusive decisions. Do not extend silently.

**Before day 1:** record the start date, weekly availability and the reference host's details:
Apple M1, 16 GB, SSD, exact macOS build and free disk space. If capacity is short, shrink the
investigation list now rather than overrunning.

### 4.1 Product workstream (G00)

| ID | Task | Output | Week |
|---|---|---|---|
| P0-D01 | Freeze the G00 go criteria from PRD §12.4 *before* collecting outcomes: ≥6 completed sessions; ≥⅔ can explain the difference between measured pitch, nearest degree and intended-note error; ≥⅔ keep a candidate; ≥½ return a second recording within 7 days. Round counts up. Also freeze the follow-up rule: a participant whose 7-day window has not closed by the scoring date is **inconclusive** for the return-use criterion, counted neither as returning nor as not returning. If fewer than 6 participants have a closed window, the return-use criterion is inconclusive. | [`discovery/g00-protocol.md`](discovery/g00-protocol.md) (**approved as written, 14 October 2026; criteria frozen**; the start date, capacity, host and names in its §10 are still to be filled in) | 1 |
| P0-D02 | Consent and data handling: permitted uses, deletion arrangements, no training use without separate consent. Input/output manifest template (PRD §7.1). | [`discovery/g00-consent-form.md`](discovery/g00-consent-form.md) (draft); the manifest template is Appendix A of the protocol | 1 |
| P0-D03 | Recruit 6–8 independent vocalists, including low-register singers and people with no MIDI melody. Recruiting starts on day 1 and the roster closes by day 10, so sessions fit before day 21. | Roster (pseudonymous) | 1–2 |
| P0-D04 | Preparation tooling on **copies** of audio (no session mutation): (a) a continuous-F0 dump plus a Sa-relative cents plot with uncertainty; (b) selected-note statistics; (c) level-matched gain/EQ/compression previews rendered with `soundcraft-cli run --cmd … --bounce`. Log every manual step and how long it took. | `tools/discovery/` scripts; preparation log | 1–2 |
| P0-D05 | Run the sessions. Each one gets a no-MIDI pitch view, a selected-note example, a processed preview for an agreed issue and a clean control. **Every initial session finishes by day 21**, so each 7-day return window closes by day 28. A session that can't be held by day 21 is still run if useful, but its return-use result is inconclusive. Track each participant's window-close date. | Session notes; per-participant outcome sheet with window-close dates | 2–3 (by day 21) |
| P0-D06 | Score against the frozen criteria. Record ties, rejection reasons, and pitch versus production value separately. A mixed result leads to a bounded redesign, not an averaged pass. Fewer than 6 sessions is inconclusive. Apply the frozen follow-up rule from P0-D01 to any window still open on the scoring date. | G00 decision memo | 4 (day 28) |

### 4.2 Technical workstream (foundation qualification)

| ID | Investigation | Method | Output | Feeds |
|---|---|---|---|---|
| P0-T01 | Build the pinned app on the reference host (**all six CI steps pass on the M4 inspection host**; M1 run pending) | `cargo xtask ci`; release build; record toolchain, `Cargo.lock` hash, OS build | Baseline record | All |
| P0-T02 | Capability matrix | One row per PRD §3.2 capability, with three independent evidence columns | `capability-matrix.md` | §3 decision |
| P0-T03 | Import, render, persistence | Generated fixtures (a click at a known sample offset): WAV 16/24/32f mono and stereo; MP3 CBR and VBR; AAC-LC in M4A. Measure the decoded onset offset (encoder delay/padding), duration and channel map. Bounce, then run a scripted 100× save/reopen loop through the CLI and compare session state and source fingerprints. | Import matrix with exact profiles; G01 pre-evidence | G01 |
| P0-T04 | Existing disk recorder (**done for the writer path**: [recorder-and-device-findings.md](recorder-and-device-findings.md); no input device opened) | Normal take; `kill -9` mid-take, then check the header and playable length; a read-only directory (file-creation failure); a small `hdiutil` RAM-disk image filled up (disk full); a stalled writer forcing ring overflow (needs a test hook); confirm errors reach the UI. | Recorder behaviour report plus a list of remaining gaps with estimates | G02, G09 |
| P0-T05 | Device and monitoring path (**software half done**: device list and callback sizes; interface, loopback latency and alignment pending: see §6 of [recorder-and-device-findings.md](recorder-and-device-findings.md)) | Name the target interface. Log real callback sizes against `BLOCK=512`. Measure a cable loopback for alignment and round-trip latency. Document hardware direct-monitor setup. Size the overlap with gaps.md G-01. | Device report; bounded repair tasks | G02, G02-SW |
| P0-T06 | Revision and publication spike | Audit when `revision` bumps (selection, view, transport). Prototype `execute_group` (snapshot `before`, apply operations, restore on any error, then one `push_undo`). Check `transport_requests`, plugin state in the player, the source pool and renderer pickup of the new `Arc<Session>`. | Spike branch plus a findings note: what reuse is safe and what needs extending | G04 |
| P0-T07 | Control-channel protection design | Per-launch random token passed to the companion at spawn. Reject requests without it. Rejection tests for no token, a wrong token, a token from the previous launch, and non-allowlisted methods. | Design note plus failing tests to make pass in P1 | G12 |
| P0-T08 | Pitch benchmark (small, **development data only**). **Scoring harness and the pYIN-style tracker done** ([pitch-eval-harness.md](pitch-eval-harness.md)); the benchmark itself needs annotated natural recordings | ~20–30 natural excerpts. Compare YIN (baseline), a pYIN-style probabilistic tracker and one neural F0 model (check licences first). Score F0 and voicing on a 10 ms grid, and note memory use and load time. | Benchmark note; tracker choice | G06, G07 |
| P0-T09 | Initial production rule fixtures | Define the rule schema: required measurements, applicability, uncertainty checks, allowed processors, bounds, maximum change, abstention. Write one rule set (headroom/level) with fixtures. | `policy/v0` plus fixtures | G08 |
| P0-T10 | Packaging and dependencies | Worker language/runtime options, model checksums and licences, resident memory against the 2 GiB budget, cold-load time against 20 s. | Licence inventory draft; runtime recommendation | G10, G13 |

**Result of P0-T10 ([packaging-and-dependencies.md](packaging-and-dependencies.md)):** the companion is written in **Rust**. It
reuses `soundcraft-dsp` and `audio-io` directly, has no Python environment to package (PRD §10.3),
and fits the never-crash tooling. Add an ONNX runtime only if P0-T08 picks a neural F0 model.

### 4.3 Phase 0 exit

The output is one decision memo containing:

1. **G00 decision:** go, redesign or stop. If pitch adds no value but production does, write a scope
   amendment before continuing (PRD §12.4).
2. **Capability matrix** with all three evidence levels for every PRD §3.2 row.
3. **Imported-session editing decision** (go, repair or stop), based on P0-T03, T06 and T07.
4. **Recording decision** (go, repair or stop), based on P0-T04 and T05. If it fails, the work
   continues as an explicitly labelled imported-audio prototype (PRD §3.3).
5. **Named repairs with estimates.** No second disk writer or revision mechanism unless evidence
   shows the existing one can't meet the contract.
6. **Re-baselined estimates** for Phases 1–3.

**Stop trigger:** source preservation, valid rendering, saving or safe publication can't be
established within a bounded repair plan.

---

## 5. Phase 1: integrated imported vocal workflow

Scope: one end-to-end path with **one admitted production goal** (headroom/level, gain only) and
the smallest admitted operation set. Section 7 protections apply from the first mutation.

### 5.1 Engine extensions (generic, can be offered upstream)

| ID | Task | Detail | FR / Gate |
|---|---|---|---|
| P1-E01 | Session instance ID + content revision | Random instance ID per open/new session, regenerated on reopen. Expose `content_revision()` (spike: `crates/engine/src/group.rs`): a counter that moves only when the document differs from the one seen at the previous call, ignoring selection, playhead and zoom, and never repeats a value even if edits are undone back. Return `{instance, content_revision}` from `session.inspect` and every `engine.execute` reply. The raw `revision` stays internal (UI caches). Decide how plugin-state capture on save or bounce affects it (spike §6). Re-measure its cost on an 8-track, 10-minute session. | FR-02, FR-10 / G03, G04 |
| P1-E02 | Guarded group publish | New command `session.apply {instance, expected_content_revision, execution_id, label, ops[]}`, built from the spike's `execute_group` (all or nothing; restores document, undo and redo stacks, journal, dirty flag, revision and queued transport requests on failure). Check instance and content revision at the engine boundary on the UI thread (serialised with manual edits), stage, validate postconditions, publish once with a single undo entry, and return the before/after revision. Refuse during recording. Run P1-E06 validation and the scope diff before publishing. In the same call as the publication, append the `execution_id` lineage marker to the staged session (P1-C08) and record a receipt (P1-C07), so a repeat returns the known result and a reused ID with different content is rejected. | FR-02, FR-10 / G04 |
| P1-E03 | Authenticated control endpoint | Implement the [P0-T07 design](control-channel-protection.md): per-launch 32-byte token delivered by a 0600 file (`--control-token-file`), `auth` handshake, three failures close the connection, 10 s auth timeout, 4 KiB / 1 MiB line caps, 8 connections, allowlist enforced before the UI thread, and an HTTP first-line rejection in **both** modes. Developer mode (no token flag) keeps working. Make the 7 ignored `protection_tests` in `control_server.rs` pass and remove their `#[ignore]`; add `--token-file` to `soundcraft-cli app` and `mcp --connect`; update `docs/control-protocol.md`. Estimate 8–12 h (guess). Bind the token's lifetime to the launched session. The assistant path sees only an allowlist (`session.inspect`, `session.apply`, `session.execution_status`, render requests); no `ui.*` injection, no arbitrary execute. | FR-13 / G12 |
| P1-E04 | Capability manifest | Generated from command specs and processor parameter tables for the admitted set: command ID, parameter units, ranges, defaults, processor version, reversibility. Starts as `mix.volume` and `track.rename` only; grows per phase. | FR-02 / G03, G04 |
| P1-E05 | Automation conflict check | Block a static parameter proposal when automation on that parameter would override it, and explain why. | FR-02 / G03 |
| P1-E06 | Strict operation validation and scope diff | Native mixer commands are not isolated (§1): `set_grouped` changes active group members, exclusive solo clears other solos, and a missing `track` falls back to the selection. (a) **Strict parameters** for each admitted operation: an explicit `track` ID is required, a missing or unknown ID is an error (never a selection fallback), `exclusive` is forced to false, and unknown keys are rejected. (b) **Scope diff**: after staging, compare the staged session with `before` field by field. Any change outside the approved target IDs and attributes rejects the whole group with an explanation, and nothing is published. (c) In Phase 1, an operation whose diff reaches active group members is rejected and the conflict explained; whether to support group-aware scope is decision D8. Tests: grouped tracks (volume, mute, solo), exclusive solo with `solo_mode` set to `xor`, missing and unknown IDs, no `track` param with an active selection, and a changed selection between review and publish. | FR-02 / G03, G04 |
| P1-E07 | Compressed-import repairs | From P0-T03 ([import-matrix.md](import-matrix.md)). (a) Honour the MP4 edit list and iTunSMPB so AAC-LC M4A imports with onset error ≤ 1 sample against the ffmpeg reference on the `afconvert` and ffmpeg fixtures at 44.1 and 48 kHz, mono and stereo. Prefer reading the gapless data in `soundcraft-audio-io` and trimming after decode; symphonia's files are MPL-2.0, so a patch to them would have to be published (decision D10). (b) Keep the original compressed file's name, size and content hash in the session's source entry or the sidecar, so a source imported from MP3 or AAC still has its identity after the decoded PCM replaces it on save (PRD §4.2). (c) Report which gapless case applied (declared, untagged or unknown) in the import result. (d) Regression tests from the fixture generator, plus onset measurement after bounce on the timeline. | FR-11 / G01 |

### 5.2 Contracts and controller (companion)

| ID | Task | Detail | FR / Gate |
|---|---|---|---|
| P1-C01 | `assist-contract` crate | Versioned Plan, Approval, ExecutionReceipt, RecoveryEntry (PRD §7.1), AnalysisRequest/Result (§8.1, §8.3). Serde with strict schema and domain validation. The Plan carries `approval_path` (`audition` or `change_list`), set by the controller from the plan's origin and never by the model; an approval whose path does not match its plan is refused (UI-1). | FR-10 |
| P1-C02 | Task journal | Local append-only JSONL with `fsync` at each durable phase. Lifecycle Draft → Validated → Previewing → Ready → Approved → Committing → Applied, plus Rejected, Cancelled, Stale, Failed and Recovery required (PRD §7.5). Persist `execution_id` before publishing. | FR-10 / G04, G09 |
| P1-C03 | Reconciliation | After a lost response or restart, follow the P1-C09 crash table. Query `session.execution_status` while the same DAW instance is still running; otherwise use the saved-state evidence from P1-C08. An outcome that the evidence can't resolve stays **Recovery required**. Never blind-retry. Pending approvals expire on restart. | FR-10 / G04, G09 |
| P1-C04 | Approval binding | The approval holds the plan digest, candidate digest and expected content revision. Any edit invalidates the candidate and the approval. Relevant session changes mark the plan Stale. **An approval is created only by Keep in the audition (production proposals) or Approve in the change list (explicit assignments)**; a chat message never creates one. Declining records a reason (`rejected_before_preview`, `kept_original`, `discarded`, `rejected`) (UI-1). | FR-02, FR-12 / G04 |
| P1-C05 | Project sidecar | Versioned sidecar next to the `.scraft` for provenance and recovery. **Every path that writes a session also writes the sidecar:** save, save as, `save_copy`, and the two-minute crash-recovery autosave (`crates/ui-egui/src/lib.rs:513-532`, which calls `save_copy`). Each sidecar carries a digest of the session file it was written with, and is written before the atomic `.scraft` rename, so a mismatched pair is detected and treated as unverified. The core session stays usable with AI disabled. Unknown sidecar versions are rejected safely with their bytes preserved. | FR-10, FR-11 / G01, G09 |
| P1-C06 | Limits | 16 ops per plan, 3 model calls per task, one transient retry, 30 s per call, 120 s per task. One heavy job at a time. Cancellation acknowledged in ≤ 500 ms and the worker stopped in ≤ 2 s. | FR-13 / G10, G12 |
| P1-C07 | Execution receipts and status query | Receipts are **historical outcomes only**. A receipt says "execution X was published in instance I, taking the revision from R1 to R2". It never claims the change is in the current or saved state; P1-C08 answers that. The engine keeps a receipt table keyed by `(instance, execution_id)`: plan digest, before and after revision, outcome and undo-entry reference. The receipt is inserted in the same `&mut Engine` call that publishes, so publication and receipt can't diverge inside a running DAW. It sits outside the undo history. New allowlisted query `session.execution_status {execution_id}` returns `published`, `rejected` (with reason) or `unknown` for the current instance. On every session write (P1-C05), the instance's receipts go into the sidecar's execution log. | FR-10 / G04, G09 |
| P1-C08 | Saved-state evidence | Whether a task's change is present in a given state is decided by evidence from that state, never by a receipt. (a) **Lineage marker:** the published group also appends the `execution_id` to an optional `Session.applied_tasks` list (`#[serde(default)]`, written only when non-empty). The marker lives inside the `Arc<Session>` snapshot, so undo removes it, redo restores it, and save, `save_copy` and autosave persist it with the document. Older builds ignore the field because Session has no `deny_unknown_fields`. If an upstream build re-saves the file it drops the marker, and the check falls back to (b). (b) **Content check:** compare the state's values at the task's target attributes with the receipt's before and after values. (c) **Result for each task and state:** `present` (marker present, values equal the after-values), `superseded` (marker present, values changed by later edits), `absent` (no marker; values match the before-values or something else), `undetermined` (unverified sidecar pair, or the marker was lost and the values are ambiguous). Tests: Apply → Save; Apply → Undo → Save; Apply → Undo → Redo → Save; Apply → manual edit of the same parameter → Save; Apply → unrelated manual edit → Save; and each of these followed by restart and by an autosave instead of a save. | FR-10, FR-11 / G04, G09 |
| P1-C09 | Crash reconciliation contract | **Write order for each task:** (1) companion fsyncs `Committing` with `execution_id`, plan digest, instance and expected content revision; (2) `session.apply`; (3) engine publishes and records the receipt atomically; (4) companion fsyncs `Applied` with the receipt. After (1), a missing receipt is **not** evidence of publication or non-publication: a crash after (1) but before (2) runs leaves the same records as a crash after (3). **Evidence sources, newest first:** the running instance's status query if it survived; then every saved state written after the `Committing` timestamp (the session file and the latest autosave for that session), each read through P1-C08. **Outcomes:** if the DAW instance survived, its status query resolves the outcome. Otherwise a saved state showing `present` or `superseded` proves publication. With no such evidence, the execution outcome stays **Recovery required (unknown)**, even though the change is shown as `absent` from the state being reopened. The user sees both facts: "outcome unknown" and "not in the session you're reopening; the latest autosave is from <time>". The task leaves Recovery required only when the user explicitly closes it after seeing that evidence, and closure is recorded as a user decision, not as a resolved outcome. The change is **never reapplied** automatically. **Crash cases, each tested separately:** companion only; DAW before (2) runs; DAW after (3) and before any save; DAW after a save or autosave; both processes, at each step. | FR-10 / G04, G09 |

### 5.3 Analysis

| ID | Task | Detail | FR / Gate |
|---|---|---|---|
| P1-A01 | Audio broker | Pulls immutable PCM from one coherent session snapshot. Integer sample ranges, start-inclusive and end-exclusive. Maps source to session through trim and sample-rate conversion. Blocks analysis on unmapped transforms. Signal locations per PRD §8.2. | FR-01, FR-03 |
| P1-A02 | Level features | Peak, true-peak, RMS, integrated and short-term loudness, clipping, for each named range and signal location. Must supply the headroom rule's inputs: true peak at the channel output after the fader, clipped-sample count and active milliseconds, each with a declared method version. | FR-01, FR-04 |
| P1-A03 | F0 tracker v1 | Start from `crates/dsp/src/f0.rs` (the probabilistic tracker built in P0-T08: 20 tests plus an exact posterior check; uncalibrated scores; its low-register and 0 dB limits are listed in [pitch-eval-harness.md](pitch-eval-harness.md) §6), unless the natural-recording benchmark shows it must be replaced. It implements the harness's `Tracker` trait through an adapter. Still to do: confidence thresholds fixed on development data, a longer analysis window for low voices, speed (it is a plain O(window × lag) loop per frame), in `crates/dsp/src/f0.rs`. Continuous Hz and cents, a voiced mask, separate voicing and F0 confidence, 65–1100 Hz, ≤ 5-minute jobs. Confidence is never shown as a probability unless it has been calibrated. | FR-03 / G06, G07 |
| P1-A04 | Cache and invalidation | Keyed by source fingerprint, range, signal dependencies, algorithm version and settings. Changing Sa remaps without rerunning F0. Stale annotations are shown and can't be used as evidence (PRD §8.4). | FR-03 / G07 |

### 5.4 Policy, model and audition

| ID | Task | Detail | FR / Gate |
|---|---|---|---|
| P1-M01 | Production policy v1: headroom/level | Start from the P0-T09 crate and rule `level.headroom` 0.1.0 ([production-policy.md](production-policy.md); 44 fixtures, property tests). Deterministic: the same fingerprint, goal, controls, rule version and manifest give the same plan. Records matched rule, measurements, before and after, limits and expected effect. Abstains when evidence is missing. **To add:** an input and abstention code for a track in an active mix group (the fader move would reach its siblings, and P1-E06 would reject the plan after audition); the controller adds the content fingerprint and processor-manifest digest; confirm the thresholds against G00 and listening checks, bumping the rule version for any change. | FR-04 / G08 |
| P1-M02 | Gain processor conformance | Gain-factor tests at known levels, bypass, persistence. | FR-04 / G05 |
| P1-M03 | Model adapter (cloud, opt in, text only) | Maps intent to a supported goal, asks for clarification, explains the supplied evidence. **Never supplies parameter values.** Pseudonymous track IDs. Provider disclosure screen. Credentials in the macOS Keychain. **Can't call the provider until P1-M04 passes its tests.** | FR-04, FR-13 / G03, G12 |
| P1-M04 | Cloud budget enforcement | PRD §11.2. (a) The user sets a currency budget per task or per day before the first cloud call; with no budget set, cloud calls are refused. (b) A versioned rate table records the provider, model, per-token rates and the date they were checked. Missing or stale rates (older than a configured age, or an unknown model) block automatic continuation and ask the user. (c) Before each call, reserve a conservative upper bound (maximum input tokens plus maximum output tokens times the rates, for the call and its one allowed retry). Refuse the call if the reservation exceeds the remaining budget. (d) After each call, record provider-reported usage, release the unused reservation, and reconcile; log any mismatch. (e) The UI states that charges created outside the app are not capped. Tests: no budget set, budget exhausted mid-task, stale rate table, provider usage above the reservation. **Lands before the first cloud-enabled prototype build.** | FR-13 / G12 |
| P1-U01 | Local goal controls | The same controller and policy as the assistant; natural language gains no extra authority. | FR-04 |
| P1-U02 | Render original and candidate | `render_bounce` on the published snapshot and on the staged snapshot, using the same rate, range and implementation. Tail and warm-up recorded separately. Non-finite output is rejected. | FR-12 / G05 |
| P1-U03 | Repeat-render null test | The same candidate rendered twice from the same build, input, initial DSP state and block configuration: peak residual ≤ 1e-6 FS (−120 dBFS), with the RMS residual reported separately. This proves repeatability only; P1-U05 covers original versus candidate. | FR-12 / G05 |
| P1-U04 | Audition matching | Declared, versioned loudness method using `LoudnessMeter` (add true-peak if missing). Matching is visible, within 0.5 LU, and never applied to export. Manual matching for short or near-silent regions. | FR-12 / G08 |
| P1-U05 | Render acceptance checks | (a) **Original versus candidate timing:** an impulse fixture through the candidate's processors, aligned within one sample after declared latency compensation. (b) **Newly introduced clipping:** sample peaks and true-peak overs in the candidate that are absent from the original over the same range are flagged in the audition view. (c) **Render-path qualification:** list every processor on the path (track, sends, context buses). Any unqualified or non-reproducible processor (third-party plugins, randomised processing) makes audition unavailable, with an explanation. (d) **Scope:** the candidate differs from the original only in approved-scope parameters (the P1-E06 diff). (e) **Fidelity after Keep:** render the published session after Keep, and again after save and reopen; each must match the retained candidate render (P1-U06) within the G05 residual tolerance. | FR-12 / G05, G08 |
| P1-U06 | Version retention | Defines the only versions that can be exported or compared later. Undo snapshots are not a version store: they are capped at 200, can't be looked up by revision and are cleared on reopen (`crates/engine/src/lib.rs:104,182`). (a) **`current`**: the live published state, bound to `(instance, revision)` when the user chooses it. If the revision moves before the export starts, the export is refused and the user re-confirms. (b) **Task-pinned snapshots:** the before and after `Arc<Session>` of each AI task, held for the life of the instance, independent of the undo cap. Looked up by `(instance, revision)`. At most 20 tasks, least recently used evicted first, with the memory counted in the G10 budget. Released when the session is closed or reopened. (c) **Retained renders:** the original and candidate renders from each audition, at full precision and **before audition loudness matching**. Stored in project-managed storage outside the cache folder (deleting the cache doesn't remove them), indexed in the sidecar with their provenance and range, and kept across reopen until the user deletes them from task history. A render covers only its own range. **Unavailable versions:** a request for an evicted, released or deleted version is refused with the reason and the list of available versions. Another version is never substituted silently. Arbitrary historical revisions are not exportable in this release. Tests: export of each kind; eviction; reopen; user deletion; cache deletion. | FR-12 / G05, G08 |

### 5.5 UI (L6)

| ID | Surface | Notes |
|---|---|---|
| P1-I01 | Assistant panel | Scope, mode (Explain / Suggest / Apply), progress, cancel, evidence-linked answers. No mode upgrades itself. Design: board 3b ([ui-design-spec.md](design/ui-design-spec.md)). |
| P1-I02 | Vocal inspector | Waveform, continuous pitch, Sa and octave control, uncertainty band, precise range selection. No MIDI needed. Design: boards 3a and 4a (the vocalist workspace). |
| P1-I03 | Change review | Target names and IDs, before and after, units, rule provenance, affected scope. Raw JSON only on demand. Design: board 3c. **Approval flow decided (UI-1, [design/decisions.md](design/decisions.md)):** a production proposal shows Preview and Reject only, with no Approve; an explicit assignment shows Approve and Reject. Messages for the policy's other result codes still need wording. |
| P1-I04 | Audition view | A/B, solo and in-context listening, visible loudness matching, Keep and Discard. Design: board 3d. **Keep this change is the approval action for production proposals** (it binds the candidate); Keep the original, Discard and Keep this change stay equal in size and weight, and initial focus is the A/B switch (UI-1). |
| P1-I05 | Task history | Request, state, applied changes, outcome, undo availability, recovery instructions. Undo right after a task reverts the whole task; later manual edits are never undone silently (PRD §7.4). Design: board 3e. |
| P1-I06 | Accessibility | Keyboard operation, readable contrast, no meaning carried by colour alone. Check by rendering with `ui_shot`. The design's contrast values were recomputed and pass its own minimums. |
| P1-I07 | Design-system theme *(proposed)* | Implement the received design tokens in `crates/ui-egui`: dark and light colour tokens, type scale, sizes, radii, the offset shadow, motion timings and the reduced-motion rule, from [tokens.json](design/tokens.json), including the hit areas for the dense controls. Bundle IBM Plex Sans (400, 500, 600) and Mono (400, 500) with their licence texts and `ATTRIBUTION.md` rows. A live dark and light switch. |
| P1-I08 | Arrange window restyle *(proposed)* | Restyle the track header (232 × 74, compact 40), clip, toolbar and transport to the redlines in [ui-design-spec.md](design/ui-design-spec.md) §4, with a collapsible ruler and the inspector pane; check each against the boards with side-by-side `ui_shot` renders. Needs the app's real fader law (the mockup's scale is a placeholder). Whether this lands in Phase 1 is a product decision, since the plan holds Phase 1 until the G00 result. |
| P1-I09 | Icon set in code *(proposed)* | Draw the 63 icons from [icons.json](design/icons.json) at 20 px and 16 px: a small path renderer for `M L H V A C S Z` (absolute and relative; circular arcs), 1 px round strokes with the half-pixel offset, and filled shapes for record, record-arm, automation-write and overflow. Resolve first the open icon issues in [ui-design-spec.md](design/ui-design-spec.md) §9 (zoom versus zoom-out, the sparkle smart tool, the stale icon). Every icon is original and drawn in code, as the repo rule requires; the design tool's imitation notes are in `icons.json` for the owner's review. | FR-13 |

### 5.6 Phase 1 tests and exit

- **G04 matrix** for each admitted operation: stale revision, duplicate request, lost response,
  concurrent manual edit, cancellation, failed staging, and the P1-E06 scope cases (grouped tracks,
  exclusive solo, missing IDs, selection fallback). No partial publication and no loss of unrelated
  work.
- **G09 subset:** forced stops at each write-order step in P1-C09, run separately for a companion
  crash, a DAW crash before `session.apply` runs, a DAW crash after publication with no save, a DAW
  crash after a save or autosave, and both crashing together. A crash after `Committing` with no
  later saved-state evidence must end in Recovery required, never in "published".
- **Saved-state evidence:** the P1-C08 sequences (apply, undo, redo, later edits, then save or
  autosave, then restart) give the expected `present`, `superseded`, `absent` or `undetermined`.
- **Render checks:** P1-U03 and P1-U05 on real vocals, including fidelity after Keep and after
  save/reopen, measured against the retained render from P1-U06.
- **Cloud budget:** P1-M04 tests pass before any cloud-enabled build is given to a user.
- **G03 subset:** a locked prompt set for the admitted goal and operations, including duplicate
  names, changed selection, missing media, unknown processors, malicious metadata and a reopened
  session with a reused revision number.
- **G01 subset:** the import matrix from P0-T03 as automated fixtures.
- **G12 local control:** unauthorised IPC and metadata injection cause no mutation and no
  disclosure.
- **Exit (PRD §13.2):** the path works on real vocals, including a stale proposal, an interrupted
  task, a duplicate request and a later manual edit. Subset passes are **not** recorded as full gate
  passes.
- **Start now for Phase 2:** corpus licensing and recording (P2-Q01).

---

## 6. Phase 2: vocal quality and product qualification

| ID | Task | Detail | FR / Gate |
|---|---|---|---|
| P2-M01 | Vocal clarity goal (EQ) | Rule set plus fixtures. EQ response checked against `eq7_response`/`eq1_response`. Add `mix.insert` and `mix.insert_param(s)` for the qualified EQ to the manifest. | FR-04 / G05, G08 |
| P2-M02 | Even level goal (compression) | Rule set plus fixtures. Compressor static curve and attack/release tested with known signals. Processor order, bypass, state persistence. | FR-04 / G05, G08 |
| P2-M03 | Manifest growth | `mix.pan`, `mix.mute`, `mix.solo`, `mix.insert_bypass`, `mix.insert_remove`. Each goes through the full G04 matrix before admission. | FR-02 / G04 |
| P2-X01 | Guarded export | `file.bounce_mix` writes straight to `path` (§1), so the workflow uses a new command, `file.export_version {version, start, end, format, bit_depth, path, replace?}`, rather than calling it directly. (a) Version, range, format and destination are all required; no defaults are taken from the selection. `version` must be one of the P1-U06 kinds; anything else is refused as unavailable. (b) **Source protection:** canonicalise the path, following symlinks, and refuse any path that resolves to a session source file, a recorded take, the `.scraft` file, its sidecar or the project audio folder. The AI workflow has no override. (c) **Replacement:** an existing file is refused unless `replace` carries a confirmation bound to that exact canonical path and the target's identity (device, inode, size, modification time), given by an explicit user action. (d) **Protection at final publication, not just before rendering.** Render to a temporary file in the destination folder, then re-run (b) and (c) at publish time. A **new** file is published with `std::fs::hard_link(temp, dest)`, which fails atomically if anything has appeared at `dest`. A **confirmed replacement** first renames the existing file aside to a reported recovery name, checks that the moved file's identity matches the confirmation, and only then hard-links the new file into place. If the identity doesn't match or `dest` reappeared, the moved file is linked back and the export is refused. If it can't be linked back because something new now holds `dest`, it stays under the reported recovery name. Either way, no file is destroyed without a matching confirmation. This uses only safe `std` calls, so no new `unsafe`. On file systems without hard links, the new-file path uses `OpenOptions::create_new` on the destination and removes the partial file on failure; replacement there is refused. (e) No normalising or limiting unless the user selected and reviewed it. Dither is recorded in the export record, never applied silently. (f) Qualified formats are WAV PCM 16/24 and 32-bit float only. (g) A separate history record that isn't an undoable session edit (FR-10). Tests: export onto a source, a take, a symlink to a source, and the session file; replacing with and without a confirmation; and, **during rendering**, a destination that is created, modified, replaced by a different file, or swapped for a symlink to a source; also an unavailable version. **Must land before P2-Q05 (G11).** | FR-12, FR-10 / G08, G09 |
| P2-P01 | Selected-note practice | Target swara, octave and sustained region. Cents deviation, stability and confidence on valid voiced samples only, with transitions shown. | FR-03 / G07 |
| P2-P02 | Nearest-degree overlay | Optional scale. Equal temperament is a named option; custom cents offsets are under advanced controls. Never snaps or replaces the contour, and never calls a slide an error. | FR-03 / G07 |
| P2-P03 | Advanced MIDI comparison | Monophonic clip mapped to sample positions. User confirms timing and transposition. Overlapping notes, unsupported bends and unresolved timing are rejected with an explanation. | FR-03 / G07 |
| P2-Q01 | Pitch corpus | ≥160 excerpts, ≥16 singers, ≥30 min; holdout of ≥80 excerpts from ≥8 singers who are never used for tuning. Holdout includes ≥3 low-register and ≥3 high-register voices, and ≥10 excerpts and ≥3 singers per condition (low, high, breathy, slides, ornamentation). Reference F0 checked independently; disputed frames adjudicated. Commercial-use and consent terms kept separately. | FR-03 / G06, G07 |
| P2-Q02 | Freeze thresholds | Confidence thresholds and labels fixed on development data, then **frozen and versioned before the holdout is touched**. | G06, G07 |
| P2-Q03 | Scoring harness (**built in P0-T08:** `crates/pitch-eval`, see [pitch-eval-harness.md](pitch-eval-harness.md); extend rather than rewrite) | 10 ms grid, no octave folding, abstention counts as a miss. Primary and 20 ms-interior scores. Frame-pooled, per-singer, per-condition and singer-macro results. Singer-cluster bootstrap with ≥2,000 resamples, a named seed and 95% CIs. | G06, G07 |
| P2-Q04 | G03 full set | ≥200 locked prompts (valid, ambiguous, unsupported). Intended target recorded independently of the planner. ≥98% correct and zero wrong-target mutations. | G03 |
| P2-Q05 | G11 pilot | ≥10 target users on the working app, unaided. ≥8 complete, ≥7 keep a candidate, ≥7 do a second session within 14 days. A/B order randomised; the clean control is excluded from the preference target. | G11 |

**Exit (PRD §13.3):** the full imported workflow passes G03 to G08 and G11, with no unresolved
integrity defect. Condition-specific pitch failures and rejected processing results are published.

---

## 7. Phase 3: recording and early access

| ID | Task | Detail | FR / Gate |
|---|---|---|---|
| P3-R01 | Device controls | Input and output device selection, mono input channel, qualified buffer sizes. This delivers the relevant part of gaps.md G-01 (macOS only for this envelope). | FR-11 / G02 |
| P3-R02 | Alignment compensation | Measured input/output latency compensation for recorded takes; ≤ 2 ms alignment error and drift. | FR-11 / G02 |
| P3-R03 | Recorder repairs | The gaps measured in P0-T04 ([recorder-and-device-findings.md](recorder-and-device-findings.md) §4): a live recording-health state the UI shows during the take; stop and warn on the first write error and count what is discarded afterwards (13.9 s of a 24.1 s take were silently lost on a full disk); a header repair from the file length (the header trails the data by up to about 1 s after a crash); the manual `kill_nine` and `disk_full` tests as CI regressions; takes stored or linked in the project. Input opens only on an explicit record or monitor action. Estimate 8 to 14 h (guess). | FR-11 / G02, G09 |
| P3-R04 | Hardware direct-monitor workflow | Setup guide; warn against monitoring through the app at the same time. | FR-11 / G02 |
| P3-R05 | Crash-loss test | Kill the app mid-take: ≤ 1 s of captured audio lost under the declared conditions. Power loss is out of scope. | FR-11 / G09 |
| P3-R06 | Endurance | 3 × 30-minute runs at 44.1 kHz and 48 kHz on the named interface, with zero dropouts. | G02 |
| P3-R07 | *(Optional)* software monitoring | Measured input-to-output latency ≤ 15 ms on each advertised configuration. Advertise it only if it passes. | G02-SW |
| P3-F01 | Performance | 8-track workload, 30 minutes, zero dropouts with background work allowed. Callback p99 below 80% of its deadline and within 5% of baseline. Heavy jobs suspend while recording. UI p95 ≤ 200 ms. 60 s pitch job ≤ 30 s and 60 s render ≤ 60 s at p95. Companion and worker memory ≤ 2 GiB. | FR-13 / G10 |
| P3-P01 | Privacy and degradation | Offline run with zero app-originated requests. Captured payloads match the disclosed categories. Logs exclude credentials and raw audio. Opt-in, previewable crash reports. Users can delete chat and analysis data. Provider failure leaves manual use working. | FR-13 / G12 |
| P3-D01 | Packaging | Signed and notarised macOS (Apple Silicon) package bundling the companion and models, per the changes listed in [packaging-and-dependencies.md](packaging-and-dependencies.md) §5 (companion signed before the app, no new entitlements, models and notices under `Resources/`). Installs on the declared OS and interface matrix. | G13 |
| P3-D02 | Licence inventory | Start from the P0-T10 inventory ([packaging-and-dependencies.md](packaging-and-dependencies.md)): 223 external packages in the macOS app, 16 of them MPL-2.0 (symphonia). Add: model weights and their training-data terms (none verified yet), evaluation recordings, bundled assets, the OFL and Ubuntu font notices, the `self_cell` licence choice, `cargo-deny` in CI, and a legal check of MP3/AAC codec patents. Commercial redistribution checked if commercial distribution is chosen. | G13 |
| P3-D03 | Branding | Remove the ArtCraft branding as upstream requires for modified forks. Apply the final product name after review. | G13 |
| P3-D04 | Release documentation | Supported workflow, hardware and OS versions, monitoring mode, known limitations, privacy behaviour, who pays for inference, recovery instructions. No unqualified feature described as available. | G13 |

**Exit (PRD §13.4):** G01 to G13 pass for the declared envelope, plus G02-SW if software monitoring
is advertised. A recording failure keeps prototype status until a repaired build passes or a
versioned scope change is approved.

---

## 8. Traceability

### 8.1 Gates to tasks

| Gate | Evaluated in | Main tasks |
|---|---|---|
| G00 Early usefulness | Phase 0 | P0-D01 – P0-D06 |
| G01 Import and persistence | P1 subset, P3 full | P0-T03, P1-E07, P1-C05, P1 G01 subset |
| G02 Recording | Phase 3 | P0-T04, P0-T05, P3-R01 – P3-R04, P3-R06 |
| G02-SW Software monitoring *(optional)* | Phase 3 | P0-T05, P3-R07 |
| G03 Intent and targeting | P1 subset, P2 full | P1-E01, P1-E04 – P1-E06, P1-M03, P2-Q04 |
| G04 Execution safety | P1 subset, P2 full | P0-T06, P1-E02, P1-E06, P1-C02 – P1-C04, P1-C07 – P1-C09, P2-M03 |
| G05 Render fidelity | P1, P2 | P1-M02, P1-U02, P1-U03, P1-U05, P1-U06, P2-M01, P2-M02 |
| G06 Pitch accuracy | Phase 2 | P0-T08, P1-A03, P2-Q01 – P2-Q03 |
| G07 Confidence and references | Phase 2 | P1-A03, P1-A04, P2-P01 – P2-P03, P2-Q02, P2-Q03 |
| G08 Proposals and audition | P1 subset, P2 full | P0-T09, P1-M01, P1-U04 – P1-U06, P2-M01, P2-M02, P2-X01 |
| G09 Recovery | P1 subset, P3 full | P0-T04, P1-C02, P1-C03, P1-C05, P1-C07 – P1-C09, P2-X01, P3-R03, P3-R05 |
| G10 Performance | Phase 3 | P0-T10, P1-C06, P3-F01 |
| G11 Product pilot | Phase 2 | P2-Q05 |
| G12 Privacy and degradation | P1 local control, P3 full | P0-T07, P1-E03, P1-M03, P1-M04, P1-C06, P3-P01 |
| G13 Release readiness | Phase 3 | P0-T10, P3-D01 – P3-D04 |

### 8.2 Requirements to tasks

| Requirement | Tasks |
|---|---|
| FR-01 Session understanding | P1-E01, P1-A01, P1-A02, P1-I01 |
| FR-02 Controlled session editing | P1-E01, P1-E02, P1-E04 – P1-E06, P1-C04, P2-M03 |
| FR-03 Vocal pitch analysis | P0-T08, P1-A01, P1-A03, P1-A04, P1-I02, P2-P01 – P2-P03, P2-Q01 |
| FR-04 Focused vocal production | P0-T09, P1-A02, P1-M01 – P1-M03, P1-U01, P1-I03, P2-M01, P2-M02 |
| FR-10 Task history and recovery | P1-E01, P1-E02, P1-C01 – P1-C05, P1-C07 – P1-C09, P1-I05, P2-X01 |
| FR-11 Recording and project integrity | P0-T03, P0-T04, P1-E07, P1-C05, P1-C08, P3-R01 – P3-R05 |
| FR-12 Candidate comparison and export | P1-C04, P1-U02 – P1-U06, P1-I04, P2-X01 |
| FR-13 Job and capability management | P1-E03, P1-C06, P1-M03, P1-M04, P3-F01, P3-P01 |
| PRD §5.3 Required surfaces | P1-I01 – P1-I06 (design received; P1-I07 to P1-I09 proposed) |
| FR-05 to FR-09 | Deferred (PRD §6.9). No tasks in this plan. |

---

## 9. Open decisions

| # | Decision | Default in this plan | Resolved by | When |
|---|---|---|---|---|
| D1 | Companion language and runtime | **Resolved by P0-T10: Rust**, see [packaging-and-dependencies.md](packaging-and-dependencies.md). ONNX engine if a neural model is chosen: `tract` first (pure Rust, no library to bundle) | P0-T10 (done) | Closed |
| D2 | F0 tracker | Chosen by benchmark; YIN is the baseline | P0-T08 | Phase 0 exit |
| D3 | Reference audio interface | One named interface, to be chosen | P0-T05 | Week 1 |
| D4 | Cloud text-model provider | One replaceable adapter; terms, rates and intent reliability checked | P1-M03, P1-M04 | Phase 1 |
| D5 | Does a selection change invalidate a plan? | **Resolved by P0-T06: no.** The raw revision moves on selection and playhead changes, which would stale every plan. Use the content revision (P1-E01). Selection-reading commands are still blocked by P1-E06's no-fallback rule | P0-T06 (done) | Closed |
| D6 | Licence and pricing model | Unresolved (open source vs open core) | Product owner | Before external distribution or a paid pilot |
| D7 | Product name | "SoundCraft-AI" working name | Branding review | Before P3-D03 |
| D8 | Grouped tracks in assistant edits | Phase 1 rejects any operation whose scope diff reaches active group members, and explains why. Later options: list group members as explicit targets in review, or add an engine parameter that suspends group propagation for one call | P1-E06 findings; G00/G11 feedback | Phase 2 |
| D9 | Untagged MP3 delay | Don't guess: import as decoded (+1104 samples) and report "no gapless information" in the import result. Alternative: apply the standard 1105-sample LAME delay by default | P0-T03 finding; user feedback | Phase 1 (with P1-E07) |
| D10 | How to fix the AAC start trim without modifying symphonia (16 MPL-2.0 crates: modified files must be published) | Read the container's gapless data (edit list, iTunSMPB) in `soundcraft-audio-io` and trim after decode, leaving symphonia unmodified. A symphonia patch is the fallback, with the changed files published under MPL-2.0 and listed in the notices | P1-E07; [packaging-and-dependencies.md](packaging-and-dependencies.md) §3 | Phase 1 |

## 10. Risks and triggers

| Risk | Early signal | Response |
|---|---|---|
| Snapshot swap misses side effects (player plugin state, source pool, queued transport) | P0-T06 finds state outside `Session` | Keep those operations out of the manifest; extend staging before admitting them |
| Recorder fails integrity tests | P0-T04 data loss or a silent writer error | Continue as an imported-audio prototype; repairs in P3-R03; G02 can't pass on direct monitoring alone |
| Pitch fails on low register, breath or ornamentation | Per-condition scores in P0-T08 or P2-Q03 | Report per-condition results; expand the corpus before expanding claims |
| Deterministic processing sounds poor | G00 or G11 rejections | Keep "keep original" as a valid outcome; revise the rules under a new version |
| Corpus is late | P2-Q01 not started by mid-Phase 1 | Start licensing in Phase 1; Phase 2 gates wait for it rather than using a smaller corpus |
| Phase 0 overruns | Hours > 40 or week 4 reached | Issue go, repair, stop or inconclusive on the deadline (PRD §13.1) |
| Native command side effects reach outside the approved scope (group propagation, exclusive solo, selection fallback) | P1-E06 scope diff rejects a plan | Reject and explain; never publish a partial or widened change; resolve via D8 |
| A recovered task is reported as applied when it wasn't, or the reverse | A receipt or a missing receipt is read as proof of current state | Receipts are history only (P1-C07); presence comes from saved-state evidence (P1-C08); unresolved outcomes stay Recovery required (P1-C09) |
| Upstream drift | Upstream changes `Engine`, `record.rs` or the control server | Keep `fix/*` patches small; re-run workflow regression tests when merging upstream |

---

## Revision history

| Version | Date | Change |
|---|---|---|
| 0.1 | 14 October 2026 | First plan from PRD v0.3, grounded in source inspection of `6cf896e`. |
| 0.2 | 14 October 2026 | Review fixes: strict operation validation and scope diff (P1-E06, D8); receipt store, status query and crash-case contract (P1-C07); cloud budget enforcement (P1-M04); render acceptance checks (P1-U05); guarded export (P2-X01); discovery sessions by day 21, with a frozen rule for open follow-up windows (P0-D01, D03, D05, D06). |
| 0.3 | 14 October 2026 | Review fixes: receipts made history-only (P1-C07); saved-state evidence from a lineage marker and a content check that survives undo, redo and later edits (P1-C08); crash contract keeps unresolved outcomes as Recovery required and reads autosaves (P1-C09); the sidecar is also written by `save_copy` and autosave (P1-C05); version retention (P1-U06); export protection re-checked at final publication with atomic no-overwrite (P2-X01). |
| 0.4 | 14 October 2026 | P0-T01 and P0-T03 results: CI passes on Rust 1.97.0 (the repository needs 1.95 or later); AAC priming not trimmed, so P1-E07 and D9 added; import and save/reopen evidence recorded in `import-matrix.md`. |
| 0.5 | 14 October 2026 | P0-T06 spike results: the raw revision moves on selection and playhead changes, so staleness uses a content revision (P1-E01, P1-E02); D5 closed; guarded group publication shown to work on the existing history; findings in `spike-guarded-publish.md`. |
| 0.6 | 14 October 2026 | P0-T07: control channel protection designed and tested (`control-channel-protection.md`); the browser-to-loopback attack is demonstrated; P1-E03 now names the design, the 7 ignored tests and an estimate. |
| 0.7 | 14 October 2026 | P0-T09: production policy rule format and `level.headroom` 0.1.0 with 44 fixtures and property tests (`production-policy.md`); a bounds-clamp defect found by the property tests and fixed; P1-M01 and P1-A02 point at it; the new crate is listed in the code-location table. |
| 0.8 | 14 October 2026 | P0-T10: companion language decided (Rust, D1 closed); resource measurements (YIN over 60 s: 2.8 s and 48 MiB; the 8 × 10 min stereo envelope: about 2.0 to 2.3 GiB in the engine process); licence inventory (symphonia is MPL-2.0, so D10); model and runtime options with code licences read from the repositories; packaging plan. Findings in `packaging-and-dependencies.md`. |
| 0.9 | 14 October 2026 | Drafted the G00 protocol (criteria, count rules, rubric, decision rules, manifest) and consent form for P0-D01 and P0-D02. Both await your approval; nothing is frozen yet. |
| 0.10 | 14 October 2026 | Phase 0 technical tasks finished as far as they can go without hardware or data: P0-T01 (all CI steps pass on the inspection host), P0-T04 (recorder writer path measured), P0-T05 software half (device list, callback sizes), P0-T08 scoring harness (`crates/pitch-eval`). Remaining: M1 run, interface and loopback, annotated recordings and the pYIN-style tracker. |
| 0.11 | 14 October 2026 | G00 protocol approved as written (P0-D01): criteria, rubric and decision rules frozen. Still open before day 1: the start date and capacity record, and the consent form's blanks and approval. |
| 0.12 | 14 October 2026 | P0-T08: the probabilistic (pYIN-style) F0 tracker is built in `soundcraft-dsp` with a voicing score and a pitch score, verified against an exact posterior on a tiny model; two defects found by failing tests were fixed. It beats YIN on synthetic noisy tones and phrases; neither works at 0 dB SNR. Natural-recording accuracy remains unmeasured. |
| 0.13 | 14 October 2026 | UI design received (Claude Design bundle, rounds 1 to 5) and digested into `design/ui-design-spec.md`, `design/tokens.json` and `design/copy.json`; P1-I01 to P1-I05 now name their boards; P1-I07 (theme) and P1-I08 (Arrange restyle) proposed; the Approve versus Keep conflict is recorded under P1-I03. |
| 0.14 | 14 October 2026 | UI-1 approval flow decided (`design/decisions.md`): a production proposal is approved by Keep in the audition and has no Approve in the change review; an explicit assignment is approved in the change list; the controller sets `approval_path`. P1-C01, P1-C04, P1-I03 to P1-I05 updated with acceptance tests. |
| 0.15 | 15 October 2026 | Round 6 design imported and reviewed: 63 icons (16 px genuinely redrawn), 15 messages, the real fader law confirmed, hit areas, state sheets; extractor extended (`icons.json`, `messages.json`); P1-I09 (icons in code) proposed; open issues listed in `design/ui-design-spec.md` §9. |
