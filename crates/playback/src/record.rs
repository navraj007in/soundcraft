//! Audio input capture for recording.
//!
//! A [`Recorder`] opens the default input device. While armed, the input callback copies each
//! block into a preallocated lock-free ring (it never locks or allocates), and a writer thread
//! streams the ring into one WAV file per take, rewriting the header every second so a crash leaves
//! a playable file.

use std::fs::File;
use std::io::{Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use soundcraft_model::TrackId;

/// What [`Recorder::finish`] returns: the takes that hold audio and the first problem met.
pub type Takes = (Vec<(TrackId, PathBuf)>, Option<String>);

const RING_SECONDS: usize = 4;

/// Single-producer single-consumer FIFO of samples (input callback → writer thread). The slots
/// are atomics, so it needs no lock and no `unsafe`.
struct CaptureRing {
    slots: Box<[AtomicU32]>,
    mask: usize,
    head: AtomicUsize,
    tail: AtomicUsize,
}

impl CaptureRing {
    fn new(capacity: usize) -> CaptureRing {
        let n = capacity.max(2).next_power_of_two();
        CaptureRing { slots: (0..n).map(|_| AtomicU32::new(0)).collect(), mask: n - 1, head: AtomicUsize::new(0), tail: AtomicUsize::new(0) }
    }

    /// Queue up to `n` samples (`sample(i)` is the i-th); returns how many fit.
    fn push(&self, n: usize, sample: impl Fn(usize) -> f32) -> usize {
        let head = self.head.load(Ordering::Relaxed);
        let n = n.min(self.slots.len().saturating_sub(head.wrapping_sub(self.tail.load(Ordering::Acquire))));
        for i in 0..n {
            if let Some(slot) = self.slots.get(head.wrapping_add(i) & self.mask) {
                slot.store(sample(i).to_bits(), Ordering::Relaxed);
            }
        }
        self.head.store(head.wrapping_add(n), Ordering::Release);
        n
    }

    fn pop_all(&self, out: &mut Vec<f32>) {
        let (tail, head) = (self.tail.load(Ordering::Relaxed), self.head.load(Ordering::Acquire));
        out.extend(
            (0..head.wrapping_sub(tail))
                .map(|i| self.slots.get(tail.wrapping_add(i) & self.mask).map_or(0.0, |s| f32::from_bits(s.load(Ordering::Relaxed)))),
        );
        self.tail.store(head, Ordering::Release);
    }
}

struct Shared {
    armed: AtomicBool,
    ring: CaptureRing,
    /// Samples lost while the ring was full.
    dropped: AtomicUsize,
}

impl Shared {
    /// The input callback's work while armed. Samples that don't fit are owed (`owed`) and queued
    /// as silence before the next block, so the take keeps its timing.
    fn capture(&self, data: &[f32], owed: &mut usize) {
        *owed -= self.ring.push(*owed, |_| 0.0);
        let kept = if *owed == 0 { self.ring.push(data.len(), |i| data.get(i).copied().unwrap_or(0.0)) } else { 0 };
        *owed = owed.saturating_add(data.len() - kept);
        self.dropped.fetch_add(data.len() - kept, Ordering::Relaxed);
    }
}

pub struct Recorder {
    shared: Arc<Shared>,
    /// Live input for monitoring (always fed while the input stream runs).
    pub monitor: Arc<InputRing>,
    #[cfg(not(target_os = "freebsd"))]
    _stream: Option<cpal::Stream>,
    pub device_name: String,
    pub sample_rate: u32,
    pub channels: usize,
    writer: Option<std::thread::JoinHandle<Takes>>,
}

impl Recorder {
    /// Open the default input device. Errors when there is none (the caller shows a message).
    #[cfg(not(target_os = "freebsd"))]
    pub fn open() -> Result<Recorder, String> {
        use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
        let host = cpal::default_host();
        let device = host.default_input_device().ok_or("no audio input device")?;
        #[allow(deprecated)]
        let name = device.name().unwrap_or_else(|_| "Input".into());
        let cfg = device.default_input_config().map_err(|e| e.to_string())?;
        if cfg.sample_format() != cpal::SampleFormat::F32 {
            return Err(format!("unsupported input sample format {:?}", cfg.sample_format()));
        }
        let config = cfg.config();
        let channels = usize::from(config.channels).max(1);
        let ring = CaptureRing::new((config.sample_rate.0 as usize).saturating_mul(channels * RING_SECONDS).min(1 << 25));
        let shared = Arc::new(Shared { armed: AtomicBool::new(false), ring, dropped: AtomicUsize::new(0) });
        let sh = Arc::clone(&shared);
        let monitor = InputRing::new(usize::from(config.channels), config.sample_rate.0);
        let mon = Arc::clone(&monitor);
        let mut owed = 0;
        let stream = device
            .build_input_stream(
                &config,
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    crate::mark_audio_thread();
                    mon.push(data);
                    if sh.armed.load(Ordering::Acquire) {
                        sh.capture(data, &mut owed);
                    } else {
                        owed = 0;
                    }
                },
                |e| {
                    // Possibly the audio thread (see `crate::mark_audio_thread`).
                    crate::mark_audio_thread();
                    log::warn!("input stream error: {e}");
                },
                None,
            )
            .map_err(|e| e.to_string())?;
        stream.play().map_err(|e| e.to_string())?;
        Ok(Recorder { monitor, shared, _stream: Some(stream), device_name: name, sample_rate: config.sample_rate.0, channels, writer: None })
    }

    #[cfg(target_os = "freebsd")]
    pub fn open() -> Result<Recorder, String> {
        Err("recording is not supported on this platform yet".into())
    }

    /// Create each take's file and start recording into them: a track's take records the given
    /// input channels into a new file.
    pub fn arm(&mut self, takes: Vec<(TrackId, Vec<usize>, PathBuf)>) -> Result<(), String> {
        if self.writer.is_some() {
            return Err("already recording".into());
        }
        let takes =
            takes.into_iter().map(|(id, inputs, path)| TakeWriter::create(id, inputs, path, self.sample_rate)).collect::<Result<Vec<_>, _>>()?;
        let ring = &self.shared.ring;
        ring.tail.store(ring.head.load(Ordering::Acquire), Ordering::Release);
        self.shared.armed.store(true, Ordering::Release);
        let (shared, channels) = (Arc::clone(&self.shared), self.channels);
        match std::thread::Builder::new().name("take writer".into()).spawn(move || write_takes(&shared, takes, channels)) {
            Ok(w) => self.writer = Some(w),
            Err(e) => {
                self.shared.armed.store(false, Ordering::Release);
                return Err(e.to_string());
            }
        }
        Ok(())
    }

    pub fn finish(&mut self) -> Takes {
        self.shared.armed.store(false, Ordering::Release);
        let (takes, problem) = self.writer.take().and_then(|w| w.join().ok()).unwrap_or_default();
        let lost = self.shared.dropped.swap(0, Ordering::Relaxed) / self.channels.max(1);
        let slow =
            (lost > 0).then(|| format!("{} ms of input lost: the disk was too slow", lost.saturating_mul(1000) / self.sample_rate.max(1) as usize));
        (takes, problem.or(slow))
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        self.finish();
    }
}

fn write_takes(shared: &Shared, mut takes: Vec<TakeWriter>, channels: usize) -> Takes {
    let (mut block, mut header_written) = (Vec::new(), Instant::now());
    loop {
        let stopping = !shared.armed.load(Ordering::Acquire);
        // The ring holds interleaved samples and a pop can end mid-frame: write whole frames and
        // keep the remainder for the next pop, so channels never shift.
        shared.ring.pop_all(&mut block);
        let whole = block.len() - block.len() % channels.max(1);
        takes.iter_mut().for_each(|t| t.write(&block[..whole], channels));
        block.drain(..whole);
        if stopping {
            break;
        }
        if header_written.elapsed() >= Duration::from_secs(1) {
            header_written = Instant::now();
            takes.iter_mut().for_each(TakeWriter::write_header);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let (mut done, mut problem) = (Vec::new(), None);
    for mut t in takes {
        t.write_header();
        if t.frames == 0 {
            let _ = std::fs::remove_file(&t.path);
        } else {
            done.push((t.id, t.path));
        }
        problem = problem.or(t.error);
    }
    (done, problem)
}

/// One take's file. After an error it stops writing; its header keeps covering what was written.
struct TakeWriter {
    id: TrackId,
    inputs: Vec<usize>,
    path: PathBuf,
    file: File,
    rate: u32,
    frames: u64,
    error: Option<String>,
}

impl TakeWriter {
    fn create(id: TrackId, inputs: Vec<usize>, path: PathBuf, rate: u32) -> Result<TakeWriter, String> {
        let file = path.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|()| File::create_new(&path));
        let file = file.map_err(|e| format!("{}: {e}", path.display()))?;
        let mut take = TakeWriter { id, inputs, path, file, rate, frames: 0, error: None };
        take.write_header();
        take.error.take().map_or(Ok(take), Err)
    }

    /// Append this take's channels of `block` (interleaved, `channels` wide).
    fn write(&mut self, block: &[f32], channels: usize) {
        let channels = channels.max(1);
        if self.error.is_some() {
            return;
        }
        let bytes: Vec<u8> = block
            .chunks_exact(channels)
            .flat_map(|f| self.inputs.iter().flat_map(move |&i| f.get(i).copied().unwrap_or(0.0).to_le_bytes()))
            .collect();
        match self.file.write_all(&bytes) {
            Ok(()) => self.frames += (block.len() / channels) as u64,
            Err(e) => self.fail(e),
        }
    }

    fn write_header(&mut self) {
        let header = match soundcraft_audio_io::wav_float_header(self.inputs.len(), self.rate, self.frames) {
            Ok(h) => h,
            Err(e) => return self.fail(e),
        };
        let f = &mut self.file;
        if let Err(e) = f.seek(SeekFrom::Start(0)).and_then(|_| f.write_all(&header)).and_then(|()| f.seek(SeekFrom::End(0))) {
            self.fail(e);
        }
    }

    fn fail(&mut self, e: impl std::fmt::Display) {
        self.error.get_or_insert_with(|| format!("{}: {e}", self.path.display()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(path: &PathBuf) -> Vec<Vec<f32>> {
        soundcraft_audio_io::decode(&std::fs::read(path).unwrap(), None).unwrap().1.channels
    }

    #[test]
    fn a_full_ring_owes_the_lost_samples_as_silence() {
        let shared = Shared { armed: AtomicBool::new(true), ring: CaptureRing::new(8), dropped: AtomicUsize::new(0) };
        let (mut owed, mut out) = (0, Vec::new());
        shared.capture(&[1.0; 6], &mut owed);
        shared.capture(&[2.0; 4], &mut owed);
        shared.ring.pop_all(&mut out);
        shared.capture(&[3.0; 2], &mut owed);
        shared.ring.pop_all(&mut out);
        assert_eq!(out, [vec![1.0; 6], vec![2.0; 2], vec![0.0; 2], vec![3.0; 2]].concat());
        assert_eq!((owed, shared.dropped.load(Ordering::Relaxed)), (0, 2));
    }

    #[test]
    fn takes_stream_into_their_files() {
        let dir = std::env::temp_dir().join(format!("soundcraft-takes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let gtr = TakeWriter::create(TrackId(1), vec![1, 2], dir.join("Gtr_01.wav"), 48_000).unwrap();
        let mut vox = TakeWriter::create(TrackId(2), vec![0], dir.join("Vox_01.wav"), 48_000).unwrap();
        assert!(TakeWriter::create(TrackId(3), vec![0], dir.join("Vox_01.wav"), 48_000).is_err(), "never overwrites a file");
        vox.write(&[0.5; 300], 3);
        vox.write_header();
        vox.write(&[0.25; 150], 3);
        assert_eq!(decode(&dir.join("Vox_01.wav"))[0].len(), 100, "the header covers what it has seen");

        let shared = Arc::new(Shared { armed: AtomicBool::new(true), ring: CaptureRing::new(1 << 16), dropped: AtomicUsize::new(0) });
        let sh = Arc::clone(&shared);
        let writer = std::thread::spawn(move || write_takes(&sh, vec![gtr, vox], 3));
        let input: Vec<f32> = (0..3 * 4_800).map(|i| i as f32 / 20_000.0).collect();
        let mut owed = 0;
        input.chunks(3 * 480).for_each(|block| shared.capture(block, &mut owed));
        shared.armed.store(false, Ordering::Release);
        let (done, problem) = writer.join().unwrap();
        let channel = |c: usize| input.iter().skip(c).step_by(3).copied().collect::<Vec<f32>>();
        assert_eq!((done.len(), problem), (2, None));
        assert_eq!(decode(&done[0].1), vec![channel(1), channel(2)]);
        assert_eq!(decode(&done[1].1), vec![[vec![0.5; 100], vec![0.25; 50], channel(0)].concat()]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// Fault tests for the recording path (plan task P0-T04). They feed synthetic audio through the real
/// writer and never open an input device. Tests named `finding_` pin today's behaviour so that a
/// later repair (plan P3-R03) has to change them on purpose.
#[cfg(test)]
mod fault_tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("soundcraft-fault-{name}-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    /// A recorder with no input device behind it, so the whole arm and finish path can run.
    fn deviceless(ring_samples: usize, channels: usize, rate: u32) -> Recorder {
        let shared = Arc::new(Shared { armed: AtomicBool::new(false), ring: CaptureRing::new(ring_samples), dropped: AtomicUsize::new(0) });
        Recorder {
            shared,
            monitor: InputRing::new(channels, rate),
            _stream: None,
            device_name: "test".into(),
            sample_rate: rate,
            channels,
            writer: None,
        }
    }

    /// Frames the file holds, from its length once the header is taken off (float32 samples).
    fn frames_on_disk(path: &PathBuf, channels: usize) -> u64 {
        let header = soundcraft_audio_io::wav_float_header(channels, 48_000, 0).unwrap().len() as u64;
        (std::fs::metadata(path).unwrap().len().saturating_sub(header)) / (4 * channels as u64)
    }

    fn frames_in_header(path: &PathBuf) -> usize {
        soundcraft_audio_io::decode(&std::fs::read(path).unwrap(), None).unwrap().1.channels[0].len()
    }

    #[cfg(unix)]
    #[test]
    fn a_read_only_folder_refuses_the_take_and_names_the_path() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp("readonly");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();
        let path = dir.join("Vox_01.wav");
        let err = TakeWriter::create(TrackId(1), vec![0], path.clone(), 48_000).err().expect("must fail");
        assert!(err.contains("Vox_01.wav"), "the message names the file: {err}");
        assert!(!path.exists(), "no partial file");
        // Through the recorder: arm fails, nothing is armed, and a later arm elsewhere still works.
        let mut r = deviceless(1 << 12, 1, 48_000);
        assert!(r.arm(vec![(TrackId(1), vec![0], path)]).is_err());
        assert!(!r.shared.armed.load(Ordering::Acquire), "a failed arm must not leave the recorder armed");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        let other = temp("readonly-ok").join("Vox_02.wav");
        assert!(r.arm(vec![(TrackId(1), vec![0], other)]).is_ok());
        let _ = r.finish();
    }

    #[test]
    fn the_whole_path_without_a_device_keeps_every_sample() {
        let dir = temp("path");
        let mut r = deviceless(1 << 16, 2, 48_000);
        r.arm(vec![(TrackId(1), vec![1], dir.join("A.wav"))]).unwrap();
        let input: Vec<f32> = (0..2 * 9_600).map(|i| (i % 1000) as f32 / 1000.0).collect();
        let mut owed = 0;
        for block in input.chunks(2 * 480) {
            r.shared.capture(block, &mut owed);
            std::thread::sleep(Duration::from_millis(2));
        }
        let (takes, problem) = r.finish();
        assert_eq!(problem, None);
        let right: Vec<f32> = input.iter().skip(1).step_by(2).copied().collect();
        assert_eq!(soundcraft_audio_io::decode(&std::fs::read(&takes[0].1).unwrap(), None).unwrap().1.channels, vec![right]);
    }

    #[test]
    fn finding_overflow_is_reported_only_when_the_take_ends() {
        let dir = temp("overflow");
        let mut r = deviceless(8, 1, 48_000); // a ring of 8 samples: any larger block overflows it
        r.arm(vec![(TrackId(1), vec![0], dir.join("A.wav"))]).unwrap();
        let mut owed = 0;
        r.shared.capture(&[0.5; 4_800], &mut owed);
        assert!(r.shared.dropped.load(Ordering::Relaxed) > 4_000, "the loss is counted while recording...");
        // ...but nothing in the recorder's public surface exposes it before `finish`.
        let (_takes, problem) = r.finish();
        let problem = problem.expect("a loss must be reported at the end");
        assert!(problem.contains("ms of input lost"), "{problem}");
    }

    #[test]
    fn finding_a_failed_write_is_silent_until_the_end_and_the_loss_is_not_counted() {
        let dir = temp("writefail");
        let path = dir.join("A.wav");
        TakeWriter::create(TrackId(1), vec![0], path.clone(), 48_000).unwrap();
        // Reopen the file read-only so every write fails, as a full disk or a vanished volume would.
        let broken = TakeWriter {
            id: TrackId(1),
            inputs: vec![0],
            path: path.clone(),
            file: File::open(&path).unwrap(),
            rate: 48_000,
            frames: 0,
            error: None,
        };
        let shared = Arc::new(Shared { armed: AtomicBool::new(true), ring: CaptureRing::new(1 << 16), dropped: AtomicUsize::new(0) });
        let sh = Arc::clone(&shared);
        let writer = std::thread::spawn(move || write_takes(&sh, vec![broken], 1));
        let mut owed = 0;
        for _ in 0..50 {
            shared.capture(&[0.25; 480], &mut owed); // 0.5 s of audio, 10 ms per block
            std::thread::sleep(Duration::from_millis(10));
        }
        let still_counted_as_lost = shared.dropped.load(Ordering::Relaxed);
        shared.armed.store(false, Ordering::Release);
        let (takes, problem) = writer.join().unwrap();
        assert_eq!(still_counted_as_lost, 0, "the ring kept draining, so nothing looked lost while 0.5 s was being discarded");
        assert!(problem.is_some(), "the error does surface, at the end");
        assert!(takes.is_empty(), "a take with no frames is removed");
    }

    #[test]
    fn finding_a_crash_leaves_audio_the_header_does_not_cover() {
        let dir = temp("crash");
        let path = dir.join("A.wav");
        let mut t = TakeWriter::create(TrackId(1), vec![0], path.clone(), 48_000).unwrap();
        t.write(&vec![0.5; 4_800], 1);
        t.write_header(); // the once-a-second header refresh
        t.write(&vec![0.5; 2_400], 1);
        drop(t); // an app crash: no final header, nothing buffered in the process (File is unbuffered)
        assert_eq!(frames_in_header(&path), 4_800, "a reader trusting the header stops at the last refresh");
        assert_eq!(frames_on_disk(&path, 1), 7_200, "but all the written audio is on disk");
    }

    /// Child half of the real `kill -9` test: records a paced 48 kHz mono take until killed.
    #[test]
    #[ignore = "helper process for kill_nine_*; not a test on its own"]
    fn child_records_until_killed() {
        let Ok(dir) = std::env::var("SC_KILL_DIR") else { return };
        let path = PathBuf::from(dir).join("A.wav");
        let take = TakeWriter::create(TrackId(1), vec![0], path, 48_000).unwrap();
        let shared = Arc::new(Shared { armed: AtomicBool::new(true), ring: CaptureRing::new(1 << 18), dropped: AtomicUsize::new(0) });
        let sh = Arc::clone(&shared);
        std::thread::spawn(move || write_takes(&sh, vec![take], 1));
        let mut owed = 0;
        loop {
            shared.capture(&[0.25; 480], &mut owed);
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// A real SIGKILL mid-take. Run with `cargo test -p soundcraft-playback kill_nine -- --ignored --nocapture`.
    #[cfg(unix)]
    #[test]
    #[ignore = "spawns and kills a child process and takes about 4 s"]
    fn kill_nine_loses_at_most_the_last_header_interval() {
        let dir = temp("kill9");
        let exe = std::env::current_exe().unwrap();
        let mut child = std::process::Command::new(exe)
            .args(["--exact", "record::fault_tests::child_records_until_killed", "--ignored", "--nocapture"])
            .env("SC_KILL_DIR", &dir)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        // The header refreshes about once a second, so where in that cycle the kill lands decides the
        // lag; `SC_KILL_AFTER_MS` lets a sweep sample the whole cycle.
        let after: u64 = std::env::var("SC_KILL_AFTER_MS").ok().and_then(|v| v.parse().ok()).unwrap_or(3_500);
        std::thread::sleep(Duration::from_millis(after));
        child.kill().unwrap(); // SIGKILL
        child.wait().unwrap();
        let path = dir.join("A.wav");
        let (header, disk) = (frames_in_header(&path) as f64, frames_on_disk(&path, 1) as f64);
        let (lost_to_header, total) = ((disk - header) / 48_000.0, disk / 48_000.0);
        println!(
            "KILL-9 RESULT (killed at {after} ms): {total:.2} s on disk; header covers {:.2} s; header lag {lost_to_header:.2} s",
            header / 48_000.0
        );
        assert!(total > 1.0, "the child recorded for a while ({total:.2} s)");
        assert!(lost_to_header <= 1.2, "the header trails the data by at most about one refresh interval: {lost_to_header:.2} s");
        assert!(frames_in_header(&path) > 0, "the file still decodes");
    }

    /// Fill a small volume. Needs a nearly full or tiny volume: see the plan note for a RAM disk recipe.
    /// Run with `SC_FULL_DISK_DIR=/Volumes/RAMDISK cargo test -p soundcraft-playback disk_full -- --ignored --nocapture`.
    #[test]
    #[ignore = "needs SC_FULL_DISK_DIR pointing at a small volume"]
    fn disk_full_is_reported_but_only_at_the_end() {
        let Ok(dir) = std::env::var("SC_FULL_DISK_DIR") else { return };
        let path = PathBuf::from(dir).join("Full_01.wav");
        let take = TakeWriter::create(TrackId(1), vec![0], path.clone(), 48_000).unwrap();
        let shared = Arc::new(Shared { armed: AtomicBool::new(true), ring: CaptureRing::new(1 << 18), dropped: AtomicUsize::new(0) });
        let sh = Arc::clone(&shared);
        let writer = std::thread::spawn(move || write_takes(&sh, vec![take], 1));
        let mut owed = 0;
        let started = Instant::now();
        let mut lost_while_recording = 0usize;
        for _ in 0..2_000 {
            shared.capture(&[0.25; 480], &mut owed);
            lost_while_recording = shared.dropped.load(Ordering::Relaxed);
            std::thread::sleep(Duration::from_millis(10));
        }
        let recorded_for = started.elapsed().as_secs_f64();
        shared.armed.store(false, Ordering::Release);
        let (takes, problem) = writer.join().unwrap();
        let on_disk = std::fs::metadata(&path)
            .map(|m| {
                m.len().saturating_sub(soundcraft_audio_io::wav_float_header(1, 48_000, 0).map_or(44, |h| h.len() as u64)) as f64 / 4.0 / 48_000.0
            })
            .unwrap_or(0.0);
        println!(
            "DISK-FULL RESULT: fed {recorded_for:.1} s, {on_disk:.1} s reached the disk, counted as lost while recording: {lost_while_recording}, reported at end: {problem:?}, takes returned: {}",
            takes.len()
        );
        assert!(problem.is_some(), "a full disk must be reported at the end");
    }
}

/// Ring state behind the lock: interleaved input frames at the input device's rate, plus the
/// fractional read position (in input frames) of the next output sample.
struct RingState {
    data: std::collections::VecDeque<f32>,
    phase: f64,
}

/// A small interleaved FIFO from the input callback to the output callback, for input
/// monitoring. Both sides only `try_lock`; it holds at most ~0.5 s and drops the oldest frames.
/// The input device may run at a different rate than the session, so the reader converts
/// rates with linear interpolation (cheap, allocation-free, fine for monitoring).
pub struct InputRing {
    state: Mutex<RingState>,
    pub channels: std::sync::atomic::AtomicUsize,
    /// Input device sample rate in Hz (0 when unknown, which reads as "same as the session").
    rate: u32,
    /// Most samples kept: half a second of input, or 24 000 frames when the rate is unknown.
    cap: usize,
}

impl InputRing {
    pub fn new(channels: usize, rate: u32) -> Arc<InputRing> {
        let ch = channels.max(1);
        let frames = if rate == 0 { 24_000 } else { rate as usize / 2 };
        Arc::new(InputRing {
            state: Mutex::new(RingState { data: std::collections::VecDeque::with_capacity(rate.max(48_000) as usize * ch), phase: 0.0 }),
            channels: std::sync::atomic::AtomicUsize::new(ch),
            rate,
            cap: frames * ch,
        })
    }

    pub fn push(&self, samples: &[f32]) {
        if let Ok(mut s) = self.state.try_lock() {
            s.data.extend(samples.iter().copied());
            while s.data.len() > self.cap {
                s.data.pop_front();
            }
        }
    }

    /// Fill `out` (planar, at `session_rate`) with up to `frames` frames, converting from the
    /// input rate. Missing frames are silence.
    pub fn pop_into(&self, out: &mut [Vec<f32>], frames: usize, session_rate: u32) {
        let ch = self.channels.load(Ordering::Relaxed).max(1);
        for c in out.iter_mut() {
            c.iter_mut().take(frames).for_each(|x| *x = 0.0);
        }
        let Ok(mut s) = self.state.try_lock() else { return };
        // Input frames consumed per output frame.
        let step = if self.rate == 0 || session_rate == 0 { 1.0 } else { f64::from(self.rate) / f64::from(session_rate) };
        for f in 0..frames {
            // Drop input frames we have moved past; keep the one at `phase` and the next one.
            while s.phase >= 1.0 && s.data.len() >= 2 * ch {
                s.data.drain(..ch);
                s.phase -= 1.0;
            }
            if s.phase >= 1.0 || s.data.len() < 2 * ch {
                break;
            }
            let fr = s.phase as f32;
            for c in 0..ch {
                let a = s.data.get(c).copied().unwrap_or(0.0);
                let b = s.data.get(ch + c).copied().unwrap_or(a);
                if let Some(x) = out.get_mut(c).and_then(|o| o.get_mut(f)) {
                    *x = a + (b - a) * fr;
                }
            }
            s.phase += step;
        }
    }
}

#[cfg(test)]
mod ring_tests {
    use super::InputRing;

    /// A 1 kHz sine recorded at 16 kHz must come out as a 1 kHz sine at 48 kHz, across calls.
    #[test]
    fn monitor_converts_input_rate_to_session_rate() {
        let ring = InputRing::new(1, 16_000);
        let input: Vec<f32> = (0..4_000).map(|i| (2.0 * std::f64::consts::PI * 1_000.0 * f64::from(i) / 16_000.0).sin() as f32).collect();
        ring.push(&input);
        let mut out = vec![vec![0.0f32; 48]];
        let mut k = 0usize;
        for _ in 0..10 {
            ring.pop_into(&mut out, 48, 48_000);
            for v in &out[0] {
                let want = (2.0 * std::f64::consts::PI * 1_000.0 * k as f64 / 48_000.0).sin() as f32;
                assert!((v - want).abs() < 0.05, "sample {k}: got {v}, want {want}");
                k += 1;
            }
        }
    }

    /// Input faster than the session (96 kHz into 48 kHz): every other frame is read, in order.
    #[test]
    fn monitor_decimates_when_input_is_faster() {
        let ring = InputRing::new(1, 96_000);
        let input: Vec<f32> = (0..1_000).map(|i| i as f32).collect();
        ring.push(&input);
        let mut out = vec![vec![0.0f32; 100]];
        ring.pop_into(&mut out, 100, 48_000);
        for (k, v) in out[0].iter().enumerate() {
            assert_eq!(*v, (2 * k) as f32, "output {k}");
        }
    }

    /// Same rate: samples pass through unchanged.
    #[test]
    fn monitor_passes_through_at_equal_rates() {
        let ring = InputRing::new(2, 48_000);
        let input: Vec<f32> = (0..200).map(|i| i as f32).collect();
        ring.push(&input);
        let mut out = vec![vec![0.0f32; 50], vec![0.0f32; 50]];
        ring.pop_into(&mut out, 50, 48_000);
        assert_eq!(out[0][0], 0.0);
        assert_eq!(out[1][0], 1.0);
        assert_eq!(out[0][49], 98.0);
    }

    /// Nothing buffered: silence, not stale data.
    #[test]
    fn monitor_underrun_is_silence() {
        let ring = InputRing::new(1, 16_000);
        let mut out = vec![vec![1.0f32; 16]];
        ring.pop_into(&mut out, 16, 48_000);
        assert!(out[0].iter().all(|v| *v == 0.0));
    }
}
