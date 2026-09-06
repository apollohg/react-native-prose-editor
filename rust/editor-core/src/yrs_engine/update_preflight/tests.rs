use super::{preflight_update_v1, PreflightReader};
use crate::boundary::ResourceLimits;
use std::collections::HashMap;
use std::sync::Arc;
use yrs::types::Attrs;
use yrs::{Any, Doc, ReadTxn, StateVector, Text, Transact};

fn any_update(any: &[u8]) -> Vec<u8> {
    let mut update = vec![1, 1, 1, 0, 8, 1, 0, 1];
    update.extend_from_slice(any);
    update.push(0); // empty delete set
    update
}

fn any_array(count: u32) -> Vec<u8> {
    let mut any = vec![117];
    push_var_u32(&mut any, count);
    any.extend(std::iter::repeat_n(126, count as usize));
    any
}

fn nested_array(depth: usize) -> Vec<u8> {
    let mut any = Vec::with_capacity(depth * 2 + 1);
    for _ in 0..depth {
        any.extend_from_slice(&[117, 1]);
    }
    any.push(126);
    any
}

fn push_var_u32(bytes: &mut Vec<u8>, mut value: u32) {
    while value >= 0x80 {
        bytes.push((value as u8 & 0x7f) | 0x80);
        value >>= 7;
    }
    bytes.push(value as u8);
}

fn push_string(bytes: &mut Vec<u8>, value: &str) {
    push_var_u32(bytes, value.len() as u32);
    bytes.extend_from_slice(value.as_bytes());
}

fn json_update(tag: u8, key: Option<&str>, value: &str) -> Vec<u8> {
    let mut update = vec![1, 1, 1, 0, tag, 1, 0];
    if let Some(key) = key {
        push_string(&mut update, key);
    }
    push_string(&mut update, value);
    update.push(0);
    update
}

#[test]
fn any_depth_accepts_the_exact_boundary_and_rejects_one_over() {
    let limits = ResourceLimits {
        max_document_depth: 8,
        ..ResourceLimits::default()
    };

    preflight_update_v1(&any_update(&nested_array(7)), &limits).unwrap();
    let error = preflight_update_v1(&any_update(&nested_array(8)), &limits).unwrap_err();

    assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
    assert_eq!(error.limit, Some(8));
    assert_eq!(error.actual, Some(9));
    assert_eq!(
        error.details,
        Some(serde_json::json!({
            "field": "encodedState",
            "phase": "updatePreflight",
            "dimension": "anyDepth"
        }))
    );
}

#[test]
fn impossible_declared_collection_lengths_fail_without_allocating() {
    let limits = ResourceLimits::default();
    for (name, bytes) in [
        ("clients", vec![127]),
        ("itemAny", vec![1, 1, 1, 0, 8, 1, 0, 127]),
        ("array", any_update(&[117, 127])),
        ("map", any_update(&[118, 127])),
        ("deleteRanges", vec![0, 1, 1, 127]),
    ] {
        let error = preflight_update_v1(&bytes, &limits).unwrap_err();
        assert_eq!(error.code, "COLLABORATION_DECODE_FAILED", "{name}");
        assert_eq!(
            error.details,
            Some(serde_json::json!({
                "field": "encodedState",
                "phase": "updatePreflight",
                "reason": "declaredLength"
            })),
            "{name}"
        );
    }
}

#[test]
fn collection_allocations_and_aggregate_work_use_exact_node_boundaries() {
    let collection_limits = ResourceLimits {
        max_document_nodes: 1,
        ..ResourceLimits::default()
    };
    let declared_bytes = vec![0; 129];
    let reader = PreflightReader::new(&declared_bytes, &collection_limits);
    reader.require_declared_count(128, 1).unwrap();
    let error = reader.require_declared_count(129, 1).unwrap_err();
    assert_eq!(error.limit, Some(128));
    assert_eq!(error.actual, Some(129));
    assert_eq!(
        error.details.as_ref().unwrap()["dimension"],
        "collectionItems"
    );

    let error = preflight_update_v1(&any_update(&any_array(129)), &collection_limits).unwrap_err();
    assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
    assert_eq!(error.limit, Some(128));
    assert_eq!(error.actual, Some(129));
    assert_eq!(
        error.details,
        Some(serde_json::json!({
            "field": "encodedState",
            "phase": "updatePreflight",
            "dimension": "collectionItems"
        }))
    );

    let work_limits = ResourceLimits {
        max_document_nodes: 1,
        ..ResourceLimits::default()
    };
    preflight_update_v1(&any_update(&any_array(125)), &work_limits).unwrap();
    let error = preflight_update_v1(&any_update(&any_array(126)), &work_limits).unwrap_err();
    assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
    assert_eq!(error.limit, Some(128));
    assert_eq!(error.actual, Some(129));
    assert_eq!(
        error.details,
        Some(serde_json::json!({
            "field": "encodedState",
            "phase": "updatePreflight",
            "dimension": "work"
        }))
    );
}

#[test]
fn json_embed_and_format_use_exact_collection_work_boundaries() {
    let limits = ResourceLimits {
        max_document_nodes: 1,
        ..ResourceLimits::default()
    };
    let exact_array = format!("[{}]", vec!["null"; 125].join(","));
    let exact_object = format!(
        "{{{}}}",
        (0..125)
            .map(|index| format!(r#""k{index}":null"#))
            .collect::<Vec<_>>()
            .join(",")
    );
    preflight_update_v1(&json_update(5, None, &exact_array), &limits).unwrap();
    preflight_update_v1(&json_update(6, Some("mark"), &exact_object), &limits).unwrap();

    let over_array = format!("[{}]", vec!["null"; 126].join(","));
    let over_object = format!(
        "{{{}}}",
        (0..126)
            .map(|index| format!(r#""k{index}":null"#))
            .collect::<Vec<_>>()
            .join(",")
    );
    for update in [
        json_update(5, None, &over_array),
        json_update(6, Some("mark"), &over_object),
    ] {
        let error = preflight_update_v1(&update, &limits).unwrap_err();
        assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
        assert_eq!(error.limit, Some(128));
        assert_eq!(error.actual, Some(129));
        assert_eq!(error.details.as_ref().unwrap()["dimension"], "work");
    }
}

#[test]
fn json_depth_and_decoded_string_payload_have_exact_boundaries() {
    let exact_depth = ResourceLimits {
        max_document_depth: 2,
        ..ResourceLimits::default()
    };
    preflight_update_v1(&json_update(5, None, "[null]"), &exact_depth).unwrap();
    let error = preflight_update_v1(&json_update(5, None, "[[null]]"), &exact_depth).unwrap_err();
    assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
    assert_eq!(error.limit, Some(2));
    assert_eq!(error.actual, Some(3));
    assert_eq!(error.details.as_ref().unwrap()["dimension"], "jsonDepth");

    let exact_string = ResourceLimits {
        max_input_bytes: 3,
        ..ResourceLimits::default()
    };
    preflight_update_v1(&json_update(5, None, r#""abc""#), &exact_string).unwrap();
    let over_string = ResourceLimits {
        max_input_bytes: 2,
        ..ResourceLimits::default()
    };
    let error = preflight_update_v1(&json_update(5, None, r#""abc""#), &over_string).unwrap_err();
    assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
    assert_eq!(error.limit, Some(2));
    assert_eq!(error.actual, Some(3));
    assert_eq!(
        error.details.as_ref().unwrap()["dimension"],
        "jsonStringBytes"
    );
}

#[test]
fn json_preflight_matches_yrs_syntax_escapes_and_number_acceptance() {
    let limits = ResourceLimits::default();
    for value in [
        "null",
        "true",
        "false",
        r#""escaped\n\uD83D\uDE00""#,
        "-1",
        "-9223372036854775809",
        "18446744073709551616",
        "1.5",
        "1e2",
        r#"[true,false,null,{"x":"value"}]"#,
    ] {
        Any::from_json(value).unwrap();
        preflight_update_v1(&json_update(5, None, value), &limits).unwrap();
    }

    for value in [
        "01",
        r#"{"a":}"#,
        r#""\uD800""#,
        "1e400",
        "9223372036854775808",
        "18446744073709551615",
    ] {
        assert!(Any::from_json(value).is_err(), "{value}");
        let error = preflight_update_v1(&json_update(5, None, value), &limits).unwrap_err();
        assert_eq!(error.code, "COLLABORATION_DECODE_FAILED", "{value}");
        assert_eq!(error.details.as_ref().unwrap()["reason"], "invalidJson");
    }
}

#[test]
fn accepts_generated_standard_format_update_with_nested_json_value() {
    let doc = Doc::new();
    let text = doc.get_or_insert_text("text");
    let nested = Any::Map(Arc::new(HashMap::from([(
        "nested".to_string(),
        Any::Array(vec![Any::Bool(true), Any::Null].into()),
    )])));
    text.insert_with_attributes(
        &mut doc.transact_mut(),
        0,
        "formatted",
        Attrs::from([("meta".into(), nested)]),
    );
    let update = doc
        .transact()
        .encode_state_as_update_v1(&StateVector::default());

    preflight_update_v1(&update, &ResourceLimits::default()).unwrap();
}

#[test]
fn truncation_invalid_utf8_and_trailing_bytes_fail_preflight() {
    let limits = ResourceLimits::default();
    for bytes in [vec![1], vec![1, 1, 1, 0, 4, 1, 0, 1, 255, 0], vec![0, 0, 0]] {
        let error = preflight_update_v1(&bytes, &limits).unwrap_err();
        assert_eq!(error.code, "COLLABORATION_DECODE_FAILED");
        assert_eq!(error.details.as_ref().unwrap()["phase"], "updatePreflight");
    }
}

#[test]
fn accepts_empty_and_generated_standard_update_v1() {
    let limits = ResourceLimits::default();
    preflight_update_v1(&[0, 0], &limits).unwrap();

    let doc = Doc::new();
    doc.get_or_insert_text("text")
        .push(&mut doc.transact_mut(), "hello");
    let update = doc
        .transact()
        .encode_state_as_update_v1(&StateVector::default());
    preflight_update_v1(&update, &limits).unwrap();
}
