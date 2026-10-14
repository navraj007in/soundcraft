//! End-to-end runs of `soundcraft-pitch-eval` on a tiny generated corpus of synthetic tones.
//! Tones validate the tool only; they say nothing about natural-vocal accuracy.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn wav(path: &Path, hz: f64) {
    let rate = 48_000u32;
    let samples: Vec<i16> =
        (0..(1.5 * rate as f64) as usize).map(|i| (12_000.0 * (2.0 * std::f64::consts::PI * hz * i as f64 / rate as f64).sin()) as i16).collect();
    let mut b = Vec::new();
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + samples.len() as u32 * 2).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    for v in [1u16, 1] {
        b.extend_from_slice(&v.to_le_bytes());
    }
    b.extend_from_slice(&rate.to_le_bytes());
    b.extend_from_slice(&(rate * 2).to_le_bytes());
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&(samples.len() as u32 * 2).to_le_bytes());
    samples.iter().for_each(|s| b.extend_from_slice(&s.to_le_bytes()));
    std::fs::write(path, b).unwrap();
}

/// Builds a corpus in a fresh folder. `singer(i)` names the singer of excerpt `i` (0..7); the last two
/// excerpts are holdout.
fn corpus(name: &str, singer: impl Fn(usize) -> String, reference: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("soundcraft-pitch-eval-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut ex = Vec::new();
    for i in 0..8usize {
        let hz = 110.0 * 2f64.powf(i as f64 / 6.0);
        wav(&dir.join(format!("e{i}.wav")), hz);
        std::fs::write(dir.join(format!("e{i}.csv")), reference.replace("HZ", &hz.to_string())).unwrap();
        let split = if i >= 6 { "holdout" } else { "dev" };
        ex.push(format!(
            r#"{{"id":"e{i}","singer":"{}","split":"{split}","audio":"e{i}.wav","reference":"e{i}.csv","conditions":["low"]}}"#,
            singer(i)
        ));
    }
    let manifest = dir.join("manifest.json");
    std::fs::write(&manifest, format!(r#"{{"corpus_version":"synthetic-test","excerpts":[{}]}}"#, ex.join(","))).unwrap();
    manifest
}

fn run(manifest: &Path, extra: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_soundcraft-pitch-eval")).arg(manifest).args(extra).output().unwrap()
}

fn text(o: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))
}

const GOOD: &str = "time,f0\n0,HZ\n1.5,0\n";
fn by_pair(i: usize) -> String {
    format!("s{}", i / 2)
}

#[test]
fn a_dev_run_scores_and_reports_its_method_seed_and_checklist() {
    let m = corpus("dev", by_pair, GOOD);
    let json = m.with_file_name("out.json");
    let o = run(&m, &["--bootstrap", "2000", "--seed", "42", "--json", json.to_str().unwrap()]);
    let t = text(&o);
    assert!(o.status.success(), "{t}");
    for needle in ["split dev", "seed 42", "percentile bootstrap", "G06: >= 90%", "corpus checklist", "low "] {
        assert!(t.contains(needle), "missing `{needle}` in:\n{t}");
    }
    let v: serde_json::Value = serde_json::from_slice(&std::fs::read(&json).unwrap()).unwrap();
    assert_eq!(v["excerpts"], 6, "only the dev split is scored by default");
    assert!(v["pooled"]["within50"].as_f64().unwrap() > 0.9, "{v}");
    assert_eq!(v["bootstrap"]["seed"], 42);
    // The same seed gives the same interval.
    let again = run(&m, &["--bootstrap", "2000", "--seed", "42", "--json", json.to_str().unwrap()]);
    assert!(again.status.success());
    let v2: serde_json::Value = serde_json::from_slice(&std::fs::read(&json).unwrap()).unwrap();
    assert_eq!(v["pooled_within50_ci"], v2["pooled_within50_ci"]);
}

#[test]
fn the_holdout_cannot_be_scored_without_naming_frozen_thresholds() {
    let m = corpus("holdout", by_pair, GOOD);
    for split in ["holdout", "all"] {
        let o = run(&m, &["--split", split]);
        assert!(!o.status.success(), "--split {split} must be refused");
        assert!(text(&o).contains("frozen"), "{}", text(&o));
    }
    let o = run(&m, &["--split", "holdout", "--frozen-thresholds", "thresholds-v1"]);
    assert!(o.status.success(), "{}", text(&o));
    assert!(text(&o).contains("frozen thresholds: thresholds-v1"));
}

#[test]
fn a_singer_in_both_dev_and_holdout_is_refused() {
    let m = corpus("overlap", |i| if i == 0 || i == 7 { "shared".into() } else { format!("s{i}") }, GOOD);
    let o = run(&m, &[]);
    assert!(!o.status.success() && text(&o).contains("both dev and holdout"), "{}", text(&o));
}

#[test]
fn bad_input_is_named_and_stops_the_run() {
    let m = corpus("badref", by_pair, "0,HZ\nnot,a number\n");
    let o = run(&m, &[]);
    assert!(!o.status.success() && text(&o).contains("e0.csv") && text(&o).contains("line 2"), "{}", text(&o));
    let m = corpus("noaudio", by_pair, GOOD);
    std::fs::remove_file(m.with_file_name("e1.wav")).unwrap();
    let o = run(&m, &[]);
    assert!(!o.status.success() && text(&o).contains("e1.wav"), "{}", text(&o));
    assert!(!run(&m.with_file_name("nope.json"), &[]).status.success());
    assert!(!run(&m, &["--split", "everything"]).status.success());
}
