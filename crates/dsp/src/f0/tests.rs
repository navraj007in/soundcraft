use super::*;

const SR: u32 = 48_000;

/// Small deterministic noise source in -1..1.
struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        ((self.0 >> 33) as f32 / (1u64 << 31) as f32) * 2.0 - 1.0
    }
}

/// A harmonic tone whose instantaneous frequency is `hz(t)` (0 means silence); harmonics fall off as 1/h.
fn tone(seconds: f64, harmonics: usize, hz: impl Fn(f64) -> f64, amp: f32) -> Vec<f32> {
    let n = (seconds * f64::from(SR)) as usize;
    let mut phase = 0.0f64;
    (0..n)
        .map(|i| {
            let t = i as f64 / f64::from(SR);
            let f = hz(t);
            if f <= 0.0 {
                return 0.0;
            }
            phase += 2.0 * std::f64::consts::PI * f / f64::from(SR);
            (1..=harmonics).map(|h| (phase * h as f64).sin() as f32 / h as f32).sum::<f32>() * amp
        })
        .collect()
}

fn tracker() -> PyinTracker {
    PyinTracker::new(PyinParams::default()).unwrap()
}

/// Share of reference-voiced frames within `cents` of the reference, and share of reference-unvoiced
/// frames wrongly decoded as voiced. `skip` frames at each voiced edge are left out.
fn score(frames: &[F0Frame], reference: impl Fn(f64) -> Option<f64>, cents: f64, skip: usize) -> (f64, f64) {
    let (mut voiced, mut hits, mut quiet, mut false_voiced) = (0usize, 0usize, 0usize, 0usize);
    for (i, f) in frames.iter().enumerate() {
        // Windows hanging off either end of the signal are padded with zeros and less exact.
        if i < 3 || i + 3 >= frames.len() {
            continue;
        }
        let t = i as f64 * 0.01;
        let near_edge = |d: f64| (0..=skip).any(|k| reference(t + d * k as f64 * 0.01).is_some() != reference(t).is_some());
        if near_edge(1.0) || near_edge(-1.0) {
            continue;
        }
        match reference(t) {
            Some(r) => {
                voiced += 1;
                if f.f0_hz.is_some_and(|e| (1200.0 * (f64::from(e) / r).log2()).abs() <= cents) {
                    hits += 1;
                }
            }
            None => {
                quiet += 1;
                false_voiced += usize::from(f.f0_hz.is_some());
            }
        }
    }
    (hits as f64 / voiced.max(1) as f64, false_voiced as f64 / quiet.max(1) as f64)
}

#[test]
fn one_frame_per_hop_covering_the_signal() {
    let f = tracker().track(&vec![0.1; SR as usize], SR);
    assert_eq!(f.len(), 101, "1.0 s at a 10 ms hop is frames 0..=100");
}

#[test]
fn a_steady_tone_is_tracked_tightly_with_high_scores() {
    let x = tone(1.0, 6, |_| 220.0, 0.4);
    let f = tracker().track(&x, SR);
    let (hits, _) = score(&f, |_| Some(220.0), 10.0, 5);
    assert!(hits >= 0.98, "{hits}");
    let mid = &f[20..80];
    assert!(mid.iter().all(|f| f.voicing_score > 0.9 && f.f0_score > 0.9), "{:?}", mid.first());
}

#[test]
fn silence_is_unvoiced_with_low_scores() {
    let f = tracker().track(&vec![0.0; SR as usize], SR);
    assert!(f.iter().all(|f| f.f0_hz.is_none() && f.voicing_score < 0.2 && f.f0_score == 0.0), "{:?}", f.first());
}

#[test]
fn a_glissando_is_followed() {
    let x = tone(1.0, 5, |t| 150.0 * 2f64.powf(t), 0.4); // 150 to 300 Hz in 1 s
    let f = tracker().track(&x, SR);
    let (hits, _) = score(&f, |t| Some(150.0 * 2f64.powf(t)), 50.0, 5);
    assert!(hits >= 0.95, "{hits}");
}

#[test]
fn vibrato_is_followed() {
    let hz = |t: f64| 200.0 * 2f64.powf(40.0 / 1200.0 * (2.0 * std::f64::consts::PI * 5.5 * t).sin());
    let f = tracker().track(&tone(1.5, 6, hz, 0.4), SR);
    let (hits, _) = score(&f, |t| Some(hz(t)), 50.0, 5);
    assert!(hits >= 0.95, "{hits}");
}

#[test]
fn a_tone_with_silence_either_side_is_voiced_only_where_it_sounds() {
    let hz = |t: f64| if (0.5..1.5).contains(&t) { 220.0 } else { 0.0 };
    let f = tracker().track(&tone(2.0, 6, hz, 0.4), SR);
    let (hits, false_voiced) = score(&f, |t| (0.5..1.5).contains(&t).then_some(220.0), 50.0, 4);
    assert!(hits >= 0.95, "{hits}");
    assert!(false_voiced <= 0.05, "{false_voiced}");
}

#[test]
fn low_and_high_registers_are_tracked() {
    for hz in [70.0, 100.0, 440.0, 1000.0] {
        let f = tracker().track(&tone(1.0, 4, move |_| hz, 0.4), SR);
        let (hits, _) = score(&f, move |_| Some(hz), 50.0, 5);
        assert!(hits >= 0.95, "{hz} Hz: {hits}");
    }
}

#[test]
fn a_tone_in_moderate_noise_is_still_found() {
    let mut rng = Lcg(1);
    let x: Vec<f32> = tone(1.5, 6, |_| 200.0, 0.4).into_iter().map(|v| v + 0.12 * rng.next()).collect();
    let f = tracker().track(&x, SR);
    let (hits, _) = score(&f, |_| Some(200.0), 50.0, 5);
    assert!(hits >= 0.9, "{hits}");
}

#[test]
fn noise_alone_is_mostly_unvoiced() {
    let mut rng = Lcg(2);
    let x: Vec<f32> = (0..SR as usize * 2).map(|_| 0.3 * rng.next()).collect();
    let f = tracker().track(&x, SR);
    let voiced = f.iter().filter(|f| f.f0_hz.is_some()).count() as f64 / f.len() as f64;
    println!("noise: {voiced:.3} of frames decoded as voiced");
    assert!(voiced <= 0.10, "{voiced}");
}

#[test]
fn the_scores_stay_between_zero_and_one_and_are_repeatable() {
    let mut rng = Lcg(3);
    let x: Vec<f32> = tone(1.0, 5, |t| 180.0 + 60.0 * t, 0.3).into_iter().map(|v| v + 0.05 * rng.next()).collect();
    let (a, b) = (tracker().track(&x, SR), tracker().track(&x, SR));
    assert_eq!(a, b);
    assert!(a.iter().all(|f| (0.0..=1.0).contains(&f.voicing_score) && (0.0..=1.0).contains(&f.f0_score)));
    assert!(a.iter().all(|f| f.f0_hz.is_none_or(|h| (65.0..=1100.0).contains(&h))));
}

#[test]
fn hostile_input_gives_a_safe_answer() {
    let t = tracker();
    assert!(t.track(&[], SR).is_empty());
    assert!(t.track(&[0.5], SR).len() <= 1, "a single sample");
    assert!(t.track(&vec![0.1; 1000], 0).is_empty(), "rate 0");
    assert!(t.track(&vec![0.1; 1000], 7_999).is_empty(), "rate too low");
    assert!(t.track(&vec![0.1; 1000], 400_000).is_empty(), "rate too high");
    let nasty: Vec<f32> = (0..SR as usize).map(|i| [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 1e30, -1e30, 0.0][i % 6]).collect();
    let f = t.track(&nasty, SR);
    assert!(f.iter().all(|f| (0.0..=1.0).contains(&f.voicing_score) && f.voicing_score.is_finite()));
    let huge = tone(0.5, 3, |_| 200.0, 1e20);
    assert_eq!(t.track(&huge, SR).len(), 51);
    // Shorter than one analysis window.
    assert_eq!(t.track(&vec![0.2; 100], SR).len(), 1);
}

#[test]
fn bad_settings_are_refused() {
    let base = PyinParams::default();
    for p in [
        PyinParams { min_hz: 0.0, ..base.clone() },
        PyinParams { min_hz: f32::NAN, ..base.clone() },
        PyinParams { max_hz: 70.0, ..base.clone() },
        PyinParams { hop_s: 0.0, ..base.clone() },
        PyinParams { cents_per_bin: 0.1, ..base.clone() },
        PyinParams { n_thresholds: 2, ..base.clone() },
        PyinParams { max_step_bins: 0, ..base.clone() },
        PyinParams { switch_prob: 0.9, ..base.clone() },
        PyinParams { prior_beta: -1.0, ..base.clone() },
    ] {
        assert!(PyinTracker::new(p.clone()).is_err(), "{p:?}");
    }
}

#[test]
#[ignore = "timing probe: cargo test --release -p soundcraft-dsp f0::tests::timing -- --ignored --nocapture"]
fn timing_of_a_sixty_second_take() {
    let x = tone(60.0, 6, |t| 200.0 + 40.0 * (t * 0.7).sin(), 0.4);
    let t0 = std::time::Instant::now();
    let f = tracker().track(&x, SR);
    println!("60 s of audio: {} frames in {:?}", f.len(), t0.elapsed());
}

#[test]
fn a_frames_unvoiced_belief_is_what_its_candidates_leave_over() {
    let t = tracker();
    let n = t.n_bins;
    let mut e = vec![0.0f32; 2 * n];
    // A frame whose candidates hold 0.75 of the mass leaves 0.25 for unvoiced, spread over n states.
    let o = Obs { cands: vec![Cand { bin: 100, mass: 0.75, hz: 150.0 }], voiced_mass: 0.75 };
    t.emission(&o, &mut e);
    assert!((e[n + 5] - (0.25 / n as f32 + FLOOR)).abs() < 1e-9, "{}", e[n + 5]);
    // The voiced belief sits on the candidate's bin and its neighbours, in the kernel's proportions.
    assert!((e[100] - (0.75 * KERNEL[2] + FLOOR)).abs() < 1e-6 && (e[99] - (0.75 * KERNEL[1] + FLOOR)).abs() < 1e-6);
    assert!(e[300] <= FLOOR * 1.0001, "no voiced belief far from any candidate");
    // A frame with no candidates is wholly unvoiced.
    t.emission(&Obs::default(), &mut e);
    assert!((e[n + 5] - (1.0 / n as f32 + FLOOR)).abs() < 1e-9 && e[5] == FLOOR);
}

#[test]
fn a_pitch_step_lands_on_the_right_frame() {
    // 200 Hz, then 300 Hz from exactly 1.0 s. Frame i is centred on i * 10 ms, so the change belongs at frame 100.
    let x = tone(2.0, 4, |t| if t < 1.0 { 200.0 } else { 300.0 }, 0.4);
    let f = tracker().track(&x, SR);
    let near = |i: usize, hz: f32| f[i].f0_hz.is_some_and(|e| (1200.0 * (e / hz).log2()).abs() < 50.0);
    assert!(near(96, 200.0) && near(104, 300.0), "{:?} {:?}", f[96].f0_hz, f[104].f0_hz);
    let first_new = (80..120).find(|&i| near(i, 300.0)).unwrap();
    let last_old = (80..120).rev().find(|&i| near(i, 200.0)).unwrap();
    // The 43 ms window straddles the step for a couple of frames either side of frame 100.
    assert!(
        (98..=100).contains(&last_old) && (101..=103).contains(&first_new),
        "old pitch ends at frame {last_old}, new pitch starts at {first_new}"
    );
}

#[test]
fn the_pitch_is_refined_well_below_the_bin_width() {
    // Pitches that fall between the 10-cent bin centres: the answer must be far closer than half a bin (5 cents).
    for hz in [221.3, 187.9, 333.3, 143.7] {
        let f = tracker().track(&tone(1.0, 5, move |_| hz, 0.4), SR);
        let worst = f[10..90].iter().map(|f| f.f0_hz.map_or(999.0, |e| (1200.0 * (f64::from(e) / hz).log2()).abs())).fold(0.0f64, f64::max);
        assert!(worst < 2.0, "{hz} Hz: worst error {worst:.2} cents");
    }
}

#[test]
fn the_pitch_score_falls_where_the_belief_is_split_and_recovers_after() {
    // At a note change the evidence points two ways for a few frames: the score must say so.
    let f = tracker().track(&tone(2.0, 4, |t| if t < 1.0 { 200.0 } else { 300.0 }, 0.4), SR);
    let low_near_the_step = (96..=106).map(|i| f[i].f0_score).fold(1.0f32, f32::min);
    println!("pitch score: lowest near the step {low_near_the_step:.2}, at frame 80 {:.2}, at frame 120 {:.2}", f[80].f0_score, f[120].f0_score);
    assert!(f[80].f0_score > 0.95 && f[120].f0_score > 0.95, "steady stretches are confident");
    assert!(low_near_the_step < 0.7, "the score dips where the pitch is changing: {low_near_the_step}");
}

#[test]
fn a_note_change_is_followed_at_once_not_ramped_through_pitches_never_sung() {
    let f = tracker().track(&tone(2.0, 4, |t| if t < 1.0 { 200.0 } else { 300.0 }, 0.4), SR);
    let sung = |hz: f32| {
        f.iter()
            .skip(60)
            .take(80)
            .filter(|fr| fr.f0_hz.is_some_and(|e| (1200.0 * (e / 200.0).log2()).abs() > 50.0 && (1200.0 * (e / hz).log2()).abs() > 50.0))
            .count()
    };
    let in_between = sung(300.0);
    println!("frames between the two sung pitches: {in_between}");
    assert!(in_between <= 2, "the contour ramped through {in_between} frames of pitches that were never sung");
}

#[test]
fn candidate_mass_is_what_the_frame_observed() {
    let t = tracker();
    let sr = f64::from(SR);
    let tau_min = ((sr / 1100.0).floor() as usize).max(2);
    let tau_max = (sr / 65.0).ceil() as usize;
    let window_len = ((0.043 * sr).round() as usize).max(2 * tau_max);
    let g = Geometry { sr, tau_min, tau_max, window_len, integ: window_len - tau_max };
    let mut scratch = Scratch::new(&g);
    let clean = tone(1.0, 6, |_| 220.0, 0.4);
    let o = t.observe(&clean, 24_000, &g, &mut scratch);
    let total: f32 = o.cands.iter().map(|c| c.mass).sum();
    assert!((o.voiced_mass - total).abs() < 1e-5, "voiced mass {} must equal its candidates' {total}", o.voiced_mass);
    assert!(o.voiced_mass > 0.9 && o.voiced_mass <= 1.0 + 1e-5, "a clean tone is nearly all voiced mass: {}", o.voiced_mass);
    let hz = o.cands.iter().max_by(|a, b| a.mass.total_cmp(&b.mass)).map(|c| c.hz).unwrap();
    assert!((hz - 220.0).abs() < 0.5, "{hz}");
    let silent = t.observe(&vec![0.0; SR as usize], 24_000, &g, &mut scratch);
    assert!(silent.cands.is_empty() && silent.voiced_mass == 0.0);
}

/// Belief split between two pitches 150 cents apart must show as a low pitch score, not be hidden by a wide window.
#[test]
fn the_pitch_score_counts_belief_within_fifty_cents_only() {
    let t = tracker();
    let obs: Vec<Obs> = (0..20)
        .map(|_| Obs { cands: vec![Cand { bin: 100, mass: 0.45, hz: 150.0 }, Cand { bin: 115, mass: 0.45, hz: 163.7 }], voiced_mass: 0.9 })
        .collect();
    let f = t.decode(&obs);
    let mid = &f[5..15];
    assert!(mid.iter().all(|f| f.f0_hz.is_some()), "voiced");
    let mean = mid.iter().map(|f| f.f0_score).sum::<f32>() / mid.len() as f32;
    assert!((0.3..0.7).contains(&mean), "half of the belief is 150 cents away, so the score should be about one half: {mean}");
}

/// The forward-backward scores against brute force: every state sequence of a tiny model, summed exactly.
#[test]
fn the_voicing_score_equals_the_exact_posterior_on_a_tiny_model() {
    let p = PyinParams {
        min_hz: 100.0,
        max_hz: 200.0,
        cents_per_bin: 100.0,
        max_step_bins: 2,
        jump_prob: 0.1,
        switch_prob: 0.15,
        ..PyinParams::default()
    };
    let t = PyinTracker::new(p.clone()).unwrap();
    let n = t.n_bins; // 13 bins: 26 states
    assert_eq!(n, 13);
    let obs = vec![
        Obs { cands: vec![Cand { bin: 3, mass: 0.6, hz: 119.0 }], voiced_mass: 0.6 },
        Obs { cands: vec![Cand { bin: 4, mass: 0.3, hz: 126.0 }, Cand { bin: 9, mass: 0.3, hz: 168.0 }], voiced_mass: 0.6 },
        Obs::default(),
        Obs { cands: vec![Cand { bin: 10, mass: 0.8, hz: 178.0 }], voiced_mass: 0.8 },
    ];
    let states = 2 * n;
    let tri = |d: usize| ((p.max_step_bins + 1).saturating_sub(d)) as f64 / ((p.max_step_bins + 1) * (p.max_step_bins + 1)) as f64;
    let (ps, jump, stay) = (f64::from(p.switch_prob), f64::from(p.jump_prob), 1.0 - f64::from(p.switch_prob));
    let trans = |a: usize, b: usize| -> f64 {
        let (av, bv) = (a < n, b < n);
        let (abin, bbin) = (a % n, b % n);
        if av != bv {
            return if abin == bbin { ps } else { 0.0 };
        }
        let d = abin.abs_diff(bbin);
        stay * ((1.0 - jump) * if d <= p.max_step_bins { tri(d) } else { 0.0 } + jump / n as f64)
    };
    let mut em: Vec<Vec<f64>> = Vec::new();
    for o in &obs {
        let mut e = vec![0.0f32; states];
        t.emission(o, &mut e);
        em.push(e.iter().map(|v| f64::from(*v)).collect());
    }
    // Enumerate every sequence.
    let mut seq = vec![0usize; obs.len()];
    fn walk(
        k: usize,
        seq: &mut Vec<usize>,
        w: f64,
        states: usize,
        n: usize,
        em: &[Vec<f64>],
        trans: &dyn Fn(usize, usize) -> f64,
        acc: &mut (Vec<f64>, f64, f64, Vec<usize>),
    ) {
        if k == seq.len() {
            acc.1 += w;
            for (t, s) in seq.iter().enumerate() {
                if *s < n {
                    acc.0[t] += w;
                }
            }
            if w > acc.2 {
                acc.2 = w;
                acc.3 = seq.clone();
            }
            return;
        }
        for s in 0..states {
            let step = if k == 0 { 0.5 / n as f64 } else { trans(seq[k - 1], s) };
            if step == 0.0 {
                continue;
            }
            seq[k] = s;
            walk(k + 1, seq, w * step * em[k][s], states, n, em, trans, acc);
        }
    }
    let mut acc = (vec![0.0f64; obs.len()], 0.0f64, -1.0f64, vec![0usize; obs.len()]);
    walk(0, &mut seq, 1.0, states, n, &em, &trans, &mut acc);
    let (voiced_weight, total, _best_weight, best_path) = acc;
    assert!(total > 0.0);
    let frames = t.decode(&obs);
    for (i, f) in frames.iter().enumerate() {
        let exact = voiced_weight[i] / total;
        assert!((f64::from(f.voicing_score) - exact).abs() < 2e-3, "frame {i}: tracker {} vs exact {exact}", f.voicing_score);
        assert_eq!(f.f0_hz.is_some(), best_path[i] < n, "frame {i}: the decoded path disagrees with the exact best path {best_path:?}");
    }
}
