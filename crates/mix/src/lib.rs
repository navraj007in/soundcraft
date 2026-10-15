//! The SoundCraft mix engine.
//!
//! [`MixEngine::render`] produces one block of the main output for a session at a timeline
//! position: clips (with clip gain and fades) → trim/phase → inserts → pre-fader sends → fader and
//! mute (with automation) → pan → post-fader sends → busses → aux inputs → main → master faders.
//! The same code drives realtime playback and offline bounces, so what you hear is what you get.
//! Steady-state rendering does not allocate.
//!
//! ## Surround
//! The main mix has the format of the session's main output path ([`Session::main_format`]:
//! stereo by default, up to 16 channels such as 7.1.4 or 9.1.6) and every bus has its own format.
//! Channels are in SMPTE / WAV order (L R C LFE Ls Rs …, see
//! [`ChannelFormat::speakers`](soundcraft_model::ChannelFormat::speakers)). Routing a strip into a
//! destination builds a gain matrix per block:
//! - mono/stereo sources into a stereo destination use the stereo pan exactly as before;
//! - into a multichannel destination they use the track's [`SurroundPan`] (pairwise
//!   constant-power panner over the speaker layout, see `soundcraft_dsp::pan::surround_gains`),
//!   or go to L/R with the stereo pan when the track has no surround pan;
//! - sources whose format equals the destination's pass straight through, channel for channel;
//! - other multichannel sources fold to stereo with ITU-R BS.775 coefficients (C and surrounds at
//!   -3 dB, LFE dropped), or map speaker by speaker into other multichannel layouts.
//!
//! ## Plugin instances and threads
//! [`MixEngine::sync`] brings plugin instances in line with the session. Built-in plugins are
//! cheap (allocation only) and are always created there, on whichever thread renders. Hosted
//! third-party plugins (`clap:` / `vst3:` ids, see [`is_third_party`]) are expensive and have
//! main-thread rules, so a realtime host keeps them off the audio thread:
//! - with [`MixEngine::set_external_instances`]`(true)` (realtime playback), `sync` never creates a
//!   third-party plugin. The host's control (UI) thread creates them with [`create_instance`] from
//!   [`instance_specs`] (restoring each insert's stored [`Insert::state`] and calling `prepare`
//!   there), and hands them over with [`MixEngine::adopt`]; `sync` then moves the matching one
//!   into its slot. A slot whose instance has not arrived yet passes audio through.
//! - every instance `sync` replaces or removes (built-in or not) goes to a retired list instead of
//!   being dropped; the host takes it with [`MixEngine::take_retired`] and drops it on the control
//!   thread, so plugin destruction never runs on the audio thread either.
//! - without external instances (the default: offline [`render_range`], bounces, tests) `sync`
//!   creates third-party plugins itself, synchronously, restores their stored state before the
//!   first `process`, and drops retired instances at the end of `sync`.
//!
//! An instance starts from its stored state; the insert's parameter values are then applied on
//! top before the first block (the session's values are kept in step with the plugin, including
//! edits made in its own editor, so both agree; the state carries everything else). A host reads
//! states back for saving through each instance's [`Plugin::state_reader`], on its UI thread.
//!
//! A built-in instrument's parameter values are applied the same way. A sample-playing
//! instrument (the Sampler) is handed the decoded audio of its insert's
//! [`Insert::sample`] source straight from the session's pool, shared and never copied.
//!
//! [`Insert::state`]: soundcraft_model::Insert::state
//! [`Insert::sample`]: soundcraft_model::Insert::sample
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use soundcraft_dsp::pan::{PanLaw, SpeakerPos, SurroundParams, gains};
use soundcraft_dsp::{Plugin, SampleSource, Transport, db_to_gain};
use soundcraft_midi::CtrlKind;
use soundcraft_model::{
    AutoParam, BusId, ChannelFormat, Clip, ClipContent, Route, Session, SourceAudio, Speaker, SurroundPan, Track, TrackId, TrackKind,
};
use soundcraft_time::{Range, Samples, TICKS_PER_QUARTER};
use std::collections::HashMap;
use std::ops::Bound;
use std::sync::Arc;

/// Default (stereo) main mix width. The actual width is [`main_channels`].
pub const MAIN_CHANNELS: usize = 2;

/// Widest strip, bus or main mix (9.1.6 / 3rd-order Ambisonics). Wider formats are truncated.
pub const MAX_CHANNELS: usize = 16;

/// Number of channels of a session's main mix.
pub fn main_channels(s: &Session) -> usize {
    s.main_format().channels().clamp(MAIN_CHANNELS, MAX_CHANNELS)
}

/// Slot index used for a track's instrument plugin in [`InstanceSpec`] and [`PreparedInstance`]
/// (insert slots use their index).
pub const INSTRUMENT_SLOT: usize = usize::MAX;

/// Hosted third-party plugin ids (CLAP or VST3): created off the audio thread in realtime use.
pub fn is_third_party(id: &str) -> bool {
    id.starts_with(soundcraft_clap_host::ID_PREFIX)
        || id.starts_with(soundcraft_vst3_host::ID_PREFIX)
        || id.starts_with(soundcraft_au_host::ID_PREFIX)
}

/// A third-party plugin instance a session needs: which slot, which plugin, how many channels,
/// and the state to restore.
#[derive(Debug, Clone, PartialEq)]
pub struct InstanceSpec {
    pub track: TrackId,
    /// Insert slot index, or [`INSTRUMENT_SLOT`].
    pub slot: usize,
    pub id: String,
    /// Channel count the instance is prepared for.
    pub ch: usize,
    /// Decoded [`Insert::state`](soundcraft_model::Insert::state).
    pub state: Option<Vec<u8>>,
}

/// A plugin instance created and prepared off the audio thread, handed to [`MixEngine::adopt`].
pub struct PreparedInstance {
    pub track: TrackId,
    pub slot: usize,
    pub id: String,
    pub ch: usize,
    pub plugin: Box<dyn Plugin>,
}

/// Every third-party plugin slot (inserts and instruments) of a session, with the channel count
/// [`MixEngine::sync`] prepares it for. Inactive and master tracks are included (they have strips).
pub fn instance_specs(s: &Session) -> Vec<InstanceSpec> {
    let main_ch = main_channels(s);
    let mut out = Vec::new();
    for t in &s.tracks {
        let ch = strip_channels(t, main_ch);
        for (i, ins) in t.mixer.inserts.iter().enumerate() {
            if let Some(ins) = ins
                && is_third_party(&ins.plugin)
            {
                out.push(InstanceSpec { track: t.id, slot: i, id: ins.plugin.clone(), ch, state: ins.state_bytes() });
            }
        }
        if let Some(ins) = &t.instrument
            && is_third_party(&ins.plugin)
        {
            out.push(InstanceSpec { track: t.id, slot: INSTRUMENT_SLOT, id: ins.plugin.clone(), ch: ch.max(2), state: ins.state_bytes() });
        }
    }
    out
}

/// A plugin made by [`create_instance`].
pub struct NewInstance {
    pub plugin: Box<dyn Plugin>,
    /// The plugin refused the stored state and starts from its defaults.
    pub refused_state: bool,
}

/// Creates a plugin (built-in or hosted), restores `state` (if any) and prepares it. Expensive
/// for third-party plugins: in realtime use, call it on the control thread.
pub fn create_instance(id: &str, state: Option<&[u8]>, sample_rate: f32, max_block: usize, ch: usize) -> Option<NewInstance> {
    let mut plugin = create_plugin(id)?;
    let refused_state = state.is_some_and(|st| !plugin.load_state(st));
    if refused_state {
        log::warn!("{id}: the plugin did not accept its stored state");
    }
    plugin.prepare(sample_rate, max_block, ch);
    Some(NewInstance { plugin, refused_state })
}

/// Peak meter values for one strip (linear, per channel, max over the last block).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StripMeter {
    /// First two channels (mono strips repeat channel 0), for stereo meters.
    pub peak: [f32; 2],
    /// Every channel of the strip (or of the main mix).
    pub peaks: Vec<f32>,
    /// Gain reduction of the first dynamics insert, dB.
    pub gain_reduction_db: f32,
    pub plugin_failures: PluginFailures,
}

/// How often a strip's plugins were taken out of the signal path for putting out NaN or infinity.
/// Each stays out until the engine is next reset (playback starts or stops).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PluginFailures {
    /// It only grows.
    pub count: u32,
    /// The slot of the latest: an insert index or [`INSTRUMENT_SLOT`].
    pub last: usize,
}

impl PluginFailures {
    fn record(&mut self, slot: usize) {
        self.count = self.count.wrapping_add(1);
        self.last = slot;
    }
}

impl StripMeter {
    /// Copy another meter without reallocating once the channel count is stable.
    pub fn copy_from(&mut self, o: &StripMeter) {
        self.peak = o.peak;
        self.peaks.clone_from(&o.peaks);
        self.gain_reduction_db = o.gain_reduction_db;
        self.plugin_failures = o.plugin_failures;
    }

    fn measure(&mut self, bufs: &[Vec<f32>], frames: usize, gr: f32) {
        if self.peaks.len() != bufs.len() {
            self.peaks.resize(bufs.len(), 0.0);
        }
        for (p, b) in self.peaks.iter_mut().zip(bufs.iter()) {
            *p = peak(b.get(..frames).unwrap_or(&[]));
        }
        let last = self.peaks.len().saturating_sub(1);
        self.peak = [self.peaks.first().copied().unwrap_or(0.0), self.peaks.get(1.min(last)).copied().unwrap_or(0.0)];
        self.gain_reduction_db = gr;
    }
}

/// A bus: its format and one buffer per channel.
struct BusBuf {
    fmt: ChannelFormat,
    buf: Vec<Vec<f32>>,
}

struct PluginSlot {
    id: String,
    /// Channel count the plugin was prepared for.
    ch: usize,
    plugin: Box<dyn Plugin>,
    /// Last values pushed to the plugin, in the insert's parameter-map order.
    applied: Vec<f32>,
    failed: bool,
}

struct Strip {
    buf: Vec<Vec<f32>>,
    /// Pre-fader copy for pre-fader sends.
    pre: Vec<Vec<f32>>,
    scratch: Vec<f32>,
    plugins: Vec<Option<PluginSlot>>,
    instrument: Option<PluginSlot>,
    /// The pool audio handed to the instrument with `set_sample` (sample-playing instruments).
    sample: Option<Arc<SourceAudio>>,
    /// MIDI notes currently sounding on the instrument (pitch), for note-offs at stops/seek.
    /// Notes sounding on the instrument: pitch and the sample their note-off is due at. Kept apart
    /// from clips so a note-off still arrives after its clip stops overlapping the block.
    held: Vec<(u8, u8, Samples)>,
    /// MIDI this strip sends to external gear this block (a MIDI track with a hardware output and
    /// no hosted instrument), and the track that owns them once it has done so.
    midi_out: Vec<MidiOut>,
    external: Option<TrackId>,
    /// The instrument's next block starts somewhere new: send it the controllers in effect there.
    chase: bool,
    muted: bool,
    gr: f32,
    /// Per-clip effect chains (Clip Effects) and a scratch buffer to render clips into.
    clip_fx: HashMap<u64, ClipFx>,
    clipbuf: Vec<Vec<f32>>,
    /// Total latency of the active inserts (samples).
    latency: usize,
    /// Delay compensation: align this strip with the slowest one (post- and pre-fader paths),
    /// plus extra delay for direct-to-main outputs so they line up with aux returns.
    align: DelayLine,
    align_pre: DelayLine,
    out_extra: DelayLine,
    /// Per-send alignment to the destination bus, and the scratch a delayed send is read from.
    send_extra: Vec<DelayLine>,
    sendbuf: Vec<Vec<f32>>,
    outbuf: Vec<Vec<f32>>,
    failures: PluginFailures,
}

/// Most latency delay compensation handles: 2^20 samples, about 22 s at 48 kHz. A strip reports
/// at most this, whatever its plugins claim, so a bogus value can't stall offline renders.
const MAX_LATENCY: usize = 1 << 20;

/// A multichannel delay line (samples), resized only when the delay changes.
#[derive(Default)]
struct DelayLine {
    bufs: Vec<Vec<f32>>,
    idx: usize,
    len: usize,
}

impl DelayLine {
    fn set(&mut self, len: usize, ch: usize) {
        let len = len.min(MAX_LATENCY);
        if len != self.len || self.bufs.len() != ch {
            self.len = len;
            self.idx = 0;
            self.bufs = vec![vec![0.0; len]; ch];
        }
    }
    /// Forget the audio in flight (a stop, seek or session change) without reallocating.
    fn clear(&mut self) {
        self.bufs.iter_mut().for_each(|b| b.fill(0.0));
        self.idx = 0;
    }
    fn process(&mut self, io: &mut [Vec<f32>], frames: usize) {
        if self.len == 0 {
            return;
        }
        let start = self.idx;
        for (c, ch) in io.iter_mut().enumerate() {
            let Some(ring) = self.bufs.get_mut(c) else { continue };
            let mut i = start;
            for x in ch.iter_mut().take(frames) {
                if let Some(r) = ring.get_mut(i) {
                    std::mem::swap(r, x);
                }
                i += 1;
                if i >= self.len {
                    i = 0;
                }
            }
        }
        self.idx = (start + frames) % self.len.max(1);
    }
}

/// Clip Effects: an EQ and a compressor per clip, configured from `edit.values`
/// (`clip_fx.<clip>.eq.<param>`, `clip_fx.<clip>.comp.<param>`, `clip_fx.<clip>.gain`).
struct ClipFx {
    eq: Option<Box<dyn Plugin>>,
    comp: Option<Box<dyn Plugin>>,
    gain_db: f32,
    sr: f32,
    ch: usize,
}

impl Strip {
    /// Drop compensation audio so nothing from before a discontinuity plays after it.
    fn clear_delays(&mut self) {
        self.align.clear();
        self.align_pre.clear();
        self.out_extra.clear();
        self.send_extra.iter_mut().for_each(DelayLine::clear);
        self.outbuf.iter_mut().for_each(|b| b.fill(0.0));
    }

    fn new() -> Self {
        Strip {
            buf: Vec::new(),
            pre: Vec::new(),
            scratch: Vec::new(),
            plugins: Vec::new(),
            instrument: None,
            sample: None,
            held: Vec::with_capacity(MAX_HELD),
            midi_out: Vec::new(),
            external: None,
            chase: true,
            muted: false,
            gr: 0.0,
            clip_fx: HashMap::new(),
            clipbuf: Vec::new(),
            latency: 0,
            align: DelayLine::default(),
            align_pre: DelayLine::default(),
            out_extra: DelayLine::default(),
            send_extra: Vec::new(),
            sendbuf: Vec::new(),
            outbuf: Vec::new(),
            failures: PluginFailures::default(),
        }
    }
}

/// Renders sessions. Keep one per playback stream; plugin state persists across blocks.
pub struct MixEngine {
    sample_rate: f32,
    max_block: usize,
    strips: HashMap<TrackId, Strip>,
    busses: HashMap<BusId, BusBuf>,
    main: Vec<Vec<f32>>,
    main_fmt: ChannelFormat,
    pub meters: HashMap<TrackId, StripMeter>,
    pub main_meter: StripMeter,
    pub pan_law: PanLaw,
    /// Last rendered position end (to detect seeks).
    last_end: Samples,
    /// Snapshot key of the session the strips/order were synced to.
    synced: (usize, usize, usize),
    order: Vec<usize>,
    /// Current total compensation delay (samples) of the main mix: the slowest track and aux
    /// returns, and the master faders' inserts.
    pub latency: usize,
    /// Delay already applied to audio written onto busses: the slowest track insert.
    /// A bus render drops this so the bus lines up with the timeline. The extra delay
    /// that lines direct outputs up with aux returns stays in `latency` and is removed
    /// only from the main mix.
    track_latency: usize,
    /// Per-bus arrival latency (samples) of the audio on it: the slowest feeder, so every route
    /// into a bus (and into the main mix) is delayed to meet it.
    bus_latency: HashMap<BusId, usize>,
    /// Post-fader output latency of every live strip this block, before route alignment.
    out_latency: HashMap<TrackId, usize>,
    /// Live input for the next block (planar), set by the audio host for input monitoring.
    pub input: Vec<Vec<f32>>,
    /// Transport stopped: only monitored inputs sound (no clips).
    pub monitor_only: bool,
    /// Recording: record-armed tracks hear their input (auto input).
    pub recording: bool,
    /// MIDI messages that arrived from devices since the last block (status, data 1, data 2),
    /// set by the audio host before each `render`. They play the instruments of record-armed or
    /// input-monitored tracks; the host clears them afterwards.
    pub live_midi: Vec<[u8; 3]>,
    /// MIDI that MIDI tracks routed to a hardware port produced this block, in track order, for
    /// the audio host to send; the host clears it after reading. Also filled by [`MixEngine::reset`]
    /// (note-offs for what was sounding).
    pub midi_out: Vec<MidiOut>,
    /// Mix the metronome (Options > Click) into the output. Playback turns this on; offline renders
    /// (bounces, stems, exports) leave it off so the click is never printed into a file.
    pub metronome: bool,
    /// Third-party instances come from [`MixEngine::adopt`] instead of being created in `sync`.
    external: bool,
    /// Instances handed over by the host, waiting for `sync` to place them.
    adopt: HashMap<(TrackId, usize), PreparedInstance>,
    /// Instances `sync` took out of service, for the host to drop (see [`MixEngine::take_retired`]).
    retired: Vec<Box<dyn Plugin>>,
    /// Realtime serial/pool decision (see [`ParallelPolicy`]).
    parallel: ParallelPolicy,
    /// Tracks implicitly soloed this block (reused buffer).
    implicit_solo: Vec<TrackId>,
    /// Ownership workspace reused for parallel strip processing.
    work: Vec<(usize, Strip)>,
}

/// What `sync` needs to fill a slot.
struct SlotCtx<'a> {
    sr: f32,
    mb: usize,
    external: bool,
    adopt: &'a mut HashMap<(TrackId, usize), PreparedInstance>,
    retired: &'a mut Vec<Box<dyn Plugin>>,
}

impl SlotCtx<'_> {
    fn retire(&mut self, slot: Option<PluginSlot>) {
        if let Some(s) = slot {
            self.retired.push(s.plugin);
        }
    }

    /// True when the host handed over an instance for this slot. It replaces whatever the slot
    /// holds, even an instance of the same plugin: the host made it because it no longer tracks
    /// the old one (it was removed and re-added between two renders), and drives its editor.
    fn waiting(&self, t: TrackId, idx: usize) -> bool {
        self.external && self.adopt.contains_key(&(t, idx))
    }

    /// A new instance for `ins` on track `t`, slot `idx`, prepared for `ch` channels.
    fn make(&mut self, t: TrackId, idx: usize, ins: &soundcraft_model::Insert, ch: usize) -> Option<PluginSlot> {
        let third = is_third_party(&ins.plugin);
        let plugin = if third && self.external {
            match self.adopt.remove(&(t, idx)) {
                Some(p) if p.id == ins.plugin && p.ch == ch => Some(p.plugin),
                Some(p) => {
                    self.retired.push(p.plugin);
                    None
                }
                None => None,
            }
        } else {
            let state = if third { ins.state_bytes() } else { None };
            create_instance(&ins.plugin, state.as_deref(), self.sr, self.mb, ch).map(|n| n.plugin)
        }?;
        Some(PluginSlot { id: ins.plugin.clone(), ch, plugin, applied: Vec::new(), failed: false })
    }
}

impl MixEngine {
    pub fn new(sample_rate: f32, max_block: usize) -> Self {
        let max_block = max_block.clamp(16, 16_384);
        MixEngine {
            sample_rate: if sample_rate.is_finite() && sample_rate > 0.0 { sample_rate } else { 48_000.0 },
            max_block,
            strips: HashMap::new(),
            busses: HashMap::new(),
            main: vec![vec![0.0; max_block]; MAIN_CHANNELS],
            main_fmt: ChannelFormat::Stereo,
            meters: HashMap::new(),
            main_meter: StripMeter::default(),
            pan_law: PanLaw::Minus3,
            last_end: 0,
            synced: (0, 0, 0),
            order: Vec::new(),
            latency: 0,
            track_latency: 0,
            implicit_solo: Vec::new(),
            parallel: ParallelPolicy::new(),
            bus_latency: HashMap::new(),
            out_latency: HashMap::new(),
            input: Vec::new(),
            monitor_only: false,
            recording: false,
            live_midi: Vec::with_capacity(LIVE_MIDI_CAPACITY),
            midi_out: Vec::with_capacity(MIDI_OUT_CAPACITY),
            metronome: false,
            external: false,
            adopt: HashMap::new(),
            retired: Vec::new(),
            work: Vec::new(),
        }
    }

    /// Realtime mode: third-party plugins are created by the host and handed over with
    /// [`MixEngine::adopt`] instead of being created in `sync` (see the crate docs).
    pub fn set_external_instances(&mut self, on: bool) {
        self.external = on;
    }

    /// Takes instances created off the audio thread; the next render's `sync` places them.
    /// An earlier instance waiting for the same slot is retired.
    pub fn adopt(&mut self, instances: Vec<PreparedInstance>) {
        for p in instances {
            if let Some(old) = self.adopt.insert((p.track, p.slot), p) {
                self.retired.push(old.plugin);
            }
        }
        // Force a re-sync so waiting slots pick their instance up.
        self.synced = (usize::MAX, usize::MAX, usize::MAX);
    }

    /// Instances taken out of service since the last call (drop them off the audio thread).
    /// Does not allocate.
    pub fn take_retired(&mut self) -> Vec<Box<dyn Plugin>> {
        std::mem::take(&mut self.retired)
    }

    /// Every instance this engine holds (strips, instruments, clip effects excluded) plus the
    /// retired and waiting ones, leaving the engine empty: hand them to another thread to drop
    /// before replacing the engine.
    pub fn drain_plugins(&mut self) -> Vec<Box<dyn Plugin>> {
        let mut out = std::mem::take(&mut self.retired);
        for st in self.strips.values_mut() {
            out.extend(st.plugins.iter_mut().filter_map(Option::take).map(|s| s.plugin));
            out.extend(st.instrument.take().map(|s| s.plugin));
        }
        out.extend(self.adopt.drain().map(|(_, p)| p.plugin));
        self.synced = (usize::MAX, usize::MAX, usize::MAX);
        out
    }

    /// The state of every live plugin that has one (third-party plugins), as
    /// `(track, slot or INSTRUMENT_SLOT, state)`. Calls into the plugins and allocates.
    pub fn capture_states(&mut self) -> Vec<(TrackId, usize, Vec<u8>)> {
        let mut out = Vec::new();
        for (tid, st) in self.strips.iter_mut() {
            for (i, slot) in st.plugins.iter_mut().enumerate() {
                if let Some(data) = slot.as_mut().and_then(|s| s.plugin.save_state()) {
                    out.push((*tid, i, data));
                }
            }
            if let Some(data) = st.instrument.as_mut().and_then(|s| s.plugin.save_state()) {
                out.push((*tid, INSTRUMENT_SLOT, data));
            }
        }
        out.sort_by_key(|(t, s, _)| (*t, *s));
        out
    }

    pub fn max_block(&self) -> usize {
        self.max_block
    }

    /// Format of the main mix as of the last sync.
    pub fn main_format(&self) -> ChannelFormat {
        self.main_fmt
    }

    /// Channels of the main mix as of the last sync.
    pub fn main_channels(&self) -> usize {
        self.main.len()
    }

    /// Reset all plugin state (on stop / seek).
    pub fn reset(&mut self) {
        for s in self.strips.values_mut() {
            for p in s.plugins.iter_mut().flatten() {
                p.plugin.reset();
                p.failed = false;
            }
            if let Some(i) = &mut s.instrument {
                i.plugin.all_notes_off();
                i.plugin.reset();
                i.failed = false;
            }
            match s.external {
                Some(track) => release_held(&mut s.held, &mut self.midi_out, track, 0),
                None => s.held.clear(),
            }
            s.clear_delays();
            s.chase = true;
        }
        self.meters.clear();
        self.main_meter = StripMeter::default();
    }

    /// Bring plugin instances in line with the session (creates/destroys as needed). Allocates;
    /// call outside the realtime thread when the document changes, or let `render` do it.
    pub fn sync(&mut self, s: &Session) {
        let mb = self.max_block;
        let main_ch = main_channels(s);
        self.main_fmt = s.main_format();
        if self.main.len() != main_ch {
            self.main = vec![vec![0.0; mb]; main_ch];
        }
        let mut cx = SlotCtx { sr: self.sample_rate, mb, external: self.external, adopt: &mut self.adopt, retired: &mut self.retired };
        let gone: Vec<TrackId> = self.strips.keys().filter(|id| s.track(**id).is_none()).copied().collect();
        for id in gone {
            if let Some(mut st) = self.strips.remove(&id) {
                for p in st.plugins.drain(..) {
                    cx.retire(p);
                }
                cx.retire(st.instrument.take());
            }
        }
        for t in &s.tracks {
            let ch = strip_channels(t, main_ch);
            let strip = self.strips.entry(t.id).or_insert_with(Strip::new);
            if strip.buf.len() != ch || strip.buf.first().map_or(0, Vec::len) != mb {
                strip.buf = vec![vec![0.0; mb]; ch];
            }
            while strip.plugins.len() > t.mixer.inserts.len() {
                let p = strip.plugins.pop().flatten();
                cx.retire(p);
            }
            strip.plugins.resize_with(t.mixer.inserts.len(), || None);
            for (idx, (slot, ins)) in strip.plugins.iter_mut().zip(t.mixer.inserts.iter()).enumerate() {
                match ins {
                    Some(i) if slot.as_ref().is_none_or(|p| p.id != i.plugin || p.ch != ch) || cx.waiting(t.id, idx) => {
                        let old = slot.take();
                        cx.retire(old);
                        *slot = cx.make(t.id, idx, i, ch);
                    }
                    None => {
                        let old = slot.take();
                        cx.retire(old);
                    }
                    _ => {}
                }
            }
            let want = t.instrument.as_ref();
            let ich = ch.max(2);
            if strip.instrument.as_ref().map(|p| (p.id.as_str(), p.ch)) != want.map(|i| (i.plugin.as_str(), ich))
                || (want.is_some() && cx.waiting(t.id, INSTRUMENT_SLOT))
            {
                let old = strip.instrument.take();
                cx.retire(old);
                strip.instrument = want.and_then(|i| cx.make(t.id, INSTRUMENT_SLOT, i, ich));
                strip.sample = None;
            }
            sync_sample(strip, want, s);
        }
        // Instances nobody claimed (the session changed again before they arrived).
        let leftovers: Vec<(TrackId, usize)> = cx.adopt.keys().copied().collect();
        for k in leftovers {
            if let Some(p) = cx.adopt.remove(&k) {
                cx.retired.push(p.plugin);
            }
        }
        if !self.external {
            self.retired.clear();
        }
        for b in &s.busses {
            let n = b.format.channels().clamp(1, MAX_CHANNELS);
            let bus = self.busses.entry(b.id).or_insert_with(|| BusBuf { fmt: b.format, buf: Vec::new() });
            bus.fmt = b.format;
            if bus.buf.len() != n {
                bus.buf = vec![vec![0.0; mb]; n];
            }
        }
        self.busses.retain(|id, _| s.bus(*id).is_some());
    }

    /// Re-syncs plugin instances and the routing order when the session's structure changed
    /// since the last sync (or instances were adopted). `render` calls it; a realtime host may
    /// also call it right after swapping sessions so slots are filled before playback starts.
    pub fn ensure_synced(&mut self, s: &Session) {
        let key = (structure_fingerprint(s), s.tracks.len(), s.busses.len());
        if key != self.synced {
            self.sync(s);
            self.order = processing_order(s);
            self.synced = key;
        }
    }

    /// Render `frames` (≤ max_block) samples starting at `pos` into `out` (≥ 2 channels).
    pub fn render(&mut self, s: &Session, pos: Samples, frames: usize, out: &mut [Vec<f32>]) {
        let started = on_audio_thread().then(std::time::Instant::now);
        self.render_block(s, pos, frames, out);
        if let Some(t0) = started {
            let budget = frames.min(self.max_block) as f64 / f64::from(self.sample_rate);
            self.parallel.observe(t0.elapsed().as_secs_f64(), budget);
        }
    }

    fn render_block(&mut self, s: &Session, pos: Samples, frames: usize, out: &mut [Vec<f32>]) {
        let frames = frames.min(self.max_block);
        if pos != self.last_end {
            // Seek: silence held notes and ringing plugins.
            for st in self.strips.values_mut() {
                if let Some(i) = &mut st.instrument {
                    i.plugin.all_notes_off();
                }
                match st.external {
                    Some(track) => release_held(&mut st.held, &mut st.midi_out, track, 0),
                    None => st.held.clear(),
                }
                st.clear_delays();
                st.chase = true;
            }
        }
        self.last_end = pos.saturating_add(frames as i64);
        self.ensure_synced(s);
        for b in self.busses.values_mut() {
            for c in b.buf.iter_mut() {
                c.iter_mut().take(frames).for_each(|x| *x = 0.0);
            }
        }
        for c in &mut self.main {
            c.iter_mut().take(frames).for_each(|x| *x = 0.0);
        }
        let any_solo = s.tracks.iter().any(|t| t.mixer.solo && !t.inactive);
        let mut implicit_solo = std::mem::take(&mut self.implicit_solo);
        implicit_solo_set(s, any_solo, &mut implicit_solo);
        let order = std::mem::take(&mut self.order);
        let live = |t: &Track| !(t.kind == TrackKind::Master || t.inactive || t.kind == TrackKind::Vca);
        let is_aux = |t: &Track| matches!(t.kind, TrackKind::Aux | TrackKind::Folder);
        // Phase 1: independent strips (audio, instrument, MIDI) in parallel.
        let mut work = std::mem::take(&mut self.work);
        work.clear();
        for &i in &order {
            let Some(t) = s.tracks.get(i).filter(|t| live(t) && !is_aux(t)) else { continue };
            if let Some(st) = self.strips.remove(&t.id) {
                work.push((i, st));
            }
        }
        for (_, st) in work.iter_mut() {
            if st.scratch.len() < self.max_block {
                st.scratch.resize(self.max_block, 0.0);
            }
        }
        let busses = std::mem::take(&mut self.busses);
        let live_in = LiveInput { input: &self.input, monitor_only: self.monitor_only, recording: self.recording, midi: &self.live_midi };
        // Offline renders always parallelise; the audio callback follows the realtime policy.
        let parallel = !on_audio_thread() || self.parallel.use_parallel(realtime_parallel());
        run_strips(&mut work, parallel, |(i, st)| {
            if let Some(t) = s.tracks.get(*i) {
                process_strip(s, t, st, &busses, &live_in, pos, frames, any_solo, &implicit_solo);
            }
        });
        self.busses = busses;
        // Delay compensation over the whole routing graph. Every track is aligned to the slowest
        // track; an aux's output then arrives later by its input's arrival plus its own inserts;
        // every route into a bus or the main mix is delayed to meet the slowest feeder; the master
        // inserts come last. Latencies are read before anything is routed, or the first block (and
        // a one-block bounce probe) would still see zero.
        let pdc = s.edit.delay_compensation;
        if pdc {
            for t in s.tracks.iter().filter(|t| (is_aux(t) || t.kind == TrackKind::Master) && !t.inactive) {
                if let Some(st) = self.strips.get_mut(&t.id) {
                    note_insert_latency(t, st, pos);
                }
            }
        }
        let lmax_t = if pdc { work.iter().map(|(_, st)| st.latency).max().unwrap_or(0) } else { 0 };
        let main_in = self.plan_latency(s, &order, lmax_t, pdc, &work);
        self.track_latency = lmax_t;
        let master_latency = if pdc {
            s.tracks.iter().filter(|t| t.kind == TrackKind::Master && !t.inactive).filter_map(|t| self.strips.get(&t.id)).map(|st| st.latency).sum()
        } else {
            0
        };
        self.latency = main_in.saturating_add(master_latency);
        for (i, st) in work.iter_mut() {
            let Some(t) = s.tracks.get(*i) else { continue };
            let ch = st.buf.len();
            st.align.set(lmax_t.saturating_sub(st.latency), ch);
            st.align.process(&mut st.buf, frames);
            if t.mixer.sends.iter().flatten().any(|x| x.pre_fader) {
                st.align_pre.set(lmax_t.saturating_sub(st.latency), ch);
                st.align_pre.process(&mut st.pre, frames);
            }
            self.set_route_delays(t, st, main_in);
        }
        for (i, st) in work.iter_mut() {
            if let Some(t) = s.tracks.get(*i) {
                self.route_strip(t, st, pos, frames);
            }
        }
        for (i, st) in work.drain(..) {
            if let Some(t) = s.tracks.get(i) {
                self.strips.insert(t.id, st);
            }
        }
        self.work = work;
        let is_aux = |t: &Track| matches!(t.kind, TrackKind::Aux | TrackKind::Folder);
        // Phase 2: auxes in dependency order (they read busses).
        for t in order.iter().filter_map(|&i| s.tracks.get(i)).filter(|t| live(t)).filter(|t| is_aux(t)) {
            let Some(mut st) = self.strips.remove(&t.id) else { continue };
            if st.scratch.len() < self.max_block {
                st.scratch.resize(self.max_block, 0.0);
            }
            let live_in = LiveInput { input: &self.input, monitor_only: self.monitor_only, recording: self.recording, midi: &self.live_midi };
            process_strip(s, t, &mut st, &self.busses, &live_in, pos, frames, any_solo, &implicit_solo);
            let ch = st.buf.len();
            st.align.set(0, ch);
            st.align_pre.set(0, ch);
            self.set_route_delays(t, &mut st, main_in);
            self.route_strip(t, &mut st, pos, frames);
            self.strips.insert(t.id, st);
        }
        self.order = order;
        self.implicit_solo = implicit_solo;
        // Master faders process the main mix.
        let transport = transport_at(s, pos, !self.monitor_only, self.recording);
        for t in s.tracks.iter().filter(|t| t.kind == TrackKind::Master && !t.inactive) {
            if let Some(strip) = self.strips.get_mut(&t.id) {
                for (dst, src) in strip.buf.iter_mut().zip(self.main.iter()) {
                    if let (Some(d), Some(sr)) = (dst.get_mut(..frames), src.get(..frames)) {
                        d.copy_from_slice(sr);
                    }
                }
                for (i, ins) in t.mixer.inserts.iter().enumerate() {
                    if let (Some(ins), Some(Some(slot))) = (ins, strip.plugins.get_mut(i))
                        && ins.active
                        && !ins.bypass
                        && !slot.failed
                    {
                        apply_params(slot, ins, t, i, pos);
                        slot.plugin.set_transport(&transport);
                        slot.plugin.process(&mut strip.buf, frames);
                        if contain_failure(slot, &mut strip.buf, frames) {
                            strip.failures.record(i);
                        }
                    }
                }
                let v0 = volume_db_at(t, pos);
                let v1 = volume_db_at(t, pos + frames as i64);
                for (ch, src) in strip.buf.iter().enumerate().take(self.main.len()) {
                    if let Some(m) = self.main.get_mut(ch) {
                        for i in 0..frames {
                            let g = lerp_gain(v0, v1, i, frames);
                            if let (Some(d), Some(x)) = (m.get_mut(i), src.get(i)) {
                                *d = x * g;
                            }
                        }
                    }
                }
                let m = self.meters.entry(t.id).or_default();
                m.measure(&self.main, frames, 0.0);
                m.plugin_failures = strip.failures;
            }
        }
        // Metronome: summed after the master fader, so it reaches the speakers without passing
        // through any track, send or insert. Only the playback engine sets `metronome`.
        // Not while monitoring only (transport stopped): the position does not advance there, so the
        // start of the same beat would repeat every block as a buzz.
        if self.metronome && !self.monitor_only && s.edit.click && (!s.edit.flag("click.only_during_record") || self.recording) {
            click::render(s, pos, frames, &mut self.main);
        }
        // Output: min(out.len(), main channels) channels; any further output channels are silent.
        for (c, o) in out.iter_mut().enumerate() {
            let src = self.main.get(c);
            for i in 0..frames {
                let v = src.and_then(|m| m.get(i)).copied().unwrap_or(0.0);
                if let Some(d) = o.get_mut(i) {
                    *d = if v.is_finite() { v } else { 0.0 };
                }
            }
        }
        self.main_meter.measure(&self.main, frames, 0.0);
        // What MIDI tracks routed to hardware ports sent this block.
        for st in self.strips.values_mut().filter(|st| !st.midi_out.is_empty()) {
            let room = MIDI_OUT_CAPACITY.saturating_sub(self.midi_out.len());
            self.midi_out.extend(st.midi_out.drain(..).take(room));
        }
    }

    /// The delay-compensation latency of the audio written onto `bus`: when its slowest feeder
    /// (a track, or an aux return, however deep the chain) delivers it.
    fn bus_latency(&self, bus: BusId) -> usize {
        self.bus_latency.get(&bus).copied().unwrap_or(self.track_latency)
    }

    /// Copy one rendered block of `bus` into `out`. A missing bus stays silent.
    fn copy_bus(&self, bus: BusId, frames: usize, out: &mut [Vec<f32>]) {
        for o in out.iter_mut() {
            for x in o.iter_mut().take(frames) {
                *x = 0.0;
            }
        }
        let Some(src) = self.busses.get(&bus) else { return };
        for (o, c) in out.iter_mut().zip(src.buf.iter()) {
            for i in 0..frames {
                let v = c.get(i).copied().unwrap_or(0.0);
                if let Some(d) = o.get_mut(i) {
                    *d = if v.is_finite() { v } else { 0.0 };
                }
            }
        }
    }

    /// Work out, in dependency order, when each strip's output and each bus's contents arrive
    /// (`out_latency`, `bus_latency`); returns the arrival latency at the main mix. All zero
    /// without delay compensation.
    fn plan_latency(&mut self, s: &Session, order: &[usize], lmax_t: usize, pdc: bool, work: &[(usize, Strip)]) -> usize {
        self.out_latency.clear();
        self.bus_latency.clear();
        if !pdc {
            return 0;
        }
        for b in &s.busses {
            self.bus_latency.insert(b.id, lmax_t);
        }
        let mut main_in = lmax_t;
        let feed = |bus_latency: &mut HashMap<BusId, usize>, main_in: &mut usize, t: &Track, out: usize| {
            let dests = std::iter::once(&t.mixer.output).chain(t.mixer.sends.iter().flatten().map(|sn| &sn.target));
            for d in dests {
                match d {
                    Route::Main => *main_in = (*main_in).max(out),
                    Route::Bus(b) => {
                        let e = bus_latency.entry(*b).or_insert(lmax_t);
                        *e = (*e).max(out);
                    }
                    _ => {}
                }
            }
        };
        for (i, _) in work {
            let Some(t) = s.tracks.get(*i) else { continue };
            self.out_latency.insert(t.id, lmax_t);
            feed(&mut self.bus_latency, &mut main_in, t, lmax_t);
        }
        for t in order.iter().filter_map(|&i| s.tracks.get(i)).filter(|t| matches!(t.kind, TrackKind::Aux | TrackKind::Folder) && !t.inactive) {
            let own = self.strips.get(&t.id).map_or(0, |st| st.latency);
            let arrive = match &t.mixer.input {
                Route::Bus(b) => self.bus_latency.get(b).copied().unwrap_or(lmax_t),
                _ => lmax_t,
            };
            let out = arrive.saturating_add(own);
            self.out_latency.insert(t.id, out);
            feed(&mut self.bus_latency, &mut main_in, t, out);
        }
        main_in
    }

    /// Delay each of a strip's routes (its output and its sends) so it lands together with the
    /// slowest other feeder of the same destination.
    fn set_route_delays(&self, t: &Track, st: &mut Strip, main_in: usize) {
        let ch = st.buf.len();
        let out = self.out_latency.get(&t.id).copied().unwrap_or(0);
        let gap = |d: &Route| -> usize {
            let dest = match d {
                Route::Main => main_in,
                Route::Bus(b) => self.bus_latency.get(b).copied().unwrap_or(0),
                _ => return 0,
            };
            dest.saturating_sub(out)
        };
        st.out_extra.set(gap(&t.mixer.output), ch);
        if st.send_extra.len() < t.mixer.sends.len() {
            st.send_extra.resize_with(t.mixer.sends.len(), DelayLine::default);
        }
        for (d, snd) in st.send_extra.iter_mut().zip(t.mixer.sends.iter()) {
            d.set(snd.as_ref().map_or(0, |sn| gap(&sn.target)), ch);
        }
    }

    /// Send, meter and pan a processed strip into the busses / main mix.
    fn route_strip(&mut self, t: &Track, strip: &mut Strip, pos: Samples, frames: usize) {
        let m = self.meters.entry(t.id).or_default();
        m.measure(&strip.buf, frames, strip.gr);
        m.plugin_failures = strip.failures;
        let src_fmt = strip_format(t, strip.buf.len());
        let law = self.pan_law;
        for (i, snd) in t.mixer.sends.iter().enumerate() {
            let Some(snd) = snd else { continue };
            if snd.mute || strip.muted {
                continue;
            }
            let lvl = send_level(t, i, snd.level_db, pos);
            let gain = db_to_gain(lvl);
            let Route::Bus(b) = &snd.target else { continue };
            if gain <= 0.0 || !self.busses.contains_key(b) {
                continue;
            }
            let delay = strip.send_extra.get_mut(i).filter(|d| d.len > 0);
            let src: &Vec<Vec<f32>> = if let Some(d) = delay {
                let from = if snd.pre_fader { &strip.pre } else { &strip.buf };
                if strip.sendbuf.len() != from.len() {
                    strip.sendbuf = vec![Vec::new(); from.len()];
                }
                for (c, f) in strip.sendbuf.iter_mut().zip(from.iter()) {
                    c.clear();
                    c.extend_from_slice(f.get(..frames).unwrap_or(&[]));
                }
                d.process(&mut strip.sendbuf, frames);
                &strip.sendbuf
            } else if snd.pre_fader {
                &strip.pre
            } else {
                &strip.buf
            };
            let Some(dst) = self.busses.get_mut(b) else { continue };
            // Mono sends use the send's own pan; stereo sends go straight to L/R.
            let stereo = if src.len() == 1 { [snd.pan, 0.0] } else { [-1.0, 1.0] };
            let surround = snd.surround.as_ref().or(if snd.follow_main_pan { t.mixer.surround.as_ref() } else { None });
            let m = build_matrix(src_fmt, src.len(), dst.fmt, dst.buf.len(), &PanSpec { stereo, surround }, law);
            apply_matrix(src, frames, &mut dst.buf, &m, gain);
        }
        let src: &[Vec<f32>] = match &t.mixer.output {
            Route::Main | Route::Bus(_) if strip.out_extra.len > 0 => {
                if strip.outbuf.len() != strip.buf.len() {
                    strip.outbuf = vec![Vec::new(); strip.buf.len()];
                }
                for (d, src) in strip.outbuf.iter_mut().zip(strip.buf.iter()) {
                    d.clear();
                    d.extend_from_slice(src.get(..frames).unwrap_or(&[]));
                }
                strip.out_extra.process(&mut strip.outbuf, frames);
                &strip.outbuf
            }
            _ => &strip.buf,
        };
        let (dst, dst_fmt) = match &t.mixer.output {
            Route::Main => (&mut self.main, self.main_fmt),
            Route::Bus(b) => match self.busses.get_mut(b) {
                Some(d) => (&mut d.buf, d.fmt),
                None => return,
            },
            _ => return,
        };
        let mut start = 0;
        while start < frames {
            let (at, end) = pan_segment(t, pos, start, frames);
            let spec = PanSpec { stereo: [pan_at(t, 0, at), pan_at(t, 1, at)], surround: t.mixer.surround.as_ref() };
            let m = build_matrix(src_fmt, src.len(), dst_fmt, dst.len(), &spec, law);
            apply_matrix_span(src, start..end, dst, &m, 1.0);
            start = end;
        }
    }
}

/// Source → trim → inserts → (pre-fader copy) → mute/fader for one strip. Independent of other
/// strips except that aux inputs read `busses`, so non-aux strips can run in parallel.
/// Live input routed to monitored tracks.
struct LiveInput<'a> {
    input: &'a [Vec<f32>],
    monitor_only: bool,
    recording: bool,
    midi: &'a [[u8; 3]],
}

/// Most live MIDI messages one block takes from the host (further ones are dropped).
pub const LIVE_MIDI_CAPACITY: usize = 1024;

fn process_strip(
    s: &Session,
    t: &Track,
    strip: &mut Strip,
    busses: &HashMap<BusId, BusBuf>,
    live: &LiveInput<'_>,
    pos: Samples,
    frames: usize,
    any_solo: bool,
    implicit_solo: &[TrackId],
) {
    for c in strip.buf.iter_mut() {
        c.iter_mut().take(frames).for_each(|x| *x = 0.0);
    }
    let transport = transport_at(s, pos, !live.monitor_only, live.recording);
    match t.kind {
        TrackKind::Audio if !live.input.is_empty() && (t.mixer.input_monitor || (t.mixer.record_arm && (live.monitor_only || live.recording))) => {
            // Input monitoring: the track hears its hardware input instead of its clips.
            let first = match &t.mixer.input {
                Route::Hardware(h) => {
                    h.trim_start_matches("In ").split(['-', ' ']).next().and_then(|n| n.parse::<usize>().ok()).map_or(0, |n| n.saturating_sub(1))
                }
                _ => 0,
            };
            let n_in = live.input.len().max(1);
            for (k, dst) in strip.buf.iter_mut().enumerate() {
                if let Some(src) = live.input.get((first + k) % n_in) {
                    for (d, x) in dst.iter_mut().zip(src.iter()).take(frames) {
                        *d = *x;
                    }
                }
            }
        }
        TrackKind::Audio if live.monitor_only => {}
        // Stopped: only an instrument you can play live (armed or input-monitored) keeps sounding.
        TrackKind::Instrument | TrackKind::Midi if live.monitor_only && !(t.mixer.record_arm || t.mixer.input_monitor) => {}
        TrackKind::Audio => {
            let block = Range { start: pos, end: pos.saturating_add(frames as i64) };
            for clip in t.clips() {
                if clip.muted || !clip.is_audio() || !clip.range().overlaps(&block) {
                    continue;
                }
                match clip_fx_params(s, clip.id.0) {
                    None => render_clip(s, clip, pos, frames, &mut strip.buf, &mut strip.scratch),
                    Some(params) => {
                        let nch = strip.buf.len();
                        if strip.clipbuf.len() != nch || strip.clipbuf.first().map_or(0, Vec::len) < frames {
                            strip.clipbuf = vec![vec![0.0; frames.max(strip.scratch.len())]; nch];
                        }
                        for c in strip.clipbuf.iter_mut() {
                            c.iter_mut().take(frames).for_each(|x| *x = 0.0);
                        }
                        render_clip(s, clip, pos, frames, &mut strip.clipbuf, &mut strip.scratch);
                        let fx = strip.clip_fx.entry(clip.id.0).or_insert_with(|| ClipFx::new(s.sample_rate.as_f64() as f32, nch));
                        fx.configure(params);
                        fx.process(&mut strip.clipbuf, frames);
                        for (d, c) in strip.buf.iter_mut().zip(strip.clipbuf.iter()) {
                            for (x, y) in d.iter_mut().zip(c.iter()).take(frames) {
                                *x += *y;
                            }
                        }
                    }
                }
            }
        }
        TrackKind::Aux | TrackKind::Folder => {
            if let Route::Bus(b) = &t.mixer.input
                && let Some(src) = busses.get(b)
            {
                let (ns, nd) = (src.buf.len(), strip.buf.len());
                let dst_fmt = strip_format(t, nd);
                let m = if src.fmt == dst_fmt && ns == nd {
                    identity_matrix(ns)
                } else if ns == 1 && nd == 2 {
                    // A mono bus feeds both sides of a stereo aux.
                    let mut m = [[0.0; MAX_CHANNELS]; MAX_CHANNELS];
                    m[0][0] = 1.0;
                    m[0][1] = 1.0;
                    m
                } else {
                    let stereo = if ns == 1 { [0.0, 0.0] } else { [-1.0, 1.0] };
                    build_matrix(src.fmt, ns, dst_fmt, nd, &PanSpec { stereo, surround: None }, PanLaw::Minus3)
                };
                apply_matrix(&src.buf, frames, &mut strip.buf, &m, 1.0);
            }
        }
        // A MIDI track with a hardware output and no instrument plays external gear.
        TrackKind::Midi if t.instrument.is_none() && matches!(t.mixer.output, Route::Hardware(_)) => {
            let silent = t.mixer.mute || any_solo && !t.mixer.solo && !t.mixer.solo_safe && !implicit_solo.contains(&t.id);
            strip.external = Some(t.id);
            if strip.midi_out.capacity() < MIDI_OUT_PER_STRIP {
                strip.midi_out.reserve(MIDI_OUT_PER_STRIP);
            }
            if live.monitor_only || silent {
                // Not playing, or silenced: whatever was sounding is released.
                release_held(&mut strip.held, &mut strip.midi_out, t.id, 0);
            } else {
                let mut sink = OutSink { events: &mut strip.midi_out, track: t.id };
                schedule_notes(s, t, pos, frames, &mut sink, &mut strip.held);
            }
        }
        TrackKind::Instrument | TrackKind::Midi => {
            if let Some(inst) = strip.instrument.as_mut().filter(|i| !i.failed) {
                // #152: push the instrument's stored parameter values into the live plugin
                // before processing, the same way inserts do below. Without this, the values
                // loaded from a session (or set via `mix.insert_state`) never reach the plugin
                // instance and the rendered audio matches the plugin's defaults — e.g. a saved
                // `level = -12 dB` sounds the same as `level = -6 dB`.
                if let Some(ins) = &t.instrument {
                    apply_params(inst, ins, t, INSTRUMENT_SLOT, pos);
                }
                inst.plugin.set_transport(&transport);
                if std::mem::take(&mut strip.chase) {
                    chase_controllers(s, t, pos, inst);
                }
                // A stopped transport plays no clips (its position does not advance, so the same
                // notes would retrigger every block); live input still plays.
                if !live.monitor_only {
                    schedule_notes(s, t, pos, frames, inst, &mut strip.held);
                }
                if t.mixer.record_arm || t.mixer.input_monitor {
                    for msg in live.midi {
                        inst.plugin.midi(0, *msg);
                    }
                }
                if strip.buf.len() >= 2 {
                    inst.plugin.process(&mut strip.buf, frames);
                } else if let Some(c0) = strip.buf.first_mut() {
                    // Mono MIDI track: render the instrument in stereo and fold.
                    let mut st = [std::mem::take(c0), std::mem::take(&mut strip.scratch)];
                    inst.plugin.process(&mut st, frames);
                    let [a, b] = st;
                    *c0 = a;
                    strip.scratch = b;
                    for (x, y) in c0.iter_mut().zip(strip.scratch.iter()).take(frames) {
                        *x = 0.5 * (*x + *y);
                    }
                }
                if contain_failure(inst, &mut strip.buf, frames) {
                    strip.failures.record(INSTRUMENT_SLOT);
                }
            }
        }
        _ => {}
    }
    let trim = db_to_gain(t.mixer.trim_db) * if t.mixer.phase_invert { -1.0 } else { 1.0 };
    if (trim - 1.0).abs() > f32::EPSILON {
        for c in strip.buf.iter_mut() {
            c.iter_mut().take(frames).for_each(|x| *x *= trim);
        }
    }
    let mut gr = 0.0f32;
    let mut latency = 0usize;
    for (i, ins) in t.mixer.inserts.iter().enumerate() {
        let (Some(ins), Some(Some(slot))) = (ins, strip.plugins.get_mut(i)) else { continue };
        if !ins.active || ins.bypass || slot.failed {
            continue;
        }
        apply_params(slot, ins, t, i, pos);
        slot.plugin.set_transport(&transport);
        slot.plugin.process(&mut strip.buf, frames);
        if contain_failure(slot, &mut strip.buf, frames) {
            strip.failures.record(i);
            continue;
        }
        latency = latency.saturating_add(slot.plugin.latency());
        if gr == 0.0 {
            gr = slot.plugin.gain_reduction_db();
        }
    }
    strip.gr = gr;
    strip.latency = latency.min(MAX_LATENCY);
    let auto_mute = t.mixer.automation_mode.reads() && t.lane(&AutoParam::Mute).is_some_and(|l| l.value_at(pos, 0.0) >= 0.5);
    let soloed_out = any_solo && !t.mixer.solo && !t.mixer.solo_safe && !implicit_solo.contains(&t.id);
    strip.muted = t.mixer.mute || auto_mute || soloed_out;
    if t.mixer.sends.iter().flatten().any(|x| x.pre_fader) {
        strip.pre.resize_with(strip.buf.len(), Vec::new);
        for (d, src) in strip.pre.iter_mut().zip(strip.buf.iter()) {
            d.clear();
            d.extend_from_slice(src.get(..frames).unwrap_or(&[]));
        }
    }
    // Endpoints are the first and last sample of this block. The next block starts on its own
    // first sample, so a breakpoint there does not ramp the block before it.
    let last = pos.saturating_add(frames.saturating_sub(1) as i64);
    let mut v0 = volume_db_at(t, pos);
    let mut v1 = volume_db_at(t, last);
    // Trim automation (an offset on top of the volume curve).
    if let Some(l) = t
        .automation
        .iter()
        .find(|l| matches!(&l.param, AutoParam::Plugin { slot: u8::MAX, param } if param == "trim"))
        .filter(|l| !l.points.is_empty())
    {
        v0 += l.value_at(pos, 0.0);
        v1 += l.value_at(last, 0.0);
    }
    // VCA master: its fader offsets the member's, and its mute mutes the member.
    if let Some(vid) = t.mixer.vca
        && let Some(vca) = s.tracks.iter().find(|x| x.id.0 == vid && x.kind == TrackKind::Vca)
    {
        v0 += volume_db_at(vca, pos);
        v1 += volume_db_at(vca, last);
        if vca.mixer.mute {
            strip.muted = true;
        }
    }
    let (g0, g1) = if strip.muted { (-144.0, -144.0) } else { (v0.min(12.0), v1.min(12.0)) };
    for c in strip.buf.iter_mut() {
        for i in 0..frames {
            if let Some(x) = c.get_mut(i) {
                *x *= lerp_gain(g0, g1, i, frames.saturating_sub(1));
            }
        }
    }
}

/// Silences a block the plugin filled with NaN or infinity and takes the plugin out until the
/// next [`MixEngine::reset`]. True then.
fn contain_failure(slot: &mut PluginSlot, buf: &mut [Vec<f32>], frames: usize) -> bool {
    // A branch-free fold vectorizes; `all` runs about 8 times slower here.
    if buf.iter().all(|c| c.get(..frames).unwrap_or(c).iter().fold(true, |ok, x| ok & x.is_finite())) {
        return false;
    }
    for c in buf.iter_mut() {
        c.iter_mut().take(frames).for_each(|x| *x = 0.0);
    }
    slot.failed = true;
    true
}

/// Clip-effect parameters of a clip, or None when it has none or they are bypassed.
fn clip_fx_params(s: &Session, clip: u64) -> Option<impl Iterator<Item = (&str, f32)> + Clone> {
    if s.edit.values.is_empty() || Key::new(format_args!("clip_fx.bypass.{clip}")).is_none_or(|k| s.edit.flag(k.as_str())) {
        return None;
    }
    let pre = Key::new(format_args!("clip_fx.{clip}."))?;
    let mut params = s
        .edit
        .values
        .range::<str, _>((Bound::Included(pre.as_str()), Bound::Unbounded))
        .take_while(move |(k, _)| k.starts_with(pre.as_str()))
        .map(move |(k, v)| (k.get(pre.len..).unwrap_or(""), *v as f32))
        .peekable();
    params.peek()?;
    Some(params)
}

/// A short key formatted on the stack: lookups made on the audio thread must not allocate.
#[derive(Clone, Copy)]
struct Key {
    buf: [u8; 64],
    len: usize,
}

impl Key {
    /// None when the key doesn't fit.
    fn new(args: std::fmt::Arguments<'_>) -> Option<Key> {
        let mut k = Key { buf: [0; 64], len: 0 };
        std::fmt::Write::write_fmt(&mut k, args).ok()?;
        Some(k)
    }

    fn as_str(&self) -> &str {
        self.buf.get(..self.len).and_then(|b| std::str::from_utf8(b).ok()).unwrap_or("")
    }
}

impl std::fmt::Write for Key {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        let end = self.len.checked_add(s.len()).ok_or(std::fmt::Error)?;
        self.buf.get_mut(self.len..end).ok_or(std::fmt::Error)?.copy_from_slice(s.as_bytes());
        self.len = end;
        Ok(())
    }
}

impl ClipFx {
    fn new(sr: f32, ch: usize) -> ClipFx {
        ClipFx { eq: None, comp: None, gain_db: 0.0, sr, ch }
    }
    fn make(&self, id: &str) -> Option<Box<dyn Plugin>> {
        soundcraft_dsp::create(id).map(|mut p| {
            p.prepare(self.sr, 16_384, self.ch.max(1));
            p
        })
    }
    /// Modules exist only when one of their parameters is set.
    fn configure<'a>(&mut self, params: impl Iterator<Item = (&'a str, f32)> + Clone) {
        self.gain_db = 0.0;
        let has_eq = params.clone().any(|(k, _)| k.starts_with("eq."));
        let has_comp = params.clone().any(|(k, _)| k.starts_with("comp."));
        if has_eq && self.eq.is_none() {
            self.eq = self.make("eq_7band");
        } else if !has_eq {
            self.eq = None;
        }
        if has_comp && self.comp.is_none() {
            self.comp = self.make("compressor");
        } else if !has_comp {
            self.comp = None;
        }
        for (k, v) in params {
            if let Some(p) = k.strip_prefix("eq.") {
                if let Some(eq) = &mut self.eq
                    && eq.param(p) != Some(v)
                {
                    eq.set_param(p, v);
                }
            } else if let Some(p) = k.strip_prefix("comp.") {
                if let Some(c) = &mut self.comp
                    && c.param(p) != Some(v)
                {
                    c.set_param(p, v);
                }
            } else if k == "gain" {
                self.gain_db = v;
            }
        }
    }
    fn process(&mut self, io: &mut [Vec<f32>], frames: usize) {
        if let Some(eq) = &mut self.eq {
            eq.process(io, frames);
        }
        if let Some(c) = &mut self.comp {
            c.process(io, frames);
        }
        if self.gain_db.abs() > 1e-6 {
            let g = db_to_gain(self.gain_db);
            for ch in io.iter_mut() {
                ch.iter_mut().take(frames).for_each(|x| *x *= g);
            }
        }
    }
}

/// Hash of everything `sync`/`processing_order` depend on (tracks, kinds, routing, plugins, sends).
fn structure_fingerprint(s: &Session) -> usize {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for t in &s.tracks {
        t.id.hash(&mut h);
        t.kind.hash(&mut h);
        t.format.hash(&mut h);
        t.inactive.hash(&mut h);
        t.mixer.inserts.len().hash(&mut h);
        t.mixer.input.hash(&mut h);
        t.mixer.output.hash(&mut h);
        for i in &t.mixer.inserts {
            i.as_ref().map(|x| x.plugin.as_str()).hash(&mut h);
        }
        for snd in t.mixer.sends.iter().flatten() {
            snd.target.hash(&mut h);
        }
        t.instrument.as_ref().map(|x| x.plugin.as_str()).hash(&mut h);
        let sample = t.instrument.as_ref().and_then(|x| x.sample);
        sample.hash(&mut h);
        // Which decoded audio it is, so new audio under the same id (another session) re-syncs.
        sample.and_then(|id| s.pool.get(id)).map(|a| Arc::as_ptr(a) as usize).hash(&mut h);
    }
    for b in &s.busses {
        b.id.hash(&mut h);
        b.format.hash(&mut h);
    }
    s.main_format().hash(&mut h);
    h.finish() as usize
}

std::thread_local! {
    static AUDIO_THREAD: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Mark the calling thread as the realtime audio thread (`soundcraft_playback`'s cpal callbacks
/// do, every call: one thread-local store). Strips [`MixEngine::render`] hands to worker threads
/// count as the audio thread while they render for it. Code that might run there asks
/// [`on_audio_thread`] before doing anything that blocks; the desktop app's logger does, so a
/// `log::` record from the audio thread never takes a lock or touches a file.
pub fn mark_audio_thread() {
    AUDIO_THREAD.set(true);
}

/// True on the thread [`mark_audio_thread`] marked, and on a worker while it renders strips for
/// it. Lock-free and allocation-free.
pub fn on_audio_thread() -> bool {
    AUDIO_THREAD.get()
}

/// Run `f` on this thread as (or as not) the audio thread, then restore the thread's own mark
/// (also when `f` unwinds): a pool worker renders for realtime playback and offline bounces alike.
#[cfg(not(target_arch = "wasm32"))]
fn as_audio_thread(audio: bool, f: impl FnOnce()) {
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            AUDIO_THREAD.set(self.0);
        }
    }
    let _restore = Restore(AUDIO_THREAD.replace(audio));
    f();
}

/// Whether the audio callback may hand strips to the shared worker pool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RealtimeParallel {
    /// Always render serially on the callback thread.
    Off,
    /// Serial until the mix nears its time budget, then the worker pool; back to serial (for a
    /// cool-down) when the pool starts missing deadlines. The default.
    Adaptive,
    /// Always use the pool (when there are enough strips).
    On,
}

static REALTIME_PARALLEL: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(1);

/// Choose how the audio callback uses its worker pool (see [`prepare_realtime`]). The pool shares a
/// block's strips with the callback, which works through them alongside the workers, so it never
/// waits for a thread that has not started; it can still wait for a worker the OS descheduled
/// part-way through a strip, which is why the default only engages it when serial rendering nears
/// its budget. Offline renders always parallelise (through rayon).
pub fn set_realtime_parallel(mode: RealtimeParallel) {
    let v = match mode {
        RealtimeParallel::Off => 0,
        RealtimeParallel::Adaptive => 1,
        RealtimeParallel::On => 2,
    };
    REALTIME_PARALLEL.store(v, std::sync::atomic::Ordering::Relaxed);
}

/// The current choice (see [`set_realtime_parallel`]).
pub fn realtime_parallel() -> RealtimeParallel {
    match REALTIME_PARALLEL.load(std::sync::atomic::Ordering::Relaxed) {
        0 => RealtimeParallel::Off,
        2 => RealtimeParallel::On,
        _ => RealtimeParallel::Adaptive,
    }
}

/// Decides, block by block, whether the callback renders serially or through the pool.
///
/// Each block's render time is compared with its budget (the block's duration). Serial rendering
/// that sits above 60 % of the budget (or overruns repeatedly) switches to the pool; the pool is
/// abandoned for a few seconds when it overruns repeatedly itself. Pure arithmetic on durations: no
/// allocation, no locking.
#[derive(Debug, Clone)]
struct ParallelPolicy {
    parallel: bool,
    /// Smoothed render time as a fraction of the budget, in the current mode.
    load: f64,
    /// Decaying count of blocks that used more than 90 % of their budget.
    overruns: f64,
    /// Seconds of audio left before the pool may be tried again.
    cooldown: f64,
}

impl ParallelPolicy {
    const ENGAGE_LOAD: f64 = 0.6;
    const OVERRUN_LOAD: f64 = 0.9;
    const OVERRUN_LIMIT: f64 = 2.0;
    const COOLDOWN_S: f64 = 4.0;

    fn new() -> Self {
        ParallelPolicy { parallel: false, load: 0.0, overruns: 0.0, cooldown: 0.0 }
    }

    /// Whether the next block should use the pool.
    fn use_parallel(&self, mode: RealtimeParallel) -> bool {
        match mode {
            RealtimeParallel::Off => false,
            RealtimeParallel::On => true,
            RealtimeParallel::Adaptive => self.parallel,
        }
    }

    /// Record one block that took `elapsed_s` of a `budget_s` budget.
    fn observe(&mut self, elapsed_s: f64, budget_s: f64) {
        if !(budget_s > 0.0 && elapsed_s.is_finite()) {
            return;
        }
        let load = elapsed_s / budget_s;
        self.load = self.load * 0.95 + load * 0.05;
        self.overruns = self.overruns * 0.98 + f64::from(u8::from(load > Self::OVERRUN_LOAD));
        self.cooldown = (self.cooldown - budget_s).max(0.0);
        if self.parallel {
            if self.overruns > Self::OVERRUN_LIMIT {
                self.parallel = false;
                self.cooldown = Self::COOLDOWN_S;
                self.load = load;
                self.overruns = 0.0;
            }
        } else if self.cooldown == 0.0 && (self.load > Self::ENGAGE_LOAD || self.overruns > Self::OVERRUN_LIMIT) {
            self.parallel = true;
            self.load = load;
            self.overruns = 0.0;
        }
    }
}

/// The audio callback's worker threads (see [`prepare_realtime`]).
#[cfg(not(target_arch = "wasm32"))]
static RT_POOL: std::sync::OnceLock<soundcraft_rt_pool::Pool> = std::sync::OnceLock::new();

/// Start the threads the audio callback shares strips with. Call once from a normal thread before
/// playback starts: the callback never creates threads, and without this it renders serially.
/// Idempotent. Offline renders do not use these threads.
pub fn prepare_realtime() {
    #[cfg(not(target_arch = "wasm32"))]
    {
        RT_POOL.get_or_init(|| soundcraft_rt_pool::Pool::new(soundcraft_rt_pool::default_workers()));
    }
}

/// Process strips, in parallel on native targets when `parallel` and there are enough of them.
/// On the audio thread that means the callback's own worker pool: the callback works through the
/// strips itself alongside the workers, so it does not wait for a thread that has not started, and
/// it falls back to serial when the pool is not prepared or busy. Offline renders use rayon.
#[cfg(not(target_arch = "wasm32"))]
fn run_strips<T: Send>(work: &mut [T], parallel: bool, f: impl Fn(&mut T) + Sync + Send) {
    use rayon::prelude::*;
    if work.len() < 4 || !parallel {
        work.iter_mut().for_each(f);
    } else if on_audio_thread() {
        match RT_POOL.get().filter(|p| p.workers() > 0) {
            Some(pool) => match pool.run(work, &|w| as_audio_thread(true, || f(w))) {
                // Another batch is in flight (two engines on one pool): nothing ran, so do it here.
                Err(soundcraft_rt_pool::RunError::Busy) => work.iter_mut().for_each(f),
                // Every strip was attempted; a panicking one is skipped for this block.
                Ok(()) | Err(soundcraft_rt_pool::RunError::Panicked) => {}
            },
            None => work.iter_mut().for_each(f),
        }
    } else {
        work.par_iter_mut().for_each(|w| as_audio_thread(false, || f(w)));
    }
}

#[cfg(target_arch = "wasm32")]
fn run_strips<T: Send>(work: &mut [T], _parallel: bool, f: impl Fn(&mut T) + Sync + Send) {
    work.iter_mut().for_each(f);
}

fn strip_channels(t: &Track, main_ch: usize) -> usize {
    match t.kind {
        TrackKind::Master => main_ch,
        TrackKind::Instrument => t.channels().clamp(2, MAX_CHANNELS),
        _ => t.channels().clamp(1, MAX_CHANNELS),
    }
}

/// The format a strip's buffer actually carries (a mono instrument renders in stereo).
fn strip_format(t: &Track, n: usize) -> ChannelFormat {
    if t.format.channels().min(MAX_CHANNELS) == n { t.format } else { ChannelFormat::for_channels(n) }
}

/// The aux and folder tracks that feed track `i`'s input bus (by their output or a send). Empty when
/// `i`'s input is not a bus.
fn feeders_of(s: &Session, i: usize) -> Vec<usize> {
    let Some(Route::Bus(b)) = s.tracks.get(i).map(|t| &t.mixer.input) else { return Vec::new() };
    s.tracks
        .iter()
        .enumerate()
        .filter(|(_, t)| matches!(t.kind, TrackKind::Aux | TrackKind::Folder))
        .filter(|(_, t)| t.mixer.output == Route::Bus(*b) || t.mixer.sends.iter().flatten().any(|sn| sn.target == Route::Bus(*b)))
        .map(|(j, _)| j)
        .collect()
}

fn is_aux_kind(t: &Track) -> bool {
    matches!(t.kind, TrackKind::Aux | TrackKind::Folder)
}

/// Topological order of the aux tracks (Kahn's algorithm, ties by track index so the order is
/// stable), and any auxes left over because they sit on, or downstream of, a routing loop. The
/// leftovers are in track order too.
fn aux_order(s: &Session) -> (Vec<usize>, Vec<usize>) {
    let auxes: Vec<usize> = (0..s.tracks.len()).filter(|&i| s.tracks.get(i).is_some_and(is_aux_kind)).collect();
    let mut feeders: HashMap<usize, Vec<usize>> = HashMap::new();
    let mut pending: HashMap<usize, usize> = HashMap::new();
    let mut fed: HashMap<usize, Vec<usize>> = HashMap::new();
    for &i in &auxes {
        let f = feeders_of(s, i);
        pending.insert(i, f.len());
        for &j in &f {
            fed.entry(j).or_default().push(i);
        }
        feeders.insert(i, f);
    }
    let mut ready: std::collections::BinaryHeap<std::cmp::Reverse<usize>> =
        auxes.iter().filter(|i| pending.get(i) == Some(&0)).map(|&i| std::cmp::Reverse(i)).collect();
    let mut order = Vec::with_capacity(auxes.len());
    while let Some(std::cmp::Reverse(i)) = ready.pop() {
        order.push(i);
        for &k in fed.get(&i).into_iter().flatten() {
            if let Some(n) = pending.get_mut(&k) {
                *n = n.saturating_sub(1);
                if *n == 0 {
                    ready.push(std::cmp::Reverse(k));
                }
            }
        }
    }
    let left: Vec<usize> = auxes.into_iter().filter(|i| !order.contains(i)).collect();
    (order, left)
}

/// A routing loop, if the session has one: the tracks that feed each other through busses, in
/// feeding order. A loop has no valid processing order, so commands refuse to create one and the
/// mix cuts it (see [`processing_order`]).
pub fn routing_cycle(s: &Session) -> Option<Vec<TrackId>> {
    let (_, left) = aux_order(s);
    // Every leftover is fed by another leftover; walking feeders must therefore revisit one.
    let mut cur = *left.first()?;
    let mut seen: Vec<usize> = Vec::new();
    loop {
        if let Some(at) = seen.iter().position(|&x| x == cur) {
            return Some(seen.get(at..)?.iter().filter_map(|&i| s.tracks.get(i)).map(|t| t.id).collect());
        }
        seen.push(cur);
        cur = feeders_of(s, cur).into_iter().find(|f| left.contains(f))?;
    }
}

/// Audio/instrument tracks first, then auxes so that every bus producer comes before its consumers.
/// Auxes on a routing loop (which commands refuse to create, but a loaded file can contain) come
/// last, in track order: each reads the bus as the other strips left it that block, so the closing
/// edge of the loop carries no signal rather than hanging or oscillating.
fn processing_order(s: &Session) -> Vec<usize> {
    let mut order: Vec<usize> = (0..s.tracks.len()).filter(|&i| s.tracks.get(i).is_some_and(|t| !is_aux_kind(t))).collect();
    let (sorted, looped) = aux_order(s);
    order.extend(sorted);
    order.extend(looped);
    order
}

/// Implicit solo: every strip the audible soloed material flows through, however many auxes deep.
/// An aux is not muted while a soloed track, or another implicitly soloed aux, feeds its input bus.
/// `out` is reused between blocks; the walk visits each track at most once, so cycles terminate.
fn implicit_solo_set(s: &Session, any_solo: bool, out: &mut Vec<TrackId>) {
    out.clear();
    if !any_solo {
        return;
    }
    let feeds = |o: &Track, b: BusId| o.mixer.output == Route::Bus(b) || o.mixer.sends.iter().flatten().any(|sn| sn.target == Route::Bus(b));
    let mut grew = true;
    while grew {
        grew = false;
        for t in &s.tracks {
            let Route::Bus(b) = &t.mixer.input else { continue };
            if t.inactive || out.contains(&t.id) {
                continue;
            }
            if s.tracks.iter().any(|o| !o.inactive && (o.mixer.solo || out.contains(&o.id)) && feeds(o, *b)) {
                out.push(t.id);
                grew = true;
            }
        }
    }
}

/// Metronome: a short synthesized blip on every beat of the tempo map, accented on beat 1 of
/// each bar. Settings come from `setup.click_countoff` (`click.*` values).
mod click {
    use super::*;

    /// Blip length in seconds.
    const LEN_S: f64 = 0.03;

    pub fn render(s: &Session, pos: Samples, frames: usize, main: &mut [Vec<f32>]) {
        let sr = s.sample_rate;
        let rate = sr.as_f64();
        let len = (LEN_S * rate) as i64;
        let end = pos.saturating_add(frames as i64);
        let gain = f64::from(db_to_gain(s.edit.value("click.volume_db", -6.0).clamp(-60.0, 12.0) as f32));
        let accent = (note_hz(s.edit.value("click.accent_note", 84.0)), s.edit.value("click.accent_velocity", 127.0));
        let normal = (note_hz(s.edit.value("click.normal_note", 72.0)), s.edit.value("click.normal_velocity", 100.0));
        // Start one blip length early so a click that began in an earlier block still rings into this one.
        let t0 = s.tempo.samples_to_ticks(pos.saturating_sub(len), sr).max(0).saturating_sub(1);
        let mut tick = t0.saturating_sub(s.tempo.bar_beat_at_tick(t0).tick);
        // Blocks hold a few beats at most; the guard only protects against a malformed tempo map.
        for _ in 0..4096 {
            let at = s.tempo.tick_to_samples(tick, sr);
            if at >= end {
                break;
            }
            if at.saturating_add(len) > pos {
                let (hz, vel) = if s.tempo.bar_beat_at_tick(tick).beat == 1 { accent } else { normal };
                let amp = gain * (vel.clamp(1.0, 127.0) / 127.0);
                blip(main, at - pos, frames, hz, amp as f32, rate);
            }
            tick = tick.saturating_add(s.tempo.meter_at_tick(tick).ticks_per_beat());
        }
    }

    /// Adds one decaying sine blip whose first sample sits `rel` frames into the block (negative
    /// when it began before the block) to every channel.
    fn blip(main: &mut [Vec<f32>], rel: i64, frames: usize, hz: f64, amp: f32, rate: f64) {
        let len = (LEN_S * rate) as usize;
        let w = std::f64::consts::TAU * hz / rate;
        let first = usize::try_from(-rel).unwrap_or(0);
        for i in first..len {
            let Ok(at) = usize::try_from(rel + i as i64) else { continue };
            if at >= frames {
                break;
            }
            let env = (-6.0 * i as f64 / len as f64).exp() as f32;
            let v = ((w * i as f64).sin() as f32) * env * amp;
            for ch in main.iter_mut() {
                if let Some(d) = ch.get_mut(at) {
                    *d += v;
                }
            }
        }
    }

    /// Click pitch for a MIDI note setting (clamped to the MIDI range).
    fn note_hz(note: f64) -> f64 {
        soundcraft_dsp::midi_to_hz(note.clamp(0.0, 127.0))
    }
}

fn render_clip(s: &Session, clip: &Clip, pos: Samples, frames: usize, buf: &mut [Vec<f32>], _scratch: &mut [f32]) {
    let ClipContent::Audio { source, offset } = clip.content else { return };
    let block = Range { start: pos, end: pos.saturating_add(frames as i64) };
    let Some(isect) = clip.range().intersect(&block) else { return };
    let Some(audio) = s.pool.get(source) else { return };
    let nsrc = audio.buffer.channels.len();
    if nsrc == 0 {
        return;
    }
    let stretch = if clip.stretch.is_finite() && clip.stretch > 0.0 { clip.stretch } else { 1.0 };
    let i0 = usize::try_from(isect.start - pos).unwrap_or(0);
    let i1 = usize::try_from(isect.end - pos).unwrap_or(0).min(frames);
    // Gain is evaluated every 32 samples and interpolated (fades and envelopes stay smooth).
    const STEP: usize = 32;
    for (ch, dst) in buf.iter_mut().enumerate() {
        let src = audio.buffer.channels.get(ch % nsrc).map(Vec::as_slice).unwrap_or(&[]);
        let mut i = i0;
        while i < i1 {
            let seg_end = (i + STEP).min(i1);
            let rel0 = pos.saturating_add(i as i64).saturating_sub(clip.start);
            // The last sample of this step, not the first sample of the next one, so a breakpoint
            // on the boundary does not ramp the audio before it.
            let rel1 = pos.saturating_add(seg_end.saturating_sub(1) as i64).saturating_sub(clip.start);
            let g0 = clip.gain_at(rel0);
            let g1 = clip.gain_at(rel1.min(clip.length));
            let n = (seg_end - i).saturating_sub(1).max(1) as f32;
            for k in i..seg_end {
                let rel = pos.saturating_add(k as i64).saturating_sub(clip.start);
                let v = if (stretch - 1.0).abs() < 1e-9 {
                    usize::try_from(offset.saturating_add(rel)).ok().and_then(|x| src.get(x)).copied().unwrap_or(0.0)
                } else {
                    let f = offset as f64 + rel as f64 / stretch;
                    let x0 = f.floor();
                    let fr = (f - x0) as f32;
                    let a = if x0 >= 0.0 { src.get(x0 as usize).copied().unwrap_or(0.0) } else { 0.0 };
                    let b = if x0 + 1.0 >= 0.0 { src.get((x0 as usize).saturating_add(1)).copied().unwrap_or(0.0) } else { 0.0 };
                    a + (b - a) * fr
                };
                let t = (k - i) as f32 / n;
                if let Some(d) = dst.get_mut(k) {
                    *d += v * (g0 + (g1 - g0) * t);
                }
            }
            i = seg_end;
        }
    }
}

/// A MIDI message a MIDI track sends to the hardware output port it is routed to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MidiOut {
    /// The MIDI track it came from (its `output` route names the port).
    pub track: TrackId,
    /// Samples after the start of the block it was rendered in.
    pub offset: u32,
    pub msg: [u8; 3],
}

/// Most messages one MIDI track sends in a block, and all of them together.
const MIDI_OUT_PER_STRIP: usize = 512;
pub const MIDI_OUT_CAPACITY: usize = 2048;

/// Where a MIDI track's notes and controllers go: a hosted instrument, or an external port.
trait MidiSink {
    fn note_on(&mut self, channel: u8, offset: usize, pitch: u8, velocity: u8);
    fn note_off(&mut self, channel: u8, offset: usize, pitch: u8);
    /// A controller-style message already encoded on channel 1 (status low nibble 0).
    fn midi(&mut self, channel: u8, offset: usize, msg: [u8; 3]);
}

/// A hosted instrument has no channel on its note calls (it hears everything as one channel).
impl MidiSink for PluginSlot {
    fn note_on(&mut self, _channel: u8, offset: usize, pitch: u8, velocity: u8) {
        self.plugin.note_on(offset, pitch, velocity);
    }
    fn note_off(&mut self, _channel: u8, offset: usize, pitch: u8) {
        self.plugin.note_off(offset, pitch);
    }
    fn midi(&mut self, _channel: u8, offset: usize, msg: [u8; 3]) {
        self.plugin.midi(offset, msg);
    }
}

/// Note-offs for every note still sounding on external gear, at `offset` into the block.
fn release_held(held: &mut Vec<(u8, u8, Samples)>, out: &mut Vec<MidiOut>, track: TrackId, offset: u32) {
    let mut sink = OutSink { events: out, track };
    let mut channels = 0u16;
    for (channel, pitch, _) in held.drain(..) {
        sink.note_off(channel, offset as usize, pitch);
        channels |= 1 << (channel & 0x0F);
    }
    // And "all notes off" on each channel that had one, for gear that missed a message.
    for channel in (0..16u8).filter(|c| channels & (1 << c) != 0) {
        sink.push(offset as usize, [0xB0 | channel, 123, 0]);
    }
}

/// External gear: each message keeps its channel.
struct OutSink<'a> {
    events: &'a mut Vec<MidiOut>,
    track: TrackId,
}

impl OutSink<'_> {
    fn push(&mut self, offset: usize, msg: [u8; 3]) {
        if self.events.len() < MIDI_OUT_PER_STRIP {
            self.events.push(MidiOut { track: self.track, offset: u32::try_from(offset).unwrap_or(u32::MAX), msg });
        }
    }
}

impl MidiSink for OutSink<'_> {
    fn note_on(&mut self, channel: u8, offset: usize, pitch: u8, velocity: u8) {
        self.push(offset, [0x90 | (channel & 0x0F), pitch & 0x7F, velocity & 0x7F]);
    }
    fn note_off(&mut self, channel: u8, offset: usize, pitch: u8) {
        self.push(offset, [0x80 | (channel & 0x0F), pitch & 0x7F, 0]);
    }
    fn midi(&mut self, channel: u8, offset: usize, msg: [u8; 3]) {
        self.push(offset, [(msg[0] & 0xF0) | (channel & 0x0F), msg[1], msg[2]]);
    }
}

/// Most simultaneous notes tracked per instrument.
const MAX_HELD: usize = 128;

/// The track's MIDI Track Offset (`midi.offset.<id>` in the edit values), in samples. Looked up
/// without formatting the key, since this runs on the audio thread every block.
fn midi_offset(s: &Session, t: &Track) -> i64 {
    const PREFIX: &str = "midi.offset.";
    s.edit
        .values
        .range::<str, _>((std::ops::Bound::Included(PREFIX), std::ops::Bound::Unbounded))
        .take_while(|(k, _)| k.starts_with(PREFIX))
        .find(|(k, _)| k.get(PREFIX.len()..).and_then(|id| id.parse::<u64>().ok()) == Some(t.id.0))
        .map(|(_, v)| *v)
        .filter(|v| v.is_finite())
        .map_or(0, |v| v as i64)
}

/// Send note on/offs and controllers for MIDI clips overlapping the block, and the offs still owed
/// from earlier blocks (a note that ends on a block edge, or whose clip was trimmed or removed while
/// sounding).
fn schedule_notes(s: &Session, t: &Track, pos: Samples, frames: usize, sink: &mut impl MidiSink, held: &mut Vec<(u8, u8, Samples)>) {
    let sr = s.sample_rate;
    // MIDI Track Offsets: the track's notes sound `offset` samples later (earlier if negative), so
    // the block looks that far back into the clips.
    let offset = midi_offset(s, t);
    let pos = pos.saturating_sub(offset);
    let end = pos.saturating_add(frames as i64);
    held.retain(|&(channel, pitch, off)| {
        if off < end {
            sink.note_off(channel, usize::try_from(off.saturating_sub(pos)).unwrap_or(0), pitch);
            false
        } else {
            true
        }
    });
    for clip in t.clips() {
        if clip.muted {
            continue;
        }
        let ClipContent::Midi { sequence } = &clip.content else { continue };
        if !clip.range().overlaps(&Range { start: pos, end }) {
            continue;
        }
        let base = s.tempo.samples_to_ticks(clip.start, sr);
        // MIDI Real-Time Properties (non-destructive velocity/transpose/duration/delay).
        let rtp = |name: &str, d: f64| Key::new(format_args!("rtp.{}.{name}", t.id.0)).map_or(d, |k| s.edit.value(k.as_str(), d));
        let (dv, dp, dur, delay) = if s.edit.values.is_empty() {
            (0i64, 0i64, 1.0f64, 0i64)
        } else {
            (
                rtp("velocity", 0.0).clamp(-127.0, 127.0) as i64,
                rtp("transpose", 0.0).clamp(-127.0, 127.0) as i64,
                rtp("duration", 100.0).clamp(1.0, 1000.0) / 100.0,
                rtp("delay", 0.0).clamp(-1e7, 1e7) as i64,
            )
        };
        for n0 in &sequence.notes {
            let mut n = *n0;
            n.velocity = (i64::from(n.velocity) + dv).clamp(1, 127) as u8;
            n.pitch = (i64::from(n.pitch) + dp).clamp(0, 127) as u8;
            n.length = ((n.length as f64 * dur) as i64).max(1);
            n.start = n.start.saturating_add(delay).max(0);
            let on = s.tempo.tick_to_samples(base.saturating_add(n.start), sr);
            let off = s.tempo.tick_to_samples(base.saturating_add(n.start).saturating_add(n.length), sr).min(clip.end());
            if on >= clip.end() {
                continue;
            }
            if on >= pos && on < end {
                sink.note_on(n.channel, usize::try_from(on - pos).unwrap_or(0), n.pitch, n.velocity);
                if off < end {
                    sink.note_off(n.channel, usize::try_from(off.saturating_sub(pos)).unwrap_or(0), n.pitch);
                } else if held.len() < MAX_HELD {
                    held.push((n.channel, n.pitch, off));
                }
            }
        }
        let near = s.tempo.samples_to_ticks(pos, sr).saturating_sub(base).saturating_sub(1)
            ..=s.tempo.samples_to_ticks(end, sr).saturating_sub(base).saturating_add(1);
        // Events before tick 0 are what a start trim hid: they don't play.
        for c in sequence.ctrls.iter().filter(|c| c.tick >= 0) {
            let tick = c.tick.saturating_add(delay).max(0);
            if !near.contains(&tick) {
                continue;
            }
            let at = s.tempo.tick_to_samples(base.saturating_add(tick), sr);
            if at >= pos && at < end && at < clip.end() {
                sink.midi(c.channel, usize::try_from(at - pos).unwrap_or(0), ctrl_message(c.kind, c.value, dp));
            }
        }
    }
}

/// A controller event as a MIDI 1.0 message on channel 1 (where the notes go), with `transpose`
/// applied to a poly-pressure key.
fn ctrl_message(kind: CtrlKind, value: i32, transpose: i64) -> [u8; 3] {
    let v7 = value.clamp(0, 127) as u8;
    match kind {
        CtrlKind::Cc(n) => [0xB0, n & 0x7F, v7],
        CtrlKind::PitchBend => {
            let v = value.saturating_add(8192).clamp(0, 16383);
            [0xE0, (v & 0x7F) as u8, (v >> 7) as u8]
        }
        CtrlKind::ChannelPressure => [0xD0, v7, 0],
        CtrlKind::PolyPressure(k) => [0xA0, (i64::from(k) + transpose).clamp(0, 127) as u8, v7],
        CtrlKind::Program => [0xC0, v7, 0],
    }
}

/// Before an instrument plays from `pos`: reset its controllers, then send each one the last value
/// it had on this track before `pos`.
fn chase_controllers(s: &Session, t: &Track, pos: Samples, inst: &mut PluginSlot) {
    inst.plugin.midi(0, [0xB0, 121, 0]);
    let pos = pos.saturating_sub(midi_offset(s, t));
    const BEND: usize = 128;
    const PRESSURE: usize = 129;
    const PROGRAM: usize = 130;
    let mut last = [(i64::MIN, 0i32); 131];
    let sr = s.sample_rate;
    for clip in t.clips() {
        let ClipContent::Midi { sequence } = &clip.content else { continue };
        if clip.muted || clip.start >= pos {
            continue;
        }
        let base = s.tempo.samples_to_ticks(clip.start, sr);
        let until = pos.min(clip.end());
        for c in sequence.ctrls.iter().filter(|c| c.tick >= 0) {
            let slot = match c.kind {
                CtrlKind::Cc(n) => usize::from(n & 0x7F),
                CtrlKind::PitchBend => BEND,
                CtrlKind::ChannelPressure => PRESSURE,
                CtrlKind::Program => PROGRAM,
                CtrlKind::PolyPressure(_) => continue,
            };
            let at = s.tempo.tick_to_samples(base.saturating_add(c.tick), sr);
            if at < until
                && let Some(l) = last.get_mut(slot)
                && at >= l.0
            {
                *l = (at, c.value);
            }
        }
    }
    for (slot, &(at, value)) in last.iter().enumerate() {
        let kind = match slot {
            BEND => CtrlKind::PitchBend,
            PRESSURE => CtrlKind::ChannelPressure,
            PROGRAM => CtrlKind::Program,
            n => CtrlKind::Cc(u8::try_from(n).unwrap_or(0)),
        };
        if at != i64::MIN {
            inst.plugin.midi(0, ctrl_message(kind, value, 0));
        }
    }
}

/// Session audio shared with a sample-playing instrument.
struct PoolSample(Arc<SourceAudio>);

impl SampleSource for PoolSample {
    fn sample_rate(&self) -> f32 {
        self.0.buffer.sample_rate as f32
    }
    fn num_channels(&self) -> usize {
        self.0.buffer.num_channels()
    }
    fn channel(&self, ch: usize) -> Option<&[f32]> {
        self.0.channel(ch)
    }
}

/// Hands a strip's instrument the audio of its insert's sample source when it changed (a new
/// sample, a cleared one, or a new instance). Missing media plays nothing.
fn sync_sample(strip: &mut Strip, want: Option<&soundcraft_model::Insert>, s: &Session) {
    let audio = want.and_then(|i| i.sample).and_then(|id| s.pool.get(id));
    let same = match (strip.sample.as_ref(), audio) {
        (Some(a), Some(b)) => Arc::ptr_eq(a, b),
        (None, None) => true,
        _ => false,
    };
    if same {
        return;
    }
    if let Some(inst) = strip.instrument.as_mut() {
        let shared = audio.map(|a| Arc::new(PoolSample(Arc::clone(a))) as Arc<dyn SampleSource>);
        inst.plugin.set_sample(shared);
    }
    strip.sample = audio.cloned();
}

/// Record insert latency without processing audio, so this block can compensate for it.
fn note_insert_latency(t: &Track, strip: &mut Strip, pos: Samples) {
    let mut latency = 0usize;
    for (i, ins) in t.mixer.inserts.iter().enumerate() {
        let (Some(ins), Some(Some(slot))) = (ins, strip.plugins.get_mut(i)) else { continue };
        if !ins.active || ins.bypass {
            continue;
        }
        apply_params(slot, ins, t, i, pos);
        latency = latency.saturating_add(slot.plugin.latency());
    }
    strip.latency = latency.min(MAX_LATENCY);
}

fn apply_params(slot: &mut PluginSlot, ins: &soundcraft_model::Insert, t: &Track, i: usize, pos: Samples) {
    // Automation lanes address insert slots; the instrument (`INSTRUMENT_SLOT`) has none, so it
    // must not pick up slot A's lanes.
    let Ok(slot_u8) = u8::try_from(i) else {
        return apply_values(slot, ins, |_, v| v);
    };
    let automated = t.mixer.automation_mode.reads()
        && t.automation.iter().any(|l| matches!(&l.param, AutoParam::Plugin { slot, .. } if *slot == slot_u8) && !l.points.is_empty());
    apply_values(slot, ins, |k, val| {
        if automated
            && let Some(l) = t.automation.iter().find(|l| matches!(&l.param, AutoParam::Plugin { slot, param } if *slot == slot_u8 && param == k))
            && !l.points.is_empty()
        {
            return l.value_at(pos, val);
        }
        val
    });
}

/// Sends each stored param (as mapped by `value`) to the plugin when it changed since last time.
fn apply_values(slot: &mut PluginSlot, ins: &soundcraft_model::Insert, value: impl Fn(&str, f32) -> f32) {
    if slot.applied.len() != ins.params.len() {
        slot.applied = vec![f32::NAN; ins.params.len()];
    }
    for ((k, v), last) in ins.params.iter().zip(slot.applied.iter_mut()) {
        let val = value(k, *v);
        if val.to_bits() != last.to_bits() {
            slot.plugin.set_param(k, val);
            *last = val;
        }
    }
}

fn transport_at(s: &Session, pos: Samples, playing: bool, recording: bool) -> Transport {
    let sr = s.sample_rate;
    let ticks = Some(s.tempo.samples_to_ticks_f(pos, sr)).filter(|t| t.is_finite()).unwrap_or(0.0);
    let tick = ticks.floor() as i64;
    let meter = s.tempo.meter_at_tick(tick);
    let bar = s.tempo.bar_beat_at_tick(tick).bar;
    let q = TICKS_PER_QUARTER as f64;
    Transport {
        playing,
        recording,
        tempo: s.tempo.tempo_at_tick(tick),
        numerator: meter.numerator,
        denominator: meter.denominator,
        samples: pos,
        seconds: pos as f64 / sr.as_f64().max(1.0),
        quarters: ticks / q,
        bar_start: s.tempo.bar_start_tick(bar) as f64 / q,
        bar: bar - 1,
    }
}

fn volume_db_at(t: &Track, pos: Samples) -> f32 {
    if t.mixer.automation_mode.reads()
        && t.mixer.automation_mode != soundcraft_model::AutomationMode::Write
        && let Some(l) = t.lane(&AutoParam::Volume)
        && !l.points.is_empty()
    {
        return l.value_at(pos, t.mixer.volume_db);
    }
    t.mixer.volume_db
}

fn pan_at(t: &Track, idx: usize, pos: Samples) -> f32 {
    let base = t.mixer.pan.get(idx).copied().unwrap_or(if idx == 0 { 0.0 } else { 1.0 });
    if t.mixer.automation_mode.reads()
        && let Some(l) = t.lane(&AutoParam::Pan(u8::try_from(idx).unwrap_or(0)))
        && !l.points.is_empty()
    {
        return l.value_at(pos, base);
    }
    base
}

/// Pan automation is read on a grid of [`PAN_STEP`] timeline samples rather than once per block,
/// so playback (callback-sized blocks) and bounces (1024-frame blocks) pan identically.
const PAN_STEP: Samples = 64;

/// The position to read the pan at for the frames from `start` of the block at `pos`, and the end
/// of the frames that share it. Without pan automation the whole block shares one pan.
fn pan_segment(t: &Track, pos: Samples, start: usize, frames: usize) -> (Samples, usize) {
    let automated = t.mixer.automation_mode.reads() && t.automation.iter().any(|l| matches!(l.param, AutoParam::Pan(_)) && !l.points.is_empty());
    if !automated {
        return (pos, frames);
    }
    let here = pos.saturating_add(start as i64);
    let at = here.div_euclid(PAN_STEP).saturating_mul(PAN_STEP);
    let left = usize::try_from(at.saturating_add(PAN_STEP).saturating_sub(here)).unwrap_or(1).max(1);
    (at, start.saturating_add(left).min(frames))
}

fn send_level(t: &Track, i: usize, base: f32, pos: Samples) -> f32 {
    if t.mixer.automation_mode.reads()
        && let Some(l) = t.lane(&AutoParam::SendLevel(u8::try_from(i).unwrap_or(0)))
        && !l.points.is_empty()
    {
        return l.value_at(pos, base);
    }
    base
}

fn lerp_gain(db0: f32, db1: f32, i: usize, n: usize) -> f32 {
    let g0 = db_to_gain(db0);
    let g1 = db_to_gain(db1);
    if n == 0 { g0 } else { g0 + (g1 - g0) * (i as f32 / n as f32) }
}

fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0.0f32, |m, v| if v.is_finite() { m.max(v.abs()) } else { m })
}

/// Per-block routing gains: `m[src][dst]`.
type Matrix = [[f32; MAX_CHANNELS]; MAX_CHANNELS];

/// How a source is panned: the stereo pan pair (mono uses `[0]`) and the optional surround pan.
struct PanSpec<'a> {
    stereo: [f32; 2],
    surround: Option<&'a SurroundPan>,
}

fn identity_matrix(n: usize) -> Matrix {
    let mut m = [[0.0; MAX_CHANNELS]; MAX_CHANNELS];
    for (i, row) in m.iter_mut().enumerate().take(n) {
        if let Some(g) = row.get_mut(i) {
            *g = 1.0;
        }
    }
    m
}

/// ITU-R BS.775 stereo fold-down gains (to L, to R) for one speaker: C and surrounds at -3 dB,
/// the centre surround at -6 dB per side, LFE dropped.
pub fn itu_stereo_fold(sp: Speaker) -> (f32, f32) {
    use Speaker::*;
    let h = std::f32::consts::FRAC_1_SQRT_2;
    match sp {
        L | Lc | Lw => (1.0, 0.0),
        R | Rc | Rw => (0.0, 1.0),
        C => (h, h),
        Lfe => (0.0, 0.0),
        Cs => (0.5, 0.5),
        Ls | Lss | Lrs | Ltf | Ltm | Ltr => (h, 0.0),
        Rs | Rss | Rrs | Rtf | Rtm | Rtr => (0.0, h),
    }
}

/// Speaker directions of a format (for the DSP panner). Returns the count written.
fn layout_of(fmt: ChannelFormat, out: &mut [SpeakerPos; MAX_CHANNELS]) -> usize {
    let sp = fmt.speakers();
    for (o, s) in out.iter_mut().zip(sp.iter()) {
        *o = SpeakerPos { az: s.azimuth(), el: s.elevation(), lfe: s.is_lfe() };
    }
    sp.len().min(MAX_CHANNELS)
}

fn surround_params(sp: &SurroundPan, dx: f32) -> SurroundParams {
    SurroundParams { x: (sp.x + dx).clamp(-1.0, 1.0), y: sp.y, z: sp.z, divergence: sp.divergence, center: sp.center / 100.0 }
}

/// Routing gains from a source (`ns` channels of `src_fmt`) into a destination (`nd` channels of
/// `dst_fmt`). See the crate docs for the rules.
fn build_matrix(src_fmt: ChannelFormat, ns: usize, dst_fmt: ChannelFormat, nd: usize, pan: &PanSpec<'_>, law: PanLaw) -> Matrix {
    let mut m = [[0.0f32; MAX_CHANNELS]; MAX_CHANNELS];
    let (ns, nd) = (ns.min(MAX_CHANNELS), nd.min(MAX_CHANNELS));
    if ns == 0 || nd == 0 {
        return m;
    }
    let set = |m: &mut Matrix, s: usize, d: usize, g: f32| {
        if let Some(x) = m.get_mut(s).and_then(|r| r.get_mut(d)) {
            *x += g;
        }
    };
    if nd == 1 {
        // Mono destination: the stereo routing summed at -6 dB.
        let st = build_matrix(src_fmt, ns, ChannelFormat::Stereo, 2, pan, law);
        for (row, srow) in m.iter_mut().zip(st.iter()).take(ns) {
            row[0] = 0.5 * (srow[0] + srow[1]);
        }
        return m;
    }
    let src_sp = src_fmt.speakers();
    let src_amb = src_fmt.is_ambisonic() && ns >= 4;
    if nd == 2 && !dst_fmt.is_ambisonic() {
        match ns {
            1 => {
                let (gl, gr) = gains(pan.stereo[0], law);
                m[0][0] = gl;
                m[0][1] = gr;
            }
            2 if !src_amb => {
                let (a_l, a_r) = gains(pan.stereo[0], PanLaw::Zero);
                let (b_l, b_r) = gains(pan.stereo[1], PanLaw::Zero);
                m[0] = [0.0; MAX_CHANNELS];
                m[0][0] = a_l;
                m[0][1] = a_r;
                m[1][0] = b_l;
                m[1][1] = b_r;
            }
            _ if src_amb => {
                for (d, az) in [(0usize, -30.0f32), (1, 30.0)] {
                    let g = soundcraft_dsp::pan::ambisonic_decode(az, 0.0, 2);
                    for (c, gc) in g.iter().enumerate() {
                        set(&mut m, c, d, *gc);
                    }
                }
            }
            _ if src_sp.len() == ns => {
                for (s, sp) in src_sp.iter().enumerate() {
                    let (l, r) = itu_stereo_fold(*sp);
                    set(&mut m, s, 0, l);
                    set(&mut m, s, 1, r);
                }
            }
            _ => {
                // Unknown layout: first two channels to L/R, the rest to the centre.
                let h = std::f32::consts::FRAC_1_SQRT_2;
                m[0][0] = 1.0;
                m[1][1] = 1.0;
                for row in m.iter_mut().take(ns).skip(2) {
                    row[0] = h;
                    row[1] = h;
                }
            }
        }
        return m;
    }
    if ns > 2 && src_fmt == dst_fmt && ns == nd {
        return identity_matrix(ns);
    }
    // Multichannel destination.
    let mut layout = [SpeakerPos::default(); MAX_CHANNELS];
    let nl = layout_of(dst_fmt, &mut layout);
    let layout = layout.get(..nl).unwrap_or(&[]);
    let dst_sp = dst_fmt.speakers();
    let dst_amb = dst_fmt.is_ambisonic();
    let find = |sp: Speaker| dst_sp.iter().position(|x| *x == sp).filter(|i| *i < nd);
    let mut g = [0.0f32; MAX_CHANNELS];
    if dst_amb {
        if src_amb || (src_fmt.is_ambisonic() && ns > 2) {
            return identity_matrix(ns.min(nd));
        }
        let mut enc = |m: &mut Matrix, s: usize, az: f32, el: f32, spread: f32| {
            soundcraft_dsp::pan::ambisonic_encode(az, el, spread, &mut g);
            for (d, gd) in g.iter().enumerate().take(nd) {
                set(m, s, d, *gd);
            }
        };
        match (ns, pan.surround) {
            (1 | 2, Some(sp)) => {
                for s in 0..ns {
                    let dx = if ns == 2 { if s == 0 { -1.0 } else { 1.0 } } else { 0.0 };
                    let p = surround_params(sp, dx);
                    enc(&mut m, s, soundcraft_dsp::pan::puck_azimuth(&[], p.x, p.y), p.z * 90.0, p.divergence);
                }
            }
            (1, None) => enc(&mut m, 0, pan.stereo[0].clamp(-1.0, 1.0) * 30.0, 0.0, 0.0),
            (2, None) => {
                enc(&mut m, 0, -30.0, 0.0, 0.0);
                enc(&mut m, 1, 30.0, 0.0, 0.0);
            }
            _ => {
                for (s, sp) in src_sp.iter().enumerate().take(ns) {
                    if !sp.is_lfe() {
                        enc(&mut m, s, sp.azimuth(), sp.elevation(), 0.0);
                    }
                }
            }
        }
        return m;
    }
    if dst_sp.len() != nd {
        // No layout to pan on: channel for channel.
        return identity_matrix(ns.min(nd));
    }
    let (li, ri) = (find(Speaker::L), find(Speaker::R));
    match (ns, pan.surround) {
        (1 | 2, Some(sp)) if !src_amb => {
            for s in 0..ns {
                let dx = if ns == 2 { if s == 0 { -1.0 } else { 1.0 } } else { 0.0 };
                soundcraft_dsp::pan::surround_gains(layout, &surround_params(sp, dx), &mut g);
                for (d, gd) in g.iter().enumerate().take(nd) {
                    set(&mut m, s, d, *gd);
                }
                if let Some(lfe) = find(Speaker::Lfe)
                    && sp.lfe_db > -143.9
                {
                    set(&mut m, s, lfe, db_to_gain(sp.lfe_db.min(12.0)) / ns as f32);
                }
            }
        }
        (1, None) => {
            let (gl, gr) = gains(pan.stereo[0], law);
            if let (Some(l), Some(r)) = (li, ri) {
                set(&mut m, 0, l, gl);
                set(&mut m, 0, r, gr);
            }
        }
        (2, None) if !src_amb => {
            let (a_l, a_r) = gains(pan.stereo[0], PanLaw::Zero);
            let (b_l, b_r) = gains(pan.stereo[1], PanLaw::Zero);
            if let (Some(l), Some(r)) = (li, ri) {
                set(&mut m, 0, l, a_l);
                set(&mut m, 0, r, a_r);
                set(&mut m, 1, l, b_l);
                set(&mut m, 1, r, b_r);
            }
        }
        _ if src_amb => {
            let n_spk = dst_sp.iter().filter(|s| !s.is_lfe()).count();
            for (d, sp) in dst_sp.iter().enumerate().take(nd) {
                if sp.is_lfe() {
                    continue;
                }
                let dec = soundcraft_dsp::pan::ambisonic_decode(sp.azimuth(), sp.elevation(), n_spk);
                for (c, gc) in dec.iter().enumerate() {
                    set(&mut m, c, d, *gc);
                }
            }
        }
        _ if src_sp.len() == ns => {
            for (s, sp) in src_sp.iter().enumerate() {
                if let Some(d) = find(*sp) {
                    set(&mut m, s, d, 1.0);
                } else if sp.is_lfe() {
                    // No LFE in the destination: dropped.
                } else {
                    soundcraft_dsp::pan::direction_gains(layout, sp.azimuth(), sp.elevation(), &mut g);
                    for (d, gd) in g.iter().enumerate().take(nd) {
                        set(&mut m, s, d, *gd);
                    }
                }
            }
        }
        _ => return identity_matrix(ns.min(nd)),
    }
    m
}

/// `dst[d] += Σ_s src[s] × gain × m[s][d]` for every destination channel (no allocation).
fn apply_matrix(src: &[Vec<f32>], frames: usize, dst: &mut [Vec<f32>], m: &Matrix, gain: f32) {
    apply_matrix_span(src, 0..frames, dst, m, gain);
}

/// [`apply_matrix`] over the frames in `span` only.
fn apply_matrix_span(src: &[Vec<f32>], span: std::ops::Range<usize>, dst: &mut [Vec<f32>], m: &Matrix, gain: f32) {
    let ns = src.len().min(MAX_CHANNELS);
    for (d, out) in dst.iter_mut().enumerate().take(MAX_CHANNELS) {
        let mut idx = [(0usize, 0.0f32); MAX_CHANNELS];
        let mut k = 0;
        for (s, row) in m.iter().enumerate().take(ns) {
            let g = row.get(d).copied().unwrap_or(0.0);
            if g != 0.0
                && let Some(slot) = idx.get_mut(k)
            {
                *slot = (s, g);
                k += 1;
            }
        }
        let taps = idx.get(..k).unwrap_or(&[]);
        if taps.is_empty() {
            continue;
        }
        for (i, o) in out.iter_mut().enumerate().take(span.end).skip(span.start) {
            let mut acc = 0.0f32;
            for &(s, g) in taps {
                acc += src.get(s).and_then(|c| c.get(i)).copied().unwrap_or(0.0) * gain * g;
            }
            *o += acc;
        }
    }
}

/// Ceiling on one in-memory offline render (samples summed over channels): 2^29 f32 = 2 GiB, about
/// 93 minutes of stereo at 48 kHz. [`render_range`] and [`render_bus`] truncate longer requests;
/// [`try_render_range`] and [`try_render_bus`] refuse them; [`RangeRender`] streams any length.
pub const MAX_RENDER_SAMPLES: usize = 1 << 29;

/// A render request too large to hold in memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderTooLong {
    /// Frames asked for.
    pub requested: u64,
    /// Most frames one render of this channel count may produce.
    pub max: u64,
}

impl std::fmt::Display for RenderTooLong {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} frames requested but one render holds at most {}; render it in shorter ranges", self.requested, self.max)
    }
}

impl std::error::Error for RenderTooLong {}

fn check_render_len(range: Range, channels: usize) -> Result<(), RenderTooLong> {
    let requested = u64::try_from(range.len().max(0)).unwrap_or(0);
    let max = (MAX_RENDER_SAMPLES / channels.max(1)) as u64;
    if requested > max { Err(RenderTooLong { requested, max }) } else { Ok(()) }
}

/// [`render_range`], but an oversize request is an error instead of a silently shortened result.
pub fn try_render_range(s: &Session, range: Range, block: usize) -> Result<Vec<Vec<f32>>, RenderTooLong> {
    check_render_len(range, main_channels(s))?;
    Ok(render_range(s, range, block))
}

/// [`render_bus`], but an oversize request is an error instead of a silently shortened result.
pub fn try_render_bus(s: &Session, bus: BusId, range: Range, block: usize) -> Result<Vec<Vec<f32>>, RenderTooLong> {
    check_render_len(range, s.bus(bus).map_or(1, |b| b.format.channels()))?;
    Ok(render_bus(s, bus, range, block))
}

/// Offline render of the main mix over `range` (planar, [`main_channels`] channels in SMPTE/WAV
/// order). The mix's delay-compensation latency is removed, so the result lines up with the
/// timeline.
pub fn render_range(s: &Session, range: Range, block: usize) -> Vec<Vec<f32>> {
    render_in_memory(s, None, range, block)
}

/// Offline render of one bus over `range` (planar, the bus format's channel count).
/// The compensation delay written onto the bus is removed (track inserts, and the aux returns
/// when an aux returns onto it), so the bus lines up with the timeline. An unknown bus is silent.
pub fn render_bus(s: &Session, bus: BusId, range: Range, block: usize) -> Vec<Vec<f32>> {
    render_in_memory(s, Some(bus), range, block)
}

fn render_in_memory(s: &Session, bus: Option<BusId>, range: Range, block: usize) -> Vec<Vec<f32>> {
    let nch = source_channels(s, bus);
    let len = usize::try_from(range.len().max(0)).unwrap_or(0).min(MAX_RENDER_SAMPLES / nch.max(1));
    let mut out: Vec<Vec<f32>> = (0..nch).map(|_| Vec::with_capacity(len)).collect();
    let mut render = RangeRender::new(s, bus, Range::new(range.start, range.start + len as i64), block);
    while let Some((chunk, n)) = render.next_block(s) {
        for (o, c) in out.iter_mut().zip(chunk) {
            o.extend_from_slice(c.get(..n).unwrap_or_default());
        }
    }
    out
}

/// Channels of the main mix, or of `bus` (none for an unknown bus).
pub fn source_channels(s: &Session, bus: Option<BusId>) -> usize {
    match bus {
        Some(id) => s.bus(id).map_or(0, |b| b.format.channels()),
        None => main_channels(s),
    }
}

/// [`render_range`] or [`render_bus`] of any length, a block at a time.
pub struct RangeRender {
    eng: MixEngine,
    bus: Option<BusId>,
    tmp: Vec<Vec<f32>>,
    start: Samples,
    done: usize,
    total: usize,
    latency: usize,
}

impl RangeRender {
    /// Renders the main mix over `range`, or `bus` when one is named.
    pub fn new(s: &Session, bus: Option<BusId>, range: Range, block: usize) -> Self {
        let nch = source_channels(s, bus);
        let eng = MixEngine::new(s.sample_rate.as_f64() as f32, block);
        let tmp = vec![vec![0.0f32; eng.max_block()]; nch];
        let len = if nch == 0 { 0 } else { usize::try_from(range.len().max(0)).unwrap_or(0) };
        RangeRender { eng, bus, tmp, start: range.start, done: 0, total: len, latency: 0 }
    }

    fn render(&mut self, s: &Session, pos: Samples, n: usize) {
        match self.bus {
            Some(id) => {
                self.eng.render(s, pos, n, &mut []);
                self.eng.copy_bus(id, n, &mut self.tmp);
            }
            None => self.eng.render(s, pos, n, &mut self.tmp),
        }
    }

    /// Renders one more block of the mix of `s` (the session given to [`RangeRender::new`]) and
    /// returns it as `(channels, n)`: the next `n` samples of the range lead each channel (none
    /// while the mix's latency is still being skipped). `None` once the range is done.
    pub fn next_block(&mut self, s: &Session) -> Option<(&[Vec<f32>], usize)> {
        if self.done >= self.total {
            return None;
        }
        let n = (self.total - self.done).min(self.eng.max_block());
        self.render(s, self.start + self.done as i64, n);
        if self.done == 0 {
            // The first block tells the mix's latency: render that much more and drop the head.
            self.latency = match self.bus {
                Some(b) => self.eng.bus_latency(b),
                None => self.eng.latency,
            };
            self.total = self.total.saturating_add(self.latency);
        }
        let skip = self.latency.saturating_sub(self.done).min(n);
        self.done += n;
        if skip > 0 && skip < n {
            for c in &mut self.tmp {
                c.copy_within(skip..n, 0);
            }
        }
        Some((&self.tmp, n - skip))
    }
}

/// ITU-R BS.775 stereo fold-down of planar audio in `fmt` (C and surrounds at -3 dB, LFE dropped;
/// Ambisonics play their W channel). Mono/stereo input is returned unchanged.
pub fn fold_down_stereo(fmt: ChannelFormat, ch: &[Vec<f32>]) -> Vec<Vec<f32>> {
    if ch.len() <= 2 {
        return ch.to_vec();
    }
    let len = ch.iter().map(Vec::len).max().unwrap_or(0);
    let mut out = vec![vec![0.0f32; len]; 2];
    let sp = fmt.speakers();
    let h = std::f32::consts::FRAC_1_SQRT_2;
    for (c, src) in ch.iter().enumerate() {
        let (gl, gr) = match sp.get(c) {
            Some(x) if sp.len() == ch.len() => itu_stereo_fold(*x),
            _ if c == 0 => (h, h),
            _ => (0.0, 0.0),
        };
        for (g, dst) in [(gl, 0usize), (gr, 1)] {
            if g == 0.0 {
                continue;
            }
            if let Some(d) = out.get_mut(dst) {
                for (o, x) in d.iter_mut().zip(src.iter()) {
                    *o += x * g;
                }
            }
        }
    }
    out
}

/// Render only one track's clips (no mixer processing) — Consolidate.
pub fn render_clips(s: &Session, track: TrackId, range: Range) -> Vec<Vec<f32>> {
    let Some(t) = s.track(track) else { return Vec::new() };
    let len = usize::try_from(range.len().max(0)).unwrap_or(0).min(MAX_RENDER_SAMPLES / t.channels().max(1));
    let mut out = vec![vec![0.0f32; len]; t.channels().max(1)];
    let mut scratch = Vec::new();
    for c in t.clips().iter().filter(|c| !c.muted && c.range().overlaps(&range)) {
        match clip_fx_params(s, c.id.0) {
            None => render_clip(s, c, range.start, len, &mut out, &mut scratch),
            Some(params) => {
                let mut tmp = vec![vec![0.0f32; len]; out.len()];
                render_clip(s, c, range.start, len, &mut tmp, &mut scratch);
                let mut fx = ClipFx::new(s.sample_rate.as_f64() as f32, tmp.len());
                fx.configure(params);
                let mut done = 0;
                while done < len {
                    let n = (len - done).min(4096);
                    let mut block: Vec<Vec<f32>> = tmp.iter().map(|ch| ch.get(done..done + n).map(<[f32]>::to_vec).unwrap_or_default()).collect();
                    fx.process(&mut block, n);
                    for (o, b) in out.iter_mut().zip(block.iter()) {
                        if let Some(dst) = o.get_mut(done..done + n) {
                            for (x, y) in dst.iter_mut().zip(b.iter()) {
                                *x += *y;
                            }
                        }
                    }
                    done += n;
                }
            }
        }
    }
    out
}

/// Render one track through its inserts (pre-fader) — Commit / Freeze.
pub fn render_track_pre_fader(s: &Session, track: TrackId, range: Range) -> Vec<Vec<f32>> {
    let mut solo = s.clone();
    // Multichannel tracks render into a main mix of their own format (straight through).
    let fmt = s.track(track).map_or(ChannelFormat::Stereo, |t| if t.channels() > 2 { t.format } else { ChannelFormat::Stereo });
    solo.outputs = vec![soundcraft_model::OutputPath { name: "Out".into(), first_channel: 0, format: fmt }];
    for t in &mut solo.tracks {
        if t.id == track {
            t.mixer.volume_db = 0.0;
            t.mixer.mute = false;
            t.mixer.solo = false;
            t.mixer.output = Route::Main;
            t.mixer.pan = match t.channels() {
                1 => vec![0.0],
                _ => vec![-1.0, 1.0],
            };
            t.automation.retain(|l| matches!(l.param, AutoParam::Plugin { .. }));
            t.mixer.sends.iter_mut().for_each(|x| *x = None);
            t.mixer.surround = None;
        } else {
            t.inactive = true;
        }
    }
    let mut out = render_range(&solo, range, 1024);
    // Mono tracks: fold back to one channel (undo the centre pan law).
    if s.track(track).is_some_and(|t| t.channels() == 1)
        && let Some(l) = out.first().cloned()
    {
        let g = std::f32::consts::SQRT_2;
        out = vec![l.iter().map(|x| x * g).collect()];
    }
    out
}

/// A built-in plugin, else a hosted CLAP plugin (`clap:<id>`).
fn create_plugin(id: &str) -> Option<Box<dyn Plugin>> {
    soundcraft_dsp::create(id)
        .or_else(|| soundcraft_clap_host::create(id))
        .or_else(|| soundcraft_vst3_host::create(id))
        .or_else(|| soundcraft_au_host::create(id))
}

#[cfg(test)]
mod audio_thread_tests {
    use super::*;

    #[test]
    fn only_a_marked_thread_is_the_audio_thread() {
        assert!(!on_audio_thread());
        let marked = std::thread::spawn(|| {
            let before = on_audio_thread();
            mark_audio_thread();
            (before, on_audio_thread())
        })
        .join()
        .unwrap();
        assert_eq!(marked, (false, true));
        assert!(!on_audio_thread(), "marking another thread leaves this one alone");
    }

    /// What `on_audio_thread` says inside each strip job, for strips run from a thread that is
    /// (or is not) the audio thread.
    fn seen_by_strips(audio: bool) -> Vec<bool> {
        std::thread::spawn(move || {
            if audio {
                mark_audio_thread();
            }
            let mut work = vec![None; 64];
            run_strips(&mut work, true, |w| {
                // Long enough that the pool's workers take part.
                std::thread::sleep(std::time::Duration::from_millis(1));
                *w = Some(on_audio_thread());
            });
            work.into_iter().map(|w| w.unwrap()).collect()
        })
        .join()
        .unwrap()
    }

    #[test]
    fn rendering_through_the_realtime_pool_matches_serial_rendering_exactly() {
        prepare_realtime();
        use soundcraft_audio_io::AudioBuffer;
        use soundcraft_model::{ClipId, Insert, SourceAudio, SourceId};
        let mut s = Session::default();
        s.pool.insert(
            SourceId(1),
            std::sync::Arc::new(SourceAudio::new(AudioBuffer {
                sample_rate: 48000,
                channels: vec![(0..8192).map(|i| (i as f32 * 0.02).sin() * 0.3).collect()],
            })),
        );
        for k in 0..24u64 {
            let t = s.add_track(TrackKind::Audio, ChannelFormat::Mono, None);
            let tr = s.track_mut(t).unwrap();
            tr.playlist_mut().unwrap().clips.push(Clip::audio(ClipId(500 + k), "c", SourceId(1), 0, 0, 8192));
            tr.mixer.inserts[0] = Some(Insert::new("eq_7band"));
            tr.mixer.pan = vec![(k as f32 / 12.0) - 1.0, 0.0];
        }
        let s = std::sync::Arc::new(s);
        let render = |mode: RealtimeParallel| {
            let s = std::sync::Arc::clone(&s);
            std::thread::spawn(move || {
                mark_audio_thread();
                set_realtime_parallel(mode);
                let mut m = MixEngine::new(48000., 512);
                let mut all = Vec::new();
                for b in 0..16 {
                    let mut out = vec![vec![0.0f32; 512]; 2];
                    m.render(&s, b * 512, 512, &mut out);
                    all.extend(out.into_iter().flatten());
                }
                all
            })
            .join()
            .unwrap()
        };
        let serial = render(RealtimeParallel::Off);
        let pooled = render(RealtimeParallel::On);
        set_realtime_parallel(RealtimeParallel::Adaptive);
        assert!(serial.iter().any(|x| x.abs() > 0.01), "the mix is not silent");
        assert_eq!(serial, pooled, "the pool changes who renders a strip, never what it renders");
    }

    #[test]
    fn the_policy_stays_serial_while_the_mix_is_cheap() {
        let mut p = ParallelPolicy::new();
        for _ in 0..10_000 {
            p.observe(0.002, 0.0107); // ~19 % of the budget
            assert!(!p.use_parallel(RealtimeParallel::Adaptive));
        }
    }

    #[test]
    fn the_policy_engages_the_pool_under_sustained_load_and_backs_off_when_it_overruns() {
        let mut p = ParallelPolicy::new();
        let budget = 0.0107;
        for _ in 0..200 {
            p.observe(0.8 * budget, budget);
        }
        assert!(p.use_parallel(RealtimeParallel::Adaptive), "80 % serial load switches to the pool");
        // The pool starts missing deadlines: it is abandoned...
        for _ in 0..10 {
            p.observe(2.0 * budget, budget);
        }
        assert!(!p.use_parallel(RealtimeParallel::Adaptive), "repeated overruns return to serial");
        // ...and not retried during the cool-down, however loaded serial looks.
        for _ in 0..100 {
            p.observe(0.8 * budget, budget);
            assert!(!p.use_parallel(RealtimeParallel::Adaptive));
        }
        // After the cool-down it may try again.
        for _ in 0..600 {
            p.observe(0.8 * budget, budget);
        }
        assert!(p.use_parallel(RealtimeParallel::Adaptive));
    }

    #[test]
    fn a_single_slow_block_does_not_flip_the_policy_and_the_modes_override_it() {
        let mut p = ParallelPolicy::new();
        for _ in 0..100 {
            p.observe(0.002, 0.0107);
        }
        p.observe(0.05, 0.0107); // one hiccup
        assert!(!p.use_parallel(RealtimeParallel::Adaptive));
        assert!(p.use_parallel(RealtimeParallel::On));
        let mut busy = ParallelPolicy::new();
        for _ in 0..500 {
            busy.observe(0.01, 0.0107);
        }
        assert!(!busy.use_parallel(RealtimeParallel::Off), "Off never uses the pool");
    }

    #[test]
    fn strips_rendered_for_the_audio_thread_count_as_the_audio_thread_and_only_then() {
        assert!(seen_by_strips(true).iter().all(|&rt| rt), "a worker rendering for the audio thread is on it");
        assert!(seen_by_strips(false).iter().all(|&rt| !rt), "the workers don't stay marked afterwards");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soundcraft_audio_io::AudioBuffer;
    use soundcraft_model::{ClipId, Insert, SourceAudio, SourceId};
    use std::sync::Arc;

    fn impulse_session() -> (Session, TrackId) {
        let mut s = Session::default();
        let t = s.add_track(TrackKind::Audio, ChannelFormat::Mono, None);
        let mut impulses = vec![0.0; 2048];
        for i in [0, 1024, 2000] {
            impulses[i] = 0.25;
        }
        s.pool.insert(SourceId(100), Arc::new(SourceAudio::new(AudioBuffer { sample_rate: 48000, channels: vec![impulses] })));
        s.track_mut(t).unwrap().playlist_mut().unwrap().clips.push(Clip::audio(ClipId(101), "imp", SourceId(100), 0, 0, 2048));
        (s, t)
    }

    fn impulse_positions(s: &Session) -> Vec<usize> {
        render_range(s, Range::new(0, 2048), 512)[0].iter().enumerate().filter(|(_, x)| x.abs() > 1e-4).map(|(i, _)| i).collect()
    }

    fn lookahead_compressor() -> Insert {
        let mut c = Insert::new("compressor");
        c.params.insert("lookahead".into(), 5.0);
        c.params.insert("ratio".into(), 1.0);
        c
    }

    /// `n` auxes where aux `k` reads bus `k` and writes bus `k + 1`, created in reverse so the
    /// chain runs against track order.
    fn aux_chain(n: usize) -> (Session, Vec<TrackId>) {
        let mut s = Session::default();
        let busses: Vec<_> = (0..=n).map(|k| s.add_bus(&format!("B{k}"), ChannelFormat::Stereo)).collect();
        let auxes: Vec<_> = (0..n).map(|k| s.add_track(TrackKind::Aux, ChannelFormat::Stereo, Some(&format!("A{k}")))).collect();
        for k in 0..n {
            let t = s.track_mut(auxes[n - 1 - k]).unwrap();
            t.mixer.input = Route::Bus(busses[k]);
            t.mixer.output = Route::Bus(busses[k + 1]);
        }
        let chain: Vec<_> = (0..n).map(|k| auxes[n - 1 - k]).collect();
        (s, chain)
    }

    #[test]
    fn a_long_aux_chain_is_ordered_producers_first() {
        let (s, chain) = aux_chain(24);
        let order: Vec<TrackId> =
            processing_order(&s).into_iter().filter_map(|i| s.tracks.get(i)).filter(|t| t.kind == TrackKind::Aux).map(|t| t.id).collect();
        assert_eq!(order, chain, "deeper than the old eight-level cap, against track order");
        assert!(routing_cycle(&s).is_none());
    }

    #[test]
    fn routing_loops_are_found_and_the_mix_still_renders_them() {
        let (mut s, chain) = aux_chain(3);
        assert!(routing_cycle(&s).is_none());
        // The last aux feeds the first one's input bus: a loop through all three.
        let first_input = s.track(chain[0]).unwrap().mixer.input.clone();
        s.track_mut(chain[2]).unwrap().mixer.output = first_input;
        let cycle = routing_cycle(&s).expect("a loop");
        assert_eq!(cycle.len(), 3);
        assert!(chain.iter().all(|t| cycle.contains(t)));
        // A loaded file may contain one: it renders (the loop's closing edge is cut) without hanging.
        let mut m = MixEngine::new(48000., 512);
        let mut out = vec![vec![0.; 512]; 2];
        m.render(&s, 0, 512, &mut out);
        // An aux feeding itself is the smallest loop.
        let (mut s2, c2) = aux_chain(1);
        let own = s2.track(c2[0]).unwrap().mixer.input.clone();
        s2.track_mut(c2[0]).unwrap().mixer.output = own;
        assert_eq!(routing_cycle(&s2), Some(vec![c2[0]]));
        // Auxes downstream of a loop are not reported as part of it.
        let (mut s3, c3) = aux_chain(3);
        let in0 = s3.track(c3[0]).unwrap().mixer.input.clone();
        s3.track_mut(c3[1]).unwrap().mixer.output = in0;
        let cycle = routing_cycle(&s3).expect("a loop between the first two");
        assert_eq!(cycle.len(), 2);
        assert!(!cycle.contains(&c3[2]));
    }

    #[test]
    fn two_auxes_feeding_a_third_are_both_ordered_before_it() {
        let mut s = Session::default();
        let b = s.add_bus("Shared", ChannelFormat::Stereo);
        let sink = s.add_track(TrackKind::Aux, ChannelFormat::Stereo, Some("Sink"));
        let (x, y) = (s.add_track(TrackKind::Aux, ChannelFormat::Stereo, Some("X")), s.add_track(TrackKind::Aux, ChannelFormat::Stereo, Some("Y")));
        s.track_mut(sink).unwrap().mixer.input = Route::Bus(b);
        s.track_mut(x).unwrap().mixer.output = Route::Bus(b);
        s.track_mut(y).unwrap().mixer.output = Route::Bus(b);
        let order: Vec<TrackId> = processing_order(&s).into_iter().filter_map(|i| s.tracks.get(i)).map(|t| t.id).collect();
        let pos = |t| order.iter().position(|&o| o == t).unwrap();
        assert!(pos(x) < pos(sink) && pos(y) < pos(sink));
    }

    #[test]
    fn soloing_a_track_keeps_every_aux_in_its_chain_audible() {
        let (mut s, t) = impulse_session();
        let b1 = s.add_bus("B1", ChannelFormat::Stereo);
        let b2 = s.add_bus("B2", ChannelFormat::Stereo);
        let a1 = s.add_track(TrackKind::Aux, ChannelFormat::Stereo, Some("Aux1"));
        let a2 = s.add_track(TrackKind::Aux, ChannelFormat::Stereo, Some("Aux2"));
        let cycle = s.add_track(TrackKind::Aux, ChannelFormat::Stereo, Some("Loop"));
        s.track_mut(t).unwrap().mixer.output = Route::Bus(b1);
        s.track_mut(a1).unwrap().mixer.input = Route::Bus(b1);
        s.track_mut(a1).unwrap().mixer.output = Route::Bus(b2);
        s.track_mut(a2).unwrap().mixer.input = Route::Bus(b2);
        // A feedback pair that must not hang the walk.
        s.track_mut(cycle).unwrap().mixer.input = Route::Bus(b2);
        s.track_mut(cycle).unwrap().mixer.output = Route::Bus(b2);
        let level = |s: &Session| render_range(s, Range::new(0, 2048), 512)[0].iter().fold(0.0f32, |m, x| m.max(x.abs()));
        let normal = level(&s);
        s.track_mut(t).unwrap().mixer.solo = true;
        assert!((level(&s) - normal).abs() < 1e-6 && normal > 0.1, "{normal} vs {}", level(&s));
    }

    #[test]
    fn an_oversize_render_is_an_error_not_a_shortened_success() {
        let (s, _) = impulse_session();
        let hours = s.sample_rate.samples(2.0 * 3600.0);
        let e = try_render_range(&s, Range::new(0, hours), 512).unwrap_err();
        assert_eq!(e.requested, hours as u64);
        assert!(try_render_range(&s, Range::new(0, 2048), 512).is_ok());
    }

    #[test]
    fn master_insert_latency_is_removed_from_a_bounce() {
        let (mut s, _) = impulse_session();
        assert_eq!(impulse_positions(&s), vec![0, 1024, 2000]);
        let m = s.add_track(TrackKind::Master, ChannelFormat::Stereo, None);
        s.track_mut(m).unwrap().mixer.inserts[0] = Some(lookahead_compressor());
        assert_eq!(impulse_positions(&s), vec![0, 1024, 2000]);
    }

    #[test]
    fn serial_aux_latency_accumulates_and_is_removed_from_a_bounce() {
        let (mut s, t) = impulse_session();
        let b1 = s.add_bus("B1", ChannelFormat::Stereo);
        let b2 = s.add_bus("B2", ChannelFormat::Stereo);
        let a1 = s.add_track(TrackKind::Aux, ChannelFormat::Stereo, Some("Aux1"));
        let a2 = s.add_track(TrackKind::Aux, ChannelFormat::Stereo, Some("Aux2"));
        s.track_mut(t).unwrap().mixer.output = Route::Bus(b1);
        s.track_mut(a1).unwrap().mixer.input = Route::Bus(b1);
        s.track_mut(a1).unwrap().mixer.output = Route::Bus(b2);
        s.track_mut(a2).unwrap().mixer.input = Route::Bus(b2);
        for a in [a1, a2] {
            s.track_mut(a).unwrap().mixer.inserts[0] = Some(lookahead_compressor());
        }
        assert_eq!(impulse_positions(&s), vec![0, 1024, 2000]);
    }

    #[test]
    fn a_direct_track_lines_up_with_a_serial_aux_chain() {
        let (mut s, _) = impulse_session();
        let direct = s.add_track(TrackKind::Audio, ChannelFormat::Mono, None);
        s.track_mut(direct).unwrap().playlist_mut().unwrap().clips.push(Clip::audio(ClipId(102), "imp", SourceId(100), 0, 0, 2048));
        let b1 = s.add_bus("B1", ChannelFormat::Stereo);
        let b2 = s.add_bus("B2", ChannelFormat::Stereo);
        let a1 = s.add_track(TrackKind::Aux, ChannelFormat::Stereo, Some("Aux1"));
        let a2 = s.add_track(TrackKind::Aux, ChannelFormat::Stereo, Some("Aux2"));
        s.tracks[0].mixer.output = Route::Bus(b1);
        s.track_mut(a1).unwrap().mixer.input = Route::Bus(b1);
        s.track_mut(a1).unwrap().mixer.output = Route::Bus(b2);
        s.track_mut(a2).unwrap().mixer.input = Route::Bus(b2);
        for a in [a1, a2] {
            s.track_mut(a).unwrap().mixer.inserts[0] = Some(lookahead_compressor());
        }
        // Both tracks carry the same impulses; every copy must land on the same samples.
        assert_eq!(impulse_positions(&s), vec![0, 1024, 2000]);
    }

    #[test]
    fn reset_clears_audio_held_in_compensation_delays() {
        let mut s = Session::default();
        let dry = s.add_track(TrackKind::Audio, ChannelFormat::Mono, None);
        let latent = s.add_track(TrackKind::Audio, ChannelFormat::Mono, None);
        s.pool.insert(SourceId(100), Arc::new(SourceAudio::new(AudioBuffer { sample_rate: 48000, channels: vec![vec![0.5; 512]] })));
        s.track_mut(dry).unwrap().playlist_mut().unwrap().clips.push(Clip::audio(ClipId(101), "old", SourceId(100), 0, 0, 512));
        s.track_mut(latent).unwrap().mixer.inserts[0] = Some(lookahead_compressor());
        let mut m = MixEngine::new(48000., 512);
        let mut out = vec![vec![0.; 512]; 2];
        m.render(&s, 0, 512, &mut out);
        m.reset();
        m.render(&s, 4096, 512, &mut out);
        assert_eq!(out[0].iter().filter(|x| x.abs() > 0.1).count(), 0);
    }

    #[test]
    fn a_seek_clears_audio_held_in_compensation_delays() {
        let mut s = Session::default();
        let dry = s.add_track(TrackKind::Audio, ChannelFormat::Mono, None);
        let latent = s.add_track(TrackKind::Audio, ChannelFormat::Mono, None);
        s.pool.insert(SourceId(100), Arc::new(SourceAudio::new(AudioBuffer { sample_rate: 48000, channels: vec![vec![0.5; 512]] })));
        s.track_mut(dry).unwrap().playlist_mut().unwrap().clips.push(Clip::audio(ClipId(101), "old", SourceId(100), 0, 0, 512));
        s.track_mut(latent).unwrap().mixer.inserts[0] = Some(lookahead_compressor());
        let mut m = MixEngine::new(48000., 512);
        let mut out = vec![vec![0.; 512]; 2];
        m.render(&s, 0, 512, &mut out);
        m.render(&s, 4096, 512, &mut out);
        assert_eq!(out[0].iter().filter(|x| x.abs() > 0.1).count(), 0);
    }

    /// Wraps a plugin and counts the note events it receives.
    struct NoteCounter {
        inner: Box<dyn Plugin>,
        on: Arc<std::sync::atomic::AtomicUsize>,
        off: Arc<std::sync::atomic::AtomicUsize>,
    }
    impl Plugin for NoteCounter {
        fn info(&self) -> &'static soundcraft_dsp::PluginInfo {
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
            self.on.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        fn note_off(&mut self, _: usize, _: u8) {
            self.off.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    }

    /// Plays `notes` (start, length in ticks) from a clip of `clip_len` samples through eight
    /// 512-sample blocks; returns (note-ons, note-offs) the instrument received.
    fn note_events(notes: &[(i64, i64, u8)], clip_len: i64) -> (usize, usize) {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let mut s = Session::default();
        let t = s.add_track(TrackKind::Instrument, ChannelFormat::Stereo, None);
        let mut seq = soundcraft_midi::Sequence::default();
        for &(start, length, pitch) in notes {
            seq.notes.push(soundcraft_midi::Note { start, length, pitch, velocity: 100, release_velocity: 0, channel: 0 });
        }
        let tr = s.track_mut(t).unwrap();
        tr.instrument = Some(Insert::new("clap:test.notes"));
        tr.playlist_mut().unwrap().clips.push(Clip::midi(ClipId(100), "n", 0, clip_len, seq));
        let (on, off) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
        let mut plugin = NoteCounter { inner: soundcraft_dsp::create("gain").unwrap(), on: on.clone(), off: off.clone() };
        plugin.prepare(48000., 512, 2);
        let mut m = MixEngine::new(48000., 512);
        m.set_external_instances(true);
        m.adopt(vec![PreparedInstance { track: t, slot: INSTRUMENT_SLOT, id: "clap:test.notes".into(), ch: 2, plugin: Box::new(plugin) }]);
        let mut out = vec![vec![0.; 512]; 2];
        for i in 0..8 {
            m.render(&s, i * 512, 512, &mut out);
        }
        (on.load(Ordering::SeqCst), off.load(Ordering::SeqCst))
    }

    /// Counts the raw MIDI messages an instrument receives.
    struct MidiCounter(Box<dyn Plugin>, Arc<std::sync::atomic::AtomicUsize>);
    impl Plugin for MidiCounter {
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
        fn process(&mut self, io: &mut [Vec<f32>], n: usize) {
            self.0.process(io, n);
        }
        fn midi(&mut self, _offset: usize, msg: [u8; 3]) {
            // Notes only: the engine's own controller chase is not what is being counted.
            if matches!(msg[0] & 0xF0, 0x80 | 0x90) {
                self.1.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
        }
    }

    #[test]
    fn live_midi_plays_the_instruments_of_armed_or_monitored_tracks_only() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let mut s = Session::default();
        let (armed, monitored, idle) = (
            s.add_track(TrackKind::Instrument, ChannelFormat::Stereo, None),
            s.add_track(TrackKind::Instrument, ChannelFormat::Stereo, None),
            s.add_track(TrackKind::Instrument, ChannelFormat::Stereo, None),
        );
        for t in [armed, monitored, idle] {
            s.track_mut(t).unwrap().instrument = Some(Insert::new("clap:test.midi"));
        }
        s.track_mut(armed).unwrap().mixer.record_arm = true;
        s.track_mut(monitored).unwrap().mixer.input_monitor = true;
        let counts: Vec<Arc<AtomicUsize>> = (0..3).map(|_| Arc::new(AtomicUsize::new(0))).collect();
        let mut m = MixEngine::new(48000., 512);
        m.set_external_instances(true);
        m.adopt(
            [armed, monitored, idle]
                .iter()
                .zip(&counts)
                .map(|(t, c)| {
                    let mut p = MidiCounter(soundcraft_dsp::create("gain").unwrap(), Arc::clone(c));
                    p.prepare(48000., 512, 2);
                    PreparedInstance { track: *t, slot: INSTRUMENT_SLOT, id: "clap:test.midi".into(), ch: 2, plugin: Box::new(p) }
                })
                .collect(),
        );
        let mut out = vec![vec![0.; 512]; 2];
        // Stopped (monitor-only), as when playing a keyboard with the transport idle.
        m.monitor_only = true;
        m.live_midi.extend([[0x90, 60, 100], [0x80, 60, 0]]);
        m.render(&s, 0, 512, &mut out);
        m.live_midi.clear();
        let got: Vec<usize> = counts.iter().map(|c| c.load(Ordering::SeqCst)).collect();
        assert_eq!(got, vec![2, 2, 0], "armed and monitored tracks hear the keyboard; an idle one does not");
        // Nothing left over: the next block without new messages sends nothing.
        m.render(&s, 0, 512, &mut out);
        assert_eq!(counts.iter().map(|c| c.load(Ordering::SeqCst)).collect::<Vec<_>>(), vec![2, 2, 0]);
    }

    fn external_session() -> (Session, TrackId) {
        let mut s = Session::default();
        let t = s.add_track(TrackKind::Midi, ChannelFormat::Mono, None);
        let mut seq = soundcraft_midi::Sequence::default();
        // 120 bpm, 960 ticks per quarter: 25 samples per tick at 48 kHz.
        seq.notes.push(soundcraft_midi::Note { start: 0, length: 40, pitch: 60, velocity: 100, release_velocity: 0, channel: 2 });
        seq.notes.push(soundcraft_midi::Note { start: 30, length: 100, pitch: 64, velocity: 90, release_velocity: 0, channel: 5 });
        seq.ctrls.push(soundcraft_midi::CtrlEvent { tick: 10, channel: 2, kind: soundcraft_midi::CtrlKind::Cc(64), value: 127 });
        let tr = s.track_mut(t).unwrap();
        tr.mixer.output = Route::Hardware("Synth".into());
        tr.playlist_mut().unwrap().clips.push(Clip::midi(ClipId(700), "m", 0, 48_000, seq));
        (s, t)
    }

    #[test]
    fn a_midi_track_routed_to_a_port_sends_its_notes_and_controllers_with_their_channels() {
        let (s, t) = external_session();
        let mut m = MixEngine::new(48000., 512);
        let mut out = vec![vec![0.; 512]; 2];
        let mut sent: Vec<(i64, [u8; 3])> = Vec::new();
        for b in 0..8i64 {
            m.render(&s, b * 512, 512, &mut out);
            sent.extend(m.midi_out.drain(..).map(|e| {
                assert_eq!(e.track, t);
                (b * 512 + i64::from(e.offset), e.msg)
            }));
        }
        sent.sort_by_key(|e| e.0);
        assert_eq!(
            sent,
            vec![(0, [0x92, 60, 100]), (250, [0xB2, 64, 127]), (750, [0x95, 64, 90]), (1000, [0x82, 60, 0]), (3250, [0x85, 64, 0]),],
            "sample-accurate, on the notes' own channels"
        );
    }

    #[test]
    fn a_seek_or_stop_releases_notes_sounding_on_external_gear() {
        let (s, _) = external_session();
        let mut out = vec![vec![0.; 512]; 2];
        // After the first block only the channel-3 note (0..1000) is sounding; after the second, only
        // the channel-6 one (750..3250).
        for (blocks, channel, pitch) in [(1, 2u8, 60u8), (2, 5, 64)] {
            let mut m = MixEngine::new(48000., 512);
            for b in 0..blocks {
                m.render(&s, b * 512, 512, &mut out);
            }
            m.midi_out.clear();
            // Jump somewhere else: the held note is released at the start of that block.
            m.render(&s, 20_000, 512, &mut out);
            let msgs: Vec<[u8; 3]> = m.midi_out.drain(..).map(|e| e.msg).collect();
            assert!(msgs.contains(&[0x80 | channel, pitch, 0]), "{msgs:?}");
            assert!(msgs.contains(&[0xB0 | channel, 123, 0]), "all-notes-off on the channel used: {msgs:?}");
            // Stopping releases it too, without another render.
            let mut m = MixEngine::new(48000., 512);
            for b in 0..blocks {
                m.render(&s, b * 512, 512, &mut out);
            }
            m.midi_out.clear();
            m.reset();
            let msgs: Vec<[u8; 3]> = m.midi_out.drain(..).map(|e| e.msg).collect();
            assert!(msgs.contains(&[0x80 | channel, pitch, 0]), "{msgs:?}");
        }
    }

    #[test]
    fn a_muted_or_stopped_midi_track_sends_nothing_new() {
        let (mut s, t) = external_session();
        s.track_mut(t).unwrap().mixer.mute = true;
        let mut m = MixEngine::new(48000., 512);
        let mut out = vec![vec![0.; 512]; 2];
        m.render(&s, 0, 512, &mut out);
        assert!(m.midi_out.is_empty());
        s.track_mut(t).unwrap().mixer.mute = false;
        m.monitor_only = true;
        m.render(&s, 0, 512, &mut out);
        assert!(m.midi_out.is_empty(), "a stopped transport plays no clips");
    }

    #[test]
    fn a_note_ending_on_a_block_edge_still_gets_its_note_off() {
        assert_eq!(note_events(&[(0, 960, 60)], 512), (1, 1));
    }

    #[test]
    fn every_overlapping_same_pitch_note_gets_its_own_note_off() {
        assert_eq!(note_events(&[(0, 40, 60), (10, 40, 60)], 2000), (2, 2));
    }

    #[test]
    fn a_note_running_past_a_trimmed_clip_end_is_released() {
        assert_eq!(note_events(&[(0, 1_000_000, 60)], 1500), (1, 1));
    }

    fn synth_session(level_db: f32) -> Session {
        use soundcraft_midi::{Note, Sequence};
        let mut s = Session::default();
        let t = s.add_track(TrackKind::Instrument, ChannelFormat::Stereo, None);
        let mut seq = Sequence::default();
        seq.notes.push(Note { pitch: 60, velocity: 100, release_velocity: 64, channel: 0, start: 0, length: 960 });
        let cid = s.new_clip_id();
        let tr = s.track_mut(t).unwrap();
        tr.playlist_mut().unwrap().clips.push(Clip::midi(cid, "n", 0, 24_000, seq));
        let mut ins = Insert::new("subtractive_synth");
        ins.params.insert("level".into(), level_db);
        tr.instrument = Some(ins);
        s
    }

    #[test]
    fn stored_instrument_params_reach_the_plugin() {
        // #152: a saved instrument level of -12 dB must render quieter than -6 dB.
        // RMS over the held part of the note, past any parameter smoothing.
        let rms = |db: f32| {
            let out = render_range(&synth_session(db), Range::new(0, 12_000), 512);
            let tail = out[0].get(6_000..).unwrap_or_default();
            (tail.iter().map(|x| x * x).sum::<f32>() / tail.len().max(1) as f32).sqrt()
        };
        let (loud, quiet) = (rms(-6.0), rms(-12.0));
        assert!(loud > 0.0, "the synth should sound");
        assert!((quiet / loud - 0.5).abs() < 0.05, "-12 dB vs -6 dB should halve the level: {quiet} / {loud}");
    }

    #[test]
    fn insert_slot_a_automation_does_not_drive_the_instrument() {
        let mut s = synth_session(-6.0);
        let reference = render_range(&s, Range::new(0, 12_000), 512);
        let tr = &mut s.tracks[0];
        let mut lane = soundcraft_model::AutomationLane::new(AutoParam::Plugin { slot: 0, param: "level".into() });
        lane.set_point(0, -48.0);
        tr.automation.push(lane);
        assert_eq!(render_range(&s, Range::new(0, 12_000), 512), reference);
    }

    fn session_with_dc(level: f32, frames: usize) -> (Session, TrackId) {
        let mut s = Session::default();
        let t = add_dc_track(&mut s, level, frames);
        (s, t)
    }

    fn add_dc_track(s: &mut Session, level: f32, frames: usize) -> TrackId {
        let t = s.add_track(TrackKind::Audio, ChannelFormat::Mono, None);
        let buf = AudioBuffer { sample_rate: 48_000, channels: vec![vec![level; frames]] };
        s.pool.insert(SourceId(900), Arc::new(SourceAudio::new(buf)));
        let id = s.new_clip_id();
        if let Some(pl) = s.track_mut(t).and_then(|t| t.playlist_mut()) {
            pl.clips.push(Clip::audio(id, "dc", SourceId(900), 0, 0, frames as i64));
        }
        t
    }

    #[test]
    fn mono_center_pan_is_minus_3db() {
        let (s, _) = session_with_dc(0.5, 4800);
        let out = render_range(&s, Range::new(0, 4800), 512);
        let expect = 0.5 * std::f32::consts::FRAC_1_SQRT_2;
        assert!((out[0][1000] - expect).abs() < 1e-3, "{}", out[0][1000]);
        assert!((out[1][1000] - expect).abs() < 1e-3);
    }

    #[test]
    fn fader_mute_and_solo() {
        let (mut s, t) = session_with_dc(0.5, 4800);
        s.track_mut(t).unwrap().mixer.volume_db = -6.0;
        let out = render_range(&s, Range::new(0, 4800), 512);
        let expect = 0.5 * std::f32::consts::FRAC_1_SQRT_2 * db_to_gain(-6.0);
        assert!((out[0][2000] - expect).abs() < 1e-3);
        s.track_mut(t).unwrap().mixer.mute = true;
        let out = render_range(&s, Range::new(0, 4800), 512);
        assert!(out[0][2000].abs() < 1e-6);
        // Soloing another track mutes this one.
        s.track_mut(t).unwrap().mixer.mute = false;
        let other = s.add_track(TrackKind::Audio, ChannelFormat::Mono, None);
        s.track_mut(other).unwrap().mixer.solo = true;
        let out = render_range(&s, Range::new(0, 4800), 512);
        assert!(out[0][2000].abs() < 1e-6);
    }

    #[test]
    fn hard_pans() {
        let (mut s, t) = session_with_dc(0.5, 4800);
        s.track_mut(t).unwrap().mixer.pan = vec![-1.0];
        let out = render_range(&s, Range::new(0, 4800), 256);
        assert!(out[0][100] > 0.49 && out[1][100].abs() < 1e-4);
    }

    #[test]
    fn fade_curvature_reaches_the_rendered_audio() {
        for fade_in in [true, false] {
            for curvature in [-0.5, 0.0, 0.5] {
                let (mut s, t) = session_with_dc(1.0, 8192);
                s.track_mut(t).unwrap().mixer.pan = vec![-1.0];
                let fade = soundcraft_model::Fade { len: 8192, shape: soundcraft_model::FadeShape::SCurve, curvature };
                let c = &mut s.track_mut(t).unwrap().playlist_mut().unwrap().clips[0];
                if fade_in {
                    c.fade_in = fade;
                } else {
                    c.fade_out = fade;
                }
                let out = render_range(&s, Range::new(0, 8192), 512);
                let expected = if curvature < 0.0 {
                    0.2
                } else if curvature > 0.0 {
                    0.8
                } else {
                    0.5
                };
                assert!((out[0][4096] - expected).abs() < 1e-5, "{fade_in}, {curvature}: {} vs {expected}", out[0][4096]);
                assert!(out[1][4096].abs() < 1e-6);
                assert!(out[0][if fade_in { 0 } else { 8191 }].abs() < 1e-4);
            }
        }
    }

    #[test]
    fn pan_automation_does_not_depend_on_the_block_size() {
        // Bounces render in 1024-frame blocks, playback in whatever its callback asks for (#244).
        let (mut s, t) = session_with_dc(0.5, 4800);
        {
            let l = s.track_mut(t).unwrap().lane_mut(&AutoParam::Pan(0));
            l.set_point(0, -1.0);
            l.set_point(480, 1.0);
        }
        let bounce = render_range(&s, Range::new(0, 4800), 1024);
        assert!(bounce[0][600].abs() < 1e-4 && bounce[1][600] > 0.49, "{} {}", bounce[0][600], bounce[1][600]);
        for block in [512, 100, 37] {
            assert_eq!(render_range(&s, Range::new(0, 4800), block), bounce, "block {block}");
        }
    }

    #[test]
    fn fades_and_clip_gain_apply() {
        let (mut s, t) = session_with_dc(1.0, 48_000);
        {
            let c = &mut s.track_mut(t).unwrap().playlist_mut().unwrap().clips[0];
            c.fade_in = soundcraft_model::Fade { len: 1000, shape: soundcraft_model::FadeShape::Linear, curvature: 0.0 };
            c.gain_db = -6.0;
        }
        let out = render_range(&s, Range::new(0, 2000), 512);
        let g = std::f32::consts::FRAC_1_SQRT_2 * db_to_gain(-6.0);
        assert!(out[0][0].abs() < 1e-3);
        assert!((out[0][500] - 0.5 * g).abs() < 0.02, "{}", out[0][500]);
        assert!((out[0][1500] - g).abs() < 1e-3);
    }

    #[test]
    fn sends_feed_aux_via_bus() {
        let (mut s, t) = session_with_dc(0.5, 4800);
        let bus = s.add_bus("Verb", ChannelFormat::Stereo);
        let aux = s.add_track(TrackKind::Aux, ChannelFormat::Stereo, Some("Verb"));
        s.track_mut(aux).unwrap().mixer.input = Route::Bus(bus);
        {
            let tr = s.track_mut(t).unwrap();
            tr.mixer.output = Route::None;
            let mut snd = soundcraft_model::SendSlot::new(Route::Bus(bus));
            snd.level_db = 0.0;
            tr.mixer.sends[0] = Some(snd);
        }
        let out = render_range(&s, Range::new(0, 4800), 512);
        assert!(out[0][1000] > 0.1, "aux should carry the send: {}", out[0][1000]);
    }

    #[test]
    fn render_bus_is_the_bus_not_the_main_mix() {
        let (mut s, t) = session_with_dc(0.5, 4800);
        let stem = s.add_bus("Stem", ChannelFormat::Stereo);
        let empty = s.add_bus("Empty", ChannelFormat::Stereo);
        s.track_mut(t).unwrap().mixer.output = Route::Bus(stem);
        let main = render_range(&s, Range::new(0, 4800), 512);
        let bus = render_bus(&s, stem, Range::new(0, 4800), 512);
        let silent = render_bus(&s, empty, Range::new(0, 4800), 512);
        assert!(main[0][1000].abs() < 1e-4, "main should not carry a track routed to the bus: {}", main[0][1000]);
        assert!(bus[0][1000].abs() > 0.1, "bus should carry the track: {}", bus[0][1000]);
        assert!(silent.iter().all(|c| c.iter().all(|x| x.abs() < 1e-6)));
    }

    #[test]
    fn volume_automation_ramps() {
        let (mut s, t) = session_with_dc(0.5, 48_000);
        {
            let tr = s.track_mut(t).unwrap();
            let l = tr.lane_mut(&AutoParam::Volume);
            l.set_point(0, 0.0);
            l.set_point(24_000, -144.0);
        }
        let out = render_range(&s, Range::new(0, 48_000), 512);
        assert!(out[0][100] > 0.3);
        assert!(out[0][30_000].abs() < 1e-4);
    }

    #[test]
    fn inserts_process_and_survive_unknown_plugins() {
        let (mut s, t) = session_with_dc(0.5, 4800);
        s.track_mut(t).unwrap().mixer.inserts[0] = Some(Insert::new("does-not-exist"));
        let out = render_range(&s, Range::new(0, 4800), 512);
        assert!(out[0][100] > 0.3);
        let id = soundcraft_dsp::plugins().iter().find(|p| p.id.contains("gain")).map(|p| p.id).unwrap_or("gain");
        let mut ins = Insert::new(id);
        ins.params.insert("gain".into(), -6.0);
        s.track_mut(t).unwrap().mixer.inserts[0] = Some(ins);
        let _ = render_range(&s, Range::new(0, 4800), 512);
    }

    /// A built-in gain behind a third-party id, with a log of parameter sets.
    struct Stateful {
        inner: Box<dyn Plugin>,
        blob: Vec<u8>,
        sets: Arc<std::sync::atomic::AtomicUsize>,
    }

    impl Plugin for Stateful {
        fn info(&self) -> &'static soundcraft_dsp::PluginInfo {
            self.inner.info()
        }
        fn prepare(&mut self, sr: f32, mb: usize, ch: usize) {
            self.inner.prepare(sr, mb, ch);
        }
        fn reset(&mut self) {
            self.inner.reset();
        }
        fn set_param(&mut self, id: &str, v: f32) -> bool {
            self.sets.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            self.inner.set_param(id, v)
        }
        fn param(&self, id: &str) -> Option<f32> {
            self.inner.param(id)
        }
        fn process(&mut self, io: &mut [Vec<f32>], frames: usize) {
            self.inner.process(io, frames);
        }
        fn state_reader(&mut self) -> Option<Box<dyn soundcraft_dsp::PluginStateReader>> {
            let blob = self.blob.clone();
            Some(Box::new(move || Some(blob.clone())))
        }
    }

    struct Clock {
        inner: Box<dyn Plugin>,
        seen: Arc<std::sync::Mutex<Vec<Transport>>>,
    }

    impl Plugin for Clock {
        fn info(&self) -> &'static soundcraft_dsp::PluginInfo {
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
        fn process(&mut self, io: &mut [Vec<f32>], frames: usize) {
            self.inner.process(io, frames);
        }
        fn set_transport(&mut self, t: &Transport) {
            self.seen.lock().unwrap().push(*t);
        }
    }

    #[test]
    fn plugins_get_the_session_transport() {
        let (mut s, t) = session_with_dc(0.5, 48_000 * 4);
        s.tempo.set_tempo(0, 90.0).unwrap();
        s.tempo.set_meter(0, 3, 4).unwrap();
        s.track_mut(t).unwrap().mixer.inserts[0] = Some(Insert::new("clap:test.clock"));
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let clock = Clock { inner: soundcraft_dsp::create("gain").unwrap(), seen: Arc::clone(&seen) };
        let mut eng = MixEngine::new(48_000.0, 512);
        eng.set_external_instances(true);
        eng.adopt(vec![PreparedInstance { track: t, slot: 0, id: "clap:test.clock".into(), ch: 1, plugin: Box::new(clock) }]);
        render_block(&mut eng, &s, 96_000);
        eng.monitor_only = true;
        render_block(&mut eng, &s, 96_000);
        let seen = seen.lock().unwrap();
        let t0 = seen[0];
        assert!(t0.playing && !t0.recording);
        assert_eq!((t0.tempo, t0.numerator, t0.denominator, t0.samples, t0.bar), (90.0, 3, 4, 96_000, 1));
        assert!((t0.seconds - 2.0).abs() < 1e-9 && (t0.quarters - 3.0).abs() < 1e-9 && (t0.bar_start - 3.0).abs() < 1e-9);
        assert!(!seen[1].playing);
    }

    struct Ears {
        inner: Box<dyn Plugin>,
        heard: Arc<std::sync::Mutex<Vec<(usize, [u8; 3])>>>,
    }

    impl Plugin for Ears {
        fn info(&self) -> &'static soundcraft_dsp::PluginInfo {
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
        fn process(&mut self, io: &mut [Vec<f32>], frames: usize) {
            self.inner.process(io, frames);
        }
        fn midi(&mut self, offset: usize, msg: [u8; 3]) {
            self.heard.lock().unwrap().push((offset, msg));
        }
    }

    #[test]
    fn controllers_play_at_their_offsets_and_are_chased_after_a_seek() {
        let mut s = Session::default();
        let t = s.add_track(TrackKind::Instrument, ChannelFormat::Stereo, None);
        s.track_mut(t).unwrap().instrument = Some(Insert::new("clap:test.ears"));
        let cc = |tick, kind, value| soundcraft_midi::CtrlEvent { tick, channel: 0, kind, value };
        let ctrls = vec![cc(4, CtrlKind::Cc(64), 127), cc(960, CtrlKind::PitchBend, -8192), cc(1920, CtrlKind::Cc(64), 0)];
        let seq = soundcraft_midi::Sequence { ctrls, ..Default::default() };
        let cid = s.new_clip_id();
        s.track_mut(t).unwrap().playlist_mut().unwrap().clips.push(Clip::midi(cid, "m", 0, 48_000 * 4, seq));
        let heard = Arc::new(std::sync::Mutex::new(Vec::new()));
        let ears = Ears { inner: soundcraft_dsp::create("gain").unwrap(), heard: Arc::clone(&heard) };
        let mut eng = MixEngine::new(48_000.0, 512);
        eng.set_external_instances(true);
        eng.adopt(vec![PreparedInstance { track: t, slot: INSTRUMENT_SLOT, id: "clap:test.ears".into(), ch: 2, plugin: Box::new(ears) }]);
        render_block(&mut eng, &s, 0);
        assert_eq!(std::mem::take(&mut *heard.lock().unwrap()), vec![(0, [0xB0, 121, 0]), (100, [0xB0, 64, 127])]);
        render_block(&mut eng, &s, 36_000);
        assert_eq!(std::mem::take(&mut *heard.lock().unwrap()), vec![(0, [0xB0, 121, 0]), (0, [0xB0, 64, 127]), (0, [0xE0, 0, 0])]);
        render_block(&mut eng, &s, 36_512);
        assert!(heard.lock().unwrap().is_empty());
    }

    struct Keys {
        inner: Box<dyn Plugin>,
        heard: Arc<std::sync::Mutex<Vec<(u8, u8)>>>,
    }

    impl Plugin for Keys {
        fn info(&self) -> &'static soundcraft_dsp::PluginInfo {
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
        fn process(&mut self, io: &mut [Vec<f32>], frames: usize) {
            self.inner.process(io, frames);
        }
        fn note_on(&mut self, _offset: usize, note: u8, velocity: u8) {
            self.heard.lock().unwrap().push((note, velocity));
        }
    }

    #[test]
    fn real_time_properties_reach_the_instrument() {
        let mut s = Session::default();
        let t = s.add_track(TrackKind::Instrument, ChannelFormat::Stereo, None);
        s.track_mut(t).unwrap().instrument = Some(Insert::new("clap:test.keys"));
        let mut seq = soundcraft_midi::Sequence::default();
        seq.notes.push(soundcraft_midi::Note { pitch: 60, velocity: 100, release_velocity: 64, channel: 0, start: 0, length: 480 });
        let cid = s.new_clip_id();
        s.track_mut(t).unwrap().playlist_mut().unwrap().clips.push(Clip::midi(cid, "m", 0, 48_000, seq));
        s.edit.values.insert(format!("rtp.{}.transpose", t.0), 12.0);
        s.edit.values.insert(format!("rtp.{}.velocity", t.0), -20.0);
        let heard = Arc::new(std::sync::Mutex::new(Vec::new()));
        let keys = Keys { inner: soundcraft_dsp::create("gain").unwrap(), heard: Arc::clone(&heard) };
        let mut eng = MixEngine::new(48_000.0, 512);
        eng.set_external_instances(true);
        eng.adopt(vec![PreparedInstance { track: t, slot: INSTRUMENT_SLOT, id: "clap:test.keys".into(), ch: 2, plugin: Box::new(keys) }]);
        render_block(&mut eng, &s, 0);
        assert_eq!(*heard.lock().unwrap(), vec![(72, 80)]);
    }

    fn render_block(eng: &mut MixEngine, s: &Session, pos: i64) -> f32 {
        let mut out = vec![vec![0.0f32; 512]; 2];
        eng.render(s, pos, 512, &mut out);
        out[0][500]
    }

    #[test]
    fn external_instances_are_adopted_and_retired() {
        let (mut s, t) = session_with_dc(0.5, 48_000);
        let mut ins = Insert::new("clap:test.fake");
        ins.params.insert("gain".into(), -6.0);
        s.track_mut(t).unwrap().mixer.inserts[0] = Some(ins);
        let specs = instance_specs(&s);
        assert_eq!(specs, vec![InstanceSpec { track: t, slot: 0, id: "clap:test.fake".into(), ch: 1, state: None }]);
        let mut eng = MixEngine::new(48_000.0, 512);
        eng.set_external_instances(true);
        // Nothing to adopt yet: the slot passes audio through (and sync never tried to load a
        // CLAP plugin on this thread).
        let dry = render_block(&mut eng, &s, 0);
        assert!(dry > 0.3, "{dry}");
        let sets = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut gain = soundcraft_dsp::create("gain").unwrap();
        gain.prepare(48_000.0, 512, 1);
        let fake = Stateful { inner: gain, blob: Vec::new(), sets: Arc::clone(&sets) };
        // An instance for the wrong channel count is retired, not used.
        let wrong = Stateful { inner: soundcraft_dsp::create("gain").unwrap(), blob: Vec::new(), sets: Arc::clone(&sets) };
        eng.adopt(vec![PreparedInstance { track: t, slot: 0, id: "clap:test.fake".into(), ch: 2, plugin: Box::new(wrong) }]);
        assert!(render_block(&mut eng, &s, 512) > 0.3);
        assert_eq!(eng.take_retired().len(), 1);
        eng.adopt(vec![PreparedInstance { track: t, slot: 0, id: "clap:test.fake".into(), ch: 1, plugin: Box::new(fake) }]);
        let mut wet = 0.0;
        for b in 2..20 {
            wet = render_block(&mut eng, &s, b * 512);
        }
        assert!((wet / dry - 0.501).abs() < 0.01, "{wet} vs {dry}");
        assert!(sets.load(std::sync::atomic::Ordering::Relaxed) > 0, "no stored state: session params are pushed");
        assert!(eng.take_retired().is_empty());
        // Removing the insert retires the instance instead of dropping it here.
        s.track_mut(t).unwrap().mixer.inserts[0] = None;
        render_block(&mut eng, &s, 20 * 512);
        assert_eq!(eng.take_retired().len(), 1);
        // Instances nobody claims are retired too; drain_plugins empties the engine.
        eng.adopt(vec![PreparedInstance { track: t, slot: 3, id: "clap:x".into(), ch: 1, plugin: soundcraft_dsp::create("gain").unwrap() }]);
        render_block(&mut eng, &s, 21 * 512);
        assert_eq!(eng.take_retired().len(), 1);
        assert!(eng.drain_plugins().is_empty());
    }

    struct Nan(Box<dyn Plugin>);

    impl Plugin for Nan {
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
            io.iter_mut().for_each(|c| c.iter_mut().take(frames).for_each(|x| *x = f32::NAN));
        }
    }

    fn nan(track: TrackId, slot: usize, ch: usize) -> Vec<PreparedInstance> {
        vec![PreparedInstance { track, slot, id: "clap:test.nan".into(), ch, plugin: Box::new(Nan(soundcraft_dsp::create("gain").unwrap())) }]
    }

    #[test]
    fn an_insert_that_puts_out_nan_is_taken_out_until_the_next_reset() {
        let (mut s, a) = session_with_dc(0.5, 48_000);
        let b = add_dc_track(&mut s, 0.5, 48_000);
        s.track_mut(a).unwrap().mixer.inserts[2] = Some(Insert::new("clap:test.nan"));
        let mut eng = MixEngine::new(48_000.0, 512);
        eng.set_external_instances(true);
        let both = render_block(&mut eng, &s, 0);
        eng.adopt(nan(a, 2, 1));
        let only_b = render_block(&mut eng, &s, 512);
        assert!(only_b > 0.3 && (only_b - both / 2.0).abs() < 1e-6, "{only_b} vs {both}");
        assert_eq!(eng.meters[&a].plugin_failures, PluginFailures { count: 1, last: 2 });
        assert_eq!(eng.meters[&b].plugin_failures, PluginFailures::default());
        assert_eq!(render_block(&mut eng, &s, 1024), both);
        eng.reset();
        assert_eq!(render_block(&mut eng, &s, 1536), only_b);
        assert_eq!(eng.meters[&a].plugin_failures.count, 2);
    }

    #[test]
    fn nan_from_an_instrument_or_a_master_insert_is_contained() {
        let mut s = synth_session(-6.0);
        let synth = s.tracks[0].id;
        s.track_mut(synth).unwrap().instrument = Some(Insert::new("clap:test.nan"));
        let b = add_dc_track(&mut s, 0.5, 48_000);
        let mut eng = MixEngine::new(48_000.0, 512);
        eng.set_external_instances(true);
        eng.adopt(nan(synth, INSTRUMENT_SLOT, 2));
        let only_b = render_block(&mut eng, &s, 0);
        assert!(only_b > 0.3, "{only_b}");
        assert_eq!(eng.meters[&synth].plugin_failures, PluginFailures { count: 1, last: INSTRUMENT_SLOT });
        assert_eq!(eng.meters[&b].plugin_failures, PluginFailures::default());
        let m = s.add_track(TrackKind::Master, ChannelFormat::Stereo, None);
        s.track_mut(m).unwrap().mixer.inserts[0] = Some(Insert::new("clap:test.nan"));
        eng.adopt(nan(m, 0, 2));
        assert_eq!(render_block(&mut eng, &s, 512), 0.0);
        assert_eq!(eng.meters[&m].plugin_failures, PluginFailures { count: 1, last: 0 });
        assert_eq!(render_block(&mut eng, &s, 1024), only_b);
    }

    #[test]
    fn a_new_instance_replaces_a_matching_one_in_its_slot() {
        // The host removed and re-added the plugin, and both session updates reached the audio
        // thread in one go: the slot still holds the old instance (same id, same channels), but
        // the host made a new one and drives its editor. The new one must win, or the editor
        // talks to an instance that was retired ("the plugin is gone").
        let (mut s, t) = session_with_dc(0.5, 48_000);
        s.track_mut(t).unwrap().mixer.inserts[0] = Some(Insert::new("clap:test.fake"));
        let mut eng = MixEngine::new(48_000.0, 512);
        eng.set_external_instances(true);
        let sets = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let make = |blob: &[u8]| {
            let mut gain = soundcraft_dsp::create("gain").unwrap();
            gain.prepare(48_000.0, 512, 1);
            Box::new(Stateful { inner: gain, blob: blob.to_vec(), sets: Arc::clone(&sets) })
        };
        eng.adopt(vec![PreparedInstance { track: t, slot: 0, id: "clap:test.fake".into(), ch: 1, plugin: make(b"old") }]);
        render_block(&mut eng, &s, 0);
        assert!(eng.take_retired().is_empty());
        eng.adopt(vec![PreparedInstance { track: t, slot: 0, id: "clap:test.fake".into(), ch: 1, plugin: make(b"new") }]);
        render_block(&mut eng, &s, 512);
        let mut retired = eng.take_retired();
        assert_eq!(retired.len(), 1, "the old instance is retired");
        assert_eq!(retired[0].save_state(), Some(b"old".to_vec()), "the new instance stays");
    }

    #[test]
    fn stored_state_comes_first_then_session_params() {
        let (mut s, t) = session_with_dc(0.5, 48_000);
        let mut ins = Insert::new("vst3:00000000000000000000000000000001");
        ins.params.insert("gain".into(), -6.0);
        ins.set_state_bytes(b"stored");
        s.track_mut(t).unwrap().mixer.inserts[0] = Some(ins);
        assert_eq!(instance_specs(&s)[0].state.as_deref(), Some(&b"stored"[..]));
        let mut eng = MixEngine::new(48_000.0, 512);
        eng.set_external_instances(true);
        let sets = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let fake = Stateful { inner: soundcraft_dsp::create("gain").unwrap(), blob: Vec::new(), sets: Arc::clone(&sets) };
        eng.adopt(vec![PreparedInstance { track: t, slot: 0, id: "vst3:00000000000000000000000000000001".into(), ch: 1, plugin: Box::new(fake) }]);
        render_block(&mut eng, &s, 0);
        assert_eq!(sets.load(std::sync::atomic::Ordering::Relaxed), 1, "session params apply once");
        render_block(&mut eng, &s, 512);
        assert_eq!(sets.load(std::sync::atomic::Ordering::Relaxed), 1);
        s.track_mut(t).unwrap().mixer.inserts[0].as_mut().unwrap().params.insert("gain".into(), -12.0);
        render_block(&mut eng, &s, 1024);
        assert_eq!(sets.load(std::sync::atomic::Ordering::Relaxed), 2, "later session edits apply");
        // Offline (not external): unknown third-party plugins are simply absent.
        let out = render_range(&s, Range::new(0, 1024), 512);
        assert!(out[0][100] > 0.3);
    }

    #[test]
    fn clip_effects_gain_and_trim_automation_apply() {
        let (mut s, t) = session_with_dc(0.5, 4800);
        let cid = s.track(t).unwrap().clips()[0].id.0;
        s.edit.values.insert(format!("clip_fx.{cid}.gain"), -6.0);
        let out = render_range(&s, Range::new(0, 4800), 512);
        let expect = 0.5 * std::f32::consts::FRAC_1_SQRT_2 * db_to_gain(-6.0);
        assert!((out[0][2000] - expect).abs() < 0.01, "{}", out[0][2000]);
        s.edit.set_flag(&format!("clip_fx.bypass.{cid}"), true);
        let out = render_range(&s, Range::new(0, 4800), 512);
        assert!((out[0][2000] - 0.5 * std::f32::consts::FRAC_1_SQRT_2).abs() < 0.01);
        s.track_mut(t).unwrap().lane_mut(&AutoParam::Plugin { slot: u8::MAX, param: "trim".into() }).set_point(0, -12.0);
        let out = render_range(&s, Range::new(0, 4800), 512);
        assert!(out[0][2000] < 0.2, "{}", out[0][2000]);
    }

    #[test]
    fn delay_compensation_aligns_latent_tracks() {
        let mut s = Session::default();
        let mut imp = vec![0.0f32; 48_000];
        imp[1000] = 1.0;
        s.pool.insert(SourceId(900), Arc::new(SourceAudio::new(AudioBuffer { sample_rate: 48_000, channels: vec![imp] })));
        let mut ids = Vec::new();
        for (pan, fx) in [(-1.0f32, true), (1.0, false)] {
            let t = s.add_track(TrackKind::Audio, ChannelFormat::Mono, None);
            let cid = s.new_clip_id();
            let tr = s.track_mut(t).unwrap();
            tr.playlist_mut().unwrap().clips.push(Clip::audio(cid, "i", SourceId(900), 0, 0, 48_000));
            tr.mixer.pan = vec![pan];
            if fx {
                tr.mixer.inserts[0] = Some(Insert::new("maximizer"));
            }
            ids.push(t);
        }
        let argmax = |c: &[f32]| c.iter().enumerate().max_by(|a, b| a.1.abs().total_cmp(&b.1.abs())).map(|(i, _)| i).unwrap();
        let lat = soundcraft_dsp::create("maximizer").unwrap().latency();
        assert!(lat > 0, "test needs a latent plugin");
        let out = render_range(&s, Range::new(0, 8000), 512);
        assert_eq!(argmax(&out[0]), argmax(&out[1]), "PDC should align L and R");
        s.edit.delay_compensation = false;
        let out = render_range(&s, Range::new(0, 8000), 512);
        assert_ne!(argmax(&out[0]), argmax(&out[1]));
    }

    /// A reset (stop, locate) must not leave the previous position's audio in the alignment delay.
    #[test]
    fn reset_clears_delay_compensation_history() {
        let (mut s, t) = session_with_dc(0.5, 4800);
        s.edit.click = false;
        s.track_mut(t).unwrap().mixer.pan = vec![1.0];
        let latent = s.add_track(TrackKind::Audio, ChannelFormat::Mono, None);
        let mut comp = Insert::new("compressor");
        comp.params.insert("lookahead".into(), 5.0);
        s.track_mut(latent).unwrap().mixer.inserts[0] = Some(comp);
        let mut eng = MixEngine::new(48_000.0, 512);
        let mut out = vec![vec![0.0f32; 512]; 2];
        for b in 0..9 {
            eng.render(&s, b * 512, 512, &mut out);
        }
        assert!(eng.latency > 0 && out[1][500] > 0.3, "the fixture plays through the alignment delay");
        eng.reset();
        eng.render(&s, 1_000_000, 512, &mut out);
        assert!(out.iter().flatten().all(|x| *x == 0.0), "old aligned audio after a reset");
    }

    fn impulse_at(ch: &[f32]) -> Vec<usize> {
        ch.iter().enumerate().filter(|(_, v)| v.abs() > 1e-4).map(|(i, _)| i).collect()
    }

    /// An aux insert's lookahead is part of the mix latency on the first block, not the block after.
    #[test]
    fn delay_compensation_aligns_an_aux_insert() {
        let mut s = Session::default();
        let n = 8192;
        let mut left = vec![0.0f32; n];
        for at in [0usize, 2048, 8000] {
            left[at] = 0.25;
        }
        s.pool.insert(SourceId(901), Arc::new(SourceAudio::new(AudioBuffer { sample_rate: 48_000, channels: vec![left.clone(), left] })));
        let bus = s.add_bus("Test Bus", ChannelFormat::Stereo);
        let source = s.add_track(TrackKind::Audio, ChannelFormat::Stereo, Some("Source"));
        let aux = s.add_track(TrackKind::Aux, ChannelFormat::Stereo, Some("Return"));
        let cid = s.new_clip_id();
        s.track_mut(source).unwrap().playlist_mut().unwrap().clips.push(Clip::audio(cid, "impulses", SourceId(901), 0, 0, n as i64));
        s.track_mut(source).unwrap().mixer.output = Route::Bus(bus);
        s.track_mut(aux).unwrap().mixer.input = Route::Bus(bus);
        let render = |s: &Session, start: i64, end: i64| render_range(s, Range::new(start, end), 1024);
        let control = render(&s, 0, n as i64);
        assert_eq!(impulse_at(&control[0]), vec![0, 2048, 8000]);
        assert_eq!(impulse_at(&control[1]), impulse_at(&control[0]));

        let mut comp = Insert::new("compressor");
        comp.params.insert("lookahead".into(), 5.0);
        comp.params.insert("ratio".into(), 1.0);
        s.track_mut(source).unwrap().mixer.inserts[0] = Some(comp.clone());
        let on_source = render(&s, 0, n as i64);
        assert_eq!(impulse_at(&on_source[0]), impulse_at(&control[0]));

        s.track_mut(source).unwrap().mixer.inserts[0] = None;
        s.track_mut(aux).unwrap().mixer.inserts[0] = Some(comp);
        let on_aux = render(&s, 0, n as i64);
        assert_eq!(impulse_at(&on_aux[0]), vec![0, 2048, 8000], "aux lookahead must stay inside the bounce");
        assert_eq!(impulse_at(&on_aux[1]), impulse_at(&on_aux[0]));
        assert!((on_aux[0][8000] - 0.25).abs() < 1e-3, "{}", on_aux[0][8000]);
        let partial = render(&s, 2048, 3072);
        assert_eq!(impulse_at(&partial[0]), vec![0]);

        s.track_mut(aux).unwrap().mixer.inserts[0].as_mut().unwrap().params.insert("lookahead".into(), 0.0);
        let zero = render(&s, 0, n as i64);
        assert_eq!(impulse_at(&zero[0]), impulse_at(&control[0]));
    }

    #[test]
    fn offline_renders_line_up_with_the_timeline_in_any_block_size() {
        let mut s = Session::default();
        let mut imp = vec![0.0f32; 48_000];
        imp[1000] = 1.0;
        s.pool.insert(SourceId(900), Arc::new(SourceAudio::new(AudioBuffer { sample_rate: 48_000, channels: vec![imp] })));
        let t = s.add_track(TrackKind::Audio, ChannelFormat::Mono, None);
        let bus = s.add_bus("Stem", ChannelFormat::Stereo);
        let cid = s.new_clip_id();
        let tr = s.track_mut(t).unwrap();
        tr.playlist_mut().unwrap().clips.push(Clip::audio(cid, "i", SourceId(900), 0, 0, 48_000));
        tr.mixer.inserts[0] = Some(Insert::new("maximizer"));
        let mut send = soundcraft_model::SendSlot::new(Route::Bus(bus));
        send.level_db = 0.0;
        tr.mixer.sends[0] = Some(send);
        let latency = soundcraft_dsp::create("maximizer").unwrap().latency();
        assert!(latency > 16);
        for source in [None, Some(bus)] {
            for block in [16, 100, 512] {
                let mut got = Vec::new();
                let mut skipped = 0;
                let mut render = RangeRender::new(&s, source, Range::new(900, 2900), block);
                while let Some((chunk, n)) = render.next_block(&s) {
                    assert!(n <= block);
                    skipped += usize::from(n == 0);
                    got.extend_from_slice(&chunk[0][..n]);
                }
                assert_eq!(got.len(), 2000, "{source:?} block {block}");
                assert_eq!(skipped, latency / block, "{source:?} block {block}");
                let peak = got.iter().enumerate().max_by(|a, b| a.1.abs().total_cmp(&b.1.abs())).map(|(i, _)| i);
                assert_eq!(peak, Some(100), "{source:?} block {block}");
            }
        }
    }

    #[test]
    fn a_plugins_bogus_latency_is_capped() {
        struct Latent(Box<dyn Plugin>);
        impl Plugin for Latent {
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
            fn latency(&self) -> usize {
                u32::MAX as usize
            }
        }
        let (mut s, t) = session_with_dc(0.5, 48_000);
        let aux = s.add_track(TrackKind::Aux, ChannelFormat::Mono, None);
        let mut eng = MixEngine::new(48_000.0, 512);
        eng.set_external_instances(true);
        for track in [t, aux] {
            s.track_mut(track).unwrap().mixer.inserts[0] = Some(Insert::new("clap:test.latent"));
            let mut gain = soundcraft_dsp::create("gain").unwrap();
            gain.prepare(48_000.0, 512, 1);
            eng.adopt(vec![PreparedInstance { track, slot: 0, id: "clap:test.latent".into(), ch: 1, plugin: Box::new(Latent(gain)) }]);
        }
        render_block(&mut eng, &s, 0);
        assert_eq!((eng.track_latency, eng.latency), (MAX_LATENCY, 2 * MAX_LATENCY));
    }

    #[test]
    fn input_monitoring_replaces_clips() {
        let (mut s, t) = session_with_dc(0.5, 4800);
        let mut eng = MixEngine::new(48_000.0, 512);
        eng.input = vec![vec![0.25; 512]];
        let mut out = vec![vec![0.0; 512]; 2];
        // Not monitored: the clip plays.
        eng.render(&s, 0, 512, &mut out);
        assert!((out[0][100] - 0.5 * std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-3);
        // Monitored: the input replaces it, and it still sounds when stopped.
        s.track_mut(t).unwrap().mixer.input_monitor = true;
        eng.render(&s, 512, 512, &mut out);
        assert!((out[0][100] - 0.25 * std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-3, "{}", out[0][100]);
        eng.monitor_only = true;
        eng.render(&s, 1024, 512, &mut out);
        assert!(out[0][100] > 0.1);
        s.track_mut(t).unwrap().mixer.input_monitor = false;
        eng.render(&s, 1024, 512, &mut out);
        assert!(out[0][100].abs() < 1e-6, "stopped, unmonitored: silence");
    }

    #[test]
    fn empty_session_is_silent() {
        let s = Session::default();
        let out = render_range(&s, Range::new(0, 1000), 128);
        assert!(out.iter().all(|c| c.iter().all(|x| *x == 0.0)));
    }

    #[test]
    fn strip_workspace_keeps_its_allocation_between_blocks() {
        let mut s = Session::default();
        for _ in 0..6 {
            s.add_track(TrackKind::Audio, ChannelFormat::Mono, None);
        }
        let mut eng = MixEngine::new(48_000.0, 512);
        let mut out = vec![vec![0.0; 512]; 2];
        eng.render(&s, 0, 512, &mut out);
        let capacity = eng.work.capacity();
        assert!(capacity >= 6);
        eng.render(&s, 512, 512, &mut out);
        assert_eq!(eng.work.capacity(), capacity);
    }

    #[test]
    fn stopped_monitoring_does_not_click() {
        let mut s = Session::default();
        s.edit.click = true;
        let mut eng = MixEngine::new(48_000.0, 512);
        eng.metronome = true;
        eng.monitor_only = true;
        let mut out = vec![vec![0.0f32; 512]; 2];
        for _ in 0..4 {
            eng.render(&s, 0, 512, &mut out);
            assert!(out.iter().all(|c| c.iter().all(|x| *x == 0.0)));
        }
    }

    /// Renders `len` frames from zero the way playback does (metronome on), in blocks of `block`.
    fn render_playback(s: &Session, len: usize, block: usize, recording: bool) -> Vec<Vec<f32>> {
        let mut eng = MixEngine::new(s.sample_rate.as_f64() as f32, block);
        eng.metronome = true;
        eng.recording = recording;
        let mut out = vec![vec![0.0f32; len]; 2];
        let mut tmp = vec![vec![0.0f32; block]; 2];
        let mut done = 0usize;
        while done < len {
            let n = (len - done).min(block);
            eng.render(s, done as i64, n, &mut tmp);
            for (o, t) in out.iter_mut().zip(tmp.iter()) {
                if let (Some(d), Some(src)) = (o.get_mut(done..done + n), t.get(..n)) {
                    d.copy_from_slice(src);
                }
            }
            done += n;
        }
        out
    }

    #[test]
    fn click_sounds_on_every_beat_only_in_playback() {
        let mut s = Session::default();
        let beat = s.sample_rate.as_f64() as usize * 60 / 120; // 120 BPM quarter notes
        let len = beat * 8;
        s.edit.click = true;
        // Offline renders (bounces, stems) never print the metronome.
        let offline = render_range(&s, Range::new(0, len as i64), 512);
        assert!(offline.iter().all(|c| c.iter().all(|x| *x == 0.0)));

        let out = render_playback(&s, len, 512, false);
        for b in 0..8usize {
            let at = b * beat;
            let on: f32 = out[0][at..at + 200].iter().map(|x| x.abs()).fold(0.0, f32::max);
            let off: f32 = out[0][at + beat / 2..at + beat / 2 + 200].iter().map(|x| x.abs()).fold(0.0, f32::max);
            assert!(on > 0.1, "beat {b} should click, peak {on}");
            assert_eq!(off, 0.0, "between beats must stay silent");
            assert_eq!(out[0][at], out[1][at], "both channels get the click");
        }
        // Blocks that start mid-beat render the same clicks as a single pass.
        let split = render_playback(&s, len, 97, false);
        assert_eq!(out, split);
    }

    #[test]
    fn click_only_during_record_follows_recording() {
        let mut s = Session::default();
        s.edit.click = true;
        s.edit.set_flag("click.only_during_record", true);
        let idle = render_playback(&s, 48_000, 512, false);
        assert!(idle.iter().all(|c| c.iter().all(|x| *x == 0.0)));
        let rec = render_playback(&s, 48_000, 512, true);
        assert!(rec.iter().any(|c| c.iter().any(|x| x.abs() > 0.1)));
    }

    /// A session with one track of `fmt` playing a distinct DC level per channel (0.1, 0.2, …).
    fn multichannel_session(fmt: ChannelFormat, main: ChannelFormat, frames: usize) -> (Session, TrackId) {
        let mut s = Session::default();
        s.outputs[0].format = main;
        let t = s.add_track(TrackKind::Audio, fmt, None);
        let chans: Vec<Vec<f32>> = (0..fmt.channels()).map(|c| (0..frames).map(|i| 0.1 * (c + 1) as f32 + (i % 7) as f32 * 1e-3).collect()).collect();
        s.pool.insert(SourceId(901), Arc::new(SourceAudio::new(AudioBuffer { sample_rate: 48_000, channels: chans })));
        let id = s.new_clip_id();
        s.track_mut(t).unwrap().playlist_mut().unwrap().clips.push(Clip::audio(id, "mc", SourceId(901), 0, 0, frames as i64));
        (s, t)
    }

    #[test]
    fn surround_track_passes_through_bit_exact() {
        let (s, _) = multichannel_session(ChannelFormat::Surround51, ChannelFormat::Surround51, 4800);
        let src = s.pool.get(SourceId(901)).unwrap().buffer.channels.clone();
        let out = render_range(&s, Range::new(0, 4800), 512);
        assert_eq!(out.len(), 6);
        for (o, x) in out.iter().zip(src.iter()) {
            assert_eq!(o, x, "5.1 → 5.1 must be bit-exact");
        }
    }

    #[test]
    fn surround_to_stereo_folds_down_itu() {
        let (s, _) = multichannel_session(ChannelFormat::Surround51, ChannelFormat::Stereo, 4800);
        let out = render_range(&s, Range::new(0, 4800), 512);
        assert_eq!(out.len(), 2);
        let h = std::f32::consts::FRAC_1_SQRT_2;
        let x = |c: usize| 0.1 * (c + 1) as f32 + (1000 % 7) as f32 * 1e-3;
        // L R C LFE Ls Rs.
        let l = x(0) + h * x(2) + h * x(4);
        let r = x(1) + h * x(2) + h * x(5);
        assert!((out[0][1000] - l).abs() < 1e-5, "{} vs {l}", out[0][1000]);
        assert!((out[1][1000] - r).abs() < 1e-5, "{} vs {r}", out[1][1000]);
    }

    #[test]
    fn mono_track_on_surround_main_pans() {
        let (mut s, t) = session_with_dc(0.5, 4800);
        s.outputs[0].format = ChannelFormat::Surround51;
        // No surround pan: stereo behaviour on L/R.
        let out = render_range(&s, Range::new(0, 4800), 512);
        assert_eq!(out.len(), 6);
        let h = 0.5 * gains(0.0, PanLaw::Minus3).0;
        assert!((out[0][100] - h).abs() < 1e-6 && (out[1][100] - h).abs() < 1e-6);
        assert!(out[2..].iter().all(|c| c[100].abs() < 1e-7));
        // Front centre: the centre speaker only.
        s.track_mut(t).unwrap().mixer.surround = Some(SurroundPan::default());
        let out = render_range(&s, Range::new(0, 4800), 512);
        assert!((out[2][100] - 0.5).abs() < 1e-4, "{}", out[2][100]);
        assert!(out.iter().enumerate().all(|(c, x)| c == 2 || x[100].abs() < 1e-4));
        // Rear right with LFE: Rs plus the LFE send.
        s.track_mut(t).unwrap().mixer.surround = Some(SurroundPan { x: 1.0, y: -1.0, lfe_db: -6.0, ..SurroundPan::default() });
        let mut eng = MixEngine::new(48_000.0, 512);
        let mut buf = vec![vec![0.0; 512]; 6];
        eng.render(&s, 0, 512, &mut buf);
        assert!((buf[5][100] - 0.5).abs() < 1e-4 && (buf[3][100] - 0.5 * db_to_gain(-6.0)).abs() < 1e-4);
        assert_eq!(eng.main_meter.peaks.len(), 6);
        assert_eq!(eng.meters.get(&t).map(|m| m.peaks.len()), Some(1));
        // A stereo device buffer gets the first two channels.
        let mut two = vec![vec![1.0; 512]; 2];
        eng.render(&s, 512, 512, &mut two);
        assert!(two[0][10].abs() < 1e-6);
    }

    #[test]
    fn surround_busses_and_master_fader() {
        let (mut s, t) = multichannel_session(ChannelFormat::Surround51, ChannelFormat::Surround51, 4800);
        let bus = s.add_bus("Stem", ChannelFormat::Surround51);
        let aux = s.add_track(TrackKind::Aux, ChannelFormat::Surround51, Some("Stem"));
        s.track_mut(aux).unwrap().mixer.input = Route::Bus(bus);
        s.track_mut(t).unwrap().mixer.output = Route::Bus(bus);
        let m = s.add_track(TrackKind::Master, ChannelFormat::Surround51, None);
        s.track_mut(m).unwrap().mixer.volume_db = -6.0;
        let out = render_range(&s, Range::new(0, 4800), 512);
        let g = db_to_gain(-6.0);
        for (c, ch) in out.iter().enumerate() {
            let x = 0.1 * (c + 1) as f32 + (1000 % 7) as f32 * 1e-3;
            assert!((ch[1000] - x * g).abs() < 1e-5, "channel {c}: {} vs {}", ch[1000], x * g);
        }
        // A mono bus sums a stereo source like the old stereo bus did.
        let (mut s, t) = session_with_dc(0.5, 4800);
        let bus = s.add_bus("Mono", ChannelFormat::Mono);
        let aux = s.add_track(TrackKind::Aux, ChannelFormat::Mono, Some("M"));
        s.track_mut(aux).unwrap().mixer.input = Route::Bus(bus);
        s.track_mut(aux).unwrap().mixer.pan = vec![-1.0];
        s.track_mut(t).unwrap().mixer.output = Route::Bus(bus);
        let out = render_range(&s, Range::new(0, 4800), 512);
        assert!((out[0][1000] - 0.5 * gains(0.0, PanLaw::Minus3).0).abs() < 1e-6, "{}", out[0][1000]);
    }

    #[test]
    fn surround_upmix_and_downmix_between_layouts() {
        // 5.1 → 7.1: shared speakers map 1:1; 5.x surrounds pan between the 7.1 sides and rears.
        let (s, _) = multichannel_session(ChannelFormat::Surround51, ChannelFormat::Surround71, 2000);
        let out = render_range(&s, Range::new(0, 2000), 512);
        assert_eq!(out.len(), 8);
        let x = |c: usize| 0.1 * (c + 1) as f32 + (100 % 7) as f32 * 1e-3;
        for (c, ch) in out.iter().enumerate().take(4) {
            assert!((ch[100] - x(c)).abs() < 1e-6);
        }
        let e_ls = out[4][100].powi(2) + out[6][100].powi(2);
        assert!((e_ls - x(4).powi(2)).abs() < 1e-5, "Ls energy is preserved");
    }
}
