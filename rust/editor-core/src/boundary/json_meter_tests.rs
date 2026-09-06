use std::collections::HashMap;

use serde_json::json;

use super::{JsonMeterDimension, JsonValueMeter};

#[test]
fn deep_json_trailing_input_unwinds_on_a_constrained_stack() {
    let outcome = std::thread::Builder::new()
        .name("deep-json-trailing-input".into())
        .stack_size(192 * 1024)
        .spawn(|| {
            let depth = 1_024;
            let mut input = "[".repeat(depth);
            input.push('0');
            input.push_str(&"]".repeat(depth));
            input.push_str(" trailing");
            super::parse_json_value_stack_safe(
                &input,
                depth,
                depth,
                "DOCUMENT_LIMIT_EXCEEDED",
                "DOCUMENT_INVALID",
            )
            .map(|_| ())
            .map_err(|error| error.code)
        })
        .expect("constrained-stack thread should spawn")
        .join()
        .expect("deep trailing-input parse must not overflow");

    assert_eq!(outcome, Err("DOCUMENT_INVALID"));
}

#[test]
fn json_value_meter_matches_compact_json_and_enforces_exact_bytes() {
    let attrs = HashMap::from([
        ("escaped".into(), json!("quote\" slash\\ line\n 😀")),
        ("number".into(), json!(-123.5)),
        ("nested".into(), json!({ "a": [true, null, 7] })),
    ]);
    let expected = serde_json::to_vec(&attrs).unwrap().len();
    let mut exact = JsonValueMeter::new(expected, 64, 16, 0);
    exact.admit_object(&attrs).unwrap();
    assert_eq!(exact.bytes(), expected);

    let mut one_under = JsonValueMeter::new(expected - 1, 64, 16, 0);
    let error = one_under.admit_object(&attrs).unwrap_err();
    assert_eq!(error.dimension, JsonMeterDimension::Bytes);
    assert_eq!(error.limit, expected - 1);
    assert!(error.actual > error.limit);
}

#[test]
fn json_value_meter_enforces_depth_and_work_before_descent() {
    let nested = HashMap::from([("value".into(), json!([[0]]))]);
    JsonValueMeter::new(1024, 8, 3, 0)
        .admit_object(&nested)
        .unwrap();
    let error = JsonValueMeter::new(1024, 8, 2, 0)
        .admit_object(&nested)
        .unwrap_err();
    assert_eq!(error.dimension, JsonMeterDimension::Depth);
    assert_eq!(error.actual, 3);

    let wide = HashMap::from([("value".into(), json!([1, 2, 3, 4]))]);
    let error = JsonValueMeter::new(1024, 4, 8, 0)
        .admit_object(&wide)
        .unwrap_err();
    assert_eq!(error.dimension, JsonMeterDimension::Work);
    assert_eq!(error.actual, 5);
}

#[test]
fn json_value_meter_rejects_exhausted_work_before_scanning_the_next_key() {
    let attrs = HashMap::from([("x".repeat(128 * 1024), json!(null))]);
    let mut meter = JsonValueMeter::new(usize::MAX, 0, 8, 0);
    let error = meter.admit_object(&attrs).unwrap_err();
    assert_eq!(error.dimension, JsonMeterDimension::Work);
    assert_eq!(error.actual, 1);
    assert_eq!(meter.bytes(), 2);
}
