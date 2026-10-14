//! SPIKE (plan task P0-T06, ahead of P1-E02): guarded group publication.
//!
//! `execute_group` runs several commands as one unit against the engine's copy-on-write document:
//! it refuses a stale revision, applies every command, and either publishes them with a single
//! undo entry or restores the document, undo stack, redo stack, journal and revision exactly as
//! they were. It is a measuring instrument, not the final API: it has no instance ID, no receipt
//! table, no scope diff and no operation allowlist (P1-E01, P1-C07, P1-E06). The findings are in
//! `docs/soundcraft-ai/spike-guarded-publish.md`.

use crate::{Engine, EngineError, Result, same_document};
use serde_json::{Value, json};
use soundcraft_model::Session;
use std::sync::Arc;

/// What `Engine::content_revision` compared last: the revision and document it saw, and the
/// counter it has handed out.
pub(crate) struct ContentSeen {
    revision: u64,
    doc: Arc<Session>,
    counter: u64,
}

impl ContentSeen {
    pub(crate) fn new(doc: Arc<Session>) -> Self {
        ContentSeen { revision: 0, doc, counter: 1 }
    }
}

impl Engine {
    /// A counter that moves only when the *content* of the session changes: not for selection,
    /// playhead or zoom. It is computed on demand by comparing the current document with the one
    /// seen at the previous call, so it catches changes made by any code path (commands, undo,
    /// direct `session_mut` callers) and costs nothing between calls. It never repeats a value:
    /// editing and then undoing back to the same content between two calls still moves it if a
    /// call saw the edit, so a plan made before cannot be mistaken for current (no ABA).
    pub fn content_revision(&self) -> u64 {
        let mut seen = self.content_seen.borrow_mut();
        if seen.revision != self.revision {
            if !Arc::ptr_eq(&seen.doc, &self.doc) && !same_document(&seen.doc, &self.doc) {
                seen.counter = seen.counter.wrapping_add(1);
            }
            seen.doc = Arc::clone(&self.doc);
            seen.revision = self.revision;
        }
        seen.counter
    }

    /// Run `ops` (command id and params) as one undoable unit, if the content revision still
    /// equals `expected_content_revision`. All or nothing: on any failure the engine is left exactly as it was.
    pub fn execute_group(&mut self, expected_content_revision: u64, label: &str, ops: &[(String, Value)]) -> Result<Value> {
        let content = self.content_revision();
        if content != expected_content_revision {
            return Err(EngineError::Stale(format!("expected content revision {expected_content_revision}, the session is at {content}")));
        }
        if self.transport.recording {
            return Err(EngineError::Disabled("session.apply".into(), "not while recording".into()));
        }
        if ops.is_empty() {
            return Err(EngineError::BadParams("session.apply".into(), "no operations".into()));
        }

        // Only reversible document edits may be grouped: a command with an effect outside the
        // document (a file written, the session replaced or saved elsewhere, quitting) cannot be
        // taken back, so none is admitted, and nothing runs before every operation is checked.
        for (i, (id, _)) in ops.iter().enumerate() {
            let bad = |why: String| EngineError::BadParams("session.apply".into(), format!("operation {} (`{id}`): {why}, nothing was applied", i + 1));
            let Some(spec) = crate::find_command(id) else { return Err(bad("unknown command".into())) };
            if !spec.undoable || spec.view {
                return Err(bad("only undoable document edits can be grouped".into()));
            }
        }

        // Everything a command can change that must come back on failure. Clones are cheap: the
        // document and every history entry share their data behind an `Arc`.
        let before = Arc::clone(&self.doc);
        let (undo, redo) = (self.undo.clone(), self.redo.clone());
        let (revision, dirty, journal_len, transport_len) = (self.revision, self.dirty, self.journal.len(), self.transport_requests.len());
        let (path, saved, clipboard, messages_len) = (self.path.clone(), Arc::clone(&self.saved), self.clipboard.clone(), self.messages.len());

        let mut results = Vec::with_capacity(ops.len());
        for (i, (id, params)) in ops.iter().enumerate() {
            match self.execute(id, params) {
                Ok(v) => results.push(v),
                Err(e) => {
                    self.doc = before;
                    self.undo = undo;
                    self.redo = redo;
                    self.journal.truncate(journal_len);
                    self.transport_requests.truncate(transport_len);
                    self.dirty = dirty;
                    self.revision = revision;
                    self.merge_key = None;
                    self.path = path;
                    self.saved = saved;
                    self.clipboard = clipboard;
                    self.messages.truncate(messages_len);
                    return Err(match e {
                        EngineError::Stale(m) => EngineError::Stale(m),
                        other => EngineError::BadParams(
                            "session.apply".into(),
                            format!("operation {} (`{id}`) failed, nothing was applied: {other}", i + 1),
                        ),
                    });
                }
            }
        }

        // Replace the per-command undo entries with one entry for the whole group.
        self.undo = undo;
        self.push_undo(label, before);
        Ok(
            json!({"revision_before": revision, "revision_after": self.revision, "content_revision_before": content, "content_revision_after": self.content_revision(), "results": results}),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soundcraft_model::{ChannelFormat, TrackKind};

    #[test]
    fn external_effects_and_session_replacement_are_not_admitted() {
        let dir = std::env::temp_dir().join(format!("soundcraft-group-refuse-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("changed.scraft");
        let mut e = Engine::default();
        let revision = e.content_revision();
        for first in [
            ("session.save_as", json!({"path": path})),
            ("session.new", json!({})),
            ("file.bounce_mix", json!({"path": dir.join("x.wav")})),
            ("app.quit", json!({})),
            ("session.inspect", json!({})),
        ] {
            let r = e.execute_group(revision, "test", &[(first.0.to_string(), first.1.clone()), ("does.not.exist".into(), json!({}))]);
            assert!(r.is_err(), "{}", first.0);
            assert!(e.path.is_none(), "{}: the save path must not change", first.0);
        }
        assert!(!dir.exists(), "no file may be written by a refused group");
        assert_eq!(e.content_revision(), revision);
    }

    fn two_tracks() -> (Engine, soundcraft_model::TrackId, soundcraft_model::TrackId) {
        let mut e = Engine::default();
        let a = e.session_mut().add_track(TrackKind::Audio, ChannelFormat::Mono, None);
        let b = e.session_mut().add_track(TrackKind::Audio, ChannelFormat::Mono, None);
        (e, a, b)
    }

    fn vol(e: &Engine, t: soundcraft_model::TrackId) -> f32 {
        e.session().track(t).map_or(f32::NAN, |tr| tr.mixer.volume_db)
    }

    #[test]
    fn a_group_is_one_undo_step_and_undo_restores_everything() {
        let (mut e, a, b) = two_tracks();
        let undo_before = e.undo_history().len();
        let rev = e.revision;
        let ops = vec![
            ("mix.volume".to_string(), json!({"track": a.0, "db": -6.0})),
            ("mix.volume".to_string(), json!({"track": b.0, "db": -3.0})),
            ("track.rename".to_string(), json!({"track": a.0, "name": "Lead"})),
        ];
        let r = e.execute_group(e.content_revision(), "Vocal level", &ops).unwrap();
        assert_eq!(e.undo_history().len(), undo_before + 1, "one undo entry for three commands");
        assert_eq!(e.undo_label(), Some("Vocal level"));
        assert_eq!(r["revision_before"], json!(rev));
        assert!(e.revision > rev);
        assert_eq!((vol(&e, a), vol(&e, b)), (-6.0, -3.0));
        assert!(e.undo());
        assert_eq!((vol(&e, a), vol(&e, b)), (0.0, 0.0));
        assert_ne!(e.session().track(a).map(|t| t.name.clone()), Some("Lead".into()));
    }

    #[test]
    fn a_stale_revision_changes_nothing() {
        let (mut e, a, _) = two_tracks();
        let stale = e.content_revision();
        e.execute("mix.volume", &json!({"track": a.0, "db": -1.0})).unwrap();
        let (rev, undo, doc) = (e.revision, e.undo_history(), e.session_arc());
        let err = e.execute_group(stale, "x", &[("mix.volume".into(), json!({"track": a.0, "db": -9.0}))]).unwrap_err();
        assert!(matches!(err, EngineError::Stale(_)), "{err:?}");
        assert_eq!((e.revision, e.undo_history()), (rev, undo));
        assert!(Arc::ptr_eq(&doc, &e.session_arc()));
    }

    #[test]
    fn a_failure_in_the_last_operation_leaves_no_trace() {
        let (mut e, a, b) = two_tracks();
        e.execute("mix.volume", &json!({"track": a.0, "db": -1.0})).unwrap();
        e.undo(); // leaves one redo entry that the group must not destroy
        let (rev, undo, redo, journal, dirty, doc) = (e.revision, e.undo_history(), e.can_redo(), e.journal.len(), e.is_dirty(), e.session_arc());
        let ops = vec![
            ("mix.volume".to_string(), json!({"track": a.0, "db": -6.0})),
            ("mix.volume".to_string(), json!({"track": b.0, "db": -3.0})),
            ("mix.volume".to_string(), json!({"track": 9999, "db": -3.0})), // no such track
        ];
        assert!(e.execute_group(e.content_revision(), "bad", &ops).is_err());
        assert_eq!(e.revision, rev, "revision restored");
        assert_eq!(e.undo_history(), undo, "undo stack restored");
        assert_eq!(e.can_redo(), redo, "redo entry survives a failed group");
        assert_eq!(e.journal.len(), journal, "journal restored");
        assert_eq!(e.is_dirty(), dirty);
        assert!(Arc::ptr_eq(&doc, &e.session_arc()), "the very same document is back");
        assert_eq!((vol(&e, a), vol(&e, b)), (0.0, 0.0));
    }

    #[test]
    fn a_full_undo_stack_does_not_lose_old_entries() {
        let (mut e, a, _) = two_tracks();
        e.history_limit = 3;
        for db in [-1.0, -2.0, -3.0] {
            e.execute("mix.volume", &json!({"track": a.0, "db": db})).unwrap();
        }
        let ops: Vec<(String, Value)> = (0..5).map(|i| ("mix.volume".to_string(), json!({"track": a.0, "db": -10.0 - i as f64}))).collect();
        let mut bad = ops.clone();
        bad.push(("mix.volume".into(), json!({"track": 9999, "db": 0.0})));
        assert!(e.execute_group(e.content_revision(), "bad", &bad).is_err());
        // Every label is "Volume", so compare the states the undo stack leads back to instead.
        let mut walked = Vec::new();
        while e.undo() {
            walked.push(vol(&e, a));
        }
        assert_eq!(walked, vec![-2.0, -1.0, 0.0], "per-command pushes past the cap must not eat history on failure");
        while e.redo() {}
        assert_eq!(vol(&e, a), -3.0);
        e.execute_group(e.content_revision(), "ok", &ops).unwrap();
        assert_eq!(e.undo_history().len(), 3);
        assert_eq!(e.undo_label(), Some("ok"));
    }

    #[test]
    fn transport_commands_are_not_admitted_so_a_group_never_queues_playback() {
        let (mut e, a, _) = two_tracks();
        let queued = e.transport_requests.len();
        let ops = vec![("transport.play".to_string(), json!({})), ("mix.volume".to_string(), json!({"track": a.0, "db": -1.0}))];
        let err = e.execute_group(e.content_revision(), "play", &ops).unwrap_err();
        assert!(err.to_string().contains("only undoable document edits"), "{err}");
        assert_eq!(e.transport_requests.len(), queued);
        assert_eq!(vol(&e, a), 0.0, "nothing ran");
    }

    #[test]
    fn refused_while_recording() {
        let (mut e, a, _) = two_tracks();
        e.transport.recording = true;
        let err = e.execute_group(e.content_revision(), "x", &[("mix.volume".into(), json!({"track": a.0, "db": -1.0}))]).unwrap_err();
        assert!(matches!(err, EngineError::Disabled(..)), "{err:?}");
    }

    #[test]
    fn content_revision_ignores_selection_and_playhead_but_not_edits() {
        let mut e = crate::demo::demo_engine();
        let c0 = e.content_revision();
        for (id, p) in [
            ("edit.select", json!({"tracks": ["Kick"], "start": 0, "end": 48_000})),
            ("edit.select_none", json!({})),
            ("transport.locate", json!({"at": 96_000})),
            ("view.zoom_in", json!({})),
        ] {
            e.execute(id, &p).unwrap();
            assert_eq!(e.content_revision(), c0, "{id} must not change the content revision");
        }
        e.execute("mix.mute", &json!({"track": "Kick"})).unwrap();
        assert!(e.content_revision() > c0, "a real edit moves it");
    }

    #[test]
    fn content_revision_cannot_be_reused_by_undoing_back() {
        let (mut e, a, _) = two_tracks();
        let plan_made_at = e.content_revision();
        e.execute("mix.volume", &json!({"track": a.0, "db": -6.0})).unwrap();
        let after_edit = e.content_revision(); // someone looked while the edit was in place
        assert!(after_edit > plan_made_at);
        e.undo();
        assert!(e.content_revision() > after_edit, "undo is a change too");
        assert_ne!(e.content_revision(), plan_made_at, "same content, but the old plan is still stale (no ABA)");
        let err = e.execute_group(plan_made_at, "old plan", &[("mix.volume".into(), json!({"track": a.0, "db": -9.0}))]).unwrap_err();
        assert!(matches!(err, EngineError::Stale(_)));
    }

    #[test]
    fn a_selection_change_between_review_and_publish_does_not_stale_a_plan() {
        let (mut e, a, b) = two_tracks();
        let plan = e.content_revision();
        e.execute("edit.select", &json!({"tracks": [b.0]})).unwrap();
        e.execute_group(plan, "ok", &[("mix.volume".into(), json!({"track": a.0, "db": -6.0}))]).unwrap();
        assert_eq!(vol(&e, a), -6.0);
    }

    #[test]
    fn a_failed_group_does_not_move_the_content_revision() {
        let (mut e, a, _) = two_tracks();
        let c = e.content_revision();
        let ops = vec![("mix.volume".to_string(), json!({"track": a.0, "db": -6.0})), ("mix.volume".to_string(), json!({"track": 9999, "db": 0.0}))];
        assert!(e.execute_group(c, "bad", &ops).is_err());
        assert_eq!(e.content_revision(), c);
    }

    #[test]
    fn storing_plugin_state_moves_the_content_revision_without_an_undo_entry() {
        let (mut e, a, _) = two_tracks();
        e.execute("mix.insert", &json!({"track": a.0, "plugin": "gain", "slot": 0})).unwrap();
        let (c, undo) = (e.content_revision(), e.undo_history().len());
        let blob = soundcraft_model::b64::encode(b"state");
        e.execute("mix.insert_state", &json!({"track": a.0, "slot": 0, "state": blob})).unwrap();
        assert!(e.content_revision() > c, "save and bounce do this, so a plan made before them goes stale");
        assert_eq!(e.undo_history().len(), undo, "and it is not undoable");
    }

    #[test]
    #[ignore = "timing probe, run with --ignored --nocapture"]
    fn probe_cost_of_comparing_documents() {
        let e = crate::demo::demo_engine();
        let doc = e.session_arc();
        let t = std::time::Instant::now();
        let n = 200;
        for _ in 0..n {
            std::hint::black_box(same_document(&doc, &doc.as_ref().clone()));
        }
        println!("same_document on the demo session: {:?} per call ({} tracks, {} sources)", t.elapsed() / n, doc.tracks.len(), doc.sources.len());
    }

    // ---- findings, pinned as tests so a later change in behaviour is noticed ----

    #[test]
    fn finding_which_view_and_selection_commands_move_the_revision() {
        let mut e = crate::demo::demo_engine();
        let mut moved = Vec::new();
        for (id, p) in [
            ("edit.select", json!({"tracks": ["Kick"], "start": 0, "end": 48_000})),
            ("edit.select_none", json!({})),
            ("transport.locate", json!({"at": 96_000})),
            ("view.zoom_in", json!({})),
            ("view.scroll", json!({"to": 48_000})),
        ] {
            let rev = e.revision;
            e.execute(id, &p).unwrap();
            if e.revision != rev {
                moved.push(id);
            }
        }
        println!("REVISION MOVED BY: {moved:?}");
        assert!(moved.contains(&"edit.select"));
        assert!(!moved.contains(&"view.zoom_in") && !moved.contains(&"view.scroll"), "zoom and scroll never move it");
    }

    #[test]
    fn finding_selecting_bumps_the_revision() {
        let mut e = crate::demo::demo_engine();
        let rev = e.revision;
        e.execute("edit.select", &json!({"tracks": ["Kick"], "start": 0, "end": 48_000})).unwrap();
        assert!(e.revision > rev, "a selection change is a revision change (plan D5)");
        assert!(!e.is_dirty(), "...even though the session is not dirty");
    }

    #[test]
    fn finding_volume_on_a_grouped_track_changes_its_siblings() {
        let (mut e, a, b) = two_tracks();
        e.execute("track.group", &json!({"tracks": [a.0, b.0], "mix": true})).unwrap();
        e.execute_group(e.content_revision(), "one track", &[("mix.volume".into(), json!({"track": a.0, "db": -6.0}))]).unwrap();
        assert_eq!(vol(&e, b), -6.0, "the sibling moved although only one track was named (plan P1-E06)");
    }

    #[test]
    fn finding_exclusive_solo_clears_other_solos_and_missing_track_falls_back_to_selection() {
        let (mut e, a, b) = two_tracks();
        e.execute("mix.solo", &json!({"track": b.0, "value": true})).unwrap();
        e.execute_group(e.content_revision(), "solo a", &[("mix.solo".into(), json!({"track": a.0, "value": true, "exclusive": true}))]).unwrap();
        assert!(e.session().track(a).is_some_and(|t| t.mixer.solo));
        assert!(e.session().track(b).is_some_and(|t| !t.mixer.solo), "an unrelated solo was cleared");
        // Without `track`, the command acts on the selection.
        e.execute("edit.select", &json!({"tracks": [b.0]})).unwrap();
        e.execute_group(e.content_revision(), "no target", &[("mix.mute".into(), json!({"value": true}))]).unwrap();
        assert!(e.session().track(b).is_some_and(|t| t.mixer.mute), "the selection was muted with no `track` given");
    }
}
