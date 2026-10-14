//! Probabilistic YIN fundamental-frequency tracker (plan tasks P0-T08, P1-A03).
//!
//! This follows the pYIN idea (Mauch & Dixon, 2014): instead of one YIN threshold, many thresholds
//! drawn from a prior each give a pitch candidate, so a frame yields a set of weighted candidates and
//! a probability that it is voiced at all. A hidden Markov model over pitch bins plus an "unvoiced"
//! state then smooths the decision over time. Viterbi decoding gives the contour; a
//! forward-backward pass gives per-frame scores.
//!
//! It is an original implementation from the published description. It uses only this crate's own
//! code and no external pitch library.
//!
//! The scores are **uncalibrated**: `voicing_score` and `f0_score` rank frames, they are not
//! probabilities of being correct and must not be shown as such (PRD §8.3). Thresholds on them are
//! fixed on development data before any holdout is scored.
//!
//! Inputs are untrusted: non-finite samples count as zero, unsupported rates or settings give no
//! frames, and the length is capped. Nothing here panics.

/// Longest signal accepted, in seconds. The product limit is 5 minutes per job; this only bounds memory
/// (about 180 MB at the cap).
pub const MAX_SECONDS: f64 = 600.0;

#[derive(Debug, Clone, PartialEq)]
pub struct PyinParams {
    pub min_hz: f32,
    pub max_hz: f32,
    /// Spacing of output frames. Frame `i` is analysed around `i * hop_s`.
    pub hop_s: f32,
    /// Analysis window length (raised if too short for `min_hz`).
    pub window_s: f32,
    /// Width of one pitch bin in the model.
    pub cents_per_bin: f32,
    /// Beta prior over the YIN threshold (mean `alpha / (alpha + beta)`).
    pub prior_alpha: f32,
    pub prior_beta: f32,
    pub n_thresholds: usize,
    /// Largest pitch change between frames the model treats as likely, in bins.
    pub max_step_bins: usize,
    /// Chance per frame of switching between voiced and unvoiced.
    pub switch_prob: f32,
    /// Chance per frame that the pitch jumps to any other pitch, instead of drifting by at most
    /// `max_step_bins`. Without it a note change is followed only as fast as the drift allows, and the
    /// contour ramps through pitches that were never sung.
    pub jump_prob: f32,
    /// Windows quieter than this RMS are unvoiced without analysis.
    pub silence_rms: f32,
}

impl Default for PyinParams {
    fn default() -> Self {
        PyinParams {
            min_hz: 65.0,
            max_hz: 1100.0,
            hop_s: 0.010,
            window_s: 0.043,
            cents_per_bin: 10.0,
            prior_alpha: 2.0,
            prior_beta: 18.0,
            n_thresholds: 100,
            max_step_bins: 10,
            switch_prob: 0.02,
            jump_prob: 0.02,
            silence_rms: 1.0e-4,
        }
    }
}

/// One output frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct F0Frame {
    /// The decoded pitch, or `None` when the frame is decoded as unvoiced.
    pub f0_hz: Option<f32>,
    /// How strongly the model believes the frame is voiced, 0 to 1 (uncalibrated).
    pub voicing_score: f32,
    /// Given voiced, the share of the model's belief within 50 cents of the decoded pitch, 0 to 1
    /// (uncalibrated). Low when octave candidates compete. 0 for an unvoiced frame.
    pub f0_score: f32,
}

#[derive(Debug, Clone)]
pub struct PyinTracker {
    p: PyinParams,
    /// `(threshold, prior weight)`, weights summing to 1.
    thresholds: Vec<(f32, f32)>,
    n_bins: usize,
}

#[derive(Debug, Clone, Copy)]
struct Cand {
    bin: usize,
    mass: f32,
    hz: f32,
}

#[derive(Debug, Clone, Default)]
struct Obs {
    cands: Vec<Cand>,
    voiced_mass: f32,
}

/// Smoothing of a candidate's mass over neighbouring bins (sums to 1).
const KERNEL: [f32; 5] = [1.0 / 9.0, 2.0 / 9.0, 3.0 / 9.0, 2.0 / 9.0, 1.0 / 9.0];
const FLOOR: f32 = 1.0e-6;

impl PyinTracker {
    pub fn new(p: PyinParams) -> Result<PyinTracker, String> {
        let ok = p.min_hz.is_finite()
            && p.max_hz.is_finite()
            && p.min_hz >= 20.0
            && p.max_hz <= 4000.0
            && p.max_hz > p.min_hz * 1.5
            && (0.001..=0.1).contains(&p.hop_s)
            && (0.01..=0.2).contains(&p.window_s)
            && (1.0..=100.0).contains(&p.cents_per_bin)
            && p.prior_alpha > 0.0
            && p.prior_beta > 0.0
            && p.prior_alpha.is_finite()
            && p.prior_beta.is_finite()
            && (10..=1000).contains(&p.n_thresholds)
            && (1..=100).contains(&p.max_step_bins)
            && p.switch_prob > 0.0
            && p.switch_prob < 0.5
            && p.jump_prob >= 0.0
            && p.jump_prob < 0.5
            && p.silence_rms.is_finite()
            && p.silence_rms >= 0.0;
        if !ok {
            return Err("pitch tracker settings are out of range".into());
        }
        let n_bins = (1200.0 * (p.max_hz / p.min_hz).log2() / p.cents_per_bin).ceil() as usize + 1;
        if n_bins > 5000 {
            return Err("too many pitch bins".into());
        }
        let n = p.n_thresholds;
        let raw: Vec<(f32, f32)> = (1..=n)
            .map(|i| {
                let s = i as f32 / n as f32;
                (s, s.powf(p.prior_alpha - 1.0) * (1.0 - s).max(0.0).powf(p.prior_beta - 1.0))
            })
            .collect();
        let total: f32 = raw.iter().map(|r| r.1).sum();
        if !(total.is_finite() && total > 0.0) {
            return Err("the threshold prior has no mass".into());
        }
        let thresholds = raw.into_iter().map(|(s, w)| (s, w / total)).collect();
        Ok(PyinTracker { p, thresholds, n_bins })
    }

    pub fn params(&self) -> &PyinParams {
        &self.p
    }

    fn bin_hz(&self, bin: usize) -> f32 {
        self.p.min_hz * 2f32.powf(bin as f32 * self.p.cents_per_bin / 1200.0)
    }

    /// Track a mono signal. Returns one frame per `hop_s` from `t = 0` covering the whole signal, or
    /// nothing for an unsupported rate, an empty signal or one longer than [`MAX_SECONDS`].
    pub fn track(&self, mono: &[f32], sample_rate: u32) -> Vec<F0Frame> {
        if !(8_000..=384_000).contains(&sample_rate) || mono.is_empty() {
            return Vec::new();
        }
        let sr = f64::from(sample_rate);
        let seconds = mono.len() as f64 / sr;
        if seconds > MAX_SECONDS {
            return Vec::new();
        }
        let n_frames = (seconds / f64::from(self.p.hop_s)).floor() as usize + 1;
        let tau_min = ((sr / f64::from(self.p.max_hz)).floor() as usize).max(2);
        let tau_max = (sr / f64::from(self.p.min_hz)).ceil() as usize;
        if tau_max <= tau_min + 4 {
            return Vec::new();
        }
        let window_len = ((f64::from(self.p.window_s) * sr).round() as usize).max(2 * tau_max);
        let geom = Geometry { sr, tau_min, tau_max, window_len, integ: window_len - tau_max };
        let obs: Vec<Obs> = {
            let mut scratch = Scratch::new(&geom);
            (0..n_frames)
                .map(|i| {
                    let centre = (i as f64 * f64::from(self.p.hop_s) * sr).round() as i64;
                    self.observe(mono, centre, &geom, &mut scratch)
                })
                .collect()
        };
        self.decode(&obs)
    }

    /// Candidate pitches and their probability mass for one frame.
    fn observe(&self, x: &[f32], centre: i64, g: &Geometry, s: &mut Scratch) -> Obs {
        let start = centre - (g.window_len / 2) as i64;
        let mut energy = 0.0f64;
        for (k, w) in s.win.iter_mut().enumerate() {
            let v = usize::try_from(start + k as i64).ok().and_then(|j| x.get(j)).copied().filter(|v| v.is_finite()).unwrap_or(0.0);
            *w = v;
            energy += f64::from(v) * f64::from(v);
        }
        let rms = (energy / g.window_len as f64).sqrt() as f32;
        if rms < self.p.silence_rms || rms == 0.0 {
            return Obs::default();
        }
        // Difference function, then cumulative-mean normalisation (YIN steps 2 and 3).
        s.dp[0] = 1.0;
        let mut running = 0.0f32;
        for tau in 1..=g.tau_max {
            let mut d = 0.0f32;
            if let (Some(a), Some(b)) = (s.win.get(..g.integ), s.win.get(tau..tau + g.integ)) {
                d = a.iter().zip(b).map(|(p, q)| (p - q) * (p - q)).sum();
            }
            running += d;
            s.d[tau] = d;
            s.dp[tau] = if running > 0.0 { d * tau as f32 / running } else { 1.0 };
        }
        // Record lows from tau_min: the only places "first below a threshold" can fall.
        s.records.clear();
        let mut low = f32::INFINITY;
        for tau in g.tau_min..g.tau_max {
            let v = s.dp.get(tau).copied().unwrap_or(1.0);
            if v < low {
                low = v;
                s.records.push((tau, v));
            }
        }
        let mut obs = Obs::default();
        for &(threshold, weight) in &self.thresholds {
            let Some(&(first, _)) = s.records.iter().find(|r| r.1 < threshold) else { continue };
            let mut tau = first;
            while tau + 1 < g.tau_max && s.dp.get(tau + 1).zip(s.dp.get(tau)).is_some_and(|(n, c)| n < c) {
                tau += 1;
            }
            let (Some(&prev), Some(&here), Some(&next)) = (s.d.get(tau.saturating_sub(1)), s.d.get(tau), s.d.get(tau + 1)) else { continue };
            let denom = prev - 2.0 * here + next;
            let shift = if denom.abs() > f32::EPSILON { ((prev - next) / (2.0 * denom)).clamp(-1.0, 1.0) } else { 0.0 };
            let hz = (g.sr / (tau as f64 + f64::from(shift))) as f32;
            if !(hz >= self.p.min_hz && hz <= self.p.max_hz) {
                continue;
            }
            let bin = ((1200.0 * (hz / self.p.min_hz).log2() / self.p.cents_per_bin).round().max(0.0) as usize).min(self.n_bins - 1);
            match obs.cands.iter_mut().find(|c| c.bin == bin) {
                Some(c) => c.mass += weight,
                None => obs.cands.push(Cand { bin, mass: weight, hz }),
            }
            obs.voiced_mass += weight;
        }
        obs
    }

    /// Emission weights for one frame: `n_bins` voiced states, then `n_bins` unvoiced states.
    fn emission(&self, o: &Obs, out: &mut [f32]) {
        let n = self.n_bins;
        out.iter_mut().for_each(|v| *v = FLOOR);
        for c in &o.cands {
            for (k, w) in KERNEL.iter().enumerate() {
                let bin = (c.bin + k).checked_sub(2).filter(|b| *b < n);
                if let Some(slot) = bin.and_then(|b| out.get_mut(b)) {
                    *slot += c.mass * w;
                }
            }
        }
        // The unvoiced belief is spread over its own n states, as the voiced belief is over its bins.
        let unvoiced = (1.0 - o.voiced_mass).max(0.0) / n as f32;
        out.iter_mut().skip(n).for_each(|v| *v += unvoiced);
    }

    /// Hidden Markov model with a voiced and an unvoiced state for every pitch bin, so voiced and
    /// unvoiced alike pay the cost of pitch drift and neither is favoured by the model's shape.
    fn decode(&self, obs: &[Obs]) -> Vec<F0Frame> {
        let (n, t_len) = (self.n_bins, obs.len());
        let states = 2 * n;
        let w = self.p.max_step_bins;
        let ps = self.p.switch_prob;
        let jump = self.p.jump_prob;
        let stay = 1.0 - ps;
        let drift = stay * (1.0 - jump);
        let to_any = stay * jump / n as f32;
        let tri: Vec<f32> = (0..=2 * w).map(|k| (w + 1 - k.abs_diff(w)) as f32 / ((w + 1) * (w + 1)) as f32).collect();

        // Forward pass (sum-product) with Viterbi (max-product) alongside.
        let mut alpha = vec![0.0f32; t_len * states];
        let mut back = vec![0u16; t_len * states];
        let mut e = vec![0.0f32; states];
        let mut delta = vec![0.0f32; states];
        let mut next_delta = vec![0.0f32; states];
        for (t, o) in obs.iter().enumerate() {
            self.emission(o, &mut e);
            let (before, rest) = alpha.split_at_mut(t * states);
            let cur = &mut rest[..states];
            if t == 0 {
                for ((c, d), em) in cur.iter_mut().zip(delta.iter_mut()).zip(&e) {
                    *c = 0.5 / n as f32 * em;
                    *d = *c;
                }
            } else {
                let prev = &before[(t - 1) * states..t * states];
                let back_row = &mut back[t * states..(t + 1) * states];
                // A jump reaches every bin alike, so it needs only the totals and the best predecessor.
                let (all_v, all_u): (f32, f32) = (prev[..n].iter().sum(), prev[n..].iter().sum());
                let best_any = |from: usize| (from..from + n).fold((-1.0f32, from), |a, i| if delta[i] > a.0 { (delta[i], i) } else { a });
                let ((jump_best_v, jump_arg_v), (jump_best_u, jump_arg_u)) = (best_any(0), best_any(n));
                for b2 in 0..n {
                    // Switching keeps the pitch bin; staying drifts by the triangular step or jumps anywhere.
                    let (mut sum_v, mut best_v, mut arg_v) = (prev[n + b2] * ps + all_v * to_any, delta[n + b2] * ps, n + b2);
                    let (mut sum_u, mut best_u, mut arg_u) = (prev[b2] * ps + all_u * to_any, delta[b2] * ps, b2);
                    if jump_best_v * to_any > best_v {
                        best_v = jump_best_v * to_any;
                        arg_v = jump_arg_v;
                    }
                    if jump_best_u * to_any > best_u {
                        best_u = jump_best_u * to_any;
                        arg_u = jump_arg_u;
                    }
                    for b in b2.saturating_sub(w)..=(b2 + w).min(n - 1) {
                        let tw = tri[b + w - b2] * drift;
                        sum_v += prev[b] * tw;
                        sum_u += prev[n + b] * tw;
                        let (cv, cu) = (delta[b] * tw, delta[n + b] * tw);
                        if cv > best_v {
                            best_v = cv;
                            arg_v = b;
                        }
                        if cu > best_u {
                            best_u = cu;
                            arg_u = n + b;
                        }
                    }
                    cur[b2] = e[b2] * sum_v;
                    cur[n + b2] = e[n + b2] * sum_u;
                    next_delta[b2] = e[b2] * best_v;
                    next_delta[n + b2] = e[n + b2] * best_u;
                    back_row[b2] = arg_v as u16;
                    back_row[n + b2] = arg_u as u16;
                }
                delta.copy_from_slice(&next_delta);
            }
            let total: f32 = cur.iter().sum();
            if total > 0.0 && total.is_finite() {
                cur.iter_mut().for_each(|v| *v /= total);
            }
            let top = delta.iter().copied().fold(0.0f32, f32::max);
            if top > 0.0 && top.is_finite() {
                delta.iter_mut().for_each(|v| *v /= top);
            }
        }

        // Decode the most likely path.
        let mut path = vec![n; t_len];
        let mut state = delta.iter().enumerate().fold((0usize, -1.0f32), |a, (i, v)| if *v > a.1 { (i, *v) } else { a }).0;
        for t in (0..t_len).rev() {
            if let Some(p) = path.get_mut(t) {
                *p = state;
            }
            state = usize::from(back.get(t * states + state).copied().unwrap_or(0)).min(states - 1);
        }

        // Backward pass for the scores, and the output frames.
        let near = (50.0 / self.p.cents_per_bin).round() as usize;
        let mut beta = vec![1.0f32; states];
        let mut g = vec![0.0f32; states];
        let mut out = vec![F0Frame { f0_hz: None, voicing_score: 0.0, f0_score: 0.0 }; t_len];
        for t in (0..t_len).rev() {
            let a = &alpha[t * states..(t + 1) * states];
            let gv: f32 = (0..n).map(|b| a[b] * beta[b]).sum();
            let gu: f32 = (n..states).map(|b| a[b] * beta[b]).sum();
            let voiced_belief = if gv + gu > 0.0 { gv / (gv + gu) } else { 0.0 };
            let decoded = path.get(t).copied().unwrap_or(n);
            if let Some(f) = out.get_mut(t) {
                f.voicing_score = voiced_belief.clamp(0.0, 1.0);
                if decoded < n {
                    let near_mass: f32 = (decoded.saturating_sub(near)..=(decoded + near).min(n - 1)).map(|b| a[b] * beta[b]).sum();
                    f.f0_score = if gv > 0.0 { (near_mass / gv).clamp(0.0, 1.0) } else { 0.0 };
                    let refined = obs
                        .get(t)
                        .and_then(|o| o.cands.iter().filter(|c| c.bin.abs_diff(decoded) <= 2).max_by(|x, y| x.mass.total_cmp(&y.mass)))
                        .map(|c| c.hz);
                    f.f0_hz = Some(refined.unwrap_or_else(|| self.bin_hz(decoded)));
                }
            }
            if t == 0 {
                break;
            }
            // beta for t-1 from beta for t.
            if let Some(o) = obs.get(t) {
                self.emission(o, &mut e);
            }
            for ((gs, em), bt) in g.iter_mut().zip(&e).zip(&beta) {
                *gs = em * bt;
            }
            let mut next = vec![0.0f32; states];
            let (all_v, all_u): (f32, f32) = (g[..n].iter().sum(), g[n..].iter().sum());
            for b in 0..n {
                let (mut band_v, mut band_u) = (0.0f32, 0.0f32);
                for b2 in b.saturating_sub(w)..=(b + w).min(n - 1) {
                    let tw = tri[b2 + w - b];
                    band_v += tw * g[b2];
                    band_u += tw * g[n + b2];
                }
                next[b] = (1.0 - jump) * stay * band_v + to_any * all_v + ps * g[n + b];
                next[n + b] = (1.0 - jump) * stay * band_u + to_any * all_u + ps * g[b];
            }
            let total: f32 = next.iter().sum();
            if total > 0.0 && total.is_finite() {
                next.iter_mut().for_each(|v| *v /= total);
            }
            beta = next;
        }
        out
    }
}

struct Geometry {
    sr: f64,
    tau_min: usize,
    tau_max: usize,
    window_len: usize,
    integ: usize,
}

/// Buffers reused from frame to frame.
struct Scratch {
    win: Vec<f32>,
    d: Vec<f32>,
    dp: Vec<f32>,
    records: Vec<(usize, f32)>,
}

impl Scratch {
    fn new(g: &Geometry) -> Scratch {
        Scratch { win: vec![0.0; g.window_len], d: vec![0.0; g.tau_max + 2], dp: vec![1.0; g.tau_max + 2], records: Vec::new() }
    }
}

#[cfg(test)]
mod tests;
