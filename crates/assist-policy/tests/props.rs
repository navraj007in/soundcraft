//! Hostile and random input: the rule never panics, always explains itself, is deterministic, and
//! every proposal respects the declared bounds.

use proptest::prelude::*;
use serde_json::{Value, json};
use soundcraft_assist_policy::Policy;

fn json_value() -> impl Strategy<Value = Value> {
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::from),
        any::<i64>().prop_map(Value::from),
        any::<f64>().prop_map(|f| serde_json::Number::from_f64(f).map_or(Value::Null, Value::Number)),
        "[a-z_]{0,12}".prop_map(Value::from),
    ];
    leaf.prop_recursive(3, 24, 4, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..4).prop_map(Value::Array),
            prop::collection::hash_map("[a-z_]{1,16}", inner, 0..5).prop_map(|m| Value::Object(m.into_iter().collect()))
        ]
    })
}

/// A request that is well-formed apart from numbers that may be out of range.
fn plausible() -> impl Strategy<Value = Value> {
    (
        -200.0f64..40.0,
        -160.0f64..20.0,
        0u32..4,
        0u32..400_000,
        any::<(bool, bool, bool)>(),
        prop::sample::select(vec![-12.0f64, -9.5, -6.0, -3.0, -1.0]),
        1u64..1000,
    )
        .prop_map(|(peak, cur, clipped, active, (role, auto, current), target, track)| {
            json!({
                "goal": "headroom",
                "controls": {"target_true_peak_dbtp": target},
                "inputs": {
                    "track_id": track, "true_peak_dbtp": peak, "current_volume_db": cur, "clipped_samples": clipped, "active_ms": active,
                    "role_confirmed": role, "volume_automation_active": auto, "analysis_current": current,
                },
            })
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]

    #[test]
    fn arbitrary_json_never_panics_and_never_proposes(v in json_value()) {
        let p = Policy::level_headroom().unwrap();
        let out = p.evaluate(&v);
        prop_assert!(["abstain", "no_change", "propose"].contains(&out["outcome"].as_str().unwrap_or("")));
        prop_assert_eq!(&out, &p.evaluate(&v));
        // Random JSON has no valid inputs object, so it can never produce an operation.
        prop_assert!(out.get("operations").is_none(), "{out}");
    }

    #[test]
    fn every_proposal_respects_the_declared_bounds(r in plausible()) {
        let p = Policy::level_headroom().unwrap();
        let out = p.evaluate(&r);
        prop_assert_eq!(&out, &p.evaluate(&r), "not deterministic");
        let inputs = &r["inputs"];
        match out["outcome"].as_str().unwrap_or("") {
            "propose" => {
                let op = &out["operations"][0];
                let (before, after) = (op["before"].as_f64().unwrap(), op["after"].as_f64().unwrap());
                let eff = &out["expected_effect"];
                let change = eff["change_db"].as_f64().unwrap();
                prop_assert!((-60.0..=6.0).contains(&after), "fader {after} outside the policy range");
                prop_assert!(change.abs() <= 12.0 + 1e-9 && change.abs() >= 0.5 - 1e-9, "change {change} outside [0.5, 12]");
                prop_assert!((after - before - change).abs() < 1e-9);
                prop_assert!((eff["true_peak_dbtp_before"].as_f64().unwrap() + change - eff["true_peak_dbtp_after"].as_f64().unwrap()).abs() < 1e-9);
                prop_assert_eq!(out["branch"] == "reduce", change < 0.0);
                if change > 0.0 {
                    prop_assert_eq!(inputs["clipped_samples"].as_u64(), Some(0), "boosted a clipped source");
                }
                prop_assert_eq!(&inputs["role_confirmed"], &json!(true));
                prop_assert_eq!(&inputs["analysis_current"], &json!(true));
                prop_assert_eq!(&inputs["volume_automation_active"], &json!(false));
                prop_assert!(inputs["active_ms"].as_u64().unwrap() >= 1000);
                prop_assert_eq!(&op["target"]["track"], &inputs["track_id"]);
            }
            "no_change" => {
                prop_assert!(out.get("operations").is_none());
                let target = r["controls"]["target_true_peak_dbtp"].as_f64().unwrap();
                let peak = out["measurements"][0]["value"].as_f64().unwrap();
                prop_assert!((target - peak).abs() < 0.5 + 1e-9);
            }
            "abstain" => {
                prop_assert!(out.get("operations").is_none());
                prop_assert!(out["reason"]["code"].as_str().is_some_and(|c| !c.is_empty()));
            }
            other => prop_assert!(false, "unknown outcome {other}"),
        }
    }
}
