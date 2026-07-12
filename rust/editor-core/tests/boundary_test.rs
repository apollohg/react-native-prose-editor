use editor_core::boundary::{BoundedInput, InputKind, ResourceLimits};
use editor_core::editor::{Editor, EditorError, EditorUpdate};
use editor_core::intercept::InterceptorPipeline;
use editor_core::registry::EditorRegistry;
use editor_core::schema::{AttrSpec, NodeSpec, Schema};
use editor_core::serialize::{
    from_prosemirror_json, from_prosemirror_json_with_limits, to_prosemirror_json, JsonParseError,
    UnknownTypeMode,
};
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
fn collaboration_ffi_rejects_state_and_messages_before_json_decode() {
    let session_id = editor_core::collaboration_session_create(
        serde_json::json!({
            "resourceLimits": {
                "maxCollaborationMessageBytes": 2,
                "maxEncodedStateBytes": 3
            }
        })
        .to_string(),
    );

    let state: serde_json::Value =
        serde_json::from_str(&editor_core::collaboration_session_replace_encoded_state(
            session_id,
            "not-json".to_string(),
        ))
        .unwrap();
    assert_eq!(state["error"]["code"], "INPUT_LIMIT_EXCEEDED");
    assert_eq!(state["error"]["limit"], 3);
    assert_eq!(state["error"]["actual"], 8);

    let message: serde_json::Value = serde_json::from_str(
        &editor_core::collaboration_session_handle_message(session_id, "bad".to_string()),
    )
    .unwrap();
    assert_eq!(message["error"]["code"], "INPUT_LIMIT_EXCEEDED");
    assert_eq!(message["error"]["limit"], 2);
    assert_eq!(message["error"]["actual"], 3);

    editor_core::collaboration_session_destroy(session_id);
}

#[test]
fn collaboration_creation_and_missing_sessions_return_structured_errors() {
    for (config, code) in [
        ("{".to_string(), "CONFIG_PARSE_FAILED"),
        (
            serde_json::json!({ "resourceLimits": { "maxInputBytes": 0 } }).to_string(),
            "INVALID_RESOURCE_LIMIT",
        ),
        (serde_json::json!({ "maxLength": -1 }).to_string(), "CONFIG_INVALID"),
        (
            serde_json::json!({ "schema": { "nodes": [], "marks": [] } }).to_string(),
            "SCHEMA_INVALID",
        ),
        (
            serde_json::json!({
                "maxLength": 1,
                "initialDocumentJson": {
                    "type": "doc",
                    "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "😀x" }] }]
                }
            })
            .to_string(),
            "MAX_LENGTH_EXCEEDED",
        ),
    ] {
        let response: serde_json::Value = serde_json::from_str(
            &editor_core::collaboration_session_create_result(config),
        )
        .unwrap();
        assert_eq!(response["error"]["code"], code);
        assert!(response.get("sessionId").is_none());
    }

    for response in [
        editor_core::collaboration_session_get_document_json(u64::MAX),
        editor_core::collaboration_session_get_encoded_state(u64::MAX),
        editor_core::collaboration_session_get_peers_json(u64::MAX),
        editor_core::collaboration_session_start(u64::MAX),
        editor_core::collaboration_session_clear_local_awareness(u64::MAX),
        editor_core::collaboration_session_apply_local_document_json(
            u64::MAX,
            "{}".to_string(),
        ),
        editor_core::collaboration_session_apply_encoded_state(u64::MAX, "[]".to_string()),
        editor_core::collaboration_session_replace_encoded_state(u64::MAX, "[]".to_string()),
        editor_core::collaboration_session_handle_message(u64::MAX, "[]".to_string()),
        editor_core::collaboration_session_set_local_awareness(u64::MAX, "{}".to_string()),
    ] {
        let value: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert_eq!(value["error"]["code"], "SESSION_NOT_FOUND");
    }
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
fn editor_create_result_returns_structured_errors_and_success_ids() {
    let parse_error: serde_json::Value =
        serde_json::from_str(&editor_core::editor_create_result("{".to_string())).unwrap();
    assert_eq!(parse_error["error"]["code"], "CONFIG_PARSE_FAILED");

    let limit_error: serde_json::Value = serde_json::from_str(&editor_core::editor_create_result(
        r#"{"resourceLimits":{"maxInputBytes":0}}"#.to_string(),
    ))
    .unwrap();
    assert_eq!(limit_error["error"]["code"], "INVALID_RESOURCE_LIMIT");
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

#[test]
fn editor_creation_rejects_limits_that_exclude_its_initial_document() {
    let result: serde_json::Value = serde_json::from_str(&editor_core::editor_create_result(
        serde_json::json!({
            "resourceLimits": { "maxDocumentNodes": 1 }
        })
        .to_string(),
    ))
    .unwrap();

    assert_eq!(result["error"]["code"], "DOCUMENT_LIMIT_EXCEEDED");
    assert!(result.get("editorId").is_none());
}

#[test]
fn attacker_controlled_error_text_cannot_turn_semantic_schema_invalidity_into_a_limit_error() {
    let result: serde_json::Value = serde_json::from_str(&editor_core::editor_create_result(
        serde_json::json!({
            "schema": {
                "nodes": [
                    { "name": "doc", "role": "doc" },
                    { "name": "text", "role": "text" },
                    { "name": "work budget exceeded" },
                    { "name": "work budget exceeded" }
                ]
            }
        })
        .to_string(),
    ))
    .unwrap();

    assert!(result["editorId"].as_u64().is_some_and(|id| id > 0));
    assert!(result.get("error").is_none());
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

#[test]
fn unknown_marks_are_rejected_atomically() {
    let mut editor = Editor::new(tiptap_schema(), InterceptorPipeline::new(), false);
    editor.set_html("<p>safe</p>").unwrap();
    let before = editor.get_json();

    let error = editor
        .set_json(&serde_json::json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{
                    "type": "text",
                    "text": "unsafe",
                    "marks": [{ "type": "script" }]
                }]
            }]
        }))
        .unwrap_err();

    assert!(error.to_string().contains("UNKNOWN_MARK"));
    assert_eq!(editor.get_json(), before);
    assert!(!editor.get_html().contains("<script"));
}

#[test]
fn unknown_mark_commands_are_rejected_before_stored_marks_change() {
    let mut editor = Editor::new(tiptap_schema(), InterceptorPipeline::new(), false);
    editor.set_html("<p>safe</p>").unwrap();
    let before = editor.get_json();

    let error = match editor.toggle_mark("script") {
        Ok(_) => panic!("unknown mark command must fail"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("UNKNOWN_MARK"));
    assert_eq!(editor.get_json(), before);
    assert!(!editor.get_html().contains("<script"));
}

#[test]
fn missing_required_mark_attribute_is_rejected_atomically() {
    let mut editor = Editor::new(tiptap_schema(), InterceptorPipeline::new(), false);
    let before = editor.get_json();
    let error = editor
        .set_json(&serde_json::json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{
                    "type": "text",
                    "text": "link",
                    "marks": [{ "type": "link" }]
                }]
            }]
        }))
        .unwrap_err();

    assert!(error.to_string().contains("REQUIRED_ATTRIBUTE_MISSING"));
    assert_eq!(editor.get_json(), before);
}

#[test]
fn missing_required_void_attribute_is_rejected_atomically() {
    let schema = schema_with_required_image_src();
    let mut editor = Editor::new(schema, InterceptorPipeline::new(), false);
    editor.set_html("<p>before</p>").unwrap();
    let before = editor.get_json();

    let error = editor
        .set_json(&serde_json::json!({
            "type": "doc",
            "content": [{ "type": "image" }]
        }))
        .unwrap_err();

    assert!(error.to_string().contains("REQUIRED_ATTRIBUTE_MISSING"));
    assert_eq!(editor.get_json(), before);
}

#[test]
fn snapshot_setters_enforce_unicode_scalar_max_length() {
    use editor_core::intercept::MaxLength;

    let mut pipeline = InterceptorPipeline::new();
    pipeline.add(Box::new(MaxLength::new(1)));
    let mut editor = Editor::new(tiptap_schema(), pipeline, false);

    let error = editor.set_html("<p>😀x</p>").unwrap_err();
    assert!(error.to_string().contains("MAX_LENGTH_EXCEEDED"));
    assert_eq!(editor.get_html(), "<p></p>");
}

#[test]
fn candidate_documents_enforce_configured_node_and_depth_limits() {
    let limits = ResourceLimits {
        max_document_nodes: 3,
        max_document_depth: 2,
        ..ResourceLimits::default()
    };
    let mut editor =
        Editor::new_with_limits(tiptap_schema(), InterceptorPipeline::new(), false, limits);

    let error = editor
        .set_html("<blockquote><p>x</p></blockquote>")
        .unwrap_err();
    assert!(error.to_string().contains("DOCUMENT_LIMIT_EXCEEDED"));
    assert_eq!(editor.get_html(), "<p></p>");
}

#[test]
fn transactions_honor_configured_limits_above_defaults() {
    let limits = ResourceLimits {
        max_document_depth: 512,
        ..ResourceLimits::default()
    };
    let mut editor =
        Editor::new_with_limits(tiptap_schema(), InterceptorPipeline::new(), false, limits);
    let mut child = serde_json::json!({ "type": "paragraph" });
    for _ in 0..256 {
        child = serde_json::json!({ "type": "blockquote", "content": [child] });
    }
    let document = serde_json::json!({ "type": "doc", "content": [child] });

    editor
        .replace_json(&document)
        .expect("configured depth above the default must be honored");
}

#[test]
fn every_editor_ingestion_endpoint_admits_bytes_before_parsing() {
    let limits = ResourceLimits {
        max_input_bytes: 32,
        ..ResourceLimits::default()
    };
    let mut editor =
        Editor::new_with_limits(tiptap_schema(), InterceptorPipeline::new(), false, limits);
    let html = format!("<p>{}</p>", "x".repeat(40));
    let json = serde_json::json!({
        "type": "doc",
        "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx" }] }]
    });

    for error in [
        editor_update_error(editor.insert_content_html(&html)),
        editor_update_error(editor.replace_html(&html)),
        editor_update_error(editor.insert_content_json(&json)),
        editor_update_error(editor.replace_json(&json)),
    ] {
        assert!(error.to_string().contains("INPUT_LIMIT_EXCEEDED"));
    }
}

fn editor_update_error(result: Result<EditorUpdate, EditorError>) -> EditorError {
    match result {
        Ok(_) => panic!("operation must fail"),
        Err(error) => error,
    }
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
fn opaque_json_placement_follows_parent_content_roles() {
    let schema = Schema::from_json(&serde_json::json!({
        "nodes": [
            { "name": "doc", "content": "paragraph", "role": "doc" },
            { "name": "paragraph", "content": "inlineContainer", "role": "textBlock" },
            { "name": "inlineContainer", "content": "inline*", "group": "inline", "role": "inline" },
            { "name": "text", "content": "", "group": "inline", "role": "text" }
        ],
        "marks": []
    }))
    .unwrap();
    let original = serde_json::json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{
                "type": "inlineContainer",
                "content": [{ "type": "futureInline", "attrs": { "x": 1 } }]
            }]
        }]
    });
    let mut editor = Editor::new(schema, InterceptorPipeline::new(), false);

    editor.set_json(&original).unwrap();
    assert_eq!(editor.get_json(), original);
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

fn schema_with_required_image_src() -> Schema {
    let base = tiptap_schema();
    let mut nodes: Vec<NodeSpec> = base.all_nodes().cloned().collect();
    let image = nodes.iter_mut().find(|node| node.name == "image").unwrap();
    image.attrs.insert(
        "src".to_string(),
        AttrSpec {
            default: None,
            has_default: false,
        },
    );
    Schema::new(nodes, base.all_marks().cloned().collect())
}
