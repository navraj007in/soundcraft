use serde_json::json;
use soundcraft_audio_io::{AudioBuffer, FileFormat};
use soundcraft_dsp::{Plugin, PluginInfo};
use soundcraft_engine::{Engine, io};
use soundcraft_mix::{MixEngine, PreparedInstance};
use soundcraft_model::{ChannelFormat, Clip, ClipId, Insert, Route, Session, SourceAudio, SourceId, TrackKind};
use soundcraft_time::Range;
use std::sync::Arc;

fn dc() -> (Session, soundcraft_model::TrackId) {
    let mut s = Session::default();
    let t = s.add_track(TrackKind::Audio, ChannelFormat::Mono, None);
    s.pool.insert(SourceId(100), Arc::new(SourceAudio::new(AudioBuffer { sample_rate: 48000, channels: vec![vec![0.5; 2048]] })));
    s.track_mut(t).unwrap().playlist_mut().unwrap().clips.push(Clip::audio(ClipId(101), "DC", SourceId(100), 0, 0, 2048));
    (s, t)
}

#[test]
fn gesture_at_history_cap_cannot_undo_whole_gesture() {
    let (s, t) = dc();
    let mut e = Engine::new(s);
    e.history_limit = 2;
    for db in [-1., -2.] {
        e.execute("mix.volume", &json!({"track":t.0,"db":db})).unwrap();
    }
    for db in [-3., -4., -5.] {
        e.execute_merged("mix.volume", &json!({"track":t.0,"db":db}), "drag").unwrap();
    }
    e.end_merge();
    e.undo();
    let after = e.session().track(t).unwrap().mixer.volume_db;
    println!("Undo at history cap restored {after} dB; pre-gesture was -2 dB");
    assert_eq!(after, -4.0);
}

#[test]
fn autosave_of_second_same_named_session_reuses_first_sessions_audio() {
    let dir = std::env::temp_dir().join(format!("soundcraft-audit-autosave-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("Untitled.scraft").to_string_lossy().into_owned();
    let make = |v| {
        let mut e = Engine::default();
        io::add_source(e.session_mut(), "Vocal", AudioBuffer { sample_rate: 48000, channels: vec![vec![v; 100]] }, None, FileFormat::Wav);
        e
    };
    io::save_copy(&make(0.5), &path).unwrap();
    io::save_copy(&make(-0.25), &path).unwrap();
    let mut restored = Engine::default();
    io::open_session(&mut restored, &path).unwrap();
    let src = restored.session().sources[0].id;
    let got = restored.session().pool.get(src).unwrap().buffer.channels[0][0];
    println!("Autosave B restored {got}; B's actual sample was -0.25");
    assert_eq!(got, 0.5);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn soloing_track_mutes_second_aux_in_chain() {
    let (mut s, t) = dc();
    let b1 = s.add_bus("B1", ChannelFormat::Stereo);
    let b2 = s.add_bus("B2", ChannelFormat::Stereo);
    let a1 = s.add_track(TrackKind::Aux, ChannelFormat::Stereo, Some("Aux1"));
    let a2 = s.add_track(TrackKind::Aux, ChannelFormat::Stereo, Some("Aux2"));
    s.track_mut(t).unwrap().mixer.output = Route::Bus(b1);
    s.track_mut(a1).unwrap().mixer.input = Route::Bus(b1);
    s.track_mut(a1).unwrap().mixer.output = Route::Bus(b2);
    s.track_mut(a2).unwrap().mixer.input = Route::Bus(b2);
    let normal = soundcraft_mix::render_range(&s, Range::new(0, 1024), 512)[0][500];
    s.track_mut(t).unwrap().mixer.solo = true;
    let solo = soundcraft_mix::render_range(&s, Range::new(0, 1024), 512)[0][500];
    println!("Track -> Aux1 -> Aux2 -> Main: normal {normal}; solo {solo}");
    assert!(normal > 0.1 && solo.abs() < 1e-6);
}

struct Blob {
    inner: Box<dyn Plugin>,
    blob: Vec<u8>,
}
impl Plugin for Blob {
    fn info(&self) -> &'static PluginInfo {
        self.inner.info()
    }
    fn prepare(&mut self, sr: f32, mb: usize, ch: usize) {
        self.inner.prepare(sr, mb, ch);
    }
    fn reset(&mut self) {
        self.inner.reset();
    }
    fn set_param(&mut self, id: &str, v: f32) -> bool {
        self.inner.set_param(id, v)
    }
    fn param(&self, id: &str) -> Option<f32> {
        self.inner.param(id)
    }
    fn process(&mut self, io: &mut [Vec<f32>], n: usize) {
        self.inner.process(io, n);
    }
    fn save_state(&mut self) -> Option<Vec<u8>> {
        Some(self.blob.clone())
    }
    fn load_state(&mut self, b: &[u8]) -> bool {
        self.blob = b.to_vec();
        true
    }
}

#[test]
fn same_slot_in_new_session_keeps_old_plugin_state() {
    let (mut s, t) = dc();
    let mut ins = Insert::new("clap:audit.fake");
    ins.set_state_bytes(b"SESSION A");
    s.track_mut(t).unwrap().mixer.inserts[0] = Some(ins);
    let mut plugin = Blob { inner: soundcraft_dsp::create("gain").unwrap(), blob: b"SESSION A".to_vec() };
    plugin.prepare(48000., 512, 1);
    let mut m = MixEngine::new(48000., 512);
    m.set_external_instances(true);
    m.adopt(vec![PreparedInstance { track: t, slot: 0, id: "clap:audit.fake".into(), ch: 1, plugin: Box::new(plugin) }]);
    m.ensure_synced(&s);
    let mut next = s.clone();
    next.name = "Session B".into();
    next.track_mut(t).unwrap().mixer.inserts[0].as_mut().unwrap().set_state_bytes(b"SESSION B");
    m.reset();
    m.adopt(vec![]);
    m.ensure_synced(&next);
    let live = m.capture_states()[0].2.clone();
    println!("Session B's live plugin state: {}", String::from_utf8_lossy(&live));
    assert_eq!(live, b"SESSION A");
}

#[test]
fn loaded_session_accepts_zero_rate_and_overflowing_tempo_map() {
    let mut v = serde_json::to_value(Session::default()).unwrap();
    v["sample_rate"] = json!(0);
    v["tempo"]["tempos"] = json!([
        {"tick":0,"bpm":120.0},
        {"tick":i64::MIN,"bpm":120.0},
        {"tick":i64::MAX,"bpm":120.0}
    ]);
    let s = Session::from_json(&v.to_string()).unwrap();
    assert_eq!(s.sample_rate.hz(), 0);
    let panicked = std::panic::catch_unwind(|| s.tempo.samples_to_ticks(0, s.sample_rate)).is_err();
    println!("Accepted sample_rate=0 and unordered extreme tempo ticks; conversion panicked: {panicked}");
    assert!(panicked);
}

#[test]
fn master_and_cascaded_aux_latency_shift_and_truncate_bounce() {
    let (mut s, t) = dc();
    let mut impulses = vec![0.0; 2048];
    for i in [0, 1024, 2000] {
        impulses[i] = 0.25;
    }
    s.pool.insert(SourceId(100), Arc::new(SourceAudio::new(AudioBuffer { sample_rate: 48000, channels: vec![impulses] })));
    let peaks = |s: &Session| {
        soundcraft_mix::render_range(s, Range::new(0, 2048), 512)[0]
            .iter()
            .enumerate()
            .filter(|(_, x)| x.abs() > 1e-4)
            .map(|(i, _)| i)
            .collect::<Vec<_>>()
    };
    assert_eq!(peaks(&s), vec![0, 1024, 2000]);
    let mut compressor = Insert::new("compressor");
    compressor.params.insert("lookahead".into(), 5.0);
    compressor.params.insert("ratio".into(), 1.0);
    let mut master_session = s.clone();
    let master = master_session.add_track(TrackKind::Master, ChannelFormat::Stereo, None);
    master_session.track_mut(master).unwrap().mixer.inserts[0] = Some(compressor.clone());
    let master_peaks = peaks(&master_session);
    println!("Master with 5 ms lookahead: impulses at {master_peaks:?}; expected [0,1024,2000]");
    assert_eq!(master_peaks, vec![240, 1264]);
    let b1 = s.add_bus("B1", ChannelFormat::Stereo);
    let b2 = s.add_bus("B2", ChannelFormat::Stereo);
    let a1 = s.add_track(TrackKind::Aux, ChannelFormat::Stereo, Some("Aux1"));
    let a2 = s.add_track(TrackKind::Aux, ChannelFormat::Stereo, Some("Aux2"));
    s.track_mut(t).unwrap().mixer.output = Route::Bus(b1);
    s.track_mut(a1).unwrap().mixer.input = Route::Bus(b1);
    s.track_mut(a1).unwrap().mixer.output = Route::Bus(b2);
    s.track_mut(a2).unwrap().mixer.input = Route::Bus(b2);
    for a in [a1, a2] {
        s.track_mut(a).unwrap().mixer.inserts[0] = Some(compressor.clone());
    }
    let aux_peaks = peaks(&s);
    println!("Two auxes with 5 ms lookahead each: impulses at {aux_peaks:?}; expected [0,1024,2000]");
    assert_eq!(aux_peaks, vec![240, 1264]);
}
