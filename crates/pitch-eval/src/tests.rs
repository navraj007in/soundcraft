use super::*;

/// Silence until 0.5 s, 220 Hz until 1.5 s, silence after: 100 voiced frames in a 200-frame excerpt.
fn reference() -> Reference {
    Reference::new(vec![(0.0, 0.0), (0.5, 220.0), (1.5, 0.0), (2.0, 0.0)])
}

fn est(f: impl Fn(usize) -> Option<f32>, conf: impl Fn(usize) -> Option<f32>) -> Vec<EstFrame> {
    (0..200).map(|i| EstFrame { f0_hz: f(i), confidence: conf(i) }).collect()
}

/// A perfect tracker: the reference itself.
fn perfect() -> Vec<EstFrame> {
    let r = reference();
    est(|i| r.at(i as f64 * HOP_S).map(|f| f as f32), |_| Some(1.0))
}

fn hz_plus_cents(c: f64) -> f32 {
    (220.0 * 2f64.powf(c / 1200.0)) as f32
}

fn voiced_est(hz: f32) -> Vec<EstFrame> {
    est(|i| (50..150).contains(&i).then_some(hz), |_| Some(1.0))
}

fn score(e: &[EstFrame]) -> Scores {
    score_excerpt(&reference(), e, &[], 0.8)
}

fn close(a: Option<f64>, b: f64) -> bool {
    a.is_some_and(|a| (a - b).abs() < 1e-6)
}

#[test]
fn a_perfect_tracker_scores_perfectly() {
    let m = metrics(&score(&perfect()).primary);
    assert!(close(m.within50, 1.0) && close(m.median_cents, 0.0) && close(m.octave_rate, 0.0));
    assert!(close(m.voicing_precision, 1.0) && close(m.voicing_recall, 1.0));
    assert_eq!(score(&perfect()).primary.ref_voiced, 100);
}

#[test]
fn fifty_cents_is_the_line() {
    let at = |c: f64| metrics(&score(&voiced_est(hz_plus_cents(c))).primary);
    // Straddle the line tightly, so moving it either way is noticed.
    assert!(close(at(49.0).within50, 1.0) && close(at(51.0).within50, 0.0));
    assert!(close(at(-49.0).within50, 1.0) && close(at(-51.0).within50, 0.0), "below the reference counts the same");
    assert!(close(at(40.0).within50, 1.0));
    assert!(at(40.0).median_cents.is_some_and(|m| (m - 40.0).abs() < 0.01), "{:?}", at(40.0).median_cents);
}

#[test]
fn octave_errors_are_counted_and_fail_the_score_without_folding() {
    let m = |hz: f32| metrics(&score(&voiced_est(hz)).primary);
    for hz in [440.0, 110.0, 880.0] {
        assert!(close(m(hz).octave_rate, 1.0), "{hz} Hz is an octave multiple of 220 Hz");
        assert!(close(m(hz).within50, 0.0), "an octave error must not be folded into a hit");
    }
    assert!(close(m(330.0).octave_rate, 0.0), "a fifth (702 cents) is not an octave error");
    assert!(close(m(hz_plus_cents(1200.0 + 30.0)).octave_rate, 1.0), "within 50 cents of an octave multiple still counts");
    assert!(
        close(m(hz_plus_cents(1200.0 + 49.0)).octave_rate, 1.0) && close(m(hz_plus_cents(1200.0 + 51.0)).octave_rate, 0.0),
        "the octave tolerance is the same 50 cents"
    );
}

#[test]
fn abstaining_on_a_voiced_frame_fails_it() {
    let e = est(|i| ((50..150).contains(&i) && !(50..60).contains(&i)).then_some(220.0), |_| Some(1.0));
    let m = metrics(&score(&e).primary);
    assert!(close(m.within50, 0.9) && close(m.voicing_recall, 0.9) && close(m.voicing_precision, 1.0), "{m:?}");
}

#[test]
fn false_voicing_lowers_precision_not_recall() {
    let e = est(|i| ((0..10).contains(&i) || (50..150).contains(&i)).then_some(220.0), |_| Some(1.0));
    let m = metrics(&score(&e).primary);
    assert!(close(m.voicing_precision, 100.0 / 110.0) && close(m.voicing_recall, 1.0) && close(m.within50, 1.0), "{m:?}");
}

#[test]
fn the_interior_score_leaves_out_the_guard_around_boundaries_and_never_replaces_the_primary() {
    // Wrong (abstaining) on the two voiced frames right after the onset, inside the 20 ms guard.
    let e = est(|i| ((52..150).contains(&i)).then_some(220.0), |_| Some(1.0));
    let s = score(&e);
    assert!(close(metrics(&s.primary).within50, 0.98), "the primary score keeps them");
    assert_eq!(s.interior.ref_voiced, 96, "two frames at each end are inside the guard");
    assert!(close(metrics(&s.interior).within50, 1.0));
}

#[test]
fn disputed_ranges_are_not_scored_and_are_reported() {
    let s = score_excerpt(&reference(), &perfect(), &[(0.5, 0.7)], 0.8);
    assert_eq!(s.primary.ref_voiced, 80);
    assert!((s.excluded_s - 0.20).abs() < 1e-9, "{}", s.excluded_s);
}

#[test]
fn high_confidence_statistics_use_reference_voiced_frames_as_the_denominator() {
    let e: Vec<EstFrame> = (0..200)
        .map(|i| EstFrame {
            f0_hz: (50..150).contains(&i).then(|| if i < 60 { hz_plus_cents(80.0) } else { 220.0 }),
            confidence: Some(if (50..100).contains(&i) { 0.9 } else { 0.1 }),
        })
        .collect();
    let m = metrics(&score(&e).primary);
    assert!(close(m.hc_coverage, 0.5), "50 of 100 voiced frames are high-confidence: {m:?}");
    assert!(close(m.hc_error, 0.2), "10 of those 50 are wrong: {m:?}");
    let none = metrics(&score(&voiced_est(220.0).iter().map(|f| EstFrame { confidence: None, ..*f }).collect::<Vec<_>>()).primary);
    assert_eq!((none.hc_coverage, none.hc_error), (Some(0.0), None), "a tracker with no confidence has no high-confidence estimates");
}

#[test]
fn reference_csv_is_parsed_strictly() {
    let r = Reference::parse_csv("# comment\ntime,f0\n0.0, 0\n0.5,220.0\n\n1.5\t0\n").unwrap();
    assert_eq!((r.at(0.4), r.at(0.5), r.at(1.49), r.at(1.5), r.at(5.0)), (None, Some(220.0), Some(220.0), None, None));
    let err = Reference::parse_csv("0.0,0\n0.5,loud\n").unwrap_err();
    assert!(err.starts_with("line 2:"), "{err}");
    assert!(Reference::parse_csv("-1,100\n").is_err(), "negative time is malformed");
    // The last row holds for one hop only.
    let r = Reference::parse_csv("0.0,200\n").unwrap();
    assert_eq!((r.at(0.0), r.at(0.005), r.at(0.01)), (Some(200.0), Some(200.0), None));
}

fn counts(frames: u64, hits: u64) -> Counts {
    Counts { frames, ref_voiced: frames, hit50: hits, joint: frames, tp: frames, errs: vec![5.0; frames as usize], ..Counts::default() }
}

#[test]
fn singer_macro_and_pooled_differ_when_singers_have_unequal_amounts() {
    let per: BTreeMap<String, Counts> = [("a".to_string(), counts(100, 100)), ("b".to_string(), counts(10, 0))].into();
    let pooled = aggregate(&[]).pooled;
    assert_eq!(pooled.frames, 0);
    let mut sum = Counts::default();
    per.values().for_each(|c| sum.add(c));
    assert!(metrics(&sum).within50.is_some_and(|v| (v - 100.0 / 110.0).abs() < 1e-9));
    assert!(close(singer_macro(&per).within50, 0.5));
}

#[test]
fn the_bootstrap_is_reproducible_from_its_seed_and_respects_the_data() {
    let per: BTreeMap<String, Counts> = (0..8).map(|i| (format!("s{i}"), counts(100, 70 + i * 4))).collect();
    let (a, am) = bootstrap(&per, 2000, 20_261_014);
    let (b, bm) = bootstrap(&per, 2000, 20_261_014);
    assert_eq!((a, am), (b, bm), "same seed, same intervals");
    let (c, _) = bootstrap(&per, 2000, 1);
    assert_ne!(a.within50, c.within50, "a different seed gives different resamples");
    let (lo, hi) = (a.within50.lo.unwrap_or(f64::NAN), a.within50.hi.unwrap_or(f64::NAN));
    assert!((0.70..=0.98).contains(&lo) && (0.70..=0.98).contains(&hi) && lo < hi, "[{lo}, {hi}]");
    let mut sum = Counts::default();
    per.values().for_each(|x| sum.add(x));
    let point = metrics(&sum).within50.unwrap_or(f64::NAN);
    assert!(lo <= point && point <= hi, "the point estimate {point} lies inside [{lo}, {hi}]");
    // Singers that are all alike give no spread.
    let same: BTreeMap<String, Counts> = (0..5).map(|i| (format!("s{i}"), counts(100, 90))).collect();
    let (s, _) = bootstrap(&same, 500, 7);
    assert_eq!((s.within50.lo, s.within50.hi), (Some(0.9), Some(0.9)));
}

#[test]
fn conditions_report_how_many_singers_and_excerpts_stand_behind_them() {
    let sc = |id: &str, singer: &str, conds: &[&str]| Scored {
        id: id.into(),
        singer: singer.into(),
        conditions: conds.iter().map(|s| s.to_string()).collect(),
        scores: score(&perfect()),
    };
    let a = aggregate(&[sc("1", "a", &["slides"]), sc("2", "a", &["slides", "breathy"]), sc("3", "b", &["slides"])]);
    assert_eq!((a.per_condition["slides"].1, a.per_condition["slides"].2), (2, 3));
    assert_eq!((a.per_condition["breathy"].1, a.per_condition["breathy"].2), (1, 1));
    assert_eq!(a.per_singer.len(), 2);
}

fn sine(hz: f64, seconds: f64, rate: u32) -> Vec<f32> {
    (0..(seconds * rate as f64) as usize).map(|i| (0.5 * (2.0 * std::f64::consts::PI * hz * i as f64 / rate as f64).sin()) as f32).collect()
}

#[test]
fn yin_on_a_pure_tone_is_right_and_on_silence_abstains_implementation_check_only() {
    let t = YinTracker::default();
    let frames = t.track(&sine(220.0, 1.5, 48_000), 48_000);
    let r = Reference::new(vec![(0.0, 220.0), (1.5, 0.0)]);
    let m = metrics(&score_excerpt(&r, &frames, &[], 0.8).primary);
    assert!(m.within50.is_some_and(|v| v > 0.95), "{m:?}");
    let silent = t.track(&vec![0.0; 48_000], 48_000);
    assert!(silent.iter().all(|f| f.f0_hz.is_none()));
    assert!(frames.iter().all(|f| f.confidence.is_none()), "YIN reports no confidence");
}
