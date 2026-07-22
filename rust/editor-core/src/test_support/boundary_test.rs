//! Boundary admission and resource-limit tests for retained boundary code.
//!
//! Task 16C: this suite previously drove the legacy `Editor` harness; per the
//! 2026-07-20 user directive the legacy runtime is gone, so only the tests
//! that exercise retained boundary/serialize/schema code directly remain.
//! Dropped coverage maps to v2 suites as follows: collaboration FFI admission
//! -> collaboration_protocol_test / collaboration_outbox_test; v2 create
//! admission (schema limits, structured errors) -> ffi_v2_test /
//! session_initialization_test; editing-time limit atomicity, split
//! candidates, undoable transactions, and work budgets ->
//! yrs_engine_resource_test / yrs_engine_operation_contract_test /
//! yrs_engine_split_regression_test / yrs_engine mutation+compiler suites.

use crate::boundary::{BoundedInput, InputKind, ResourceLimits};
use crate::schema::Schema;
use crate::serialize::{
    from_prosemirror_json, from_prosemirror_json_with_limits, to_prosemirror_json, JsonParseError,
    UnknownTypeMode,
};
use crate::tiptap_schema;

#[test]
fn boundary_rejects_input_before_json_parse() {
    let limits = ResourceLimits {
        max_input_bytes: 8,
        ..ResourceLimits::default()
    };
    let error =
        BoundedInput::new("{\"document\":[]}", InputKind::DocumentJson, &limits).unwrap_err();

    assert_eq!(error.code(), "INPUT_LIMIT_EXCEEDED");
    assert_eq!(error.limit, Some(8));
    assert_eq!(error.actual, Some(15));
}

#[test]
fn dedicated_input_kinds_use_their_own_byte_limits() {
    let limits = ResourceLimits {
        max_input_bytes: 1,
        max_collaboration_message_bytes: 2,
        max_encoded_state_bytes: 3,
        ..ResourceLimits::default()
    };

    assert!(BoundedInput::new("ab", InputKind::CollaborationMessage, &limits).is_ok());
    assert!(BoundedInput::new("abc", InputKind::EncodedState, &limits).is_ok());
    assert!(BoundedInput::new("ab", InputKind::Html, &limits).is_err());
}

#[test]
fn resource_limits_use_canonical_defaults_and_camel_case_fields() {
    let limits = ResourceLimits::try_from_config(Some(&serde_json::json!({
        "maxDocumentNodes": 12_345
    })))
    .unwrap();

    assert_eq!(limits.max_input_bytes, 20 * 1024 * 1024);
    assert_eq!(limits.max_document_nodes, 12_345);
    assert_eq!(limits.max_document_depth, 256);
    assert_eq!(limits.max_schema_nodes, 1_024);
    assert_eq!(limits.max_schema_expression_bytes, 64 * 1024);
    assert_eq!(limits.max_collaboration_message_bytes, 10 * 1024 * 1024);
    assert_eq!(limits.max_encoded_state_bytes, 50 * 1024 * 1024);

    let json = serde_json::to_value(&limits).unwrap();
    assert_eq!(json["maxDocumentNodes"], 12_345);
    assert!(json.get("max_document_nodes").is_none());
}

#[test]
fn resource_limits_reject_invalid_values_and_exact_ceiling_overrides() {
    for invalid in [
        serde_json::json!({ "maxInputBytes": 0 }),
        serde_json::json!({ "maxDocumentDepth": 1.5 }),
        serde_json::json!({ "maxSchemaNodes": 10_001 }),
        serde_json::json!({ "unknownLimit": 1 }),
    ] {
        assert_eq!(
            ResourceLimits::try_from_config(Some(&invalid))
                .unwrap_err()
                .code(),
            "INVALID_RESOURCE_LIMIT"
        );
    }

    let ceilings = serde_json::json!({
        "maxInputBytes": 64 * 1024 * 1024,
        "maxDocumentNodes": 1_000_000,
        "maxDocumentDepth": 1_024,
        "maxSchemaNodes": 10_000,
        "maxSchemaExpressionBytes": 1024 * 1024,
        "maxCollaborationMessageBytes": 64 * 1024 * 1024,
        "maxEncodedStateBytes": 256 * 1024 * 1024
    });
    assert!(ResourceLimits::try_from_config(Some(&ceilings)).is_ok());
}

#[test]
fn opaque_json_round_trips_faithfully_twice() {
    let schema = tiptap_schema();
    let original = serde_json::json!({
        "type": "doc",
        "content": [{
            "type": "futureWidget",
            "attrs": { "nested": { "answer": 42 }, "enabled": true },
            "content": [{ "type": "text", "text": "do not reinterpret" }]
        }]
    });

    let first = from_prosemirror_json(&original, &schema, UnknownTypeMode::Preserve).unwrap();
    let first_json = to_prosemirror_json(&first, &schema);
    let second = from_prosemirror_json(&first_json, &schema, UnknownTypeMode::Preserve).unwrap();
    let second_json = to_prosemirror_json(&second, &schema);

    assert_eq!(first_json, original);
    assert_eq!(second_json, original);
}

#[test]
fn many_opaque_siblings_exhaust_parse_budget_before_placement_work() {
    let limits = ResourceLimits {
        max_document_nodes: 32,
        ..ResourceLimits::default()
    };
    let content = (0..1_000)
        .map(|index| serde_json::json!({ "type": format!("future{index}") }))
        .collect::<Vec<_>>();
    let json = serde_json::json!({ "type": "doc", "content": content });

    let error = from_prosemirror_json_with_limits(
        &json,
        &tiptap_schema(),
        UnknownTypeMode::Preserve,
        &limits,
    )
    .unwrap_err();
    assert!(matches!(
        error,
        JsonParseError::ResourceLimit {
            limit: 32,
            actual: 33
        }
    ));
}

#[test]
fn many_admitted_opaque_siblings_match_against_near_ceiling_schema() {
    let mut nodes = vec![
        serde_json::json!({ "name": "doc", "content": "wide*", "role": "doc" }),
        serde_json::json!({ "name": "text", "content": "", "role": "text" }),
    ];
    for index in 0..1_000 {
        nodes.push(serde_json::json!({
            "name": format!("inline{index}"),
            "content": "",
            "group": "wide",
            "role": "inline",
            "isVoid": true
        }));
    }
    let limits = ResourceLimits {
        max_document_nodes: 600,
        max_schema_nodes: 1_024,
        ..ResourceLimits::default()
    };
    let schema =
        Schema::from_json_with_limits(&serde_json::json!({ "nodes": nodes, "marks": [] }), &limits)
            .unwrap();
    assert!(schema.symbol_accepts_opaque_placement("wide", "inline"));
    assert!(!schema.symbol_accepts_opaque_placement("wide", "block"));
    let content = (0..500)
        .map(|index| serde_json::json!({ "type": format!("future{index}") }))
        .collect::<Vec<_>>();
    let json = serde_json::json!({ "type": "doc", "content": content });

    let document =
        from_prosemirror_json_with_limits(&json, &schema, UnknownTypeMode::Preserve, &limits)
            .unwrap();
    assert_eq!(document.root().child_count(), 500);
    assert!(document.root().content().unwrap().iter().all(|node| {
        node.attrs()["opaque_placement"] == serde_json::Value::String("inline".to_string())
    }));
}
