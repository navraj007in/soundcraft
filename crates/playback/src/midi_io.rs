//! MIDI input devices.
//!
//! [`MidiInputs`] opens the system MIDI ports the user enabled. Each device's thread pushes the
//! channel messages it receives into one lock-free ring; the audio callback drains it every block
//! (never locking, allocating or waiting) and plays them on the instruments of armed or monitored
//! tracks. While recording, the callback also stamps each message with the timeline position and
//! hands it to the UI through a second ring, which turns the take into MIDI clips.
//!
//! Only channel voice messages (note, controller, program, pressure, bend) are carried. System
//! messages (clock, sysex, active sensing) are dropped here.

use soundcraft_time::Samples;
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
use std::cmp::Reverse;
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
use std::collections::BinaryHeap;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
use std::time::Duration;
use std::time::Instant;

/// The system's MIDI ports, by name.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PortList {
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
}

/// List the MIDI ports the system offers now. Empty where MIDI is not supported.
pub fn list_ports() -> PortList {
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    {
        let names = |ports: Vec<String>| ports;
        let inputs = midir::MidiInput::new("SoundCraft ports")
            .map(|m| names(m.ports().iter().filter_map(|p| m.port_name(p).ok()).collect()))
            .unwrap_or_default();
        let outputs = midir::MidiOutput::new("SoundCraft ports")
            .map(|m| names(m.ports().iter().filter_map(|p| m.port_name(p).ok()).collect()))
            .unwrap_or_default();
        PortList { inputs, outputs }
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    PortList::default()
}

/// A channel voice message packed in the low 24 bits: status, data 1, data 2.
fn pack(msg: [u8; 3]) -> u32 {
    u32::from(msg[0]) << 16 | u32::from(msg[1]) << 8 | u32::from(msg[2])
}

fn unpack(w: u32) -> [u8; 3] {
    [(w >> 16) as u8, (w >> 8) as u8, w as u8]
}

/// A channel voice message from raw bytes: status 0x80..=0xEF with its data bytes (program change
/// and channel pressure carry one). Anything else is `None`.
pub fn parse_message(bytes: &[u8]) -> Option<[u8; 3]> {
    let status = *bytes.first()?;
    let need = match status & 0xF0 {
        0x80 | 0x90 | 0xA0 | 0xB0 | 0xE0 => 2,
        0xC0 | 0xD0 => 1,
        _ => return None,
    };
    if !(0x80..0xF0).contains(&status) || bytes.len() < need + 1 {
        return None;
    }
    let (d1, d2) = (*bytes.get(1)?, if need == 2 { *bytes.get(2)? } else { 0 });
    (d1 < 0x80 && d2 < 0x80).then_some([status, d1, d2])
}

/// A single-producer single-consumer FIFO of 64-bit words (slots are atomics: no `unsafe`).
struct Ring {
    slots: Box<[AtomicU64]>,
    mask: usize,
    head: AtomicUsize,
    tail: AtomicUsize,
}

impl Ring {
    fn new(capacity: usize) -> Ring {
        let n = capacity.max(2).next_power_of_two();
        Ring { slots: (0..n).map(|_| AtomicU64::new(0)).collect(), mask: n - 1, head: AtomicUsize::new(0), tail: AtomicUsize::new(0) }
    }

    /// `false` when full (the word is dropped).
    fn push(&self, w: u64) -> bool {
        let head = self.head.load(Ordering::Relaxed);
        if head.wrapping_sub(self.tail.load(Ordering::Acquire)) >= self.slots.len() {
            return false;
        }
        if let Some(slot) = self.slots.get(head & self.mask) {
            slot.store(w, Ordering::Relaxed);
        }
        self.head.store(head.wrapping_add(1), Ordering::Release);
        true
    }

    /// Two words as one unit (a reader never sees half). `false` when there is no room for both.
    fn push2(&self, a: u64, b: u64) -> bool {
        let head = self.head.load(Ordering::Relaxed);
        if head.wrapping_sub(self.tail.load(Ordering::Acquire)).saturating_add(2) > self.slots.len() {
            return false;
        }
        for (i, w) in [a, b].into_iter().enumerate() {
            if let Some(slot) = self.slots.get(head.wrapping_add(i) & self.mask) {
                slot.store(w, Ordering::Relaxed);
            }
        }
        self.head.store(head.wrapping_add(2), Ordering::Release);
        true
    }

    #[cfg_attr(not(any(target_os = "macos", target_os = "windows", target_os = "linux")), allow(dead_code))]
    fn pop2(&self) -> Option<(u64, u64)> {
        let tail = self.tail.load(Ordering::Relaxed);
        if self.head.load(Ordering::Acquire).wrapping_sub(tail) < 2 {
            return None;
        }
        let word = |i: usize| self.slots.get(tail.wrapping_add(i) & self.mask).map_or(0, |s| s.load(Ordering::Relaxed));
        let out = (word(0), word(1));
        self.tail.store(tail.wrapping_add(2), Ordering::Release);
        Some(out)
    }

    fn pop(&self) -> Option<u64> {
        let tail = self.tail.load(Ordering::Relaxed);
        if tail == self.head.load(Ordering::Acquire) {
            return None;
        }
        let w = self.slots.get(tail & self.mask).map_or(0, |s| s.load(Ordering::Relaxed));
        self.tail.store(tail.wrapping_add(1), Ordering::Release);
        Some(w)
    }
}

const RING_WORDS: usize = 4096;

/// What the device threads and the audio callback share.
pub struct MidiShared {
    /// Device threads → audio callback. Producers take `producers` so the ring has one writer at a time.
    input: Ring,
    producers: Mutex<()>,
    /// Audio callback → UI: recorded messages with their timeline position.
    record: Ring,
    dropped: AtomicU32,
}

impl MidiShared {
    pub fn new() -> Arc<MidiShared> {
        Arc::new(MidiShared { input: Ring::new(RING_WORDS), producers: Mutex::new(()), record: Ring::new(RING_WORDS), dropped: AtomicU32::new(0) })
    }

    /// From a device thread (also how tests inject a keyboard).
    pub fn push_input(&self, msg: [u8; 3]) {
        let _one_writer = self.producers.lock().unwrap_or_else(PoisonError::into_inner);
        if !self.input.push(u64::from(pack(msg))) {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// From the audio callback: move up to `room` waiting messages into `out` (no allocation while
    /// `out` has capacity) and, when `record_at` is the block's timeline position, queue each for
    /// the UI stamped with it.
    pub fn drain_into(&self, out: &mut Vec<[u8; 3]>, room: usize, record_at: Option<Samples>) {
        while out.len() < room {
            let Some(w) = self.input.pop() else { break };
            let msg = unpack(w as u32);
            out.push(msg);
            if let Some(at) = record_at {
                // 40 bits of position (days of audio) above the 24-bit message.
                let at = u64::try_from(at.max(0)).unwrap_or(0) & ((1 << 40) - 1);
                if !self.record.push(at << 24 | w & 0xFF_FFFF) {
                    self.dropped.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
    }

    /// From the UI: the messages recorded since the last call, with their timeline positions.
    pub fn take_recorded(&self, out: &mut Vec<(Samples, [u8; 3])>) {
        while let Some(w) = self.record.pop() {
            out.push(((w >> 24) as Samples, unpack((w & 0xFF_FFFF) as u32)));
        }
    }

    /// Messages lost because a ring was full, since the last call.
    pub fn take_dropped(&self) -> u32 {
        self.dropped.swap(0, Ordering::Relaxed)
    }
}

/// The open input connections.
pub struct MidiInputs {
    #[cfg_attr(not(any(target_os = "macos", target_os = "windows", target_os = "linux")), allow(dead_code))]
    shared: Arc<MidiShared>,
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    open: Vec<(String, midir::MidiInputConnection<()>)>,
}

impl MidiInputs {
    pub fn new(shared: Arc<MidiShared>) -> MidiInputs {
        MidiInputs {
            shared,
            #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
            open: Vec::new(),
        }
    }

    /// Names of the ports currently open.
    pub fn open_names(&self) -> Vec<String> {
        #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
        {
            self.open.iter().map(|(n, _)| n.clone()).collect()
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
        Vec::new()
    }

    /// Open exactly the ports called `names` (closing any others). Returns one message per port that
    /// could not be opened; a port that is not plugged in is simply skipped.
    pub fn set_enabled(&mut self, names: &[String]) -> Vec<String> {
        #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
        {
            self.open.retain(|(n, _)| names.contains(n));
            let mut problems = Vec::new();
            for name in names {
                if self.open.iter().any(|(n, _)| n == name) {
                    continue;
                }
                let input = match midir::MidiInput::new("SoundCraft input") {
                    Ok(i) => i,
                    Err(e) => {
                        problems.push(format!("MIDI is not available: {e}"));
                        break;
                    }
                };
                let Some(port) = input.ports().into_iter().find(|p| input.port_name(p).is_ok_and(|n| n == *name)) else { continue };
                let shared = Arc::clone(&self.shared);
                match input.connect(
                    &port,
                    "SoundCraft in",
                    move |_stamp, bytes, ()| {
                        if let Some(msg) = parse_message(bytes) {
                            shared.push_input(msg);
                        }
                    },
                    (),
                ) {
                    Ok(c) => self.open.push((name.clone(), c)),
                    Err(e) => problems.push(format!("{name}: {e}")),
                }
            }
            problems
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
        {
            let _ = names;
            Vec::new()
        }
    }
}

/// A stable 32-bit name for a port (FNV-1a): what the audio callback stamps on a message so the
/// output thread can find the connection without the callback touching any string table.
pub fn port_hash(name: &str) -> u32 {
    name.bytes().fold(0x811c_9dc5u32, |h, b| (h ^ u32::from(b)).wrapping_mul(0x0100_0193))
}

/// Most distinct Thru ports at once.
pub const MAX_THRU: usize = 4;

/// What the audio callback and the output thread share: timed messages for output ports.
pub struct MidiOutShared {
    /// Two words per message: when it is due (microseconds since `epoch`), then port hash and bytes.
    ring: Ring,
    epoch: Instant,
    /// Ports (by hash; 0 = unused) that receive everything played on the input devices.
    thru: [AtomicU32; MAX_THRU],
    dropped: AtomicU32,
}

impl MidiOutShared {
    pub fn new() -> Arc<MidiOutShared> {
        Arc::new(MidiOutShared {
            ring: Ring::new(RING_WORDS * 2),
            epoch: Instant::now(),
            thru: std::array::from_fn(|_| AtomicU32::new(0)),
            dropped: AtomicU32::new(0),
        })
    }

    /// Microseconds since the hub started: the clock `due` is on.
    pub fn now_us(&self) -> u64 {
        u64::try_from(self.epoch.elapsed().as_micros()).unwrap_or(u64::MAX)
    }

    /// From the audio callback only (single producer): queue `msg` for the port with `hash`, due at
    /// `due_us`. Never blocks; a full ring drops the message and counts it.
    pub fn send(&self, hash: u32, due_us: u64, msg: [u8; 3]) {
        if !self.ring.push2(due_us, u64::from(hash) << 24 | u64::from(pack(msg))) {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Ports that receive the input devices' messages as they are played (ports by name).
    pub fn set_thru(&self, names: &[String]) {
        for (i, slot) in self.thru.iter().enumerate() {
            slot.store(names.get(i).map_or(0, |n| port_hash(n)), Ordering::Relaxed);
        }
    }

    /// From the audio callback: forward a message from an input device to the Thru ports.
    pub fn thru(&self, due_us: u64, msg: [u8; 3]) {
        for slot in &self.thru {
            let hash = slot.load(Ordering::Relaxed);
            if hash != 0 {
                self.send(hash, due_us, msg);
            }
        }
    }

    pub fn take_dropped(&self) -> u32 {
        self.dropped.swap(0, Ordering::Relaxed)
    }

    /// The next queued message: (due, port hash, bytes). The output thread's; also how tests read.
    #[cfg_attr(not(any(target_os = "macos", target_os = "windows", target_os = "linux")), allow(dead_code))]
    pub(crate) fn pop(&self) -> Option<(u64, u32, [u8; 3])> {
        let (due, rest) = self.ring.pop2()?;
        Some((due, (rest >> 24) as u32, unpack((rest & 0xFF_FFFF) as u32)))
    }
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
struct OutConn {
    name: String,
    hash: u32,
    conn: midir::MidiOutputConnection,
}

/// The open output ports, and the thread that sends messages to them when they are due.
pub struct MidiOutputs {
    shared: Arc<MidiOutShared>,
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    conns: Arc<Mutex<Vec<OutConn>>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl MidiOutputs {
    pub fn new(shared: Arc<MidiOutShared>) -> MidiOutputs {
        let stop = Arc::new(AtomicBool::new(false));
        #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
        let conns: Arc<Mutex<Vec<OutConn>>> = Arc::default();
        #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
        let thread = {
            let (sh, st, cs) = (Arc::clone(&shared), Arc::clone(&stop), Arc::clone(&conns));
            std::thread::Builder::new().name("soundcraft-midi-out".into()).spawn(move || output_loop(&sh, &st, &cs)).ok()
        };
        #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
        let thread = None;
        MidiOutputs {
            shared,
            #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
            conns,
            stop,
            thread,
        }
    }

    pub fn shared(&self) -> &Arc<MidiOutShared> {
        &self.shared
    }

    /// Names of the output ports currently open.
    pub fn open_names(&self) -> Vec<String> {
        #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
        {
            self.conns.lock().unwrap_or_else(PoisonError::into_inner).iter().map(|c| c.name.clone()).collect()
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
        Vec::new()
    }

    /// Open exactly the output ports called `names` (closing any others). Returns one message per
    /// port that could not be opened; a port that is not plugged in is skipped.
    pub fn set_enabled(&mut self, names: &[String]) -> Vec<String> {
        #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
        {
            let mut conns = self.conns.lock().unwrap_or_else(PoisonError::into_inner);
            conns.retain(|c| names.contains(&c.name));
            let mut problems = Vec::new();
            for name in names {
                if conns.iter().any(|c| &c.name == name) {
                    continue;
                }
                let out = match midir::MidiOutput::new("SoundCraft output") {
                    Ok(o) => o,
                    Err(e) => {
                        problems.push(format!("MIDI is not available: {e}"));
                        break;
                    }
                };
                let Some(port) = out.ports().into_iter().find(|p| out.port_name(p).is_ok_and(|n| n == *name)) else { continue };
                match out.connect(&port, "SoundCraft out") {
                    Ok(conn) => conns.push(OutConn { name: name.clone(), hash: port_hash(name), conn }),
                    Err(e) => problems.push(format!("{name}: {e}")),
                }
            }
            problems
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
        {
            let _ = names;
            Vec::new()
        }
    }
}

impl Drop for MidiOutputs {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(t) = self.thread.take() {
            t.thread().unpark();
            let _ = t.join();
        }
    }
}

/// Sends each message when it falls due: the callback queues a block's messages ahead of time with
/// their offsets, this holds them until their moment (to about a millisecond).
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn output_loop(shared: &MidiOutShared, stop: &AtomicBool, conns: &Mutex<Vec<OutConn>>) {
    const MAX_PENDING: usize = 1 << 16;
    let mut pending: BinaryHeap<Reverse<(u64, u64, u32, [u8; 3])>> = BinaryHeap::new();
    let mut seq = 0u64;
    while !stop.load(Ordering::Acquire) {
        while let Some((due, hash, msg)) = shared.pop() {
            if pending.len() < MAX_PENDING {
                pending.push(Reverse((due, seq, hash, msg)));
                seq += 1;
            }
        }
        let now = shared.now_us();
        let mut sleep = 2_000u64;
        while let Some(Reverse((due, _, hash, msg))) = pending.peek().copied() {
            if due > now {
                sleep = sleep.min(due - now);
                break;
            }
            pending.pop();
            let mut cs = conns.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(c) = cs.iter_mut().find(|c| c.hash == hash) {
                let n = match msg[0] & 0xF0 {
                    0xC0 | 0xD0 => 2,
                    _ => 3,
                };
                let _ = c.conn.send(msg.get(..n).unwrap_or(&msg));
            }
        }
        std::thread::park_timeout(Duration::from_micros(sleep.max(200)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_messages_parse_and_everything_else_is_dropped() {
        assert_eq!(parse_message(&[0x90, 60, 100]), Some([0x90, 60, 100]));
        assert_eq!(parse_message(&[0x80, 60, 0]), Some([0x80, 60, 0]));
        assert_eq!(parse_message(&[0xB0, 64, 127]), Some([0xB0, 64, 127]));
        assert_eq!(parse_message(&[0xE0, 0, 64]), Some([0xE0, 0, 64]));
        assert_eq!(parse_message(&[0xC3, 5]), Some([0xC3, 5, 0]), "program change has one data byte");
        assert_eq!(parse_message(&[0xD0, 90]), Some([0xD0, 90, 0]));
        for bad in [&[0xF8][..], &[0xF0, 1, 2, 0xF7], &[0xFE], &[], &[0x90], &[0x90, 60], &[0x90, 200, 1], &[0x40, 1, 2]] {
            assert_eq!(parse_message(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn the_callback_gets_every_message_in_order_and_the_ui_gets_the_recorded_ones() {
        let shared = MidiShared::new();
        for m in [[0x90, 60, 100], [0x90, 64, 90], [0x80, 60, 0], [0xB0, 64, 127]] {
            shared.push_input(m);
        }
        let mut live = Vec::with_capacity(16);
        shared.drain_into(&mut live, 16, Some(48_000));
        assert_eq!(live, vec![[0x90, 60, 100], [0x90, 64, 90], [0x80, 60, 0], [0xB0, 64, 127]]);
        let mut rec = Vec::new();
        shared.take_recorded(&mut rec);
        assert_eq!(rec.len(), 4);
        assert!(rec.iter().all(|(at, _)| *at == 48_000));
        assert_eq!(rec[2].1, [0x80, 60, 0]);
        // Not recording: the callback still plays them but nothing is queued for the UI.
        shared.push_input([0x90, 62, 80]);
        live.clear();
        shared.drain_into(&mut live, 16, None);
        assert_eq!(live.len(), 1);
        rec.clear();
        shared.take_recorded(&mut rec);
        assert!(rec.is_empty());
    }

    #[test]
    fn a_full_ring_drops_and_counts_instead_of_blocking() {
        let shared = MidiShared::new();
        for i in 0..(RING_WORDS + 10) {
            shared.push_input([0x90, (i % 100) as u8, 1]);
        }
        assert_eq!(shared.take_dropped(), 10);
        let mut live = Vec::with_capacity(RING_WORDS);
        shared.drain_into(&mut live, RING_WORDS, None);
        assert_eq!(live.len(), RING_WORDS);
    }

    #[test]
    fn the_callback_takes_no_more_than_the_room_it_has() {
        let shared = MidiShared::new();
        for _ in 0..10 {
            shared.push_input([0x90, 60, 1]);
        }
        let mut live = Vec::with_capacity(4);
        shared.drain_into(&mut live, 4, None);
        assert_eq!(live.len(), 4);
        shared.drain_into(&mut live, 8, None);
        assert_eq!(live.len(), 8, "the rest waits for the next block");
    }

    #[test]
    fn output_messages_keep_their_due_time_port_and_bytes_in_order() {
        let out = MidiOutShared::new();
        out.send(port_hash("Synth A"), 1_000, [0x92, 60, 100]);
        out.send(port_hash("Synth B"), 2_500, [0xC1, 5, 0]);
        assert_eq!(out.pop(), Some((1_000, port_hash("Synth A"), [0x92, 60, 100])));
        assert_eq!(out.pop(), Some((2_500, port_hash("Synth B"), [0xC1, 5, 0])));
        assert_eq!(out.pop(), None);
        assert_ne!(port_hash("Synth A"), port_hash("Synth B"));
    }

    #[test]
    fn a_full_output_ring_drops_whole_messages_and_counts_them() {
        let out = MidiOutShared::new();
        for i in 0..(RING_WORDS * 2) {
            out.send(7, i as u64, [0x90, 60, 1]);
        }
        let dropped = out.take_dropped();
        assert!(dropped > 0);
        let mut got = 0;
        while let Some((due, hash, msg)) = out.pop() {
            assert_eq!((hash, msg), (7, [0x90, 60, 1]), "no half-written message ever shows");
            assert_eq!(due, got as u64);
            got += 1;
        }
        assert_eq!(got as u32 + dropped, (RING_WORDS * 2) as u32);
    }

    #[test]
    fn thru_forwards_to_every_chosen_port_and_only_those() {
        let out = MidiOutShared::new();
        out.thru(0, [0x90, 60, 100]);
        assert_eq!(out.pop(), None, "no Thru port chosen");
        out.set_thru(&["A".into(), "B".into()]);
        out.thru(5, [0x90, 61, 100]);
        assert_eq!((out.pop().map(|e| e.1), out.pop().map(|e| e.1)), (Some(port_hash("A")), Some(port_hash("B"))));
        out.set_thru(&[]);
        out.thru(5, [0x90, 62, 100]);
        assert_eq!(out.pop(), None);
    }

    #[test]
    fn the_output_thread_starts_and_stops_cleanly() {
        let out = MidiOutputs::new(MidiOutShared::new());
        assert!(out.open_names().is_empty());
        drop(out);
    }

    #[test]
    fn listing_ports_never_panics() {
        let _ = list_ports();
    }
}
