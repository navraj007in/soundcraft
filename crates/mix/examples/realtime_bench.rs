//! Per-block render time as the audio callback sees it (the thread is marked as the audio thread),
//! serial versus handing strips to the shared worker pool:
//! `cargo run --release -p soundcraft-mix --example realtime_bench -- [tracks] [blocks] [block_frames]`
//!
//! A block must finish within `block_frames / 48000` seconds (10.7 ms at 512). Compare the worst and
//! 99.9th-percentile times with that budget, not the mean.

use soundcraft_audio_io::AudioBuffer;
use soundcraft_mix::{MixEngine, allow_realtime_parallel, mark_audio_thread};
use soundcraft_model::{ChannelFormat, Clip, Insert, Session, SourceAudio, SourceId, TrackKind};
use std::sync::Arc;
use std::time::Instant;

fn session(tracks: usize, frames: usize) -> Session {
    let mut s = Session::default();
    let buf = AudioBuffer { sample_rate: 48_000, channels: vec![(0..frames).map(|i| ((i as f32) * 0.01).sin() * 0.1).collect()] };
    s.pool.insert(SourceId(1_000_000), Arc::new(SourceAudio::new(buf)));
    for i in 0..tracks {
        let t = s.add_track(TrackKind::Audio, ChannelFormat::Mono, None);
        let id = s.new_clip_id();
        let Some(tr) = s.track_mut(t) else { continue };
        if let Some(p) = tr.playlist_mut() {
            p.clips.push(Clip::audio(id, "c", SourceId(1_000_000), 0, 0, frames as i64));
        }
        tr.mixer.inserts[0] = Some(Insert::new("eq_7band"));
        tr.mixer.inserts[1] = Some(Insert::new("compressor"));
        if i % 4 == 0 {
            tr.mixer.inserts[2] = Some(Insert::new("plate_reverb"));
        }
    }
    s
}

fn run(s: &Session, blocks: usize, block: usize, parallel: bool) -> Vec<f64> {
    allow_realtime_parallel(parallel);
    mark_audio_thread();
    let mut m = MixEngine::new(48_000.0, block);
    let mut out = vec![vec![0.0f32; block]; 2];
    for b in 0..20 {
        m.render(s, (b * block) as i64, block, &mut out); // warm-up: plugin creation, buffers
    }
    let mut times = Vec::with_capacity(blocks);
    for b in 0..blocks {
        let t0 = Instant::now();
        m.render(s, ((b + 20) * block) as i64, block, &mut out);
        times.push(t0.elapsed().as_secs_f64() * 1000.0);
    }
    times
}

fn main() {
    let args: Vec<usize> = std::env::args().skip(1).filter_map(|a| a.parse().ok()).collect();
    let (tracks, blocks, block) = (args.first().copied().unwrap_or(64), args.get(1).copied().unwrap_or(2000), args.get(2).copied().unwrap_or(512));
    let s = session(tracks, (blocks + 40) * block);
    let budget = block as f64 / 48.0;
    for parallel in [false, true] {
        let mut t = run(&s, blocks, block, parallel);
        t.sort_by(f64::total_cmp);
        let pct = |p: f64| t.get(((t.len() as f64 * p) as usize).min(t.len().saturating_sub(1))).copied().unwrap_or(0.0);
        println!(
            "{tracks} tracks, {block}-frame blocks, {}: mean {:.2} ms, p99 {:.2}, p99.9 {:.2}, worst {:.2} (budget {budget:.1} ms)",
            if parallel { "worker pool" } else { "serial     " },
            t.iter().sum::<f64>() / t.len().max(1) as f64,
            pct(0.99),
            pct(0.999),
            t.last().copied().unwrap_or(0.0)
        );
    }
}
