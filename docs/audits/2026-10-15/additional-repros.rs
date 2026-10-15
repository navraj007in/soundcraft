use serde_json::json;
use soundcraft_audio_io::AudioBuffer;
use soundcraft_dsp::{Plugin, PluginInfo};
use soundcraft_engine::Engine;
use soundcraft_mix::{INSTRUMENT_SLOT, MixEngine, PreparedInstance};
use soundcraft_model::{ChannelFormat, Clip, ClipId, Insert, Session, SourceAudio, SourceId, TrackKind};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[test]
fn reset_leaves_old_audio_in_compensation_delay() {
    let mut s = Session::default();
    let dry = s.add_track(TrackKind::Audio, ChannelFormat::Mono, None);
    let latent = s.add_track(TrackKind::Audio, ChannelFormat::Mono, None);
    s.pool.insert(SourceId(100), Arc::new(SourceAudio::new(AudioBuffer { sample_rate: 48000, channels: vec![vec![0.5; 512]] })));
    s.track_mut(dry).unwrap().playlist_mut().unwrap().clips.push(Clip::audio(ClipId(101), "old audio", SourceId(100), 0, 0, 512));
    let mut comp = Insert::new("compressor");
    comp.params.insert("lookahead".into(), 5.0);
    s.track_mut(latent).unwrap().mixer.inserts[0] = Some(comp);
    let mut m = MixEngine::new(48000., 512);
    let mut out = vec![vec![0.; 512]; 2];
    m.render(&s, 0, 512, &mut out);
    m.reset();
    m.render(&s, 4096, 512, &mut out);
    let leaked = out[0].iter().filter(|x| x.abs() > 0.1).count();
    println!("After stop/reset and seek to silence, {leaked} old samples played");
    assert_eq!(leaked, 240);
}

struct Notes {
    inner: Box<dyn Plugin>,
    on: Arc<AtomicUsize>,
    off: Arc<AtomicUsize>,
}
impl Plugin for Notes {
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
    fn note_on(&mut self, _: usize, _: u8, _: u8) {
        self.on.fetch_add(1, Ordering::SeqCst);
    }
    fn note_off(&mut self, _: usize, _: u8) {
        self.off.fetch_add(1, Ordering::SeqCst);
    }
}
#[test]
fn note_at_clip_end_on_block_boundary_never_receives_note_off() {
    let mut s = Session::default();
    let t = s.add_track(TrackKind::Instrument, ChannelFormat::Stereo, None);
    let mut seq = soundcraft_midi::Sequence::default();
    seq.notes.push(soundcraft_midi::Note { start: 0, length: 960, pitch: 60, velocity: 100, release_velocity: 0, channel: 0 });
    let tr = s.track_mut(t).unwrap();
    tr.instrument = Some(Insert::new("clap:audit.notes"));
    tr.playlist_mut().unwrap().clips.push(Clip::midi(ClipId(100), "note", 0, 512, seq));
    let on = Arc::new(AtomicUsize::new(0));
    let off = Arc::new(AtomicUsize::new(0));
    let mut plugin = Notes { inner: soundcraft_dsp::create("gain").unwrap(), on: on.clone(), off: off.clone() };
    plugin.prepare(48000., 512, 2);
    let mut m = MixEngine::new(48000., 512);
    m.set_external_instances(true);
    m.adopt(vec![PreparedInstance { track: t, slot: INSTRUMENT_SLOT, id: "clap:audit.notes".into(), ch: 2, plugin: Box::new(plugin) }]);
    let mut out = vec![vec![0.; 512]; 2];
    for i in 0..8 {
        m.render(&s, i * 512, 512, &mut out);
    }
    println!("Note-on count {}, note-off count {} after clip end", on.load(Ordering::SeqCst), off.load(Ordering::SeqCst));
    assert_eq!(on.load(Ordering::SeqCst), 1);
    assert_eq!(off.load(Ordering::SeqCst), 0);
}
#[test]
fn failed_group_leaves_save_path_and_external_file_changed() {
    let dir = std::env::temp_dir().join(format!("soundcraft-review-group-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("changed.scraft");
    let mut e = Engine::default();
    let revision = e.content_revision();
    let result = e.execute_group(revision, "test", &[("session.save_as".into(), json!({"path":path})), ("does.not.exist".into(), json!({}))]);
    assert!(result.is_err());
    assert_eq!(e.path.as_deref(), path.to_str());
    assert!(path.is_file());
    println!("Failed group kept path {:?} and saved file: {:?}", e.path, result);
    std::fs::remove_dir_all(dir).unwrap();
}
