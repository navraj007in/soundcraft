//! Realtime playback through the system audio device (cpal: CoreAudio, WASAPI, ALSA/Pulse/
//! PipeWire via ALSA, JACK, WebAudio).
//!
//! The UI thread owns a [`Player`]; the audio callback owns the [`MixEngine`] and a snapshot of
//! the session (`Arc<Session>`) that the UI swaps whenever the document changes. Commands go to
//! the callback through a channel; position and meters come back through atomics and a
//! `try_lock`ed snapshot, so the callback never blocks. When no device is available the player
//! falls back to a silent clock thread so the transport, meters and automation still run.
//!
//! Surround: the device is opened with as many output channels as it offers, up to the width of
//! the session's main format. Main channels (SMPTE/WAV order) go to device channels in order; when
//! the device has fewer channels than the main mix, the mix is folded down to stereo (ITU-R
//! BS.775) on the first two device channels.
//!
//! Plugins and threads: hosted third-party plugins (CLAP/VST3) are created, given their stored
//! state and prepared on the thread that calls [`Player::update_session`] (the UI thread), and
//! travel to the audio thread together with the session snapshot that needs them; the mix
//! engine adopts them (see `soundcraft_mix`'s crate docs). Instances the engine retires come back
//! on a return channel and are dropped on the UI thread ([`Player::idle`]). Plugin states for
//! saving are read on request ([`Player::capture_states`]): the audio thread hands its plugins to
//! the caller (moving boxes only), the caller serialises them on its own thread and sends them
//! back, so no plugin code runs in the audio callback. Plugin editors are driven on the UI thread
//! through handles taken from each instance before it leaves ([`Player::open_editor`]).
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod record;

use soundcraft_dsp::{Plugin, PluginEditor};
use soundcraft_mix::{MAX_CHANNELS, MixEngine, PreparedInstance, StripMeter, itu_stereo_fold, main_channels};
use soundcraft_model::{Session, TrackId};
use soundcraft_time::{Range, Samples};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, PoisonError};

const BLOCK: usize = 512;

/// The realtime audio thread marker lives in the mix engine, which carries it into the strips it
/// renders on worker threads; the cpal callbacks here set it.
pub use soundcraft_mix::{mark_audio_thread, on_audio_thread};

/// Meter values for the UI.
#[derive(Debug, Clone, Default)]
pub struct MeterSnapshot {
    pub tracks: HashMap<TrackId, StripMeter>,
    pub main: StripMeter,
}

/// Plugin states read from the live instances: `(track, slot or INSTRUMENT_SLOT, state)`.
pub type PluginStates = Vec<(TrackId, usize, Vec<u8>)>;

/// Why plugin states could not be read. An empty list is a valid answer (no stateful plugins), so
/// a failure must never look like one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureError {
    /// The audio thread did not answer in time (a plugin is slow to serialise, or the device stalled).
    Timeout,
    /// The audio thread is gone.
    Unavailable,
}

impl std::fmt::Display for CaptureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CaptureError::Timeout => f.write_str("the audio engine did not answer in time"),
            CaptureError::Unavailable => f.write_str("the audio engine is not running"),
        }
    }
}

impl std::error::Error for CaptureError {}

/// How long a state capture may take before it is reported as failed.
#[cfg(not(target_arch = "wasm32"))]
const CAPTURE_WAIT: std::time::Duration = std::time::Duration::from_secs(2);

/// Read every plugin state without running plugin code on the audio thread. The audio thread hands
/// its third-party plugins over (only moving boxes), this thread serialises them (where hosts expect
/// state calls) and sends them back; while a plugin is away its slot is bypassed. Plugins always go
/// back, even when one panics or the caller gives up, and a plugin that arrives after the document
/// was replaced is dropped, never restored.
#[cfg(not(target_arch = "wasm32"))]
fn request_capture(tx: &Sender<Cmd>, wait: std::time::Duration) -> Result<PluginStates, CaptureError> {
    let (rtx, rrx) = channel();
    tx.send(Cmd::Checkout(rtx)).map_err(|_| CaptureError::Unavailable)?;
    let (generation, mut taken) = rrx.recv_timeout(wait).map_err(|e| match e {
        std::sync::mpsc::RecvTimeoutError::Timeout => CaptureError::Timeout,
        std::sync::mpsc::RecvTimeoutError::Disconnected => CaptureError::Unavailable,
    })?;
    let mut states = PluginStates::new();
    for p in &mut taken {
        let saved = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| p.plugin.save_state()));
        match saved {
            Ok(Some(data)) => states.push((p.track, p.slot, data)),
            Ok(None) => {}
            Err(_) => log::error!("plugin {} panicked while saving its state", p.id),
        }
    }
    states.sort_by_key(|(t, s, _)| (*t, *s));
    let _ = tx.send(Cmd::Return(generation, taken));
    Ok(states)
}

/// A parameter edit made in a plugin's own editor: `(track, slot, param id, value)`.
pub type EditorEdit = (TrackId, usize, String, f32);

enum Cmd {
    /// A new snapshot plus the third-party instances it needs (created on the UI thread).
    Session(Arc<Session>, Vec<PreparedInstance>),
    /// Hand every third-party plugin over for state capture, with the current generation.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    Checkout(Sender<(u64, Vec<PreparedInstance>)>),
    /// Plugins coming back from a capture, with the generation they were taken in.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    Return(u64, Vec<PreparedInstance>),
    Input(Arc<record::InputRing>),
    Recording(bool),
    Play {
        from: Samples,
        end: Option<Samples>,
        looped: Option<Range>,
    },
    Stop,
    Locate(Samples),
    /// The document was replaced: drop every plugin instance (they belong to the old one).
    ResetSession,
}

struct Shared {
    position: AtomicI64,
    playing: AtomicBool,
    /// Speed × 1000.
    speed: AtomicU32,
    meters: Mutex<MeterSnapshot>,
}

/// State owned by the audio callback.
struct AudioState {
    rx: Receiver<Cmd>,
    shared: Arc<Shared>,
    session: Arc<Session>,
    mix: MixEngine,
    pos: Samples,
    playing: bool,
    end: Option<Samples>,
    looped: Option<Range>,
    /// Rendered frames waiting to be consumed (planar, main channels) and the fractional read index.
    pending: Vec<Vec<f32>>,
    /// Stereo fold-down gains per main channel (used when the device has fewer channels).
    fold: [(f32, f32); MAX_CHANNELS],
    pending_len: usize,
    read: f64,
    device_rate: f64,
    input: Option<Arc<record::InputRing>>,
    recording: bool,
    /// Bumped whenever plugins taken out for a capture must not come back (document or engine replaced).
    generation: u64,
    /// Retired plugin instances go back to the UI thread to be dropped there.
    garbage: Sender<Vec<Box<dyn Plugin>>>,
}

fn new_engine(sr: f32) -> MixEngine {
    let mut m = MixEngine::new(sr, BLOCK);
    m.set_external_instances(true);
    m.metronome = true;
    m
}

impl AudioState {
    fn new(rx: Receiver<Cmd>, shared: Arc<Shared>, session: Arc<Session>, device_rate: f64, garbage: Sender<Vec<Box<dyn Plugin>>>) -> Self {
        let sr = session.sample_rate.as_f64() as f32;
        let nch = main_channels(&session);
        AudioState {
            generation: 0,
            garbage,
            fold: fold_gains(&session),
            rx,
            shared,
            mix: new_engine(sr),
            session,
            pos: 0,
            playing: false,
            end: None,
            looped: None,
            pending: vec![vec![0.0; BLOCK]; nch],
            pending_len: 0,
            read: 0.0,
            device_rate,
            input: None,
            recording: false,
        }
    }

    fn drain(&mut self) {
        let mut changed = false;
        while let Ok(c) = self.rx.try_recv() {
            match c {
                Cmd::Input(r) => self.input = Some(r),
                Cmd::Recording(r) => self.recording = r,
                Cmd::Checkout(reply) => {
                    // Place adopted instances first so they are handed over too.
                    self.mix.ensure_synced(&self.session);
                    let taken = self.mix.checkout_plugins();
                    if let Err(std::sync::mpsc::SendError((_, taken))) = reply.send((self.generation, taken)) {
                        // The requester gave up: nobody will send them back, so restore them now.
                        self.mix.restore_plugins(&self.session, taken);
                    }
                }
                Cmd::Return(generation, taken) => {
                    if generation == self.generation {
                        self.mix.restore_plugins(&self.session, taken);
                    } else {
                        // They belong to a replaced document or engine: never put them back.
                        let _ = self.garbage.send(taken.into_iter().map(|p| p.plugin).collect());
                    }
                }
                Cmd::Session(s, instances) => {
                    changed = true;
                    if (s.sample_rate.as_f64() - self.session.sample_rate.as_f64()).abs() > 0.5 {
                        let mut old = std::mem::replace(&mut self.mix, new_engine(s.sample_rate.as_f64() as f32));
                        let _ = self.garbage.send(old.drain_plugins());
                        // Plugins out for a capture were prepared for the old rate.
                        self.generation = self.generation.wrapping_add(1);
                    }
                    self.mix.adopt(instances);
                    // The main format changed (a document edit, not steady state): resize.
                    let nch = main_channels(&s);
                    if self.pending.len() != nch {
                        self.pending = vec![vec![0.0; BLOCK]; nch];
                        self.pending_len = 0;
                        self.read = 0.0;
                    }
                    self.fold = fold_gains(&s);
                    self.session = s;
                }
                Cmd::Play { from, end, looped } => {
                    self.pos = from;
                    self.end = end;
                    self.looped = looped.filter(|r| r.len() > 64);
                    self.playing = true;
                    self.pending_len = 0;
                    self.read = 0.0;
                    self.mix.reset();
                }
                Cmd::Stop => {
                    self.playing = false;
                    self.end = None;
                    self.looped = None;
                    self.mix.reset();
                }
                Cmd::Locate(at) => {
                    self.pos = at;
                    self.pending_len = 0;
                    self.read = 0.0;
                }
                Cmd::ResetSession => {
                    let sr = self.session.sample_rate.as_f64() as f32;
                    let mut old = std::mem::replace(&mut self.mix, new_engine(sr));
                    let _ = self.garbage.send(old.drain_plugins());
                    self.generation = self.generation.wrapping_add(1);
                    self.playing = false;
                    self.end = None;
                    self.looped = None;
                    changed = true;
                }
            }
        }
        if changed {
            // Place adopted instances now (not only once playback starts), so state captures and
            // editors see them; `sync` creates no third-party plugins here.
            self.mix.ensure_synced(&self.session);
        }
        self.send_garbage();
        self.shared.playing.store(self.playing, Ordering::Relaxed);
    }

    fn send_garbage(&mut self) {
        let g = self.mix.take_retired();
        if !g.is_empty() {
            let _ = self.garbage.send(g);
        }
    }

    /// Render the next block at the session rate into `pending`.
    fn render_next(&mut self, quantum: usize) {
        let mut n = quantum.clamp(1, BLOCK);
        if let Some(l) = self.looped
            && self.pos >= l.end
        {
            self.pos = l.start;
        }
        if let Some(l) = self.looped {
            n = n.min(usize::try_from((l.end - self.pos).max(1)).unwrap_or(BLOCK));
        } else if let Some(e) = self.end {
            if self.pos >= e {
                self.playing = false;
                self.end = None;
                self.shared.playing.store(false, Ordering::Relaxed);
                n = 0;
            } else {
                n = n.min(usize::try_from(e - self.pos).unwrap_or(BLOCK));
            }
        }
        if n == 0 {
            for c in &mut self.pending {
                c.iter_mut().for_each(|x| *x = 0.0);
            }
            self.pending_len = BLOCK;
            return;
        }
        if let Some(ring) = &self.input {
            let ch = ring.channels.load(Ordering::Relaxed).max(1);
            if self.mix.input.len() != ch {
                self.mix.input = vec![vec![0.0; BLOCK]; ch];
            }
            ring.pop_into(&mut self.mix.input, n, self.session.sample_rate.hz());
        }
        self.mix.monitor_only = !self.playing;
        self.mix.recording = self.recording;
        self.mix.render(&self.session, self.pos, n, &mut self.pending);
        self.send_garbage();
        self.pending_len = n;
        if let Ok(mut m) = self.shared.meters.try_lock() {
            m.main.copy_from(&self.mix.main_meter);
            for (k, v) in &self.mix.meters {
                match m.tracks.get_mut(k) {
                    Some(d) => d.copy_from(v),
                    None => {
                        m.tracks.insert(*k, v.clone());
                    }
                }
            }
        }
        if self.playing {
            self.pos += n as i64;
            self.shared.position.store(self.pos, Ordering::Relaxed);
        }
    }

    /// Fill an interleaved device buffer.
    fn fill(&mut self, out: &mut [f32], channels: usize) {
        self.drain();
        let channels = channels.max(1);
        let monitoring = self.input.is_some() && self.session.tracks.iter().any(|t| t.mixer.input_monitor || t.mixer.record_arm);
        if !self.playing && !monitoring {
            out.iter_mut().for_each(|x| *x = 0.0);
            if let Ok(mut m) = self.shared.meters.try_lock() {
                m.main = StripMeter::default();
                m.tracks.clear();
            }
            return;
        }
        let speed = f64::from(self.shared.speed.load(Ordering::Relaxed)) / 1000.0;
        let step = self.session.sample_rate.as_f64() / self.device_rate.max(1.0) * speed.clamp(0.1, 4.0);
        let callback_frames = out.len() / channels;
        let quantum = ((callback_frames as f64 * step).ceil() as usize).clamp(1, BLOCK);
        for frame in out.chunks_mut(channels) {
            while self.read >= self.pending_len as f64 {
                self.read -= self.pending_len as f64;
                if !self.playing && !monitoring {
                    break;
                }
                self.render_next(quantum);
                if self.pending_len == 0 {
                    break;
                }
            }
            let i0 = self.read.floor() as usize;
            let fr = (self.read - i0 as f64) as f32;
            let sounding = self.playing || monitoring;
            let nch = self.pending.len();
            let sample = |c: usize| {
                let ch = self.pending.get(c);
                let a = ch.and_then(|v| v.get(i0)).copied().unwrap_or(0.0);
                let b = ch.and_then(|v| v.get(i0 + 1)).copied().unwrap_or(a);
                a + (b - a) * fr
            };
            if nch > 2 && channels < nch {
                // Fewer device channels than the main mix: ITU fold-down to stereo.
                let (mut l, mut r) = (0.0f32, 0.0f32);
                if sounding {
                    for (c, (gl, gr)) in self.fold.iter().enumerate().take(nch) {
                        let v = sample(c);
                        l += v * gl;
                        r += v * gr;
                    }
                }
                for (c, o) in frame.iter_mut().enumerate() {
                    *o = match c {
                        0 => l.clamp(-1.0, 1.0),
                        1 => r.clamp(-1.0, 1.0),
                        _ => 0.0,
                    };
                }
            } else {
                for (c, o) in frame.iter_mut().enumerate() {
                    // Stereo mixes on a mono device play the left channel (as before).
                    *o = if c < nch.max(1) && sounding { sample(c).clamp(-1.0, 1.0) } else { 0.0 };
                }
            }
            self.read += step;
        }
    }
}

/// UI-thread bookkeeping of the third-party instances the audio thread holds.
struct Side {
    /// What was last created for each slot: (plugin id, channels, created ok).
    shadow: HashMap<(TrackId, usize), (String, usize, bool)>,
    /// Session sample rate the instances were prepared for.
    rate: f32,
    /// Editor handles of the live instances.
    editors: HashMap<(TrackId, usize), Box<dyn PluginEditor>>,
    garbage: Receiver<Vec<Box<dyn Plugin>>>,
}

impl Side {
    /// Drops retired instances (closing their editors first, inside the plugin's drop).
    fn collect_garbage(&mut self) {
        while let Ok(g) = self.garbage.try_recv() {
            drop(g);
        }
    }

    /// Forget every instance: the next `prepare` creates them all anew from the session's stored state.
    fn forget_all(&mut self) {
        let keys: Vec<(TrackId, usize)> = self.shadow.keys().copied().collect();
        for k in keys {
            self.forget(k);
        }
        let editors: Vec<(TrackId, usize)> = self.editors.keys().copied().collect();
        for k in editors {
            self.forget(k);
        }
    }

    fn forget(&mut self, key: (TrackId, usize)) {
        self.shadow.remove(&key);
        if let Some(mut e) = self.editors.remove(&key) {
            e.close();
        }
    }
}

/// The transport and audio device.
pub struct Player {
    shared: Arc<Shared>,
    tx: Sender<Cmd>,
    side: Mutex<Side>,
    #[cfg(not(target_arch = "wasm32"))]
    _stream: Option<cpal::Stream>,
    #[cfg(target_arch = "wasm32")]
    _stream: Option<cpal::Stream>,
    pub device_name: String,
    pub device_rate: u32,
    /// Output channels the device was opened with (0 for the silent clock).
    pub device_channels: u16,
    /// True when no audio device could be opened (silent clock).
    pub silent: bool,
}

impl Player {
    /// Open the default output device. Never fails: without a device it runs a silent clock.
    pub fn new(session: Arc<Session>) -> Player {
        let p = Player::open(Arc::clone(&session));
        // Hand the initial session's third-party plugins over.
        p.update_session(session);
        p
    }

    fn side(&self) -> std::sync::MutexGuard<'_, Side> {
        self.side.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn open(session: Arc<Session>) -> Player {
        let (gtx, grx) = channel();
        let side = Mutex::new(Side { shadow: HashMap::new(), rate: session.sample_rate.as_f64() as f32, editors: HashMap::new(), garbage: grx });
        let shared = Arc::new(Shared {
            position: AtomicI64::new(0),
            playing: AtomicBool::new(false),
            speed: AtomicU32::new(1000),
            meters: Mutex::new(MeterSnapshot::default()),
        });
        let (tx, rx) = channel();
        let want = session.sample_rate.hz();
        let want_ch = u16::try_from(main_channels(&session)).unwrap_or(2);
        match open_device(want, want_ch) {
            Ok((device, config, name)) => {
                let rate = config.sample_rate.0;
                let channels = usize::from(config.channels);
                let mut state = AudioState::new(rx, Arc::clone(&shared), session, f64::from(rate), gtx);
                // cpal may call this on the audio thread (on ALSA in a loop while the device
                // stays broken): marked, so the logger hands the record off instead of writing it.
                let err_fn = |e: cpal::StreamError| {
                    mark_audio_thread();
                    log::warn!("audio stream error: {e}");
                };
                use cpal::traits::{DeviceTrait, StreamTrait};
                match device.build_output_stream(
                    &config,
                    move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                        mark_audio_thread();
                        state.fill(data, channels);
                    },
                    err_fn,
                    None,
                ) {
                    Ok(stream) => {
                        if let Err(e) = stream.play() {
                            log::warn!("audio stream failed to start: {e}");
                        }
                        log::info!("audio device: {name} @ {rate} Hz, {channels} ch");
                        Player {
                            shared,
                            tx,
                            side,
                            _stream: Some(stream),
                            device_name: name,
                            device_rate: rate,
                            device_channels: config.channels,
                            silent: false,
                        }
                    }
                    Err(e) => {
                        log::warn!("cannot open audio stream: {e}; using a silent clock");
                        let (tx2, rx2) = channel();
                        let (gtx2, grx2) = channel();
                        // The state (and its receivers) moved into the failed closure; rebuild.
                        let mut side = side;
                        side.get_mut().unwrap_or_else(PoisonError::into_inner).garbage = grx2;
                        Player::silent_clock(shared, tx2, rx2, want, side, gtx2)
                    }
                }
            }
            Err(e) => {
                log::warn!("no audio output: {e}; using a silent clock");
                let s2 = Arc::clone(&shared);
                Player::silent_clock_with(s2, tx, rx, session, side, gtx)
            }
        }
    }

    fn silent_clock(
        shared: Arc<Shared>,
        tx: Sender<Cmd>,
        rx: Receiver<Cmd>,
        rate: u32,
        side: Mutex<Side>,
        gtx: Sender<Vec<Box<dyn Plugin>>>,
    ) -> Player {
        let s = Arc::new(Session::new("Untitled", soundcraft_time::SampleRate::new(rate).unwrap_or_default()));
        Player::silent_clock_with(shared, tx, rx, s, side, gtx)
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn silent_clock_with(
        shared: Arc<Shared>,
        tx: Sender<Cmd>,
        rx: Receiver<Cmd>,
        session: Arc<Session>,
        side: Mutex<Side>,
        gtx: Sender<Vec<Box<dyn Plugin>>>,
    ) -> Player {
        let rate = session.sample_rate.hz();
        let mut state = AudioState::new(rx, Arc::clone(&shared), session, f64::from(rate), gtx);
        let spawn = std::thread::Builder::new().name("soundcraft-clock".into()).spawn(move || {
            let mut buf = vec![0.0f32; 4096];
            let mut last = std::time::Instant::now();
            loop {
                std::thread::sleep(std::time::Duration::from_millis(10));
                let now = std::time::Instant::now();
                let frames = ((now - last).as_secs_f64() * state.device_rate) as usize;
                last = now;
                let n = (frames * 2).min(buf.len());
                if let Some(b) = buf.get_mut(..n) {
                    state.fill(b, 2);
                }
                if Arc::strong_count(&state.shared) <= 1 {
                    break;
                }
            }
        });
        if let Err(e) = spawn {
            log::warn!("clock thread failed: {e}");
        }
        Player {
            shared,
            tx,
            side,
            _stream: None,
            device_name: "No audio device (silent)".into(),
            device_rate: rate,
            device_channels: 0,
            silent: true,
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn silent_clock_with(
        shared: Arc<Shared>,
        tx: Sender<Cmd>,
        _rx: Receiver<Cmd>,
        session: Arc<Session>,
        side: Mutex<Side>,
        _gtx: Sender<Vec<Box<dyn Plugin>>>,
    ) -> Player {
        Player {
            shared,
            tx,
            side,
            _stream: None,
            device_name: "No audio device".into(),
            device_rate: session.sample_rate.hz(),
            device_channels: 0,
            silent: true,
        }
    }

    /// Sends a new session snapshot to the audio thread, with freshly created (state-restored,
    /// prepared) instances of any third-party plugin it newly needs. Call on the UI thread.
    pub fn update_session(&self, s: Arc<Session>) {
        let instances = self.side().prepare(&s);
        let _ = self.tx.send(Cmd::Session(s, instances));
    }

    /// The document was replaced (new, open, revert): no plugin instance carries over. Call before
    /// `update_session` with the replacement, which then creates its plugins from its own state.
    pub fn reset_session(&self) {
        self.side().forget_all();
        let _ = self.tx.send(Cmd::ResetSession);
    }

    /// Reads the state of every live third-party plugin (for saving the session), on the calling
    /// thread. While the plugins are away their slots are bypassed (an instrument is silent), so
    /// during playback this is a brief audible gap; a failure is an error, never an empty list
    /// (which means "no stateful plugins").
    pub fn capture_states(&self) -> Result<PluginStates, CaptureError> {
        self.side().collect_garbage();
        #[cfg(not(target_arch = "wasm32"))]
        {
            request_capture(&self.tx, CAPTURE_WAIT)
        }
        #[cfg(target_arch = "wasm32")]
        {
            Ok(Vec::new())
        }
    }

    /// Per-UI-frame housekeeping: drops retired instances and services plugin editors. Returns
    /// the parameter edits made in open editors since the last call.
    pub fn idle(&self) -> Vec<EditorEdit> {
        let mut side = self.side();
        side.collect_garbage();
        let mut out = Vec::new();
        for (&(t, slot), e) in side.editors.iter_mut() {
            for (id, v) in e.idle() {
                out.push((t, slot, id, v));
            }
        }
        out
    }

    /// True when the plugin in this slot has its own editor.
    pub fn has_editor(&self, track: TrackId, slot: usize) -> bool {
        self.side().editors.contains_key(&(track, slot))
    }

    /// Opens the plugin's own editor window. Main thread only.
    pub fn open_editor(&self, track: TrackId, slot: usize) -> Result<(), String> {
        match self.side().editors.get_mut(&(track, slot)) {
            Some(e) => e.open(),
            None => Err("this plugin has no editor (or it is not loaded)".into()),
        }
    }

    pub fn close_editor(&self, track: TrackId, slot: usize) {
        if let Some(e) = self.side().editors.get_mut(&(track, slot)) {
            e.close();
        }
    }

    pub fn editor_open(&self, track: TrackId, slot: usize) -> bool {
        self.side().editors.get(&(track, slot)).is_some_and(|e| e.is_open())
    }

    pub fn any_editor_open(&self) -> bool {
        self.side().editors.values().any(|e| e.is_open())
    }

    /// Shows a parameter change made elsewhere in the plugin's open editor.
    pub fn editor_set_param(&self, track: TrackId, slot: usize, id: &str, value: f32) {
        if let Some(e) = self.side().editors.get_mut(&(track, slot)) {
            e.set_param(id, value);
        }
    }

    /// Start playback at `from`, stopping at `end`, or looping `looped`.
    pub fn play(&self, from: Samples, end: Option<Samples>, looped: Option<Range>) {
        self.shared.position.store(from, Ordering::Relaxed);
        self.shared.playing.store(true, Ordering::Relaxed);
        let _ = self.tx.send(Cmd::Play { from, end, looped });
    }

    /// Feed live input (from a `record::Recorder`) into the mix for input monitoring.
    pub fn set_input(&self, ring: Arc<record::InputRing>) {
        let _ = self.tx.send(Cmd::Input(ring));
    }

    pub fn set_recording(&self, on: bool) {
        let _ = self.tx.send(Cmd::Recording(on));
    }

    pub fn stop(&self) {
        self.shared.playing.store(false, Ordering::Relaxed);
        let _ = self.tx.send(Cmd::Stop);
    }

    pub fn locate(&self, at: Samples) {
        self.shared.position.store(at, Ordering::Relaxed);
        let _ = self.tx.send(Cmd::Locate(at));
    }

    pub fn set_speed(&self, speed: f32) {
        let v = if speed.is_finite() { (speed.clamp(0.1, 4.0) * 1000.0) as u32 } else { 1000 };
        self.shared.speed.store(v, Ordering::Relaxed);
    }

    /// Playback rate last requested, 1.0 at full speed.
    pub fn speed(&self) -> f32 {
        self.shared.speed.load(Ordering::Relaxed) as f32 / 1000.0
    }

    pub fn position(&self) -> Samples {
        self.shared.position.load(Ordering::Relaxed)
    }

    pub fn is_playing(&self) -> bool {
        self.shared.playing.load(Ordering::Relaxed)
    }

    pub fn meters(&self) -> MeterSnapshot {
        self.shared.meters.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// Copy meters into reusable UI storage. Once track/channel counts are stable this performs
    /// no allocation, unlike cloning a new snapshot every video frame.
    pub fn copy_meters(&self, out: &mut MeterSnapshot) {
        let source = self.shared.meters.lock().unwrap_or_else(PoisonError::into_inner);
        copy_meter_snapshot(&source, out);
    }
}

fn copy_meter_snapshot(source: &MeterSnapshot, out: &mut MeterSnapshot) {
    out.tracks.retain(|id, _| source.tracks.contains_key(id));
    for (id, meter) in &source.tracks {
        out.tracks.entry(*id).or_default().copy_from(meter);
    }
    out.main.copy_from(&source.main);
}

impl Side {
    /// Creates the third-party instances `s` needs that the audio thread does not have yet.
    fn prepare(&mut self, s: &Session) -> Vec<PreparedInstance> {
        self.collect_garbage();
        let sr = s.sample_rate.as_f64() as f32;
        if (sr - self.rate).abs() > 0.5 {
            // The audio thread rebuilds its engine: everything is created anew.
            let keys: Vec<(TrackId, usize)> = self.shadow.keys().copied().collect();
            for k in keys {
                self.forget(k);
            }
            self.rate = sr;
        }
        let specs = soundcraft_mix::instance_specs(s);
        let stale: Vec<(TrackId, usize)> = self.shadow.keys().filter(|k| !specs.iter().any(|x| (x.track, x.slot) == **k)).copied().collect();
        for k in stale {
            self.forget(k);
        }
        let mut out = Vec::new();
        for spec in specs {
            let key = (spec.track, spec.slot);
            if self.shadow.get(&key).is_some_and(|(id, ch, _)| *id == spec.id && *ch == spec.ch) {
                continue;
            }
            self.forget(key);
            let made = soundcraft_mix::create_instance(&spec.id, spec.state.as_deref(), sr, BLOCK, spec.ch);
            self.shadow.insert(key, (spec.id.clone(), spec.ch, made.is_some()));
            if let Some(mut plugin) = made {
                if let Some(e) = plugin.editor() {
                    self.editors.insert(key, e);
                }
                out.push(PreparedInstance { track: spec.track, slot: spec.slot, id: spec.id, ch: spec.ch, plugin });
            }
        }
        out
    }
}

/// ITU stereo fold-down gains for each main channel of a session.
fn fold_gains(s: &Session) -> [(f32, f32); MAX_CHANNELS] {
    let mut g = [(0.0, 0.0); MAX_CHANNELS];
    let f = s.main_format();
    let sp = f.speakers();
    if sp.len() == f.channels() {
        for (o, x) in g.iter_mut().zip(sp.iter()) {
            *o = itu_stereo_fold(*x);
        }
    } else {
        // Ambisonics: W only, at -3 dB per side.
        g[0] = (std::f32::consts::FRAC_1_SQRT_2, std::f32::consts::FRAC_1_SQRT_2);
    }
    g
}

/// Open the default output device: f32 at the session rate if possible, with as many channels as
/// it offers up to `want_ch` (at least stereo when available).
fn open_device(want_rate: u32, want_ch: u16) -> Result<(cpal::Device, cpal::StreamConfig, String), String> {
    use cpal::traits::{DeviceTrait, HostTrait};
    let host = cpal::default_host();
    let device = host.default_output_device().ok_or_else(|| "no default output device".to_string())?;
    #[allow(deprecated)]
    let name = device.name().unwrap_or_else(|_| "Audio device".into());
    // Prefer an f32 config at the session rate; else the device default (we resample).
    let want_ch = want_ch.max(2);
    let mut chosen: Option<cpal::StreamConfig> = None;
    if let Ok(configs) = device.supported_output_configs() {
        for c in configs {
            if c.sample_format() == cpal::SampleFormat::F32
                && c.min_sample_rate().0 <= want_rate
                && c.max_sample_rate().0 >= want_rate
                && c.channels() >= 2
                && c.channels() <= want_ch
                && chosen.as_ref().is_none_or(|x| c.channels() > x.channels)
            {
                chosen = Some(c.with_sample_rate(cpal::SampleRate(want_rate)).config());
            }
        }
    }
    let config = match chosen {
        Some(c) => c,
        None => {
            let d = device.default_output_config().map_err(|e| e.to_string())?;
            if d.sample_format() != cpal::SampleFormat::F32 {
                return Err(format!("unsupported device sample format {:?}", d.sample_format()));
            }
            d.config()
        }
    };
    Ok((device, config, name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reusable_meter_snapshot_updates_and_drops_stale_tracks() {
        let keep = TrackId(1);
        let stale = TrackId(2);
        let mut source = MeterSnapshot::default();
        source.tracks.insert(keep, StripMeter { peak: [0.5, 0.25], peaks: vec![0.5, 0.25], gain_reduction_db: 3.0 });
        source.main.peaks = vec![0.75, 0.5];
        let mut out = MeterSnapshot::default();
        out.tracks.insert(stale, StripMeter::default());
        copy_meter_snapshot(&source, &mut out);
        assert_eq!(out.tracks.len(), 1);
        assert_eq!(out.tracks[&keep], source.tracks[&keep]);
        assert_eq!(out.main, source.main);
    }

    #[test]
    fn audio_state_plays_and_stops_at_end() {
        let s = Arc::new(soundcraft_model::Session::default());
        let shared = Arc::new(Shared {
            position: AtomicI64::new(0),
            playing: AtomicBool::new(false),
            speed: AtomicU32::new(1000),
            meters: Mutex::new(MeterSnapshot::default()),
        });
        let (tx, rx) = channel();
        let mut st = AudioState::new(rx, Arc::clone(&shared), s, 48_000.0, channel().0);
        tx.send(Cmd::Play { from: 0, end: Some(1000), looped: None }).unwrap();
        let mut out = vec![0.0f32; 4096];
        st.fill(&mut out, 2);
        assert!(shared.position.load(Ordering::Relaxed) >= 1000);
        assert!(!shared.playing.load(Ordering::Relaxed));
    }

    #[test]
    fn short_device_callbacks_render_short_mix_blocks() {
        let s = Arc::new(soundcraft_model::Session::default());
        let shared = Arc::new(Shared {
            position: AtomicI64::new(0),
            playing: AtomicBool::new(false),
            speed: AtomicU32::new(1000),
            meters: Mutex::new(MeterSnapshot::default()),
        });
        let (tx, rx) = channel();
        let mut st = AudioState::new(rx, Arc::clone(&shared), s, 48_000.0, channel().0);
        tx.send(Cmd::Play { from: 0, end: None, looped: None }).unwrap();
        let mut out = vec![0.0f32; 2 * 64];
        st.fill(&mut out, 2);
        assert_eq!(shared.position.load(Ordering::Relaxed), 64);
    }

    #[test]
    fn surround_main_maps_to_device_channels_or_folds_down() {
        use soundcraft_model::{ChannelFormat, Clip, SourceAudio, SourceId, TrackKind};
        let mut s = soundcraft_model::Session::default();
        s.outputs[0].format = ChannelFormat::Surround51;
        let t = s.add_track(TrackKind::Audio, ChannelFormat::Surround51, None);
        let chans: Vec<Vec<f32>> = (0..6).map(|c| vec![0.1 * (c + 1) as f32; 48_000]).collect();
        s.pool.insert(SourceId(5), Arc::new(SourceAudio::new(soundcraft_audio_io::AudioBuffer { sample_rate: 48_000, channels: chans })));
        let id = s.new_clip_id();
        s.track_mut(t).unwrap().playlist_mut().unwrap().clips.push(Clip::audio(id, "x", SourceId(5), 0, 0, 48_000));
        let s = Arc::new(s);
        let shared = Arc::new(Shared {
            position: AtomicI64::new(0),
            playing: AtomicBool::new(false),
            speed: AtomicU32::new(1000),
            meters: Mutex::new(MeterSnapshot::default()),
        });
        let (tx, rx) = channel();
        let mut st = AudioState::new(rx, Arc::clone(&shared), Arc::clone(&s), 48_000.0, channel().0);
        tx.send(Cmd::Play { from: 0, end: None, looped: None }).unwrap();
        // An 8-channel device: channels in order, the rest silent.
        let mut out = vec![0.0f32; 8 * 256];
        st.fill(&mut out, 8);
        let f = &out[8 * 100..8 * 101];
        for c in 0..6 {
            assert!((f[c] - 0.1 * (c + 1) as f32).abs() < 1e-5, "{f:?}");
        }
        assert!(f[6] == 0.0 && f[7] == 0.0);
        // A stereo device: ITU fold-down.
        let mut out = vec![0.0f32; 2 * 256];
        st.fill(&mut out, 2);
        let h = std::f32::consts::FRAC_1_SQRT_2;
        let l = (0.1 + h * 0.3 + h * 0.5f32).min(1.0);
        assert!((out[200] - l).abs() < 1e-4, "{} vs {l}", out[200]);
        assert_eq!(shared.meters.lock().unwrap().main.peaks.len(), 6);
    }

    struct Stateful(Box<dyn Plugin>);

    impl Plugin for Stateful {
        fn info(&self) -> &'static soundcraft_dsp::PluginInfo {
            self.0.info()
        }
        fn prepare(&mut self, sr: f32, mb: usize, ch: usize) {
            self.0.prepare(sr, mb, ch);
        }
        fn reset(&mut self) {
            self.0.reset();
        }
        fn set_param(&mut self, id: &str, v: f32) -> bool {
            self.0.set_param(id, v)
        }
        fn param(&self, id: &str) -> Option<f32> {
            self.0.param(id)
        }
        fn process(&mut self, io: &mut [Vec<f32>], frames: usize) {
            self.0.process(io, frames);
        }
        fn save_state(&mut self) -> Option<Vec<u8>> {
            Some(b"blob".to_vec())
        }
    }

    /// Runs the audio callback on its own thread (as a device would) until stopped; returns the
    /// state back so a test can continue synchronously.
    fn run_audio(mut st: AudioState) -> (Arc<AtomicBool>, Arc<std::sync::atomic::AtomicUsize>, std::thread::JoinHandle<AudioState>) {
        let stop = Arc::new(AtomicBool::new(false));
        let blocks = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let (s2, b2) = (Arc::clone(&stop), Arc::clone(&blocks));
        let h = std::thread::spawn(move || {
            let mut out = vec![0.0f32; 256];
            while !s2.load(Ordering::Relaxed) {
                st.fill(&mut out, 2);
                b2.fetch_add(1, Ordering::Relaxed);
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            st
        });
        (stop, blocks, h)
    }

    fn stop_audio(r: (Arc<AtomicBool>, Arc<std::sync::atomic::AtomicUsize>, std::thread::JoinHandle<AudioState>)) -> AudioState {
        r.0.store(true, Ordering::Relaxed);
        r.2.join().unwrap()
    }

    const WAIT: std::time::Duration = std::time::Duration::from_secs(5);

    #[test]
    fn instances_arrive_with_the_session_and_leave_through_the_return_channel() {
        use soundcraft_model::{ChannelFormat, Insert, TrackKind};
        let mut s = soundcraft_model::Session::default();
        let t = s.add_track(TrackKind::Audio, ChannelFormat::Stereo, None);
        s.track_mut(t).unwrap().mixer.inserts[1] = Some(Insert::new("clap:test.fake"));
        let s = Arc::new(s);
        let shared = Arc::new(Shared {
            position: AtomicI64::new(0),
            playing: AtomicBool::new(false),
            speed: AtomicU32::new(1000),
            meters: Mutex::new(MeterSnapshot::default()),
        });
        let (tx, rx) = channel();
        let (gtx, grx) = channel();
        let mut st = AudioState::new(rx, Arc::clone(&shared), Arc::new(soundcraft_model::Session::default()), 48_000.0, gtx);
        let plugin: Box<dyn Plugin> = Box::new(Stateful(soundcraft_dsp::create("gain").unwrap()));
        tx.send(Cmd::Session(Arc::clone(&s), vec![PreparedInstance { track: t, slot: 1, id: "clap:test.fake".into(), ch: 2, plugin }])).unwrap();
        // Stopped: the callback only drains commands (adopting the instance); the capture runs here.
        let audio = run_audio(st);
        assert_eq!(request_capture(&tx, WAIT).unwrap(), vec![(t, 1, b"blob".to_vec())]);
        assert_eq!(request_capture(&tx, WAIT).unwrap(), vec![(t, 1, b"blob".to_vec())], "the plugin went back into its slot");
        st = stop_audio(audio);
        let mut out = vec![0.0f32; 256];
        assert!(grx.try_recv().is_err(), "nothing retired yet");
        // A session without the insert retires the instance to the return channel.
        tx.send(Cmd::Session(Arc::new(soundcraft_model::Session::default()), Vec::new())).unwrap();
        st.fill(&mut out, 2);
        assert_eq!(grx.try_recv().unwrap().len(), 1);
        // A sample-rate change rebuilds the engine without dropping plugins on this thread.
        tx.send(Cmd::Session(
            Arc::clone(&s),
            vec![PreparedInstance {
                track: t,
                slot: 1,
                id: "clap:test.fake".into(),
                ch: 2,
                plugin: Box::new(Stateful(soundcraft_dsp::create("gain").unwrap())),
            }],
        ))
        .unwrap();
        st.fill(&mut out, 2);
        let mut s44 = (*s).clone();
        s44.sample_rate = soundcraft_time::SampleRate::new(44_100).unwrap();
        tx.send(Cmd::Session(Arc::new(s44), Vec::new())).unwrap();
        st.fill(&mut out, 2);
        assert_eq!(grx.try_recv().unwrap().len(), 1);
    }

    struct Tagged(Box<dyn Plugin>, &'static [u8]);

    impl Plugin for Tagged {
        fn info(&self) -> &'static soundcraft_dsp::PluginInfo {
            self.0.info()
        }
        fn prepare(&mut self, sr: f32, mb: usize, ch: usize) {
            self.0.prepare(sr, mb, ch);
        }
        fn reset(&mut self) {
            self.0.reset();
        }
        fn set_param(&mut self, id: &str, v: f32) -> bool {
            self.0.set_param(id, v)
        }
        fn param(&self, id: &str) -> Option<f32> {
            self.0.param(id)
        }
        fn process(&mut self, io: &mut [Vec<f32>], frames: usize) {
            self.0.process(io, frames);
        }
        fn save_state(&mut self) -> Option<Vec<u8>> {
            Some(self.1.to_vec())
        }
    }

    #[test]
    fn a_replaced_session_does_not_keep_the_previous_sessions_plugin_state() {
        use soundcraft_model::{ChannelFormat, Insert, TrackKind};
        let mut s = soundcraft_model::Session::default();
        let t = s.add_track(TrackKind::Audio, ChannelFormat::Stereo, None);
        s.track_mut(t).unwrap().mixer.inserts[1] = Some(Insert::new("clap:test.fake"));
        let s = Arc::new(s);
        let shared = Arc::new(Shared {
            position: AtomicI64::new(0),
            playing: AtomicBool::new(false),
            speed: AtomicU32::new(1000),
            meters: Mutex::new(MeterSnapshot::default()),
        });
        let (tx, rx) = channel();
        let (gtx, grx) = channel();
        let st = AudioState::new(rx, Arc::clone(&shared), Arc::new(soundcraft_model::Session::default()), 48_000.0, gtx);
        let inst = |tag: &'static [u8]| PreparedInstance {
            track: t,
            slot: 1,
            id: "clap:test.fake".into(),
            ch: 2,
            plugin: Box::new(Tagged(soundcraft_dsp::create("gain").unwrap(), tag)),
        };
        tx.send(Cmd::Session(Arc::clone(&s), vec![inst(b"SESSION A")])).unwrap();
        let audio = run_audio(st);
        assert_eq!(request_capture(&tx, WAIT).unwrap(), vec![(t, 1, b"SESSION A".to_vec())]);
        // Replacing the document resets the engine; the new session's instance is the one that lives.
        tx.send(Cmd::ResetSession).unwrap();
        tx.send(Cmd::Session(Arc::clone(&s), vec![inst(b"SESSION B")])).unwrap();
        assert_eq!(request_capture(&tx, WAIT).unwrap(), vec![(t, 1, b"SESSION B".to_vec())]);
        let _st = stop_audio(audio);
        assert_eq!(grx.try_recv().unwrap().len(), 1, "the old instance goes back to the UI thread to be dropped");
    }

    #[test]
    fn side_recreates_every_instance_after_a_session_replacement() {
        use soundcraft_model::{ChannelFormat, Insert, TrackKind};
        let (_gtx, grx) = channel();
        let mut side = Side { shadow: HashMap::new(), rate: 48_000.0, editors: HashMap::new(), garbage: grx };
        let mut s = soundcraft_model::Session::default();
        let t = s.add_track(TrackKind::Audio, ChannelFormat::Mono, None);
        s.track_mut(t).unwrap().mixer.inserts[0] = Some(Insert::new("clap:does.not.exist"));
        side.prepare(&s);
        assert_eq!(side.shadow.len(), 1);
        side.forget_all();
        assert!(side.shadow.is_empty(), "nothing is remembered across a replacement");
    }

    struct Slow(Box<dyn Plugin>);

    impl Plugin for Slow {
        fn info(&self) -> &'static soundcraft_dsp::PluginInfo {
            self.0.info()
        }
        fn prepare(&mut self, sr: f32, mb: usize, ch: usize) {
            self.0.prepare(sr, mb, ch);
        }
        fn reset(&mut self) {
            self.0.reset();
        }
        fn set_param(&mut self, id: &str, v: f32) -> bool {
            self.0.set_param(id, v)
        }
        fn param(&self, id: &str) -> Option<f32> {
            self.0.param(id)
        }
        fn process(&mut self, io: &mut [Vec<f32>], frames: usize) {
            self.0.process(io, frames);
        }
        fn save_state(&mut self) -> Option<Vec<u8>> {
            std::thread::sleep(std::time::Duration::from_millis(400));
            Some(b"late".to_vec())
        }
    }

    fn slow_session() -> (Arc<soundcraft_model::Session>, TrackId) {
        use soundcraft_model::{ChannelFormat, Insert, TrackKind};
        let mut s = soundcraft_model::Session::default();
        let t = s.add_track(TrackKind::Audio, ChannelFormat::Stereo, None);
        s.track_mut(t).unwrap().mixer.inserts[1] = Some(Insert::new("clap:test.slow"));
        (Arc::new(s), t)
    }

    fn audio_state(
        session: Arc<soundcraft_model::Session>,
        tx_session: &Sender<Cmd>,
        t: TrackId,
        rx: Receiver<Cmd>,
    ) -> (AudioState, Receiver<Vec<Box<dyn Plugin>>>) {
        let shared = Arc::new(Shared {
            position: AtomicI64::new(0),
            playing: AtomicBool::new(false),
            speed: AtomicU32::new(1000),
            meters: Mutex::new(MeterSnapshot::default()),
        });
        let (gtx, grx) = channel();
        let st = AudioState::new(rx, shared, Arc::new(soundcraft_model::Session::default()), 48_000.0, gtx);
        let plugin: Box<dyn Plugin> = Box::new(Slow(soundcraft_dsp::create("gain").unwrap()));
        tx_session.send(Cmd::Session(session, vec![PreparedInstance { track: t, slot: 1, id: "clap:test.slow".into(), ch: 2, plugin }])).unwrap();
        (st, grx)
    }

    #[test]
    fn a_slow_serializer_never_blocks_the_audio_thread() {
        let (s, t) = slow_session();
        let (tx, rx) = channel();
        let (st, _grx) = audio_state(s, &tx, t, rx);
        let audio = run_audio(st);
        std::thread::sleep(std::time::Duration::from_millis(50));
        let before = audio.1.load(Ordering::Relaxed);
        let started = std::time::Instant::now();
        assert_eq!(request_capture(&tx, WAIT).unwrap(), vec![(t, 1, b"late".to_vec())]);
        let taken = started.elapsed();
        let during = audio.1.load(Ordering::Relaxed) - before;
        assert!(taken >= std::time::Duration::from_millis(400), "the plugin really was slow: {taken:?}");
        assert!(during >= 20, "the audio thread kept rendering while the plugin serialised: {during} blocks in {taken:?}");
        // The plugin is back in its slot afterwards.
        assert_eq!(request_capture(&tx, WAIT).unwrap(), vec![(t, 1, b"late".to_vec())]);
        let _ = stop_audio(audio);
    }

    #[test]
    fn a_stalled_audio_thread_is_a_timeout_and_loses_no_plugin() {
        let (s, t) = slow_session();
        let (tx, rx) = channel();
        let (st, _grx) = audio_state(s, &tx, t, rx);
        // Nothing is draining commands yet: the capture times out rather than answering "empty".
        assert_eq!(request_capture(&tx, std::time::Duration::from_millis(50)), Err(CaptureError::Timeout));
        // When the audio thread wakes it finds the abandoned request and keeps its plugin.
        let audio = run_audio(st);
        assert_eq!(request_capture(&tx, WAIT).unwrap(), vec![(t, 1, b"late".to_vec())]);
        let _ = stop_audio(audio);
        // No audio thread at all is "unavailable", not an empty list.
        let (dead_tx, dead_rx) = channel::<Cmd>();
        drop(dead_rx);
        assert_eq!(request_capture(&dead_tx, std::time::Duration::from_millis(10)), Err(CaptureError::Unavailable));
    }

    #[test]
    fn plugins_returning_after_the_document_was_replaced_are_dropped_not_restored() {
        let (s, t) = slow_session();
        let (tx, rx) = channel();
        let (st, grx) = audio_state(s, &tx, t, rx);
        let audio = run_audio(st);
        let (rtx, rrx) = channel();
        tx.send(Cmd::Checkout(rtx)).unwrap();
        let (generation, taken) = rrx.recv_timeout(WAIT).unwrap();
        assert_eq!(taken.len(), 1);
        // The document is replaced while the plugin is away; the old plugin must not come back.
        tx.send(Cmd::ResetSession).unwrap();
        tx.send(Cmd::Return(generation, taken)).unwrap();
        let mut dropped = 0;
        while dropped == 0 {
            dropped += grx.recv_timeout(WAIT).unwrap().len();
        }
        assert_eq!(dropped, 1, "the stale instance went to the return channel to be dropped off-thread");
        assert_eq!(request_capture(&tx, WAIT).unwrap(), vec![], "nothing was restored into the new engine");
        let _ = stop_audio(audio);
    }

    #[test]
    fn side_creates_each_missing_instance_once() {
        use soundcraft_model::{ChannelFormat, Insert, TrackKind};
        let (_gtx, grx) = channel();
        let mut side = Side { shadow: HashMap::new(), rate: 48_000.0, editors: HashMap::new(), garbage: grx };
        let mut s = soundcraft_model::Session::default();
        let t = s.add_track(TrackKind::Audio, ChannelFormat::Mono, None);
        s.track_mut(t).unwrap().mixer.inserts[0] = Some(Insert::new("clap:does.not.exist"));
        s.track_mut(t).unwrap().mixer.inserts[1] = Some(Insert::new("gain"));
        assert!(side.prepare(&s).is_empty(), "the plugin is not installed; built-ins stay with the engine");
        assert_eq!(side.shadow.get(&(t, 0)), Some(&("clap:does.not.exist".to_string(), 1, false)));
        assert!(side.prepare(&s).is_empty());
        assert_eq!(side.shadow.len(), 1, "failures are remembered, not retried every update");
        s.track_mut(t).unwrap().mixer.inserts[0] = None;
        side.prepare(&s);
        assert!(side.shadow.is_empty());
    }

    #[test]
    fn looping_wraps() {
        let s = Arc::new(soundcraft_model::Session::default());
        let shared = Arc::new(Shared {
            position: AtomicI64::new(0),
            playing: AtomicBool::new(false),
            speed: AtomicU32::new(1000),
            meters: Mutex::new(MeterSnapshot::default()),
        });
        let (tx, rx) = channel();
        let mut st = AudioState::new(rx, Arc::clone(&shared), s, 48_000.0, channel().0);
        tx.send(Cmd::Play { from: 0, end: None, looped: Some(Range::new(0, 1000)) }).unwrap();
        let mut out = vec![0.0f32; 20_000];
        st.fill(&mut out, 2);
        let p = shared.position.load(Ordering::Relaxed);
        assert!(p <= 1000, "position {p} should stay inside the loop");
        assert!(shared.playing.load(Ordering::Relaxed));
    }

    #[test]
    fn armed_tracks_are_heard_and_metered_after_playback_stops() {
        use soundcraft_model::{ChannelFormat, TrackKind};
        let mut s = soundcraft_model::Session::default();
        let t = s.add_track(TrackKind::Audio, ChannelFormat::Mono, None);
        s.track_mut(t).unwrap().mixer.record_arm = true;
        let shared = Arc::new(Shared {
            position: AtomicI64::new(0),
            playing: AtomicBool::new(false),
            speed: AtomicU32::new(1000),
            meters: Mutex::new(MeterSnapshot::default()),
        });
        let (tx, rx) = channel();
        let mut st = AudioState::new(rx, Arc::clone(&shared), Arc::new(s), 48_000.0, channel().0);
        let ring = record::InputRing::new(1, 48_000);
        tx.send(Cmd::Input(Arc::clone(&ring))).unwrap();
        tx.send(Cmd::Play { from: 0, end: Some(1000), looped: None }).unwrap();
        let mut out = vec![0.0f32; 4096];
        st.fill(&mut out, 2);
        assert!(!shared.playing.load(Ordering::Relaxed));
        ring.push(&[0.5; 4096]);
        st.fill(&mut out, 2);
        assert!(out.iter().any(|x| x.abs() > 0.1), "the input is heard");
        assert!(shared.meters.lock().unwrap().tracks.get(&t).is_some_and(|m| m.peak[0] > 0.1), "and metered");
    }
}
