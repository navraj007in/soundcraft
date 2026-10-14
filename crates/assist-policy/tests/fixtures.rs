//! Runs the committed fixtures (`policy/v0/fixtures/level.headroom.json`) against the rule.
//!
//! The fixtures are the specification of the rule's behaviour. Their expected values were derived
//! independently of this crate's code and a few were worked out by hand. Changing an expectation
//! means changing the rule, which means a new `rule_version`; do not regenerate them from the code.

use serde_json::Value;
use soundcraft_assist_policy::Policy;
use std::collections::BTreeSet;

const FIXTURES: &str = include_str!("../policy/v0/fixtures/level.headroom.json");

/// Result with every `message` removed: wording is not part of the contract, the codes are.
fn without_messages(v: &Value) -> Value {
    match v {
        Value::Object(m) => Value::Object(m.iter().filter(|(k, _)| *k != "message").map(|(k, x)| (k.clone(), without_messages(x))).collect()),
        Value::Array(a) => Value::Array(a.iter().map(without_messages).collect()),
        other => other.clone(),
    }
}

fn cases() -> Vec<Value> {
    let f: Value = serde_json::from_str(FIXTURES).unwrap();
    assert_eq!(f["rule_id"], "level.headroom");
    f["cases"].as_array().unwrap().clone()
}

#[test]
fn every_fixture_gives_exactly_the_expected_result() {
    let policy = Policy::level_headroom().unwrap();
    assert_eq!(f_version(), policy.rule_version(), "fixtures are for a different rule version");
    let mut failures = Vec::new();
    for c in cases() {
        let name = c["name"].as_str().unwrap();
        let got = policy.evaluate(&c["request"]);
        if without_messages(&got) != c["expect"] {
            failures.push(format!("{name}\n  expected {}\n  got      {}", c["expect"], without_messages(&got)));
        }
    }
    assert!(failures.is_empty(), "{} fixture(s) differ:\n{}", failures.len(), failures.join("\n"));
}

fn f_version() -> String {
    serde_json::from_str::<Value>(FIXTURES).unwrap()["rule_version"].as_str().unwrap().to_string()
}

#[test]
fn every_declared_outcome_is_covered_by_a_fixture() {
    let policy = Policy::level_headroom().unwrap();
    let seen: BTreeSet<String> = cases().iter().filter_map(|c| c["expect"]["reason"]["code"].as_str().map(str::to_string)).collect();
    let missing: Vec<String> = policy.outcome_codes().into_iter().filter(|c| !seen.contains(c)).collect();
    assert!(missing.is_empty(), "no fixture reaches: {missing:?}");
    assert!(cases().iter().any(|c| c["expect"]["outcome"] == "propose"), "no fixture proposes anything");
}

#[test]
fn every_limit_and_note_a_rule_can_report_is_covered() {
    let seen: BTreeSet<String> = cases()
        .iter()
        .flat_map(|c| c["expect"]["limits"].as_array().cloned().unwrap_or_default())
        .filter_map(|l| l.as_str().map(str::to_string))
        .collect();
    assert_eq!(seen, ["fader_bounds", "max_change"].iter().map(|s| s.to_string()).collect());
    assert!(cases().iter().any(|c| c["expect"]["notes"].as_array().is_some_and(|n| n.iter().any(|x| x == "source_clipped"))));
}

#[test]
fn every_result_explains_itself_in_words() {
    let policy = Policy::level_headroom().unwrap();
    for c in cases() {
        let got = policy.evaluate(&c["request"]);
        if got["outcome"] != "propose" {
            assert!(got["reason"]["message"].as_str().is_some_and(|m| !m.is_empty()), "{}: no message", c["name"]);
        }
    }
}

#[test]
fn the_same_request_gives_the_same_result_every_time() {
    let policy = Policy::level_headroom().unwrap();
    for c in cases() {
        let first = policy.evaluate(&c["request"]);
        for _ in 0..3 {
            assert_eq!(policy.evaluate(&c["request"]), first, "{}", c["name"]);
        }
        // A freshly loaded rule behaves the same as one that has been used.
        assert_eq!(Policy::level_headroom().unwrap().evaluate(&c["request"]), first, "{}", c["name"]);
    }
}

#[test]
fn a_changed_rule_version_is_refused_not_reproduced() {
    let policy = Policy::level_headroom().unwrap();
    for c in cases() {
        let mut r = c["request"].clone();
        let Some(o) = r.as_object_mut() else { continue };
        // Only requests that are otherwise well-formed: a malformed one is rejected first.
        let known = ["goal", "rule_version", "controls", "inputs"];
        if o.get("goal").and_then(Value::as_str) != Some("headroom") || o.keys().any(|k| !known.contains(&k.as_str())) {
            continue;
        }
        o.insert("rule_version".into(), "9.9.9".into());
        assert_eq!(policy.evaluate(&r)["reason"]["code"], "rule_version_mismatch", "{}", c["name"]);
    }
}

#[test]
fn a_broken_declaration_is_rejected_at_load() {
    let text = include_str!("../policy/v0/level.headroom.json");
    let good = Policy::from_declaration(text);
    assert!(good.is_ok(), "{good:?}");
    for (from, to, why) in [
        ("\"max_change_db\": 12.0", "\"max_change_db\": 0.2", "max change below the tolerance"),
        ("\"fader_max_db\": 6.0", "\"fader_max_db\": -70.0", "fader range inverted"),
        ("\"default\": -6.0", "\"default\": -6.2", "default off its step"),
        ("\"default\": -6.0", "\"default\": 3.0", "default outside the range"),
        ("\"schema_version\": 1", "\"schema_version\": 2", "unknown schema"),
        ("\"no_change_tolerance_db\": 0.5", "\"no_change_tolerance_db\": 0.55", "threshold not a whole number of tenths"),
        ("\"limited_to_nothing\"\n  ],", "\"limited_to_nothing\", \"extra\"\n  ],", "order lists a code that is not declared"),
        ("{ \"name\": \"active_ms\"", "{ \"name\": \"track_id\"", "duplicate input"),
    ] {
        assert_eq!(text.matches(from).count(), 1, "test setup: `{from}` must appear once");
        assert!(Policy::from_declaration(&text.replace(from, to)).is_err(), "accepted a declaration with: {why}");
    }
}
