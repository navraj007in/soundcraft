# Code and architecture audit — 15 October 2026

**Reviewed baseline:** `0882cc74e27f37d081634f37011fb7d2b633da00` plus the current working tree.
**Disposition:** the crate boundaries are sound, but recovery, rendering and control contain
integrity defects that should be repaired before relying on the app for recordings or unattended edits.
This is an audit, not a repair: no production code was changed.

## Scope and evidence

Reviewed the workspace dependency rules, session model and persistence, command execution and
undo, routing and rendering, playback and recording, plugin lifecycle, TCP/MCP control, and CI
entry points. UI code was inspected for state and transport integration; visual fidelity was not
qualified. Existing UI/design edits were present and continued changing during the audit.

The prescribed `plan/STATUS.md`, `plan/execution-plan.md` and `../../craftrules/AGENTS.md` were
absent. Fetching the shared instructions from `storytold/craftrules` also returned 404. The
available repository instructions, `ROADMAP.md`, `docs/gaps.md`, architecture documentation and
SoundCraft-AI investigation notes supplied the baseline. No shared standards were invented.

Evidence labels distinguish **reproduced** behavior from **source-traced** findings. Seven new
characterization tests reproduce seven findings below; an existing, ignored HTTP protection
test also demonstrates its known vulnerability. The attached tests assert the current defective
behavior so that the observations can be repeated; they are not passing correctness regressions.

## Findings

P1 means a high-priority integrity, security or audio execution defect. P2 means a correctness
defect to schedule next. No release-wide P0 claim is made.

| ID | Priority | Finding | Evidence |
|---|---|---|---|
| A01 | P1 | Autosaves can recover another session's audio | Reproduced |
| A02 | P1 | Enabled control server forwards HTTP-framed commands without authentication | Reproduced; already documented |
| A03 | P1 | Plugin instances retain the previous session's opaque state | Reproduced with a mock hosted plugin; player path source-traced |
| A04 | P1 | Master and cascaded aux latency shift and truncate bounces | Reproduced with built-in DSP |
| A05 | P1 | Remote retries can apply a mutation twice | Reproduced with a loopback mock server |
| A06 | P1 | Save/autosave serializes plugin state inside the audio callback | Source-traced |
| A07 | P1 | Long bounces succeed with silently shortened output | Source-traced; large allocation deliberately avoided |
| A08 | P1 | Steady rendering allocates for MIDI and waits on general worker threads | Source-traced; deadline impact not benchmarked |
| A09 | P2 | Gesture undo breaks when history reaches its cap | Reproduced |
| A10 | P2 | Solo propagation stops after the first aux | Reproduced |
| A11 | P2 | Session deserialization bypasses timebase invariants | Reproduced; overflow panic demonstrated in test profile |

### A01 — Give recovery media an identity beyond a local numeric ID

At [engine/src/io.rs:573](../../../crates/engine/src/io.rs), autosave mode reuses
`<source-name>_<source-id>.wav` whenever the file exists, without checking its contents or
ownership. `SoundApp::autosave` chooses the recovery directory from the sanitized session name.
Different sessions routinely have the same name and restart IDs from 1.

Saving recovery copies of two fresh `Untitled` sessions, each with a source named `Vocal`, made
the second session recover the first session's samples: **0.5 instead of -0.25**. The operation
reported success. Use a persistent session identity for recovery directories and validate media
identity before reuse, for example with content hashes. Preserve recovery of both sessions.

### A02 — Reject HTTP framing and authenticate the control endpoint

At [control_server.rs:39](../../../apps/soundcraft/src/control_server.rs), each newline is parsed
independently. Invalid HTTP header lines receive errors, but parsing continues into a valid JSON
body, which is forwarded to the UI. There is no token check. This applies when control is enabled
with `--control` or `SOUNDCRAFT_CONTROL_PORT`; it is not a claim about the default launch.

The existing ignored `an_http_request_carrying_a_json_body_is_not_executed` test failed because
`engine.execute` was forwarded. The reproduction compiled the production server with a minimal
UI adapter that only records requests, so no real session was mutated. This confirms the known
P0-T07/P1-E03 issue in `docs/soundcraft-ai/control-channel-protection.md`. Add authentication,
first-line protocol rejection, request size and connection limits, and the planned method
allowlist. HTTP rejection also belongs in developer mode. The server's unbounded line reader
and connection threads are related resource-exhaustion risks.

### A03 — Include session generation and explicit restoration in plugin lifecycle

At [playback/src/lib.rs:679](../../../crates/playback/src/lib.rs), `Side::prepare` skips creation
when the track ID, slot, plugin ID and channel count match. It ignores the session's identity and
stored state. The mixer also retains matching slots during `sync`; its structural fingerprint
does not include opaque plugin state. `ResetSession` stops the player but does not clear these
instances or the preparation cache.

A mock hosted instance containing `SESSION A` retained that blob after resetting and syncing a
second session whose matching insert stored `SESSION B`. This can change what the user hears
after open/revert and can overwrite B's intended state on the next save capture. Add an explicit
session generation/reset and a separate state-restoration path. Avoid rebuilding merely because
a routine state capture refreshed the document. Verify open, revert and undo with a real host
fixture after repair; this audit did not load a real plugin to reproduce the state substitution.

### A04 — Calculate compensation over complete signal paths

At [mix/src/lib.rs:628](../../../crates/mix/src/lib.rs), latency is the maximum independent-track
latency plus the maximum individual aux latency. Serial aux latencies are not accumulated.
Master inserts run later, at line 681, without contributing to this total. `render_range` removes
only the reported latency and uses it to decide how many trailing frames to render.

At 48 kHz, neutral-ratio compressors with 5 ms lookahead reproduced both failures. Original
impulses were at **[0, 1024, 2000]** in a 2048-frame range. A compressor on the master, or two
compressors on chained auxes, yielded **[240, 1264]**: a 240-frame offset and the final impulse
lost. Build cumulative latency and compensation from the routing graph, include master paths,
and render the entire required drain interval before trimming.

### A05 — Do not replay mutations after an ambiguous disconnect

At [automation/src/lib.rs:119](../../../crates/automation/src/lib.rs), `Remote::call` reconnects
and repeats a request after connection-closed/reset/broken-pipe errors. The first request may
already have committed. The retry uses a new request ID, and the server has no deduplication.

A mock server committed `track.new`, dropped the first reply, then accepted the retry. **One
successful client call committed twice**, with IDs 1 and 2. Restrict automatic retry to known
read-only calls, or implement stable execution IDs with stored receipts and outcome queries.
For a mutation whose outcome is unknown, return an explicit ambiguous-result error.

### A06 — Move state serialization out of the callback

At [playback/src/lib.rs:139](../../../crates/playback/src/lib.rs), draining `CaptureStates` calls
`MixEngine::capture_states`, which invokes each plugin's `save_state` and allocates buffers.
`SoundApp::run` requests this before saves/bounces, and autosave also requests it. Performing the
work "between blocks" is still work on the audio callback's deadline. Plugin serialization has
no execution-time bound. The two-second caller timeout neither cancels that work nor reports
capture failure distinctly from a legitimate empty state result.

Use a safe ownership handoff or host-specific state capture arrangement outside the callback,
and make capture failure explicit before claiming a complete save. Qualify saves during
playback with a deliberately slow state fixture. This finding is a call-path observation, not a
measured dropout on hardware.

### A07 — Return an error or stream when a render exceeds its memory budget

At [mix/src/lib.rs:1746](../../../crates/mix/src/lib.rs), `render_range` silently takes the minimum
of requested frames and `MAX_RENDER_SAMPLES / channels`. `render_bus` does the same.
[engine/src/io.rs:261](../../../crates/engine/src/io.rs) rejects bounces only beyond four hours,
then encodes the returned buffer and reports success without checking the requested duration.

The 2^29 aggregate-sample cap permits only **93.2 minutes of stereo at 48 kHz**, or **15.5 minutes
of 12-channel audio at 48 kHz**. Longer requests within four hours produce shortened files.
Propagate a size error before allocation, or implement streaming renders/encoding. The lengths
follow directly from the code; a multi-gigabyte reproduction was intentionally not run.

### A08 — Enforce realtime rules in the render path

At [mix/src/lib.rs:1322](../../../crates/mix/src/lib.rs), scheduling an overlapping MIDI clip
formats four `rtp.<track>.<property>` lookup strings per block whenever `edit.values` is nonempty.
Any numeric setting activates this branch, even when no MIDI realtime property is set. These
heap allocations persist after warmup. Precompute typed property values or lookup keys off the
callback.

At [mix/src/lib.rs:1129](../../../crates/mix/src/lib.rs), four or more independent strips invoke
Rayon's general pool synchronously from the same callback. Completion depends on worker
scheduling and competing work; audio-thread marking only changes the thread-local label. Use
serial callback processing as a correctness baseline or a prepared worker design with explicit
realtime scheduling and bounded synchronization. Add allocator/deadline instrumentation for
steady MIDI, monitoring, parameter gestures, and save scenarios. No latency benchmark was run.

### A09 — Coalesce gestures before enforcing the history limit

At [engine/src/lib.rs:328](../../../crates/engine/src/lib.rs), coalescing assumes an undo push
increases stack length by one. Once `push_undo` evicts the oldest entry at the cap, length stays
constant, so every drag update becomes another entry and evicts the gesture's original state.

With a cap of two, starting at -2 dB and dragging through -3, -4 and -5 dB made Undo restore
**-4 dB, rather than -2 dB**. The same mechanism occurs at the default cap of 200. Preserve a
gesture's initial snapshot explicitly, independent of stack length and eviction.

### A10 — Propagate implicit solo through the routing graph

At [mix/src/lib.rs:1192](../../../crates/mix/src/lib.rs), `receives_solo` recognizes only a direct
feed from a track whose own solo bit is set. An implicitly audible aux does not carry that bit,
so the next aux in the chain is muted.

For `Audio → Aux1 → Aux2 → Main`, soloing Audio changed the observed output from **0.35397 to
0**. Compute reachable solo paths transitively, with cycle protection, and test nested returns,
sends and solo-safe behavior. This can share routing analysis with the compensation repair.

### A11 — Validate deserialized timebases before publishing a session

[time/src/lib.rs](../../../crates/time/src/lib.rs) derives `Deserialize` for `SampleRate`, bypassing
the bounds in `SampleRate::new`. `TempoMap` also derives deserialization, bypassing setter
validation and sorted-map assumptions. [Session::sanitize](../../../crates/model/src/session.rs)
does not repair these fields.

A session with rate 0 and tempo ticks `[0, i64::MIN, i64::MAX]` loaded successfully. A subsequent
`samples_to_ticks` call panicked on subtraction overflow at
[tempo.rs:182](../../../crates/time/src/tempo.rs). This was demonstrated in the normal test
profile with overflow checks; a release-profile crash was not demonstrated, and unchecked
wrapping there remains incorrect. UI and callback conversions are outside `Engine::execute`'s
panic boundary. Validate rate, tempo/meter ranges, ordering and required initial events, then
use checked arithmetic defensively. Current hostile-session tests clamp mutations to ±9e15
and do not exercise these exact integer boundaries.

## Architecture assessment and repair order

The dependency architecture is worth keeping. `cargo xtask layers` passes. The model is UI
independent, commands are shared across interfaces, realtime/offline mixing use the same DSP,
and unsafe host code has explicit crate boundaries. Property tests and synthetic fixtures are
substantial strengths. These boundaries make focused repairs possible without a rewrite.

Three cross-cutting contracts need attention:

1. **Persistent identity:** recovery media and live plugin instances need session identity, not
   just display names and document-local numbers. Repair A01 and A03 first.
2. **Routing as a graph:** ordering, solo reachability and cumulative latency currently use
   separate limited analyses. A validated graph can supply all three. Repair A04 and A10 together.
3. **Bounded execution:** realtime rendering, state capture and remote execution need explicit
   timing and delivery contracts. Repair A02/A05 before unattended control and A06/A08 before
   qualifying low-latency use. Make A07 an explicit error immediately if streaming is deferred.

Other observed architecture gaps remain:

- Recording faults are known and still present. Existing playback fault tests confirm that disk
  write failures and ring overruns reach the public API only at `finish`; a failed writer can
  discard input while the session still appears to record. Their detailed investigation is
  `docs/soundcraft-ai/recorder-and-device-findings.md`.
- `execute_group` is explicitly a spike. It accepts arbitrary command IDs while promising
  all-or-nothing behavior; snapshots cannot undo external file writes, and the rollback does
  not restore every engine field. Enforce the planned admitted-operation manifest before
  exposing this as a general transaction API.
- Some UI document edits bypass commands, for example session comments in
  `crates/ui-egui/src/windows.rs`. Move persistent edits behind commands for undo and control parity.
- Ordinary engine/DSP PRs are not covered by a general test workflow in this repository.
  Packaging, FreeBSD and Windows ARM workflows have narrow path triggers. Add the documented
  core CI matrix and preserve deterministic fixtures separately from installed-plugin checks.
  External reusable workflow internals and branch protection settings were not audited.
- Progress and architecture documentation has drifted: `docs/architecture.md` and G-06 still
  describe recording into RAM, while the implementation has a disk writer. Readiness claims
  should be updated from current evidence without counting the writer's presence as qualification.

## Verification results

| Check | Result |
|---|---|
| Core suites: engine, model, mix, playback, automation | **259 passed, 4 ignored** |
| Foundation suites: time, audio-io, MIDI, DSP, video, assist-policy, pitch-eval | **297 passed, 2 ignored** |
| Seven added characterization tests | Defects reproduced; tests kept as audit attachments |
| Existing ignored HTTP protection test | **Failed as expected:** HTTP body forwarded `engine.execute` |
| Clippy, five core library packages, `-D warnings` | Passed |
| `cargo xtask layers` | Passed |
| `cargo xtask assets` | Passed at time checked |
| `cargo fmt --all -- --check` | Failed in concurrent UI work: `icon_set.rs`, `oklch.rs`, `theme.rs` |

The first sandboxed core run failed the Apple Audio Unit insertion test because the native
component was unavailable in that environment. It passed outside the sandbox, as did the
subsequent complete core run. Loopback tests also needed sandbox exemption. Those restrictions
are not counted as code defects. Full workspace CI, WASM, Windows/Linux/FreeBSD runtime,
hardware recording, plugin crash containment, UI rendering and performance were not qualified.

### Repeating the reproductions

Run from the repository root. Copy the attachments into temporary integration-test locations
that do not already exist, then remove only those copies after the checks:

```sh
/bin/cp -f docs/audits/2026-10-15/engine-repros.rs crates/engine/tests/audit_20261015.rs
/bin/cp -f docs/audits/2026-10-15/remote-repro.rs crates/automation/tests/audit_20261015.rs
/bin/cp -f docs/audits/2026-10-15/control-repro.rs crates/engine/tests/audit_control_20261015.rs
cargo test --offline -p soundcraft-engine --test audit_20261015 -- --nocapture
cargo test --offline -p soundcraft-automation --test audit_20261015 -- --nocapture
cargo test --offline -p soundcraft-engine --test audit_control_20261015 an_http_request_carrying_a_json_body_is_not_executed -- --ignored --nocapture
rm crates/engine/tests/audit_20261015.rs crates/automation/tests/audit_20261015.rs crates/engine/tests/audit_control_20261015.rs
```

The control attachment uses the actual server source and a recording UI adapter. The HTTP check
is expected to fail until A02 is repaired. The remote test uses a mock server; neither test edits
a live SoundCraft session. Loopback access must be permitted. After repair, convert each
characterization into a correctness assertion in the appropriate crate's permanent tests.
