//! SoundCraft-AI local production policy (plan task P0-T09, PRD FR-04).
//!
//! A *rule* maps an approved goal and measured audio features to exact processor settings. It is
//! declared as data (`policy/v0/*.json`: required measurements, controls, bounds, maximum change,
//! abstention conditions) and evaluated by code in this crate. The same request, controls and rule
//! version always give the same result; the language model never supplies a parameter value.
//!
//! Determinism: measurements are first rounded to 0.1 dB (halves away from zero) and everything
//! after that is integer arithmetic in tenths of a dB, so results do not depend on the platform's
//! floating-point behaviour. Hostile input (wrong types, NaN-like or huge numbers, unknown keys)
//! yields an `abstain` result, never a panic.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use serde::Deserialize;
use serde_json::{Map, Value, json};

/// Version of the result shape returned by [`Policy::evaluate`].
pub const CONTRACT: &str = "soundcraft-assist-policy/1";

const LEVEL_HEADROOM: &str = include_str!("../policy/v0/level.headroom.json");

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum PolicyError {
    #[error("rule declaration is not valid: {0}")]
    Declaration(String),
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct InputDecl {
    name: String,
    kind: String,
    unit: Option<String>,
    signal_location: Option<String>,
    min: Option<f64>,
    max: Option<f64>,
    #[allow(dead_code)]
    used_for: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ControlDecl {
    name: String,
    unit: String,
    default: f64,
    min: f64,
    max: f64,
    step: f64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ThresholdDecl {
    min_active_ms: i64,
    silence_floor_dbtp: f64,
    no_change_tolerance_db: f64,
    max_change_db: f64,
    fader_min_db: f64,
    fader_max_db: f64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct OperationDecl {
    command: String,
    parameter: String,
    unit: String,
    #[allow(dead_code)]
    reversible: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct AbstainDecl {
    code: String,
    #[allow(dead_code)]
    when: String,
    message: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct NoChangeDecl {
    code: String,
    message: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Declaration {
    schema_version: u32,
    rule_id: String,
    rule_version: String,
    goal: String,
    #[allow(dead_code)]
    summary: String,
    inputs: Vec<InputDecl>,
    controls: Vec<ControlDecl>,
    thresholds: ThresholdDecl,
    operations: Vec<OperationDecl>,
    order: Vec<String>,
    abstain: Vec<AbstainDecl>,
    no_change: NoChangeDecl,
}

/// Thresholds in tenths of a dB (or ms), checked exact at load.
#[derive(Debug, Clone)]
struct Tenths {
    silence_floor: i64,
    tolerance: i64,
    max_change: i64,
    fader_min: i64,
    fader_max: i64,
    min_active_ms: i64,
    target_min: i64,
    target_max: i64,
    target_step: i64,
    target_default: i64,
}

/// A loaded rule.
#[derive(Debug, Clone)]
pub struct Policy {
    d: Declaration,
    t: Tenths,
}

/// Round to 0.1 dB, halves away from zero. `None` if the value is not finite or too large.
fn tenths(x: f64) -> Option<i64> {
    let v = (x * 10.0).round();
    (x.is_finite() && v.abs() < 1.0e15).then_some(v as i64)
}

/// Like [`tenths`] but the value must already be a whole number of tenths.
fn exact_tenths(x: f64) -> Option<i64> {
    let t = tenths(x)?;
    ((x * 10.0 - t as f64).abs() < 1.0e-9).then_some(t)
}

fn db(t: i64) -> f64 {
    t as f64 / 10.0
}

impl Policy {
    /// The headroom rule shipped with this build.
    pub fn level_headroom() -> Result<Policy, PolicyError> {
        Policy::from_declaration(LEVEL_HEADROOM)
    }

    /// Load and check a rule declaration.
    pub fn from_declaration(text: &str) -> Result<Policy, PolicyError> {
        let bad = |m: &str| PolicyError::Declaration(m.to_string());
        let d: Declaration = serde_json::from_str(text).map_err(|e| PolicyError::Declaration(e.to_string()))?;
        if d.schema_version != 1 {
            return Err(bad("unsupported schema_version"));
        }
        let th = &d.thresholds;
        let ex =
            |x: f64, what: &str| exact_tenths(x).ok_or_else(|| PolicyError::Declaration(format!("{what} must be a whole number of tenths of a dB")));
        let [c] = d.controls.as_slice() else { return Err(bad("exactly one control is supported")) };
        if c.unit.is_empty() || c.name.is_empty() {
            return Err(bad("the control needs a name and a unit"));
        }
        let t = Tenths {
            silence_floor: ex(th.silence_floor_dbtp, "silence_floor_dbtp")?,
            tolerance: ex(th.no_change_tolerance_db, "no_change_tolerance_db")?,
            max_change: ex(th.max_change_db, "max_change_db")?,
            fader_min: ex(th.fader_min_db, "fader_min_db")?,
            fader_max: ex(th.fader_max_db, "fader_max_db")?,
            min_active_ms: th.min_active_ms,
            target_min: ex(c.min, "control min")?,
            target_max: ex(c.max, "control max")?,
            target_step: ex(c.step, "control step")?,
            target_default: ex(c.default, "control default")?,
        };
        if t.target_step <= 0 || t.tolerance <= 0 || t.max_change < t.tolerance || t.fader_min >= t.fader_max || t.min_active_ms < 0 {
            return Err(bad("thresholds are inconsistent"));
        }
        if !(t.target_min <= t.target_default && t.target_default <= t.target_max) || t.target_default % t.target_step != 0 {
            return Err(bad("control default is outside its range or off its step"));
        }
        if d.operations.len() != 1 || d.operations.iter().any(|o| o.command.is_empty() || o.parameter.is_empty()) {
            return Err(bad("exactly one operation with a command and parameter is required"));
        }
        let mut names: Vec<&str> = d.inputs.iter().map(|i| i.name.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        if names.len() != d.inputs.len() {
            return Err(bad("duplicate input names"));
        }
        for i in &d.inputs {
            let range_ok = i.min.zip(i.max).is_none_or(|(lo, hi)| lo <= hi);
            if !range_ok
                || !["integer", "number", "boolean"].contains(&i.kind.as_str())
                || (i.kind != "boolean" && (i.min.is_none() || i.max.is_none()))
            {
                return Err(PolicyError::Declaration(format!("input `{}` is malformed", i.name)));
            }
        }
        let mut codes: Vec<&str> = d.abstain.iter().map(|a| a.code.as_str()).collect();
        let mut order: Vec<&str> = d.order.iter().map(String::as_str).collect();
        codes.sort_unstable();
        order.sort_unstable();
        if codes != order || codes.windows(2).any(|w| w[0] == w[1]) {
            return Err(bad("`order` must list each abstain code exactly once"));
        }
        Ok(Policy { d, t })
    }

    pub fn rule_id(&self) -> &str {
        &self.d.rule_id
    }

    pub fn rule_version(&self) -> &str {
        &self.d.rule_version
    }

    /// Every abstention code, in precedence order, plus the no-change code at the end.
    pub fn outcome_codes(&self) -> Vec<String> {
        self.d.order.iter().cloned().chain(std::iter::once(self.d.no_change.code.clone())).collect()
    }

    fn rule(&self) -> Value {
        json!({"id": self.d.rule_id, "version": self.d.rule_version, "goal": self.d.goal})
    }

    fn message(&self, code: &str) -> String {
        self.d.abstain.iter().find(|a| a.code == code).map(|a| a.message.clone()).unwrap_or_default()
    }

    fn abstain(&self, code: &str, field: Option<&str>, evidence: Option<(Value, Value)>) -> Value {
        let mut reason = json!({"code": code, "message": self.message(code)});
        if let (Some(f), Some(o)) = (field, reason.as_object_mut()) {
            o.insert("field".into(), json!(f));
        }
        let mut out = json!({"contract": CONTRACT, "rule": self.rule(), "outcome": "abstain", "reason": reason});
        if let (Some((controls, measurements)), Some(o)) = (evidence, out.as_object_mut()) {
            o.insert("controls".into(), controls);
            o.insert("measurements".into(), measurements);
        }
        out
    }

    /// Read one declared input. `Err(name)` names the field to blame.
    fn read(&self, inputs: &Map<String, Value>, name: &str) -> Result<Value, String> {
        let decl = self.d.inputs.iter().find(|i| i.name == name).ok_or_else(|| name.to_string())?;
        let v = inputs.get(name).ok_or_else(|| name.to_string())?;
        match decl.kind.as_str() {
            "boolean" if v.is_boolean() => Ok(v.clone()),
            "integer" | "number" => {
                let x = v.as_f64().filter(|x| x.is_finite()).ok_or_else(|| name.to_string())?;
                let in_range = decl.min.zip(decl.max).is_some_and(|(lo, hi)| x >= lo && x <= hi);
                if !in_range || (decl.kind == "integer" && x.fract() != 0.0) {
                    return Err(name.to_string());
                }
                Ok(v.clone())
            }
            _ => Err(name.to_string()),
        }
    }

    /// Evaluate a request: `{"goal", "rule_version"?, "controls"?, "inputs"}`. Always returns a
    /// result object with `outcome` of `propose`, `no_change` or `abstain`; never panics.
    pub fn evaluate(&self, request: &Value) -> Value {
        let t = &self.t;
        let Some(req) = request.as_object() else { return self.abstain("invalid_input", Some("request"), None) };
        if let Some(k) = req.keys().find(|k| !["goal", "rule_version", "controls", "inputs"].contains(&k.as_str())) {
            return self.abstain("invalid_input", Some(k), None);
        }
        let Some(goal) = req.get("goal").and_then(Value::as_str) else { return self.abstain("invalid_input", Some("goal"), None) };
        if goal != self.d.goal {
            return self.abstain("unsupported_goal", Some("goal"), None);
        }
        match req.get("rule_version") {
            None => {}
            Some(Value::String(v)) if *v == self.d.rule_version => {}
            Some(Value::String(_)) => return self.abstain("rule_version_mismatch", Some("rule_version"), None),
            Some(_) => return self.abstain("invalid_input", Some("rule_version"), None),
        }

        // Controls: declared, in range, on their step; the default when omitted.
        let control = &self.d.controls[0];
        let mut target = t.target_default;
        match req.get("controls") {
            None => {}
            Some(Value::Object(m)) => {
                if let Some(k) = m.keys().find(|k| **k != control.name) {
                    return self.abstain("invalid_input", Some(k), None);
                }
                if let Some(v) = m.get(&control.name) {
                    let ok = v.as_f64().and_then(exact_tenths).filter(|x| *x >= t.target_min && *x <= t.target_max && x % t.target_step == 0);
                    match ok {
                        Some(x) => target = x,
                        None => return self.abstain("invalid_input", Some(&control.name), None),
                    }
                }
            }
            Some(_) => return self.abstain("invalid_input", Some("controls"), None),
        }

        // Inputs: every declared one present and valid; no others.
        let Some(Value::Object(inputs)) = req.get("inputs") else { return self.abstain("invalid_input", Some("inputs"), None) };
        if let Some(k) = inputs.keys().find(|k| !self.d.inputs.iter().any(|i| i.name == **k)) {
            return self.abstain("invalid_input", Some(k), None);
        }
        let mut got: Vec<(String, Value)> = Vec::new();
        for decl in &self.d.inputs {
            match self.read(inputs, &decl.name) {
                Ok(v) => got.push((decl.name.clone(), v)),
                Err(field) => return self.abstain("invalid_input", Some(&field), None),
            }
        }
        let get = |name: &str| got.iter().find(|(n, _)| n == name).map(|(_, v)| v.clone()).unwrap_or(Value::Null);
        let num = |name: &str| get(name).as_f64().unwrap_or(0.0);
        let flag = |name: &str| get(name).as_bool().unwrap_or(false);
        let (track, peak, cur) = (num("track_id") as i64, tenths(num("true_peak_dbtp")).unwrap_or(0), tenths(num("current_volume_db")).unwrap_or(0));
        let (clipped, active) = (num("clipped_samples") as i64, num("active_ms") as i64);

        // Evidence echoed on every result from here on: what was used, as rounded.
        let measurement = |name: &str, value: Value| {
            let d = self.d.inputs.iter().find(|i| i.name == name);
            let mut m = json!({"name": name, "value": value});
            if let Some(o) = m.as_object_mut() {
                if let Some(u) = d.and_then(|d| d.unit.as_ref()) {
                    o.insert("unit".into(), json!(u));
                }
                if let Some(s) = d.and_then(|d| d.signal_location.as_ref()) {
                    o.insert("signal_location".into(), json!(s));
                }
            }
            m
        };
        let controls = json!({ control.name.clone(): db(target) });
        let measurements = json!([
            measurement("true_peak_dbtp", json!(db(peak))),
            measurement("current_volume_db", json!(db(cur))),
            measurement("clipped_samples", json!(clipped)),
            measurement("active_ms", json!(active))
        ]);
        let stop = |code: &str| self.abstain(code, None, Some((controls.clone(), measurements.clone())));

        if !flag("analysis_current") {
            return stop("stale_analysis");
        }
        if !flag("role_confirmed") {
            return stop("role_not_confirmed");
        }
        if flag("volume_automation_active") {
            return stop("automation_conflict");
        }
        if active < t.min_active_ms {
            return stop("insufficient_signal");
        }
        if peak < t.silence_floor {
            return stop("signal_too_quiet");
        }

        let wanted = target - peak;
        if wanted.abs() < t.tolerance {
            return json!({
                "contract": CONTRACT, "rule": self.rule(), "outcome": "no_change",
                "reason": {"code": self.d.no_change.code, "message": self.d.no_change.message},
                "controls": controls, "measurements": measurements,
            });
        }
        if wanted > 0 && clipped > 0 {
            return stop("clipped_source_no_boost");
        }

        let mut limits: Vec<&str> = Vec::new();
        let capped = wanted.clamp(-t.max_change, t.max_change);
        if capped != wanted {
            limits.push("max_change");
        }
        let mut new = cur.saturating_add(capped);
        if new < t.fader_min || new > t.fader_max {
            new = new.clamp(t.fader_min, t.fader_max);
            limits.push("fader_bounds");
        }
        // The bounds may only shrink a change, never reverse it or grow it: a fader far outside the
        // policy range must not be dragged into it by a jump bigger than the maximum change.
        let change = new - cur;
        if change.abs() < t.tolerance || change.signum() != wanted.signum() || change.abs() > capped.abs() {
            return stop("limited_to_nothing");
        }

        let op = &self.d.operations[0];
        let mut notes: Vec<&str> = Vec::new();
        if clipped > 0 {
            notes.push("source_clipped");
        }
        json!({
            "contract": CONTRACT,
            "rule": self.rule(),
            "outcome": "propose",
            "branch": if change < 0 { "reduce" } else { "boost" },
            "controls": controls,
            "measurements": measurements,
            "operations": [{
                "command": op.command, "target": {"track": track}, "parameter": op.parameter, "unit": op.unit,
                "before": db(cur), "after": db(new),
            }],
            "limits": limits,
            "notes": notes,
            "expected_effect": {"true_peak_dbtp_before": db(peak), "true_peak_dbtp_after": db(peak + change), "change_db": db(change)},
        })
    }
}
