# Resolution of the 15 October 2026 audit

The audit's characterization tests (`engine-repros.rs`, `additional-repros.rs`, `control-repro.rs`,
`remote-repro.rs`) assert the *defective* behaviour of the code as audited and are kept here only as
a record: they are not part of any crate and will not pass against the current code. Each finding
now has a regression test beside its fix.

| ID | Fixed in | Regression test |
|---|---|---|
| A01 | `79a3ccb` | `recovery_of_same_named_sessions_keeps_each_sessions_audio` (engine `io.rs`) |
| A15 | `788e523` | `a_failed_write_is_published_while_recording` (playback `record.rs`) |
| A05 | `decf9d5` | `a_mutation_whose_reply_is_lost_is_never_sent_twice` (automation) |
| A04 | `4dce022` | `master_insert_latency_is_removed_from_a_bounce`, `serial_aux_latency_accumulates_and_is_removed_from_a_bounce` (mix) |
| A12 | `4dce022` | `reset_clears_audio_held_in_compensation_delays`, `a_seek_clears_audio_held_in_compensation_delays` (mix) |
| A13 | `043ca9d` | `a_note_ending_on_a_block_edge_still_gets_its_note_off` and two more (mix) |
| A10 | `1229087` | `soloing_a_track_keeps_every_aux_in_its_chain_audible` (mix) |
| A03 | `1c4ea23` | `a_replaced_session_does_not_keep_the_previous_sessions_plugin_state` (playback) |
| A07 | `8ffdd15` | `an_oversize_render_is_an_error_not_a_shortened_success` (mix) |
| A11 | `1059bbe` | `hostile_timebases_are_refused_at_load` (model), `hostile_tests` (time) |
| A09 | `cf9171d` | `a_gesture_at_the_history_cap_still_undoes_to_its_starting_value` (engine) |
| A02 | `222f23e` | `protection_tests` (apps/soundcraft `control_server.rs`, all eight passing) |
| A14 | `e52e8ec` | `external_effects_and_session_replacement_are_not_admitted` (engine `group.rs`) |
| A06 | `8f02fe5`, then `4574c5b` (off-callback capture) | `a_slow_serializer_never_blocks_the_audio_thread`, `clap_capture` (playback) |
| A08 | `043ca9d`, `ca4a397`, `f69578f`; measured in `04d60fa`; dedicated pool in `soundcraft-rt-pool` | `realtime_bench` example. Shared rayon pool: 5× faster idle but a 3–4× worse tail under CPU contention. Dedicated pool: idle 128 tracks worst 1.5 ms (serial 12 ms); under 12 busy loops worst 14 ms (rayon 71 ms, serial 36 ms). `rt-pool` tests include a zero-allocation check and a Miri run |

Open: real-device dropout measurement, companion
token handling for the control channel (P1-E03), the web shell's direct session edits and the
`processing_order` depth heuristic.
