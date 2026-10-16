# Resolution of the 15 October 2026 audit

The audit's characterization tests (`engine-repros.rs`, `additional-repros.rs`, `control-repro.rs`,
`remote-repro.rs`) assert the *defective* behaviour of the code as audited and are kept here only as
a record: they are not part of any crate and will not pass against the current code. Each finding
now has a regression test beside its fix.

| ID | Fixed in | Regression test |
|---|---|---|
| A01 | `79a3ccb` | `recovery_of_same_named_sessions_keeps_each_sessions_audio` (engine `io.rs`) |
| A15 | upstream 0.7.0 (`Recorder::trouble()`, `Trouble::Disk`) | upstream's recorder tests; supersedes the fix in `788e523`, which the merge dropped |
| A05 | `decf9d5` | `a_mutation_whose_reply_is_lost_is_never_sent_twice` (automation) |
| A04 | `4dce022` | `master_insert_latency_is_removed_from_a_bounce`, `serial_aux_latency_accumulates_and_is_removed_from_a_bounce` (mix) |
| A12 | `4dce022` | `reset_clears_audio_held_in_compensation_delays`, `a_seek_clears_audio_held_in_compensation_delays` (mix) |
| A13 | `043ca9d` | `a_note_ending_on_a_block_edge_still_gets_its_note_off` and two more (mix) |
| A10 | `1229087` | `soloing_a_track_keeps_every_aux_in_its_chain_audible` (mix) |
| A03 | `1c4ea23` | `a_replaced_session_does_not_keep_the_previous_sessions_plugin_state` (playback) |
| A07 | `8ffdd15`; bounces stream since 0.7.0 | `an_oversize_render_is_an_error_not_a_shortened_success` (mix) for the in-memory renders; `RangeRender` streams any length |
| A11 | `1059bbe`, reworked in the 0.7.0 merge | `TempoMap::sanitize` and `Session::sanitize` repair a hostile timebase on load (upstream's behaviour); `MAX_TICK` bounds every tick conversion; `hostile_tempo_ticks_in_a_file_are_repaired_not_trusted` (model), `hostile_tests` (time) |
| A09 | `cf9171d` | `a_gesture_at_the_history_cap_still_undoes_to_its_starting_value` (engine) |
| A02 | `222f23e` | `protection_tests` (apps/soundcraft `control_server.rs`, all eight passing) |
| A14 | `e52e8ec` | `external_effects_and_session_replacement_are_not_admitted` (engine `group.rs`) |
| A06 | upstream 0.7.0 (per-instance `PluginStateReader` handles read on the UI thread) | `clap_capture` (playback), upstream's tests; supersedes `8f02fe5`/`4574c5b`, which the merge dropped |
| A08 | `043ca9d`, `ca4a397`, `f69578f`; measured in `04d60fa`; dedicated pool in `soundcraft-rt-pool` | `realtime_bench` example. Shared rayon pool: 5× faster idle but a 3–4× worse tail under CPU contention. Dedicated pool: idle 128 tracks worst 1.5 ms (serial 12 ms); under 12 busy loops worst 14 ms (rayon 71 ms, serial 36 ms). `rt-pool` tests include a zero-allocation check and a Miri run |

Merged with upstream 0.7.0 on 15 October 2026: where upstream fixed the same defect (A06, A15, parts of A02, A04 and A11) its design was kept.

Open: real-device dropout measurement, companion
token handling for the control channel (P1-E03), the web shell's direct session edits and the
`processing_order` depth heuristic.

## Second audit (16 October 2026)

A second review of the merged tree listed 25 findings (F-01 to F-25). Checked against the code on
16 October: F-05, F-02, F-04, F-06 and F-03 were confirmed and fixed; F-11 and F-12 were wrong
(`same_document` compares the edit flags and values, and `execute` already skips the selection check
when a call names its targets); F-01's provenance claim was overstated; F-07 is a design question;
F-08 is fixed upstream in PR #573 (open); the rest are open.

| ID | Fix | Regression test |
|---|---|---|
| F-05 | `MixEngine::adopt` returns early when there is nothing to adopt, so a fader edit no longer forces a full re-sync on the audio thread | `a_session_update_with_no_new_plugins_does_not_force_a_resync` (mix) |
| F-02 | `length_prefixed_nals` checks lengths by subtraction, so a 32-bit wrap cannot slip through | `a_huge_nal_length_is_an_error_whatever_the_pointer_width` (video) |
| F-06 | Soloing a VCA solos the tracks it controls | `soloing_a_vca_solos_the_tracks_it_controls_and_silences_the_rest` (mix) |
| F-04 | VST3 `set_param` reads a plain-to-normalized curve sampled at creation instead of calling the controller from the audio thread | `curve_tests` (vst3-host). CLAP and AU hosts not yet reviewed for the same race |
| F-03 | `Job::run` ends the batch in a drop guard, so an unwinding `wake` cannot leave a worker using freed stack | `a_panic_while_waking_workers_leaves_nothing_using_the_callers_data` (rt-pool), also run under Miri |
| F-25 | `AGENTS.md` layer table lists every crate | n/a |

Second batch (also 16 October):

| ID | Fix | Regression test |
|---|---|---|
| F-22 (midi) | `Sequence::from_performance` uses saturating arithmetic at the edges of the tick range; the take's clip length too | `ticks_at_the_edge_of_the_range_never_overflow` (midi) |
| F-17 | A freehand automation stroke is one undo step | `a_freehand_automation_stroke_is_one_undo_step` (engine) |
| F-20 | The compressor zeroes a non-finite sample before it enters the lookahead delay | `a_nan_or_infinity_never_comes_out_of_the_lookahead_delay` (dsp) |
| F-10 | `zoom_mut` no longer marks the session dirty, so `is_dirty` stops comparing the whole document after a zoom | existing `selecting_locating_and_zooming_leave_the_session_clean` |
| F-16 | MCP and the control client read lines with a 96 MiB bound; an empty JSON-RPC batch is an invalid-request error | `an_empty_batch_is_one_invalid_request_error`, `a_line_over_the_limit_is_refused_and_ends_the_session_without_buffering_it` (automation) |
| F-19 (part) | Replaced session snapshots the UI thread has not taken are held (room reserved up front) instead of dropped on the audio thread | `a_burst_of_replaced_sessions_is_held_not_freed_on_the_audio_thread` (playback) |

Checked and left as they are: F-15 (hosted-instrument parameters live in the plug-in's own state, which
is saved with the session; only the dirty flag is not set by GUI edits), F-04 for CLAP (its main and
audio thread calls follow the CLAP threading contract; only the non-macOS `on_main_thread` guard is
a stub). Open: F-08 (upstream PR #573), F-09 (the session is cloned per edit; a per-track `Arc` is a
larger change), F-01 (needs an Audio Unit to confirm), F-07 (design), F-18, F-21 (upstream #448 and
PR #586), F-23, F-24 and the rest of F-19.
