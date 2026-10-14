//! Pitch evaluation harness (plan task P0-T08; scoring rules from PRD v0.3 section 12.2).
//!
//! Everything is scored on a common 10 ms grid with no octave folding. A tracker abstaining on a
//! reference-voiced frame fails the 50-cent score. Primary scores include every adjudicated frame,
//! transitions included; a secondary "interior" score leaves out 20 ms on either side of each
//! voiced/unvoiced boundary and never replaces the primary one. Confidence intervals come from a
//! seeded bootstrap that resamples whole singers, so a singer's excerpts stay together.
//!
//! Synthetic tones validate this code. They do not count toward the natural-vocal release scores.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::collections::BTreeMap;

/// The common scoring grid: frame `i` is centred at `i * HOP_S` seconds.
pub const HOP_S: f64 = 0.010;
/// Frames closer than this to a reference voiced/unvoiced boundary are left out of the interior score.
pub const BOUNDARY_GUARD_S: f64 = 0.020;
/// A pitch estimate is "right" when within this many cents of the reference (G06).
pub const WITHIN_CENTS: f64 = 50.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EstFrame {
    /// `None` is an abstention (unvoiced, or no decision).
    pub f0_hz: Option<f32>,
    /// `None` when the tracker has no confidence output (then no high-confidence statistics exist).
    pub confidence: Option<f32>,
}

pub trait Tracker {
    fn name(&self) -> String;
    /// One frame per grid step, from `t = 0`, covering the whole signal.
    fn track(&self, mono: &[f32], sample_rate: u32) -> Vec<EstFrame>;
}

/// The existing YIN detector at the grid rate. It has no confidence output.
pub struct YinTracker {
    pub min_hz: f32,
    pub max_hz: f32,
    pub threshold: f32,
    pub window_s: f32,
}

impl Default for YinTracker {
    fn default() -> Self {
        YinTracker { min_hz: 65.0, max_hz: 1100.0, threshold: 0.15, window_s: 0.043 }
    }
}

impl Tracker for YinTracker {
    fn name(&self) -> String {
        format!("yin(min {} Hz, max {} Hz, threshold {}, window {} s)", self.min_hz, self.max_hz, self.threshold, self.window_s)
    }

    fn track(&self, mono: &[f32], sample_rate: u32) -> Vec<EstFrame> {
        let sr = sample_rate as f64;
        let win = ((self.window_s as f64 * sr) as usize).max(64);
        let frames = (mono.len() as f64 / sr / HOP_S).floor() as usize + 1;
        (0..frames)
            .map(|i| {
                let centre = (i as f64 * HOP_S * sr) as i64;
                let start = centre - (win / 2) as i64;
                let frame: Vec<f32> =
                    (0..win as i64).map(|k| usize::try_from(start + k).ok().and_then(|j| mono.get(j)).copied().unwrap_or(0.0)).collect();
                let f0 = soundcraft_dsp::pitch_detect::yin(&frame, sample_rate as f32, self.min_hz, self.max_hz, self.threshold);
                EstFrame { f0_hz: f0.filter(|f| f.is_finite() && *f > 0.0), confidence: None }
            })
            .collect()
    }
}

/// The probabilistic YIN tracker from `soundcraft-dsp`, at the harness grid. Its confidence is the
/// product of its voicing score and its pitch score: both are uncalibrated rankings, not probabilities.
pub struct PyinTracker(pub soundcraft_dsp::f0::PyinTracker);

impl PyinTracker {
    pub fn new() -> Result<PyinTracker, String> {
        soundcraft_dsp::f0::PyinTracker::new(soundcraft_dsp::f0::PyinParams::default()).map(PyinTracker)
    }
}

impl Tracker for PyinTracker {
    fn name(&self) -> String {
        let p = self.0.params();
        format!(
            "pyin({} to {} Hz, {} cents/bin, step {} bins, switch {}, {} thresholds)",
            p.min_hz, p.max_hz, p.cents_per_bin, p.max_step_bins, p.switch_prob, p.n_thresholds
        )
    }

    fn track(&self, mono: &[f32], sample_rate: u32) -> Vec<EstFrame> {
        self.0.track(mono, sample_rate).into_iter().map(|f| EstFrame { f0_hz: f.f0_hz, confidence: Some(f.voicing_score * f.f0_score) }).collect()
    }
}

/// Reference annotation: `(time_s, f0_hz)` rows, `f0_hz <= 0` meaning unvoiced. A value holds until the
/// next row; the last row holds for one hop. A file should end with an explicit unvoiced row.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Reference {
    points: Vec<(f64, f64)>,
}

impl Reference {
    pub fn new(mut points: Vec<(f64, f64)>) -> Reference {
        points.sort_by(|a, b| a.0.total_cmp(&b.0));
        Reference { points }
    }

    /// Parse `time,f0` rows. Blank lines, `#` comments and one non-numeric header line are allowed;
    /// any other malformed row is an error that names its line.
    pub fn parse_csv(text: &str) -> Result<Reference, String> {
        let mut points = Vec::new();
        let mut header_seen = false;
        for (n, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut cols = line.split([',', '\t', ' ']).filter(|c| !c.is_empty());
            let (a, b) = (cols.next(), cols.next());
            let first_is_number = a.is_some_and(|v| v.parse::<f64>().is_ok());
            match (a.and_then(|v| v.parse::<f64>().ok()), b.and_then(|v| v.parse::<f64>().ok())) {
                (Some(t), Some(f)) if t.is_finite() && f.is_finite() && t >= 0.0 => points.push((t, f)),
                // One non-numeric line at the top is a header. A numeric line that is invalid is not.
                _ if !first_is_number && !header_seen && points.is_empty() => header_seen = true,
                _ => return Err(format!("line {}: expected `time_seconds,f0_hz`, got `{line}`", n + 1)),
            }
        }
        Ok(Reference::new(points))
    }

    /// The reference F0 at time `t`, or `None` when unvoiced or outside the annotation.
    pub fn at(&self, t: f64) -> Option<f64> {
        let idx = self.points.partition_point(|p| p.0 <= t).checked_sub(1)?;
        let &(row_t, f) = self.points.get(idx)?;
        if f <= 0.0 {
            return None;
        }
        // The last row holds for one hop only: the annotation should end with an unvoiced row.
        if idx + 1 == self.points.len() && t >= row_t + HOP_S {
            return None;
        }
        Some(f)
    }
}

/// Raw tallies for one set of frames. Adding two `Counts` pools them.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Counts {
    pub frames: u64,
    pub ref_voiced: u64,
    pub hit50: u64,
    pub joint: u64,
    pub octave: u64,
    pub tp: u64,
    pub fp: u64,
    pub fn_: u64,
    pub hc_on_ref_voiced: u64,
    pub hc_wrong: u64,
    /// Absolute errors in cents on jointly voiced frames.
    pub errs: Vec<f32>,
}

impl Counts {
    pub fn add(&mut self, o: &Counts) {
        self.frames += o.frames;
        self.ref_voiced += o.ref_voiced;
        self.hit50 += o.hit50;
        self.joint += o.joint;
        self.octave += o.octave;
        self.tp += o.tp;
        self.fp += o.fp;
        self.fn_ += o.fn_;
        self.hc_on_ref_voiced += o.hc_on_ref_voiced;
        self.hc_wrong += o.hc_wrong;
        self.errs.extend_from_slice(&o.errs);
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Metrics {
    /// Share of reference-voiced frames estimated within 50 cents (abstention fails). G06 wants at least 0.90.
    pub within50: Option<f64>,
    /// Median absolute error in cents on jointly voiced frames. G06 wants at most 15.
    pub median_cents: Option<f64>,
    /// Share of reference-voiced frames estimated within 50 cents of a wrong octave. G06 wants at most 0.01.
    pub octave_rate: Option<f64>,
    pub voicing_precision: Option<f64>,
    pub voicing_recall: Option<f64>,
    /// Share of reference-voiced frames with a high-confidence estimate. G07 wants at least 0.80.
    pub hc_coverage: Option<f64>,
    /// Share of those high-confidence estimates more than 50 cents off. G07 wants at most 0.02.
    pub hc_error: Option<f64>,
}

fn ratio(n: u64, d: u64) -> Option<f64> {
    (d > 0).then(|| n as f64 / d as f64)
}

fn median(errs: &[f32]) -> Option<f64> {
    if errs.is_empty() {
        return None;
    }
    let mut v = errs.to_vec();
    let mid = v.len() / 2;
    v.select_nth_unstable_by(mid, |a, b| a.total_cmp(b));
    let hi = f64::from(*v.get(mid)?);
    if v.len() % 2 == 1 {
        return Some(hi);
    }
    let lo = f64::from(v.get(..mid)?.iter().copied().fold(f32::MIN, f32::max));
    Some((lo + hi) / 2.0)
}

pub fn metrics(c: &Counts) -> Metrics {
    Metrics {
        within50: ratio(c.hit50, c.ref_voiced),
        median_cents: median(&c.errs),
        octave_rate: ratio(c.octave, c.ref_voiced),
        voicing_precision: ratio(c.tp, c.tp + c.fp),
        voicing_recall: ratio(c.tp, c.tp + c.fn_),
        hc_coverage: ratio(c.hc_on_ref_voiced, c.ref_voiced),
        hc_error: ratio(c.hc_wrong, c.hc_on_ref_voiced),
    }
}

/// Primary tallies (every non-excluded frame) and interior tallies (boundary guard applied).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Scores {
    pub primary: Counts,
    pub interior: Counts,
    /// Seconds left out of scoring because the reviewers disputed the annotation.
    pub excluded_s: f64,
}

fn cents(est: f64, reference: f64) -> f64 {
    1200.0 * (est / reference).log2()
}

/// Is `est` within 50 cents of `reference` times a non-zero power of two?
fn is_octave_error(err_cents: f64) -> bool {
    let k = (err_cents / 1200.0).round();
    k != 0.0 && (err_cents - 1200.0 * k).abs() <= WITHIN_CENTS
}

/// Score one excerpt. `excluded` holds disputed time ranges `[start, end)` in seconds; `high_conf` is
/// the confidence at or above which an estimate counts as high-confidence.
pub fn score_excerpt(reference: &Reference, est: &[EstFrame], excluded: &[(f64, f64)], high_conf: f32) -> Scores {
    let t = |i: usize| i as f64 * HOP_S;
    let ref_f: Vec<Option<f64>> = (0..est.len()).map(|i| reference.at(t(i))).collect();
    // Reference voiced/unvoiced boundaries, as times midway between the two frames.
    let boundaries: Vec<f64> = (1..ref_f.len())
        .filter(|&i| ref_f.get(i - 1).map(Option::is_some) != ref_f.get(i).map(Option::is_some))
        .map(|i| (t(i - 1) + t(i)) / 2.0)
        .collect();
    let mut out = Scores::default();
    for (i, (e, r)) in est.iter().zip(&ref_f).enumerate() {
        let ti = t(i);
        if excluded.iter().any(|&(a, b)| ti >= a && ti < b) {
            out.excluded_s += HOP_S;
            continue;
        }
        let interior = boundaries.iter().all(|b| (ti - b).abs() > BOUNDARY_GUARD_S);
        let mut c = Counts { frames: 1, ..Counts::default() };
        let est_voiced = e.f0_hz.filter(|f| f.is_finite() && *f > 0.0);
        match (r, est_voiced) {
            (Some(_), Some(_)) => c.tp = 1,
            (Some(_), None) => c.fn_ = 1,
            (None, Some(_)) => c.fp = 1,
            (None, None) => {}
        }
        if let Some(rf) = r {
            c.ref_voiced = 1;
            if let Some(ef) = est_voiced {
                let err = cents(f64::from(ef), *rf);
                c.joint = 1;
                c.errs.push(err.abs() as f32);
                if err.abs() <= WITHIN_CENTS {
                    c.hit50 = 1;
                } else if is_octave_error(err) {
                    c.octave = 1;
                }
                if e.confidence.is_some_and(|q| q >= high_conf) {
                    c.hc_on_ref_voiced = 1;
                    c.hc_wrong = u64::from(err.abs() > WITHIN_CENTS);
                }
            }
        }
        out.primary.add(&c);
        if interior {
            out.interior.add(&c);
        }
    }
    out
}

/// One scored excerpt with the metadata the aggregation needs.
#[derive(Debug, Clone)]
pub struct Scored {
    pub id: String,
    pub singer: String,
    pub conditions: Vec<String>,
    pub scores: Scores,
}

/// Small seeded generator (SplitMix64), so confidence intervals are reproducible from a stated seed.
pub struct SplitMix64(pub u64);

impl SplitMix64 {
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    pub fn below(&mut self, n: usize) -> usize {
        if n == 0 { 0 } else { (self.next_u64() % n as u64) as usize }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Interval {
    pub lo: Option<f64>,
    pub hi: Option<f64>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct MetricIntervals {
    pub within50: Interval,
    pub median_cents: Interval,
    pub octave_rate: Interval,
    pub voicing_precision: Interval,
    pub voicing_recall: Interval,
    pub hc_coverage: Interval,
    pub hc_error: Interval,
}

pub const CI_METHOD: &str = "percentile bootstrap, resampling whole singers with replacement (each sampled singer keeps all of its excerpts)";

fn field(m: &Metrics, i: usize) -> Option<f64> {
    [m.within50, m.median_cents, m.octave_rate, m.voicing_precision, m.voicing_recall, m.hc_coverage, m.hc_error].get(i).copied().flatten()
}

/// Bootstrap intervals (95 percent) for the frame-pooled metrics and for the singer-macro metrics.
pub fn bootstrap(per_singer: &BTreeMap<String, Counts>, resamples: usize, seed: u64) -> (MetricIntervals, MetricIntervals) {
    let singers: Vec<&Counts> = per_singer.values().collect();
    let mut rng = SplitMix64(seed);
    let mut pooled: Vec<Vec<f64>> = vec![Vec::new(); 7];
    let mut macro_: Vec<Vec<f64>> = vec![Vec::new(); 7];
    for _ in 0..resamples {
        let picked: Vec<&Counts> = (0..singers.len()).filter_map(|_| singers.get(rng.below(singers.len())).copied()).collect();
        let mut sum = Counts::default();
        picked.iter().for_each(|c| sum.add(c));
        let (p, per): (Metrics, Vec<Metrics>) = (metrics(&sum), picked.iter().map(|c| metrics(c)).collect());
        for i in 0..7 {
            if let (Some(v), Some(col)) = (field(&p, i), pooled.get_mut(i)) {
                col.push(v);
            }
            let vals: Vec<f64> = per.iter().filter_map(|m| field(m, i)).collect();
            if let (false, Some(col)) = (vals.is_empty(), macro_.get_mut(i)) {
                col.push(vals.iter().sum::<f64>() / vals.len() as f64);
            }
        }
    }
    let interval = |v: &mut Vec<f64>| {
        v.sort_by(|a, b| a.total_cmp(b));
        let at = |q: f64| v.get(((v.len().saturating_sub(1)) as f64 * q).round() as usize).copied();
        Interval { lo: at(0.025), hi: at(0.975) }
    };
    let pack = |cols: &mut Vec<Vec<f64>>| {
        let mut it = cols.iter_mut().map(interval);
        let mut n = || it.next().unwrap_or_default();
        MetricIntervals {
            within50: n(),
            median_cents: n(),
            octave_rate: n(),
            voicing_precision: n(),
            voicing_recall: n(),
            hc_coverage: n(),
            hc_error: n(),
        }
    };
    (pack(&mut pooled), pack(&mut macro_))
}

/// Everything the report needs, from scored excerpts.
#[derive(Debug, Clone, Default)]
pub struct Aggregates {
    pub pooled: Counts,
    pub pooled_interior: Counts,
    pub per_singer: BTreeMap<String, Counts>,
    pub per_singer_interior: BTreeMap<String, Counts>,
    /// Condition -> (pooled counts, number of distinct singers, number of excerpts).
    pub per_condition: BTreeMap<String, (Counts, usize, usize)>,
    pub excluded_s: f64,
}

pub fn aggregate(scored: &[Scored]) -> Aggregates {
    let mut a = Aggregates::default();
    let mut cond_singers: BTreeMap<String, std::collections::BTreeSet<String>> = BTreeMap::new();
    for s in scored {
        a.pooled.add(&s.scores.primary);
        a.pooled_interior.add(&s.scores.interior);
        a.per_singer.entry(s.singer.clone()).or_default().add(&s.scores.primary);
        a.per_singer_interior.entry(s.singer.clone()).or_default().add(&s.scores.interior);
        a.excluded_s += s.scores.excluded_s;
        for c in &s.conditions {
            let e = a.per_condition.entry(c.clone()).or_default();
            e.0.add(&s.scores.primary);
            e.2 += 1;
            cond_singers.entry(c.clone()).or_default().insert(s.singer.clone());
        }
    }
    for (c, set) in cond_singers {
        if let Some(e) = a.per_condition.get_mut(&c) {
            e.1 = set.len();
        }
    }
    a
}

/// Mean of each metric over singers (singers without that metric are skipped).
pub fn singer_macro(per_singer: &BTreeMap<String, Counts>) -> Metrics {
    let ms: Vec<Metrics> = per_singer.values().map(metrics).collect();
    let mean = |i: usize| {
        let v: Vec<f64> = ms.iter().filter_map(|m| field(m, i)).collect();
        (!v.is_empty()).then(|| v.iter().sum::<f64>() / v.len() as f64)
    };
    Metrics {
        within50: mean(0),
        median_cents: mean(1),
        octave_rate: mean(2),
        voicing_precision: mean(3),
        voicing_recall: mean(4),
        hc_coverage: mean(5),
        hc_error: mean(6),
    }
}

#[cfg(test)]
mod tests;
