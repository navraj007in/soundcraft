# Production policy: rule format and the headroom rule

**Task:** P0-T09 in the [execution plan](execution-plan.md) · **Feeds:** PRD FR-04, G05, G08 · **Implements later in:** P1-M01, P1-M02
**Version:** 0.1 · **Updated:** 14 October 2026 · **Status:** Rule format and one rule written and tested. Thresholds are proposed values, not validated by listening.
**Code:** branch `spike/policy`, crate `crates/assist-policy` (`soundcraft-assist-policy`, no workspace dependencies, registered as a standalone layer-0 crate in `xtask`).

## 1. What a rule is

PRD §6.4: a local, versioned policy maps an approved goal and measured features to exact settings;
the language model classifies the request and explains the evidence but never supplies a value.
A rule here has two parts:

- **A declaration** (`policy/v0/<rule>.json`): required measurements with units, ranges and signal
  locations; controls with defaults and bounds; thresholds; the operation it may produce; every
  abstention code with the condition and the message; and the order in which conditions are checked.
- **Evaluation code** in the crate, which reads the declaration and is the only thing that turns
  measurements into values.

`Policy::evaluate(request) -> result` is a pure function: the same request and rule version always
give the same result, and no input can make it panic.

**Request:** `{"goal", "rule_version"?, "controls"?, "inputs": {…}}`. Unknown keys anywhere are
rejected. A request naming another rule version is refused, so a stored plan is never silently
reproduced by a different rule.

**Result:** `outcome` is `propose`, `no_change` or `abstain`.

| Field | `propose` | `no_change` | `abstain` |
|---|---|---|---|
| `contract`, `rule {id, version, goal}`, `outcome` | ✓ | ✓ | ✓ |
| `controls`, `measurements` (as rounded, with units and signal location) | ✓ | ✓ | ✓ once the inputs were valid |
| `operations [{command, target, parameter, unit, before, after}]` | ✓ | – | – |
| `branch` (`reduce` or `boost`), `limits`, `notes`, `expected_effect` | ✓ | – | – |
| `reason {code, message, field?}` | – | ✓ | ✓ |

Message wording is not part of the contract; codes are. A proposal is a recommendation: nothing in
it says the result sounds better or that a measurement was correct (PRD §6.4).

## 2. The headroom rule, `level.headroom` 0.1.0

**Goal.** For a confirmed lead-vocal track, move the fader so the vocal's true peak at the channel
output lands on a target. Fader only; no processors.

**Inputs (all required).** `track_id`; `true_peak_dbtp` (channel output after the fader, −150 to
+24); `current_volume_db` (−144 to +12); `clipped_samples`; `active_ms` (0 to 300 000, the PRD's
5-minute job limit); and three flags: `role_confirmed`, `volume_automation_active`,
`analysis_current`.

**Control.** `target_true_peak_dbtp`: default −6.0, range −12.0 to −1.0, step 0.5.

**Thresholds.** Minimum active signal 1 000 ms; silence floor −60 dBTP; no-change tolerance
0.5 dB; maximum change 12.0 dB; fader range −60.0 to +6.0 dB.

**Arithmetic.** Measurements are rounded to 0.1 dB (halves away from zero) and everything after is
integer tenths of a dB, so the result is identical on every platform. The values in the result are
the rounded ones.

**Steps, in this order** (the first that applies decides; the order is declared in `order`):

1. Request shape; goal is `headroom` (`unsupported_goal`); rule version matches
   (`rule_version_mismatch`); every input and control present, typed, finite, in range and on its
   step, with no unknown keys (`invalid_input`, naming the field).
2. `stale_analysis`, `role_not_confirmed`, `automation_conflict`.
3. `insufficient_signal` (active signal under 1 s), `signal_too_quiet` (true peak under −60 dBTP).
4. `wanted = target − true_peak`. If `|wanted| < 0.5 dB`: **`no_change`**. (Exactly 0.5 dB proposes.)
5. `clipped_source_no_boost`: the change would raise the level and the source clips.
6. Limit the change to ±12.0 dB (`max_change`), then keep the new fader value inside −60 to +6 dB
   (`fader_bounds`). The bounds may only **shrink** a change: if they would reverse it, make it
   smaller than 0.5 dB, or make it larger than the capped change, the result is
   `limited_to_nothing`.
7. **`propose`** `mix.volume {track, db: new}` with `expected_effect` (true peak before, after, and
   the change). A clipped source that is being turned down still gets a proposal with the note
   `source_clipped`: lowering the level gives headroom but does not repair clipping.

**Why these choices** (each a proposal to be confirmed, not a result):

- *Fader as the only lever:* it is the one qualified, reversible, undoable operation that matches
  "headroom or level adjustment" without needing EQ or compressor qualification first.
- *True peak at −6 dBTP:* a common mixing convention for leaving room, offered as an editable
  default. G00 and listening tests decide whether it suits independent vocalists.
- *Abstain rather than clamp on missing or stale evidence;* clamp and report on limits, because a
  limited change is still a correct step toward the target and `limits` says it fell short.
- *Strict inputs:* an unknown key or a number of the wrong type means the controller and the rule
  disagree about the contract, which is a bug to surface, not to paper over.

## 3. Tests and what they showed

The crate has 9 tests (7 on the fixtures and declaration, 2 property tests) plus the fixture file.

**Fixtures (44 cases):** 14 propose, 1 no-change, 29 abstain. Every declared outcome code has at
least one case (a test fails if a code is added without one). The cases cover both rounding
directions and the tolerance boundary on each side, both limits alone and together (in order), each
precedence pair that could be confused, a clipped source being reduced and refused a boost, and
13 malformed-input cases (missing, out of range, wrong type, null, fractional integer, flag given a
number, unknown input, unknown top-level key, control off range, control off step, unknown control,
non-object request, missing goal).

**How the expectations were made.** Written from the rule description by a second, independent
implementation in Python, with 12 values also worked out by hand beforehand and asserted against it.
The Rust code then matched the first 42 on its first run; the last two were added after the defect below. The fixtures are the specification: if a
rule's behaviour changes, change the fixtures **and** `rule_version`. They must not be regenerated
from the Rust code.

**Property tests** (2 000 cases each by default; 50 000 each in a one-off run, all passing): random
JSON never panics and never proposes; for plausible random requests every proposal stays within
−60 to +6 dB and 0.5 to 12 dB of change, never boosts a clipped source, requires the
confirmed-role, current-analysis and no-automation flags, targets the requested track, and the
effect arithmetic adds up.

**A defect these found before release.** My first version applied the fader bounds *after* the
12 dB cap and let them move the value either way. A fader at −125.8 dB (valid input) with +6 dB
wanted was clamped up to −60 dB, a **+65.8 dB** change that ignored the maximum. The property test
produced that case and the rule was corrected to the "bounds may only shrink a change" form above;
two fixtures pin it (`fader_far_below_policy_range_is_not_dragged_into_it`,
`fader_below_the_floor_cannot_be_cut_further`).

**Mutation check.** Nine deliberate breaks of the Rust rule each made a fixture fail: tolerance
`<` to `<=`, rounding to truncation, no clipped-source check, no fader bounds, no maximum change,
the bounds allowed to grow a change (the defect above), no stale check, no automation check, no
silence floor.

**Declaration checks at load:** eight kinds of broken declaration are rejected (inconsistent
thresholds, inverted fader range, a default off its step or outside its range, an unknown schema
version, a threshold that is not a whole number of tenths, an order listing an undeclared code, a
duplicate input).

## 4. Limits and open items

- **Grouped tracks.** The rule doesn't know a track belongs to an active mix group. The fader move
  would reach the siblings and P1-E06's scope diff would then reject the plan, after the user has
  already auditioned it. Add an input (for example `mix_group_active`) and an abstention code in
  P1-M01 so this is explained up front. Not added here to keep this task to one change at a time.
- **One control and one operation.** The loader enforces exactly one of each. EQ and compressor
  rules (P2-M01, P2-M02) need several operations in a stated order with per-processor bounds and
  states; plan a schema version 2 then.
- **True-peak measurement must exist.** P1-A02 has to supply true peak at the channel output after
  the fader, with a declared method version. It is stale after any fader change, which is correct.
- **Thresholds are untested by ears.** 1 s of active signal, a −60 dBTP floor, ±12 dB, −60 to +6 dB
  and the −6 dBTP default are reasonable starting values, nothing more. G00 and the G08 listening
  checks should change them if needed, with a rule-version bump.
- **Input identity.** PRD §6.4 also wants the *input fingerprint* in the determinism contract. The
  rule echoes the rounded measurements; the controller adds the content fingerprint and processor
  manifest digest to the plan (P1-C01), not this crate.
- **Messages are English text in the declaration.** Localisation is outside the early-access
  envelope (English only, PRD §4.2).
- **The layer registration** of the crate in `xtask` (layer 0, standalone) is a choice: it has no
  workspace dependencies and the companion will depend on it. Revisit if the policy ever needs the
  model crate.

## Revision history

| Version | Date | Change |
|---|---|---|
| 0.1 | 14 October 2026 | Rule format and `level.headroom` 0.1.0 with 44 fixtures (P0-T09). |
