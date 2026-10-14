# Recorder and device findings

**Tasks:** P0-T04 (existing disk recorder) and the software half of P0-T05 (devices and callback sizes) in the [execution plan](execution-plan.md) · **Feeds:** G02, G09, G10; plan tasks P3-R01 to P3-R06
**Version:** 0.1 · **Updated:** 14 October 2026 · **Status:** The writer path is tested and measured. **Live capture, round-trip latency, alignment and drift are not tested** (they need a named interface and a loopback cable).
**Measured on:** the inspection host (MacBook Air, Apple M4, macOS 27.0.1, internal SSD), Rust 1.97.0. **Not the M1 reference host, and not the interface the early-access release will name.**
**Code:** branch `spike/phase0-tech`: `crates/playback/src/record.rs` (module `fault_tests`) and `crates/playback/examples/device_probe.rs`.

## 1. What was and was not exercised

**No input device was opened and the microphone was never used.** The PRD (§6.6) requires audio
input to start only on the user's explicit recording or monitoring action. These tests feed
synthetic audio into the recorder's own ring, which is the same path the input callback uses, so the
writer thread, take files, headers, error reporting and `finish()` are the real code. The device
probe lists input devices and their reported formats but builds no input stream.

| Part of the recording path | Exercised here |
|---|---|
| Input callback → ring (`Shared::capture`) | Yes, by calling it directly, as the callback does |
| Ring → writer thread → take file (`write_takes`, `TakeWriter`) | Yes, the real code |
| `Recorder::arm` and `finish` | Yes, with no device behind them |
| `Recorder::open` (device, stream, permission) | **No** |
| Capture from a real device, device loss, sample-rate mismatch | **No** |
| Round-trip latency, input/output alignment, drift | **No** |

## 2. Results

| # | Case | Result | How |
|---|---|---|---|
| 1 | A normal take through arm and finish | Every sample kept; the chosen input channel recorded correctly | Test, passes |
| 2 | Take file in a **read-only folder** | Arm is refused; the message names the file; no partial file; the recorder is **not** left armed; arming elsewhere afterwards works | Test, passes |
| 3 | A take never overwrites an existing file (`create_new`) | Confirmed | Existing test |
| 4 | **Ring overflow** (writer cannot keep up) | The loss is counted, and silence is inserted so the take keeps its timing. It is reported **only when the take ends**: "N ms of input lost: the disk was too slow" | Existing test plus new test, passes |
| 5 | **Write failure** (a handle that cannot write, as a vanished or full volume would give) | The take stops writing and keeps the first error; the ring **keeps draining**, so **nothing is counted as lost**; the error appears at the end; a take with no frames is deleted | Test, passes |
| 6 | **Disk full** on a 2 MiB RAM disk, 24.1 s of audio fed at real time | **10.2 s reached the disk; 13.9 s were silently discarded.** Lost-while-recording counter: **0**. Reported at the end as `No space left on device (os error 28)`. The partial take (10.2 s) is returned | Manual, measured |
| 7 | **`kill -9` mid-take** (a real SIGKILL on a child process), swept across the 1 s header-refresh cycle (10 runs) | All written audio is on disk. The WAV header trails the data by **0.06 to 0.75 s** of audio depending on where in the cycle the kill lands. A reader that trusts the header sees the take cut short by that amount | Manual sweep, measured |
| 8 | A crash leaves audio the header does not cover | Pinned in a test: header says 4 800 frames, file holds 7 200 | Test, passes |

Case 7 detail: the child's feeder runs at about 0.9× real time (sleep overshoot), so the figures are
audio-seconds. The header is refreshed about once per second, so the worst-case lag is bounded by
that interval. At the instant of a kill, the only audio **not** on disk is what is in the ring or in
the writer's current 10 ms poll; that was not measured separately.

## 3. What this means for the PRD

- **G09 "an application-process crash loses at most 1 second of captured take audio."** In terms of
  bytes on disk the loss is tiny (the ring and one 10 ms poll). In terms of what a normal WAV reader
  shows, the take is shortened by up to about one refresh interval, and **nothing repairs the header
  afterwards**. No code reads a take's real length from the file size, and there is no startup scan
  of take folders. So the target is **met for the data, not for what the user sees**, until a
  repair step exists.
- **Errors must not wait for the end of the take.** Overflow (case 4) and above all a failed
  write (cases 5 and 6) are visible only in the status line after the user presses stop
  (`crates/ui-egui/src/lib.rs:583-603`). A singer can perform 14 seconds into a full disk with no warning.
  This is the most important finding for a recording product.
- **Silent discard after a write failure is also a counting gap.** After the first error the writer
  drops everything but the ring still looks healthy, so the "input lost" report understates the loss.
- **Where takes go.** The UI records into the autosave folder (a per-session folder) or the system
  temporary folder when there is no autosave folder (`arm_recorder`, `lib.rs:477-486`), not into the
  project's `Audio Files`. A crash before saving leaves them there.
- **No input-latency compensation.** Recorded clips are placed at the record-start position
  (`lib.rs:587`); `docs/gaps.md` G-01 notes there is no record-offset compensation. G02's ±2 ms
  alignment requirement is therefore unmet until P3-R02, and cannot be tested without a loopback.

## 4. Repairs this suggests (refines P3-R03)

| # | Repair | Why |
|---|---|---|
| 1 | A live recording-health state the UI can read every frame: frames written, milliseconds lost, last error | A warning during the take, not after it |
| 2 | On the first write error: stop the take, say so immediately, keep what was written, and count the samples discarded afterwards | Cases 5 and 6 |
| 3 | A header repair: on startup, scan take folders and rewrite each header from the file length; also use it when opening a take | Case 7 and G09 |
| 4 | Decide whether the writer needs an `fsync` policy | G09 excludes power loss, so this is a choice to record, not a requirement |
| 5 | Turn the manual `kill_nine` and `disk_full` tests into CI regressions (the RAM-disk one needs a runner that can mount one, or a fault-injecting writer) | So repairs stay repaired |
| 6 | Place takes in project-managed storage (or link them with availability checks, PRD §6.6) | Source linking |

Estimate: **roughly 8 to 14 hours** for 1 to 3, 5 and 6 together (a guess, not a plan; repair 3 needs
a decision about where the startup scan runs).

## 5. Devices and callback sizes (software half of P0-T05)

`cargo run -p soundcraft-playback --example device_probe` lists every device and plays **silence** on
the default output at several requested buffer sizes, counting the callback sizes the device really
delivers. It never opens an input stream.

On this machine (CoreAudio; default output `MacBook Air Speakers`, default input `MacBook Air Microphone`):

| Requested buffer | Callbacks in 1.5 s | Frames per callback (min / median / max) |
|---|---|---|
| default | 141 | 512 / 512 / 512 |
| 32 | 2258 | 32 / 32 / 32 |
| 64 | 1129 | 64 / 64 / 64 |
| 128 | 564 | 128 / 128 / 128 |
| 256 | 282 | 256 / 256 / 256 |
| 512 | 140 | 512 / 512 / 512 |
| 1024 | 70 | 1024 / 1024 / 1024 |

- The device reports buffer sizes from **15 to 4096 frames** and honoured every request exactly, with
  no variation in callback size.
- The app asks for the **default**, which here is **512**, equal to its fixed maximum block
  (`BLOCK = 512`, `crates/playback/src/lib.rs:36`). The app has no way to request another size
  (`docs/gaps.md` G-01), so on this device it always runs at 512 and the quantum-clamping path for
  smaller callbacks is not exercised. On a device whose default is smaller, that path runs.
- Supported rates on the built-in output: 44.1, 48, 88.2 and 96 kHz; the built-in input is mono at
  the same rates. A virtual device (`Microsoft Teams Audio`) is also present.
- This confirms the PRD's note (§12.1) that the 512 maximum alone does not give a latency: the
  callback size is the device's, and the round trip also includes the interface's own buffers and
  converters, none of which the probe sees.

## 6. What the hardware session needs (not done)

For the named interface (decision D3), with hardware direct monitoring set up per the interface's manual:

1. Set the interface as the macOS default input **and** output (the app opens the defaults only).
2. Run `device_probe` to record the interface's buffer range and the callback sizes.
3. Measure round-trip latency and alignment with a loopback cable (output 1 to input 1): a click
   train played and recorded, comparing the recorded click's position with the played one, at each
   advertised buffer size and at 44.1 and 48 kHz. **This opens the input**, so it needs your explicit
   go-ahead on the day; a small `loopback_probe` example (play and record in one program) can be
   written once the interface is chosen. A third-party tool can do the same today.
4. Record three 30-minute runs per rate (G02), counting dropouts, once the recording repairs above
   have landed. Endurance results before the repairs would not be trustworthy because dropouts
   after a write failure go unreported.
5. For software monitoring only (G02-SW): the same loopback with the processor chain in the path,
   against the 15 ms target.

## Revision history

| Version | Date | Change |
|---|---|---|
| 0.1 | 14 October 2026 | Writer-path tests, `kill -9` sweep, disk-full run and device probe (P0-T04, P0-T05 software half). |
