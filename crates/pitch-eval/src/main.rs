//! `soundcraft-pitch-eval MANIFEST.json [options]`: score a tracker on annotated vocal excerpts.
//!
//! Options: `--split dev|holdout|all` (default `dev`), `--frozen-thresholds LABEL` (required for any
//! run that touches the holdout), `--bootstrap N` (default 2000), `--seed S` (default 20261014),
//! `--hc-threshold X` (default 0.8), `--json PATH`.
//!
//! The manifest lists excerpts: `{"corpus_version", "excerpts": [{"id", "singer", "split": "dev"|"holdout",
//! "audio", "reference", "conditions": [], "excluded": [[start_s, end_s]]}]}`. Paths are relative to the
//! manifest. A reference file has `time_seconds,f0_hz` rows, `f0_hz <= 0` meaning unvoiced.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use serde::Deserialize;
use serde_json::json;
use soundcraft_pitch_eval::*;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    corpus_version: String,
    excerpts: Vec<Excerpt>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Excerpt {
    id: String,
    singer: String,
    split: String,
    audio: String,
    reference: String,
    #[serde(default)]
    conditions: Vec<String>,
    #[serde(default)]
    excluded: Vec<[f64; 2]>,
}

fn arg(args: &[String], flag: &str) -> Option<String> {
    args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned()
}

fn pct(v: Option<f64>) -> String {
    v.map_or("n/a".into(), |v| format!("{:.1}%", v * 100.0))
}

fn cents(v: Option<f64>) -> String {
    v.map_or("n/a".into(), |v| format!("{v:.1} c"))
}

fn ci(i: &Interval, f: fn(Option<f64>) -> String) -> String {
    format!("[{} .. {}]", f(i.lo), f(i.hi))
}

fn fail(msg: impl std::fmt::Display) -> ExitCode {
    eprintln!("soundcraft-pitch-eval: {msg}");
    ExitCode::from(2)
}

fn mono(path: &Path) -> Result<(Vec<f32>, u32), String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let hint = path.extension().and_then(|e| e.to_str());
    let (_, buf) = soundcraft_audio_io::decode(&bytes, hint).map_err(|e| format!("{}: {e}", path.display()))?;
    let n = buf.channels.iter().map(Vec::len).min().unwrap_or(0);
    let c = buf.channels.len().max(1) as f32;
    let m = (0..n).map(|i| buf.channels.iter().filter_map(|ch| ch.get(i)).sum::<f32>() / c).collect();
    Ok((m, buf.sample_rate))
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(manifest_path) = args.first().filter(|a| !a.starts_with("--")).map(PathBuf::from) else {
        return fail(
            "usage: soundcraft-pitch-eval MANIFEST.json [--split dev|holdout|all] [--frozen-thresholds LABEL] [--bootstrap N] [--seed S] [--hc-threshold X] [--json PATH]",
        );
    };
    let split = arg(&args, "--split").unwrap_or_else(|| "dev".into());
    let frozen = arg(&args, "--frozen-thresholds");
    let resamples: usize = arg(&args, "--bootstrap").and_then(|v| v.parse().ok()).unwrap_or(2000);
    let seed: u64 = arg(&args, "--seed").and_then(|v| v.parse().ok()).unwrap_or(20_261_014);
    let hc: f32 = arg(&args, "--hc-threshold").and_then(|v| v.parse().ok()).unwrap_or(0.8);
    if !["dev", "holdout", "all"].contains(&split.as_str()) {
        return fail("--split must be dev, holdout or all");
    }
    if resamples < 2000 {
        eprintln!("note: {resamples} bootstrap resamples is below the 2000 the protocol requires");
    }

    let text = match std::fs::read_to_string(&manifest_path) {
        Ok(t) => t,
        Err(e) => return fail(format!("{}: {e}", manifest_path.display())),
    };
    let manifest: Manifest = match serde_json::from_str(&text) {
        Ok(m) => m,
        Err(e) => return fail(format!("manifest: {e}")),
    };
    if let Some(bad) = manifest.excerpts.iter().find(|e| e.split != "dev" && e.split != "holdout") {
        return fail(format!("excerpt `{}` has split `{}`; use dev or holdout", bad.id, bad.split));
    }
    // PRD 12.2: holdout singers must not appear in threshold selection.
    let singers_in = |s: &str| -> BTreeSet<&str> { manifest.excerpts.iter().filter(|e| e.split == s).map(|e| e.singer.as_str()).collect() };
    let overlap: Vec<&str> = singers_in("dev").intersection(&singers_in("holdout")).copied().collect();
    if !overlap.is_empty() {
        return fail(format!("singers appear in both dev and holdout: {overlap:?}"));
    }
    let uses_holdout = split != "dev";
    if uses_holdout && frozen.is_none() {
        return fail(
            "the holdout may only be scored with --frozen-thresholds LABEL naming the frozen, versioned thresholds. Holdout results must not be used to tune them",
        );
    }

    let base = manifest_path.parent().map(Path::to_path_buf).unwrap_or_default();
    let tracker = YinTracker::default();
    let mut scored = Vec::new();
    let mut seconds = 0.0;
    for e in manifest.excerpts.iter().filter(|e| split == "all" || e.split == split) {
        let (samples, rate) = match mono(&base.join(&e.audio)) {
            Ok(v) => v,
            Err(m) => return fail(m),
        };
        let reference = match std::fs::read_to_string(base.join(&e.reference)).map_err(|x| x.to_string()).and_then(|t| Reference::parse_csv(&t)) {
            Ok(r) => r,
            Err(m) => return fail(format!("{}: {m}", e.reference)),
        };
        seconds += samples.len() as f64 / f64::from(rate.max(1));
        let excluded: Vec<(f64, f64)> = e.excluded.iter().map(|r| (r[0], r[1])).collect();
        let frames = tracker.track(&samples, rate);
        scored.push(Scored {
            id: e.id.clone(),
            singer: e.singer.clone(),
            conditions: e.conditions.clone(),
            scores: score_excerpt(&reference, &frames, &excluded, hc),
        });
    }
    if scored.is_empty() {
        return fail("no excerpts in the selected split");
    }

    let a = aggregate(&scored);
    let (pooled_ci, macro_ci) = bootstrap(&a.per_singer, resamples, seed);
    let (p, pi, mac) = (metrics(&a.pooled), metrics(&a.pooled_interior), singer_macro(&a.per_singer));

    println!("pitch evaluation: corpus `{}`, split {split}, tracker {}", manifest.corpus_version, tracker.name());
    if let Some(l) = &frozen {
        println!("frozen thresholds: {l}   (high-confidence cut-off {hc})");
    }
    println!(
        "{} excerpts, {} singers, {:.1} min scored; {:.2} s excluded as disputed",
        scored.len(),
        a.per_singer.len(),
        seconds / 60.0,
        a.excluded_s
    );
    println!("intervals: 95%, {resamples} resamples, seed {seed}; {CI_METHOD}");
    if a.per_singer.len() < 8 {
        println!(
            "WARNING: only {} singer(s). Intervals from so few singers are wide or meaningless, and no population-wide claim can rest on them (PRD 12.2).",
            a.per_singer.len()
        );
    }
    println!("\nframe-pooled (all adjudicated frames)               value  95% interval           gate target");
    let row = |name: &str, v: String, i: Option<String>, target: &str| println!("{name:<44} {v:>10}  {:<22} {target}", i.unwrap_or_default());
    row("within 50 cents of reference-voiced frames", pct(p.within50), Some(ci(&pooled_ci.within50, pct)), "G06: >= 90%");
    row("median error, jointly voiced frames", cents(p.median_cents), Some(ci(&pooled_ci.median_cents, cents)), "G06: <= 15 c");
    row("octave errors / reference-voiced frames", pct(p.octave_rate), Some(ci(&pooled_ci.octave_rate, pct)), "G06: <= 1%");
    row("voicing precision", pct(p.voicing_precision), Some(ci(&pooled_ci.voicing_precision, pct)), "G07: >= 95%");
    row("voicing recall", pct(p.voicing_recall), Some(ci(&pooled_ci.voicing_recall, pct)), "G07: >= 95%");
    row("high-confidence coverage", pct(p.hc_coverage), Some(ci(&pooled_ci.hc_coverage, pct)), "G07: >= 80%");
    row("high-confidence error rate (>50 c)", pct(p.hc_error), Some(ci(&pooled_ci.hc_error, pct)), "G07: <= 2%");
    println!("\nsinger-macro (mean over singers)");
    row("within 50 cents", pct(mac.within50), Some(ci(&macro_ci.within50, pct)), "");
    row("median error", cents(mac.median_cents), Some(ci(&macro_ci.median_cents, cents)), "");
    println!("\nsecondary, interior frames only (20 ms guard at each boundary; diagnostic, never replaces the primary)");
    row("within 50 cents", pct(pi.within50), None, "");
    row("voicing precision / recall", format!("{} / {}", pct(pi.voicing_precision), pct(pi.voicing_recall)), None, "");

    println!("\nconditions (G06: each >= 85% on the 50-cent score; >= 10 excerpts and >= 3 singers)");
    for (name, (c, singers, excerpts)) in &a.per_condition {
        let w = metrics(c).within50;
        let flags = [(w.is_some_and(|v| v < 0.85), "below 85%"), (*excerpts < 10, "fewer than 10 excerpts"), (*singers < 3, "fewer than 3 singers")];
        let note = flags.iter().filter(|f| f.0).map(|f| f.1).collect::<Vec<_>>().join(", ");
        println!("  {name:<24} {:>8}  {excerpts:>3} excerpts, {singers} singers   {note}", pct(w));
    }
    println!("\nper singer (within 50 cents)");
    for (s, c) in &a.per_singer {
        println!("  {s:<24} {:>8}  {} reference-voiced frames", pct(metrics(c).within50), c.ref_voiced);
    }

    // PRD 12.2 corpus minimums, as a visible checklist (not a pass/fail of the tracker).
    let count = |sp: &str| manifest.excerpts.iter().filter(|e| e.split == sp).count();
    let minutes: f64 = seconds / 60.0;
    println!("\ncorpus checklist (PRD 12.2; evaluated on the selected split, so dev-only runs will show gaps)");
    let check = |ok: bool, what: String| println!("  [{}] {what}", if ok { "x" } else { " " });
    check(manifest.excerpts.len() >= 160, format!("at least 160 excerpts in the manifest ({})", manifest.excerpts.len()));
    check(
        singers_in("dev").len() + singers_in("holdout").len() >= 16,
        format!("at least 16 singers in the manifest ({})", singers_in("dev").len() + singers_in("holdout").len()),
    );
    check(minutes >= 30.0, format!("at least 30 minutes scored ({minutes:.1})"));
    check(
        count("holdout") >= 80 && singers_in("holdout").len() >= 8,
        format!("holdout of at least 80 excerpts from at least 8 singers ({} excerpts, {} singers)", count("holdout"), singers_in("holdout").len()),
    );

    if let Some(path) = arg(&args, "--json") {
        let m = |m: &Metrics| json!({"within50": m.within50, "median_cents": m.median_cents, "octave_rate": m.octave_rate, "voicing_precision": m.voicing_precision, "voicing_recall": m.voicing_recall, "hc_coverage": m.hc_coverage, "hc_error": m.hc_error});
        let iv = |i: &Interval| json!([i.lo, i.hi]);
        let out = json!({
            "corpus_version": manifest.corpus_version, "split": split, "tracker": tracker.name(), "frozen_thresholds": frozen,
            "excerpts": scored.len(), "singers": a.per_singer.len(), "minutes": seconds / 60.0, "excluded_s": a.excluded_s,
            "bootstrap": {"resamples": resamples, "seed": seed, "method": CI_METHOD},
            "pooled": m(&p), "pooled_within50_ci": iv(&pooled_ci.within50), "pooled_median_ci": iv(&pooled_ci.median_cents),
            "singer_macro": m(&mac), "interior": m(&pi),
            "per_singer": a.per_singer.iter().map(|(k, c)| (k.clone(), m(&metrics(c)))).collect::<BTreeMap<_, _>>(),
        });
        if let Err(e) = std::fs::write(&path, serde_json::to_string_pretty(&out).unwrap_or_default()) {
            return fail(format!("{path}: {e}"));
        }
    }
    ExitCode::SUCCESS
}
