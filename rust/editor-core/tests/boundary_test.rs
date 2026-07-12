use editor_core::boundary::{BoundedInput, InputKind, ResourceLimits};
use editor_core::intercept::InterceptorPipeline;
use editor_core::registry::EditorRegistry;
use editor_core::tiptap_schema;

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
            "RESOURCE_LIMIT_INVALID"
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
fn editor_create_result_returns_structured_errors_and_success_ids() {
    let parse_error: serde_json::Value =
        serde_json::from_str(&editor_core::editor_create_result("{".to_string())).unwrap();
    assert_eq!(parse_error["error"]["code"], "CONFIG_PARSE_FAILED");

    let limit_error: serde_json::Value = serde_json::from_str(&editor_core::editor_create_result(
        r#"{"resourceLimits":{"maxInputBytes":0}}"#.to_string(),
    ))
    .unwrap();
    assert_eq!(limit_error["error"]["code"], "RESOURCE_LIMIT_INVALID");
    assert!(limit_error.get("editorId").is_none());

    let success: serde_json::Value =
        serde_json::from_str(&editor_core::editor_create_result("{}".to_string())).unwrap();
    assert!(success["editorId"].as_u64().is_some_and(|id| id > 0));
}

#[test]
fn editor_create_result_propagates_schema_limits_but_falls_back_for_semantic_errors() {
    let limited: serde_json::Value = serde_json::from_str(&editor_core::editor_create_result(
        serde_json::json!({
            "resourceLimits": { "maxSchemaNodes": 2 },
            "schema": three_node_schema_for_config()
        })
        .to_string(),
    ))
    .unwrap();
    assert_eq!(limited["error"]["code"], "SCHEMA_INVALID");
    assert!(limited.get("editorId").is_none());

    let semantic: serde_json::Value = serde_json::from_str(&editor_core::editor_create_result(
        serde_json::json!({
            "schema": {
                "nodes": [
                    { "name": "doc", "content": "missing", "role": "doc" },
                    { "name": "text", "role": "text" }
                ]
            }
        })
        .to_string(),
    ))
    .unwrap();
    assert!(semantic["editorId"].as_u64().is_some_and(|id| id > 0));
}

fn three_node_schema_for_config() -> serde_json::Value {
    serde_json::json!({
        "nodes": [
            { "name": "doc", "content": "paragraph", "role": "doc" },
            { "name": "paragraph", "role": "textBlock" },
            { "name": "text", "role": "text" }
        ]
    })
}

#[test]
fn editor_registry_stores_resolved_resource_limits() {
    let limits = ResourceLimits {
        max_document_nodes: 321,
        ..ResourceLimits::default()
    };
    let id = EditorRegistry::create_with_limits(
        tiptap_schema(),
        InterceptorPipeline::new(),
        false,
        limits,
    );
    let editor = EditorRegistry::get(id).unwrap();
    assert_eq!(
        editor.lock().unwrap().resource_limits().max_document_nodes,
        321
    );
    EditorRegistry::destroy(id);
}
