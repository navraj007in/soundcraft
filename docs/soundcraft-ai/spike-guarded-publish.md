# Spike: guarded publication and revision semantics

**Task:** P0-T06 in the [execution plan](execution-plan.md) · **Feeds:** PRD §7.2, G04 · **Code:** branch `spike/guarded-publish`, `crates/engine/src/group.rs` (+ 7 lines in `lib.rs`)
**Version:** 0.1 · **Updated:** 14 October 2026 · **Status:** Spike complete. Prototype only; not the P1-E02 implementation.

## Questions and answers

| # | Question (plan P0-T06) | Answer | Evidence |
|---|---|---|---|
| 1 | Which actions move `Engine.revision`? | Every document change, **and also selection and playhead moves**; not zoom or scroll. | Measured (test) |
| 2 | Can the existing copy-on-write history publish several operations as one undoable step, all or nothing? | **Yes**, with about 100 lines (including comments and the content revision) and no change to the history design. | Measured (tests, mutation-checked) |
| 3 | What state sits outside `Session` and could be missed by a snapshot swap? | Third-party plugin state (lives on the audio thread), queued transport requests, the clipboard and the message log. Only the first two matter for the admitted operations. | Source-inspected; transport rollback measured |
| 4 | How does the renderer pick up a published session? | The UI loop hands the player a new `Arc<Session>` whenever `engine.revision` differs from the last one it sent. | Source-inspected |
| 5 | Is publication serialised with manual edits? | Yes: control requests are drained on the UI thread, the same thread that applies manual edits. | Source-inspected |

## 1. The revision counter is too sensitive to use as the staleness check

Measured with the demo session (`finding_which_view_and_selection_commands_move_the_revision`):

| Command | Moves `revision` | Changes content |
|---|---|---|
| `edit.select` | **yes** | no |
| `edit.select_none` | **yes** | no |
| `transport.locate` | **yes** | no |
| `view.zoom_in`, `view.scroll` | no | no |
| `mix.mute` and other edits | yes | yes |

Source-inspected additions that also move it, with no action by the user in the session:

- **Saving or bouncing.** `capture_plugin_states` calls `mix.insert_state` for every live
  third-party plugin (`crates/ui-egui/src/lib.rs:342-345`, `:490`). That command changes the
  document and moves the revision, but is **not undoable** and not journaled
  (`crates/engine/src/cmd/plugin_state.rs:7-23`).
- **Plugin editor tweaks.** `apply_editor_edits` feeds parameter changes made inside a plugin's own
  window into the session through `execute_merged` every frame (`lib.rs:502-510`).
- **Undo, redo, open, and any rollback** (`crates/engine/src/lib.rs`).

**Consequence:** with "any revision change makes a plan stale" (the plan's earlier default for D5),
moving the playhead or clicking a clip would invalidate a proposal under review. That would make
the workflow unusable, not just conservative.

**Prototype fix, `Engine::content_revision()`:** a counter that moves only when the document
differs from the one seen at the previous call, ignoring selection, playhead and zoom (it reuses
the engine's existing `same_document` comparison, the one behind `is_dirty`). It is computed on
demand, so it costs nothing between calls and catches changes made by any code path, including
direct `session_mut` users. Measured behaviour (tests):

- Selecting, locating and zooming do **not** move it; an edit does.
- A plan made at content revision N stays valid across a selection change.
- **No ABA:** edit, observe, undo back to identical content, and the counter has still moved, so a
  plan made before the edit is refused. This is the "reopened session with a reused revision" family
  in the plan's G03 cases, solved at the counter level.
- A failed group does not move it.
- Storing plugin state **does** move it, so a plan reviewed before a save or bounce goes stale.
  That is correct (the document did change), but the user-facing rule needs to say so; see "Open
  items".

**Cost probe:** `same_document` takes about 12 µs on the demo session (9 tracks, 5 sources), release
build on the M4. It clones the session, so cost grows with clip count. **Re-measure on an 8-track,
10-minute session** before relying on it per request. It isn't on any per-command path: only when
the companion asks.

This resolves plan decision **D5**: use the content revision, not the raw revision.

## 2. Guarded group publication works on the existing history

`Engine::execute_group(expected_content_revision, label, ops)`:

1. Refuses a stale content revision, a recording transport and an empty list.
2. Saves the document, undo stack, redo stack, journal length, queued transport request count,
   dirty flag and revision.
3. Runs each command through the normal `execute` (so every existing command validation, panic
   catch and `enabled` check applies).
4. On any failure restores all of the above exactly (the same `Arc<Session>` comes back).
5. On success replaces the per-command undo entries with **one** entry and returns the revisions.

Behaviours verified by tests (`cargo test -p soundcraft-engine --lib group::`: 15 pass, 1 timing
probe ignored):

- Three commands produce one undo entry; one undo restores all three.
- A stale revision changes nothing: same document, same undo stack, same revision.
- A failure in the last of three operations leaves **no trace**: document pointer, revision, undo
  stack, **redo entry**, journal and dirty flag are all as before.
- With the undo stack at its cap, a failing group still doesn't eat old history (the per-command
  pushes would otherwise have drained it).
- A failed group does not leave a `transport.play` request queued.
- Refused while `transport.recording` is set.

Mutation check: each of the restore steps (undo, redo, revision, transport requests, and the
success-path undo replacement) was removed in turn; each removal made at least one test fail. One
test (the undo-cap one) was found to be too weak because every undo label is "Volume"; it now
compares the states the stack leads back to, and fails when the failure-path undo restore is
removed.

Not part of the spike: instance ID (P1-E01), receipts and status query (P1-C07), lineage marker
(P1-C08), the operation allowlist, the scope diff (P1-E06), the IPC (P1-E03), and any
postcondition verification beyond "the command returned Ok".

## 3. State outside `Session`

| State | Where it lives | Effect on guarded publish | Handling |
|---|---|---|---|
| Third-party plugin live state | Audio thread; copied into the session only on save or bounce | A snapshot or a staged copy may not contain it. A snapshot swap that adds or removes a third-party insert makes the player build fresh instances (`Player::update_session` → `prepare`, `crates/playback/src/lib.rs:523`). | Keep third-party processors out of the admitted manifest (already PRD §6.2); for audition, an unqualified processor on the render path blocks audition (P1-U05(c)). |
| `transport_requests` | `Engine` field, drained by the host each frame | A group that queued one and then failed would leave it behind. | **Fixed in the spike** (truncate on rollback). Transport commands are not in the admitted set anyway. |
| `clipboard`, `messages` | `Engine` fields | Not restored on rollback. | Edit and clipboard commands are outside the manifest. Document as a manifest rule. |
| Source pool and files | Session sources + disk | Import writes files; undoing the document doesn't remove them. | Import and delete stay outside the manifest (PRD §6.2). |
| `Engine.transport.recording` | Reported back by the host each frame | The recording check is only as fresh as the last frame. | Acceptable for the spike; for P1-E02 also check `record_arm`/input monitor state on the session itself. |
| Built-in plugin DSP state | Mixer, rebuilt from the session | Not examined here. | The determinism and initial-state tests in P1-U03 cover it. |

## 4. Scope isolation findings (confirm P1-E06)

Pinned as tests, all passing, so a later change in behaviour is noticed:

- `mix.volume` on one grouped track also moves its siblings (`finding_volume_on_a_grouped_track...`).
- `mix.solo` with `exclusive: true` clears an unrelated solo.
- `mix.mute` with no `track` acts on the current selection.

These confirm that explicit IDs alone do not isolate an edit; P1-E06's strict validation and scope
diff are needed. The spike has neither.

## 5. Recommendation for the imported-session editing decision

**Go, with repairs.** The foundation fits: the existing copy-on-write history gives all-or-nothing
publication with little new code. Required changes, all already in the plan except item 1:

1. **Content revision** (new in this spike; P1-E01 should expose `content_revision` and an instance
   ID, not the raw counter).
2. Instance ID and receipts (P1-E01, P1-C07 to C09).
3. Scope diff and strict parameters (P1-E06).
4. Authenticated control endpoint (P1-E03; P0-T07).

Phase 1 effort for items 1 to 3 in the engine is **roughly 20–30 h (a guess, to be refined when
P1-E01/E02/E06 are broken down)**. This answers only the P0-T06 input; the Phase 0 decision also
depends on P0-T03 (which found a repair: AAC trimming), P0-T07 and the product experiment.

## 6. Open items

- **User-facing rule for plugin state capture.** A save or bounce moves the content revision (a
  real document change), so a plan under review goes stale. Decide whether to exclude
  `mix.insert_state` changes from the content revision, or accept and explain. Excluding them is
  safe only if no admitted operation touches the same inserts.
- **Large-session cost** of `same_document` (see §1).
- **Selection-sensitive commands:** with the content revision, selection no longer stales a plan,
  but commands that read the selection still need P1-E06's "no selection fallback" rule.
- **Wrapping:** `revision` uses `wrapping_add`; the content counter does too. Irrelevant in
  practice (u64) but the plan's tests should not assume monotonic comparisons across restarts;
  that is what the instance ID is for.
- **Merge key:** `execute` clears the gesture merge key, so a group during an in-progress fader
  gesture would end that gesture's coalescing. Check how this interacts with a user dragging a fader
  while a plan is published (rare, but the UI thread runs both).

## Revision history

| Version | Date | Change |
|---|---|---|
| 0.1 | 14 October 2026 | Spike findings (P0-T06). |
