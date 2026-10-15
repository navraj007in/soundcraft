# Code review and architecture audit — verified follow-up

Date: 15 October 2026. Initial revision: `909f3da3e47e4ff14e8d59668341e5e1478b25be`.
During this review, concurrent UI work was committed as `ea8648f`; the reviewed core code was
unchanged. This report reviews the repository as a whole, not just that UI diff.

**Assessment:** retain the crate architecture, but repair the recovery, rendering and control
contracts before qualifying SoundCraft for dependable recording or unattended editing.
The most serious problems can silently change recovered audio, plugin sound, exported timing,
or the number of times an edit is applied. Passing unit tests do not establish those contracts.

The existing `README.md` audit and three reproduction attachments were present before this
review. Their seven characterization cases were independently rerun, and their remaining
findings were traced through the current source. Three additional cases are attached in
`additional-repros.rs`. Production code was not changed. No PR or commit was created.

## Scope and limits

| Area | Review performed |
|---|---|
| Dependency architecture | Workspace manifests, layer checker, UI/engine/model boundaries, unsafe host boundaries |
| Persistence and commands | Save/recovery paths, session loading and sanitization, undo coalescing, grouped execution |
| Audio execution | Routing, compensation, reset, MIDI event scheduling, callback work, recorder failure reporting |
| Plugin hosting | Preparation/adoption/state-capture call paths; selected FFI lifecycle and buffer handling |
| Control | TCP framing, retry behavior, command dispatch, protection tests |
| Foundation and AI additions | Existing time, audio-I/O, MIDI, DSP, video, policy and pitch-evaluation test suites; selected input validation |
| UI and web | Session/transport integration, direct model mutation and browser import paths; no visual qualification |
| Delivery | Repository CI triggers, formatting, layer and asset gates, compiler checks |

This is a broad risk-focused review, not a line-by-line proof of every DSP algorithm or unsafe
block. Physical audio devices, clock drift, latency under CPU pressure, malicious native plugins,
browser runtime and non-macOS runtime behavior were not qualified. No proprietary application
resources were read. The mandated local planning files and shared `craftrules` checkout were
absent; attempts to retrieve the shared instruction URLs returned 404.

## Prioritized findings

P1 is a high-priority integrity, security or execution-contract defect. P2 is a correctness or
API-boundary defect to schedule next. These findings include existing documented defects; they
are not all newly discovered. IDs A01–A11 preserve the earlier audit's mapping.

| ID | Priority | Finding | Evidence |
|---|---|---|---|
| A01 | P1 | Recovery can substitute another session's audio | Reproduced |
| A02 | P1 | Enabled TCP control executes unauthenticated HTTP-framed JSON | Existing negative test fails |
| A03 | P1 | Matching plugin slots retain a previous session's opaque state | Mock instance reproduction + player trace |
| A04 | P1 | Master and cascaded aux latency shifts and truncates bounces | Reproduced with built-in DSP |
| A05 | P1 | Remote retries can apply one mutation twice | Reproduced on loopback |
| A06 | P1 | Save/autosave invokes plugin serialization in the audio callback | Source-traced |
| A07 | P1 | Long renders report success with silently shortened output | Source-traced; no multi-GiB allocation attempted |
| A08 | P1 | Steady callback work allocates and depends on general worker scheduling | Source-traced; no deadline benchmark |
| A15 | P1 | Recorder write failures remain invisible until stop | Existing fault tests pass by demonstrating defect |
| A09 | P2 | Capped undo history breaks gesture coalescing | Reproduced |
| A10 | P2 | Solo propagation mutes the second aux in a chain | Reproduced |
| A11 | P2 | Deserialization bypasses timebase invariants | Reproduced, including test-profile overflow panic |
| A12 | P2 | Stop/reset leaves stale audio in compensation buffers | New reproduction |
| A13 | P2 | A note ending at a clip/block boundary misses its note-off | New reproduction |
| A14 | P2 | Group rollback keeps a changed save path and external file | New reproduction of known spike limitation |

### A01 — Separate recovery identity from display names and local IDs

`crates/engine/src/io.rs:573–581` reuses an existing `<name>_<source-id>.wav` without checking
ownership or contents. `crates/ui-egui/src/lib.rs:525–526` chooses the recovery directory using
only the sanitized session name. Two fresh same-named sessions restart their source IDs.
The second recovery copy of a `Vocal` source restored **0.5 instead of its own -0.25 sample**.
Use a persistent session identity for recovery directories and verify immutable media identity
before reuse. Preserve separate recovery histories for distinct sessions.

### A02 — Authenticate control and reject foreign protocol framing

`apps/soundcraft/src/control_server.rs:39–54` parses each line independently, continuing after
invalid HTTP headers until a valid JSON body is forwarded. There is no authentication in the
production entry point. This applies when `--control` or its environment setting enables the
listener. The isolated adapter test forwarded `engine.execute` from an HTTP-framed request.
It proves the server behavior, not exploitability in every browser/network configuration.
Reject HTTP on the first line, authenticate privileged clients, and bound line size, connection
count and idle time. The existing protection plan already calls for these measures.

### A03 — Reset plugin ownership on session replacement

`crates/playback/src/lib.rs:679–680` treats matching track ID, slot, plugin ID and channel count
as sufficient to reuse an instance. `crates/mix/src/lib.rs:516–525` similarly keeps a matching
slot; `ResetSession` only stops and locates the existing player. A mock instance containing
`SESSION A` still reported A after resetting and syncing a session storing `SESSION B`.
Introduce a session generation and explicit restore operation. Treat capture and restore as
different operations so routine state capture does not continually rebuild plugins. Add real
host-fixture coverage for open/revert and state changes; the reproduction used a mock host.

### A04 — Compute latency along complete routing paths

`crates/mix/src/lib.rs:628–633` adds a maximum independent-strip latency to a maximum individual
aux latency. It neither accumulates serial aux delays nor includes master inserts, which process
later. Offline rendering trims and drains using that incomplete number. At 48 kHz, neutral-ratio
compressors with 5 ms lookahead moved impulses from **[0, 1024, 2000] to [240, 1264]** with either
a master compressor or two serial aux compressors. Include cumulative graph and master latency,
align all merge points, and render the full drain interval before trimming.

### A05 — Do not retry a mutation whose result is unknown

`crates/automation/src/lib.rs:119–127` retries after selected disconnect errors, allocating a new
request ID without server deduplication. A mock server committed the first request and dropped
its reply: one successful client call then committed **twice**, with IDs 1 and 2. Restrict blind
retry to read-only operations. Mutations need stable execution IDs/receipts or an explicit
unknown-outcome error that the caller can reconcile.

### A06 — Move state capture off the callback and report failure

`crates/playback/src/lib.rs:139–141` calls `capture_states` while draining callback commands.
`crates/mix/src/lib.rs:436–450` invokes plugin `save_state` and allocates result buffers there.
The UI requests this for saving, bouncing and autosaving. Work between blocks is still callback
work, with no bound on native plugin serialization. The two-second wait returns an empty result
on timeout (`playback/src/lib.rs:538`), indistinguishable from having no stateful plugins.
Provide a host-appropriate off-callback ownership/state protocol and propagate capture failure
before reporting a complete save. Test with a deliberately slow serializer.

### A07 — Make render limits explicit errors or stream output

`crates/mix/src/lib.rs:1746` silently caps frame count at `2^29 / channel_count`; `render_bus`
does likewise. `crates/engine/src/io.rs:261` accepts up to four hours and encodes the shortened
buffer as success. Stereo at 48 kHz hits the cap at **93.2 minutes**; 12 channels hit it at
**15.5 minutes**. Return a size error before allocating or render/encode incrementally. A
bounded render API must convey both completion and requested range, rather than only samples.

### A08 — Enforce callback allocation and scheduling contracts

`crates/mix/src/lib.rs:1322–1331` formats four lookup strings for every overlapping MIDI clip
per block when `edit.values` is nonempty. This is steady-state heap allocation. At
`mix/src/lib.rs:1129–1132`, four or more independent strips synchronously dispatch through the
general Rayon pool. Marking workers as audio threads changes logging behavior, not scheduling
priority or completion bounds. Precompute typed MIDI properties off-thread and use a serial
realtime baseline or prepared workers with an explicit deadline/synchronization design. Add
allocator and deadline instrumentation; this review did not measure a hardware dropout.

### A09 — Coalesce gestures independently of stack length

`crates/engine/src/lib.rs:328` requires undo length to grow by one before merging entries.
Eviction at the history cap keeps the length constant. With a cap of two, dragging from -2 dB
through -3, -4 and -5 dB made Undo restore **-4 instead of -2 dB**. The default cap of 200 has
the same mechanism. Preserve the gesture's initial snapshot before applying history eviction.

### A10 — Propagate implicit solo transitively

`crates/mix/src/lib.rs:1192–1197` recognizes only a direct feed from a track with an explicit
solo bit. For `Audio → Aux1 → Aux2 → Main`, soloing Audio changed output from **0.35397 to 0**.
Compute reachability over the routing graph, including sends and solo-safe rules, with cycle
protection. Share the graph analysis with compensation and processing order.

### A11 — Validate timebases at the deserialization boundary

`crates/time/src/lib.rs:37–38` derives deserialization for `SampleRate`, bypassing `new`'s bounds;
`TempoMap` also bypasses setter validation. `Session::sanitize` does not restore their
invariants. A zero rate and tempo ticks `[0, i64::MIN, i64::MAX]` loaded successfully, then
panicked in `tempo.rs:182` on subtraction in the test profile. A release crash was not
demonstrated; wrapping arithmetic would still produce invalid time conversions. Validate rate,
event order, initial events and tempo/meter ranges before publication, and use defensive
arithmetic. Include full-width integer boundaries in hostile-session tests.

### A12 — Clear compensation buffers on discontinuities

`crates/mix/src/lib.rs:467–477` resets plugins and held notes but leaves `align`, `align_pre`
and `out_extra` untouched. `DelayLine::set` retains their samples when dimensions do not
change. After playing DC alongside a latent strip, resetting and seeking to a silent region
emitted **240 old samples**. Clear these buffers without reallocating on stop/start/seek and
session reset. Also review clip-effect and plugin tail state under the same discontinuity
contract. The added test uses built-in DSP and requires no audio device.

### A13 — Deliver note-offs even when a clip no longer overlaps the block

`crates/mix/src/lib.rs:1317–1318` skips clips outside the half-open render interval, while
`1340–1351` caps note-off to the clip end and excludes block-end events. When both ends are
sample 512, block zero excludes the off and block one skips the clip entirely. Over eight
contiguous blocks a mock instrument received **one note-on and zero note-offs**. Sustain-based
instruments can continue sounding until reset. Keep pending note-offs independently of clip
overlap, and cover block ends, trimmed clips, loops and same-pitch overlapping notes.

### A14 — Constrain grouped commands to truly reversible edits

`crates/engine/src/group.rs:64–80` snapshots only part of engine state and permits arbitrary
command IDs. `session.save_as` followed by an invalid operation returns “nothing was applied”
but leaves both the new file and `Engine.path` changed. A later ordinary save can therefore
target the wrong file. This API is explicitly a spike and was not found exposed through the
current control dispatch, so the finding concerns its public Rust API and future integration.
Admit only validated reversible document operations; prohibit external effects and session
replacement, or implement a separate staged publication protocol. Rollback must cover all
observable engine fields, not only the document and undo history.

### A15 — Publish recorder faults while recording

`crates/playback/src/record.rs:231–258` latches a writer error and silently stops writing that
take. `write_takes` keeps draining, while only `Recorder::finish` returns the failure. Existing
fault tests explicitly show half a second discarded with zero ring-loss count, and ring
overruns visible only at stop. Expose a live fault channel/status, preserve partial takes, and
give the host enough information to stop or visibly mark a failed recording immediately.
This is already documented in the recorder investigation; disk streaming exists, but its
presence does not establish safe recording behavior.

## Architecture assessment

The layer split is coherent and passes the repository gate. The UI-free model and shared
command registry enable headless testing. Shared realtime/offline DSP reduces implementation
drift. Arc-backed audio avoids duplicating sample buffers across undo snapshots, and unsafe
plugin integration is confined to named helper crates. Keep these foundations.

The dominant weaknesses are runtime contracts across those layers:

| Contract | Current weakness | Recommended boundary |
|---|---|---|
| Session/media identity | Display names and document-local IDs reused across lifetimes | Persistent session identity plus immutable media identity and runtime generation |
| Routing | Ordering, compensation and solo each use separate limited analyses | Validated routing graph with deterministic order, reachability and cumulative latency |
| Realtime ownership | Session sync, state capture and worker dispatch happen inside deadlines | Prepare immutable render plans and state off-thread; bounded handoff and retirement |
| Edit publication | Public `session_mut`, UI direct edits, partial transaction snapshots | Typed edits through a single commit/undo boundary; explicit external-effect operations |
| Native plugins | FFI crate isolation limits Rust unsafe scope but not process failure | Explicit host lifecycle/thread contracts; process isolation for crash containment |
| Media scale | Whole-file decode and whole-render buffers; takes decoded whole at stop | Streaming readers/writers with bounded caches and explicit completion/error results |
| Remote execution | Transport retry treated as safe command retry | Stable execution identity, receipts and unknown-outcome handling |
| Quality gates | Ordinary engine/DSP PR changes have no general repository test trigger | Core CI matrix, deterministic synthetic fixtures, separate device/plugin qualification |

Specific boundary erosion is visible in session comments edited directly through
`session_mut` (`ui-egui/src/windows.rs:219`) and byte imports performed directly by the web
shell (`apps/soundcraft-web/src/web.rs:157–181`). They duplicate parts of command/undo behavior
and bypass the uniform journal/error boundary. Provide byte-based engine operations or a
transaction helper rather than reimplementing publication in each frontend.

`processing_order` caps its aux-depth exploration at eight (`mix/src/lib.rs:1164–1179`), so it
is not a general topological sort or cycle validator. The graph repair should define accepted
feedback semantics and reject unsupported cycles rather than relying on a depth heuristic.
No deep-chain or feedback runtime reproduction was run in this review.

The AI policy crate's separation from the UI and its bounded declarative evaluator are useful
foundations. Keep planning separate from command admission and execution; do not expose the
group spike as an authorization or atomicity guarantee.

Documentation has drifted: `docs/architecture.md` and G-06 describe RAM recording, while the
current recorder streams WAV files. The progress docs also predate the current 0.6.1 workspace.
Update readiness from demonstrated workflows, without equating menu coverage or a feature's
presence with reliability. This review does not revise parity percentages.

## Repair sequence and acceptance criteria

1. **Protect recordings and recovery:** A01/A15, session/media identity and live recorder faults.
   Recovery tests must distinguish same-named sessions and preserve partial takes on failures.
2. **Protect external control:** A02/A05/A14. Authentication/framing tests must pass; a dropped
   reply must never cause a second mutation; a refused transaction must leave no file/path change.
3. **Correct playback and export:** A03/A04/A10/A12/A13. Use deterministic impulse and event-log
   fixtures for session switching, serial auxes, master inserts, solo paths, reset and note ends.
4. **Bound execution and media:** A06/A07/A08/A11. Assert no steady callback allocation, explicit
   oversize errors, safe state capture and validated session input; then qualify hardware deadlines.
5. **Make coverage permanent:** fix A09, convert characterization cases to correctness tests,
   add PR gates, and update architecture/progress documentation to the verified implementation.

## Verification

The attached characterization tests assert the defective behavior to make it repeatable.
Their passing result means the defect was reproduced; they must be inverted/reworked when
landed as permanent regression tests. The existing HTTP protection test expects safe behavior
and fails, as expected for A02.

- Core suites (engine/model/mix/playback/automation): **259 passed, 4 ignored**.
- Foundation suites (time/audio-I/O/MIDI/DSP/video/assist-policy/pitch-eval): **297 passed, 2 ignored**.
- Existing audit characterization cases: **7 reproduced**; new cases: **3 reproduced**.
- HTTP framing protection: **failed**, forwarding `engine.execute` to the isolated recording adapter.
- Core Clippy (Rust 1.93) and full workspace Clippy (Rust 1.97), all targets with `-D warnings`: **passed**.
- Layer gate, asset attribution gate and formatting check: **passed**.
- `cargo +1.97.0 xtask wasm` (offline): **passed**, with one dead-code warning for the
  native-only `CaptureStates` command variant.

The first sandboxed core run could not discover the Apple Audio Unit used by one test. The
native-access rerun passed all core tests. Loopback reproductions also ran with native access.
These environment restrictions are not counted as code defects.

Workspace Clippy initially could not run with the default Rust 1.93 toolchain: the UI declares
Rust 1.95 and its dependencies require it. Full workspace Clippy and the WASM check then passed
with the installed Rust 1.97 toolchain. No dependency or toolchain upgrade is part of this review.
The full workspace test suite (including UI and host-specific suites) was not run; the 556-test
total covers the twelve named packages above. WASM compilation is not browser runtime validation.

To repeat the three new cases, copy `additional-repros.rs` into a previously nonexistent
`crates/engine/tests/review_additional_20261015.rs`, run
`cargo test --offline -p soundcraft-engine --test review_additional_20261015 -- --nocapture`,
then remove only that temporary copy. The original audit's README documents its seven cases.
The temporary test copies used during this review were removed.
