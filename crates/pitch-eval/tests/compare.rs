//! YIN against the probabilistic tracker on synthetic stress cases with exactly known pitch.
//!
//! These are **implementation checks, not accuracy evidence**: tones are easy in ways singing is not,
//! and PRD v0.3 section 12.2 says synthetic tones must not raise natural-vocal scores. They show where
//! the two trackers differ and that the harness handles both.

use soundcraft_pitch_eval::*;

const SR: u32 = 48_000;

struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        ((self.0 >> 33) as f32 / (1u64 << 31) as f32) * 2.0 - 1.0
    }
}

/// A harmonic tone following `hz(t)` (0 = silent), harmonics falling as 1/h.
fn tone(seconds: f64, hz: impl Fn(f64) -> f64) -> Vec<f32> {
    let mut phase = 0.0f64;
    (0..(seconds * f64::from(SR)) as usize)
        .map(|i| {
            let f = hz(i as f64 / f64::from(SR));
            if f <= 0.0 {
                return 0.0;
            }
            phase += 2.0 * std::f64::consts::PI * f / f64::from(SR);
            (1..=6).map(|h| (phase * h as f64).sin() as f32 / h as f32).sum::<f32>() * 0.4
        })
        .collect()
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
}

/// Add white noise so the signal-to-noise ratio over the voiced part is `snr_db`.
fn with_noise(x: &[f32], snr_db: f32, seed: u64) -> Vec<f32> {
    let active: Vec<f32> = x.iter().copied().filter(|v| *v != 0.0).collect();
    let noise_rms = rms(&active) / 10f32.powf(snr_db / 20.0);
    let mut rng = Lcg(seed);
    x.iter().map(|v| v + noise_rms * 3f32.sqrt() * rng.next()).collect()
}

fn reference(seconds: f64, hz: impl Fn(f64) -> f64) -> Reference {
    let mut rows: Vec<(f64, f64)> = (0..(seconds / 0.01) as usize).map(|i| (i as f64 * 0.01, hz(i as f64 * 0.01))).collect();
    rows.push((seconds, 0.0));
    Reference::new(rows)
}

struct Case {
    name: &'static str,
    audio: Vec<f32>,
    reference: Reference,
}

fn cases() -> Vec<Case> {
    let steady = |t: f64| if (0.2..2.8).contains(&t) { 220.0 } else { 0.0 };
    let vib = |t: f64| if (0.2..2.8).contains(&t) { 200.0 * 2f64.powf(40.0 / 1200.0 * (2.0 * std::f64::consts::PI * 5.5 * t).sin()) } else { 0.0 };
    let gliss = |t: f64| if (0.2..2.8).contains(&t) { 130.0 * 2f64.powf((t - 0.2) / 2.6 * 1.5) } else { 0.0 };
    // Phrases: notes of 0.4 s with 0.15 s silences, stepping up and down.
    let phrase = |t: f64| {
        let k = (t / 0.55) as usize;
        if t % 0.55 < 0.4 { [196.0, 220.0, 247.0, 262.0, 294.0, 262.0][k % 6] } else { 0.0 }
    };
    let low = |t: f64| if (0.2..2.8).contains(&t) { 72.0 } else { 0.0 };
    let mut v = Vec::new();
    let mut add = |name, audio: Vec<f32>, r: Reference| v.push(Case { name, audio, reference: r });
    add("clean steady 220 Hz", tone(3.0, steady), reference(3.0, steady));
    for (db, seed) in [(20.0, 11), (10.0, 12), (5.0, 13), (0.0, 14)] {
        let name: &'static str = match db as i32 {
            20 => "steady, 20 dB SNR",
            10 => "steady, 10 dB SNR",
            5 => "steady, 5 dB SNR",
            _ => "steady, 0 dB SNR",
        };
        add(name, with_noise(&tone(3.0, steady), db, seed), reference(3.0, steady));
    }
    add("vibrato +-40 cents, 10 dB SNR", with_noise(&tone(3.0, vib), 10.0, 21), reference(3.0, vib));
    add("glissando 130 Hz up 1.5 octaves, 10 dB SNR", with_noise(&tone(3.0, gliss), 10.0, 22), reference(3.0, gliss));
    add("phrases with pauses, 10 dB SNR", with_noise(&tone(3.3, phrase), 10.0, 23), reference(3.3, phrase));
    add("low register 72 Hz, 10 dB SNR", with_noise(&tone(3.0, low), 10.0, 24), reference(3.0, low));
    let mut rng = Lcg(31);
    add("noise only (no voice)", (0..3 * SR as usize).map(|_| 0.2 * rng.next()).collect(), Reference::new(vec![(0.0, 0.0)]));
    v
}

fn scored(t: &dyn Tracker, c: &Case) -> (Metrics, Counts) {
    let counts = score_excerpt(&c.reference, &t.track(&c.audio, SR), &[], 0.5).primary;
    (metrics(&counts), counts)
}

fn pct(v: Option<f64>) -> String {
    v.map_or("  n/a".into(), |v| format!("{:5.1}", v * 100.0))
}

#[test]
fn yin_and_pyin_side_by_side_on_synthetic_stress_cases() {
    let (yin, pyin) = (YinTracker::default(), PyinTracker::new().unwrap());
    println!("\n{:<44} | {:^22} | {:^22}", "case (synthetic: implementation check only)", "YIN within50 prec oct", "pYIN within50 prec oct");
    let mut rows = Vec::new();
    for c in cases() {
        let ((a, ac), (b, bc)) = (scored(&yin, &c), scored(&pyin, &c));
        println!(
            "{:<44} | {} {} {} | {} {} {}",
            c.name,
            pct(a.within50),
            pct(a.voicing_precision),
            pct(a.octave_rate),
            pct(b.within50),
            pct(b.voicing_precision),
            pct(b.octave_rate)
        );
        rows.push((c.name, a, b, ac, bc));
    }
    let get = |name: &str| rows.iter().find(|r| r.0 == name).unwrap();
    // The model finds a clean tone.
    let clean = get("clean steady 220 Hz");
    assert!(clean.2.within50.unwrap() >= 0.97 && clean.2.voicing_precision.unwrap() >= 0.97);
    // Neither invents pitch out of white noise: no frame at all was decoded as voiced.
    let noise = get("noise only (no voice)");
    println!("noise only: voiced frames YIN {}, pYIN {} of {}", noise.3.fp, noise.4.fp, noise.3.frames);
    assert_eq!((noise.3.fp, noise.4.fp), (0, 0));
    // Where YIN's single fixed threshold gives up, the probabilistic model still finds the pitch.
    let hard = get("steady, 5 dB SNR");
    assert!(hard.1.within50.unwrap() < 0.5 && hard.2.within50.unwrap() >= 0.95, "{:?} {:?}", hard.1.within50, hard.2.within50);
    // Elsewhere it is no worse than YIN by more than half a point.
    for r in &rows {
        assert!(r.2.within50.unwrap_or(1.0) + 0.005 >= r.1.within50.unwrap_or(1.0), "{}: pYIN {:?} vs YIN {:?}", r.0, r.2.within50, r.1.within50);
        assert!(r.2.octave_rate.unwrap_or(0.0) <= 0.01, "{}: octave errors {:?}", r.0, r.2.octave_rate);
    }
    // And neither does anything useful at 0 dB: a limit to state, not hide.
    assert!(get("steady, 0 dB SNR").2.within50.unwrap() < 0.5);
}
