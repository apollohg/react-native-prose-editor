use serde_json::json;

use crate::boundary::{BoundaryError, ResourceLimits};
use crate::session::{
    CollaborationLimits, DocumentState, ErrorDomain, OperationFailureClass, SessionError,
    TransportState,
};
use crate::yrs_engine::{EditingLimits, OperationError, YrsEngineError};

use super::types::{FfiError, FfiJsonResult, FfiUnitResult, ERROR_DOMAINS, OPERATION_ERROR_CODES};

fn shared_contract() -> serde_json::Value {
    let fixtures: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../scripts/tests/security-contract-fixtures.json"
    ))
    .unwrap();
    fixtures["ffiV2ErrorContract"].clone()
}

#[test]
fn decimal_u64_serializer_keeps_the_full_u64_domain_as_canonical_strings() {
    for value in [0, (1_u64 << 53) + 1, u64::MAX] {
        assert_eq!(
            super::types::decimal_u64(value),
            json!(value.to_string()),
            "u64 {value} must cross the v2 wire as decimal text"
        );
    }
}

#[test]
fn request_envelope_errors_preserve_admitted_zero_but_omit_unadmitted_ids() {
    let editor_id = create_editor(json!({
        "initialization": { "type": "localEmpty" },
    }));

    let admitted = super::editor::editor_v2_replace_document(
        editor_id.clone(),
        json!({
            "version": 1,
            "requestId": "0",
            "history": "resetAndClear",
            "unexpected": true,
        })
        .to_string(),
    )
    .error
    .expect("invalid request must carry an error");
    assert_eq!(admitted.code, "CONFIG_INVALID");
    assert_eq!(admitted.request_id.as_deref(), Some("0"));

    let unadmitted = super::editor::editor_v2_replace_document(
        editor_id.clone(),
        json!({
            "version": 1,
            "requestId": "00",
            "history": "resetAndClear",
        })
        .to_string(),
    )
    .error
    .expect("invalid request id must carry an error");
    assert_eq!(unadmitted.code, "CONFIG_INVALID");
    assert_eq!(unadmitted.request_id, None);

    assert_eq!(
        super::editor::editor_v2_destroy(editor_id).value,
        Some(true)
    );
}

type BridgeMutationEntry = fn(String, String) -> FfiJsonResult;

fn bridge_mutation_entries() -> [(&'static str, BridgeMutationEntry, serde_json::Value); 4] {
    [
        (
            "applyInput",
            super::editor::editor_v2_apply_input,
            json!({ "text": "x" }),
        ),
        (
            "applyCommand",
            super::editor::editor_v2_apply_command,
            json!({ "command": { "type": "toggleBlockquote" } }),
        ),
        (
            "setSelection",
            super::editor::editor_v2_set_selection,
            json!({ "selection": { "type": "all" } }),
        ),
        (
            "applyLocalApi",
            super::editor::editor_v2_apply_local_api,
            json!({
                "setJson": { "type": "doc" },
                "history": "resetAndClear",
            }),
        ),
    ]
}

#[test]
fn bridge_mutation_parse_errors_recover_an_admitted_zero_request_id() {
    let editor_id = create_editor(json!({
        "initialization": { "type": "localEmpty" },
    }));

    for (label, apply, payload) in bridge_mutation_entries() {
        let mut request = json!({
            "version": 1,
            "requestId": "0",
            "baseDocumentRevision": "0",
            "unexpected": true,
        });
        for (key, value) in payload.as_object().expect("payload object") {
            request[key] = value.clone();
        }

        let error = apply(editor_id.clone(), request.to_string())
            .error
            .expect("malformed request must carry an error");
        assert_eq!(error.code, "CONFIG_INVALID", "{label}: {error:?}");
        assert_eq!(
            error.request_id.as_deref(),
            Some("0"),
            "{label}: canonical request id must survive parse failure"
        );
    }

    assert_eq!(
        super::editor::editor_v2_destroy(editor_id).value,
        Some(true)
    );
}

#[test]
fn bridge_mutation_parse_errors_omit_an_unadmitted_request_id() {
    let editor_id = create_editor(json!({
        "initialization": { "type": "localEmpty" },
    }));

    for (label, apply, payload) in bridge_mutation_entries() {
        let mut request = json!({
            "version": 1,
            "requestId": "00",
            "baseDocumentRevision": "0",
        });
        for (key, value) in payload.as_object().expect("payload object") {
            request[key] = value.clone();
        }

        let error = apply(editor_id.clone(), request.to_string())
            .error
            .expect("malformed request must carry an error");
        assert_eq!(error.code, "CONFIG_INVALID", "{label}: {error:?}");
        assert_eq!(
            error.request_id, None,
            "{label}: noncanonical request id must not be admitted"
        );
    }

    assert_eq!(
        super::editor::editor_v2_destroy(editor_id).value,
        Some(true)
    );
}

const CREATE_LIMIT_CASES: [(&str, &str, u64); 21] = [
    ("resource", "maxInputBytes", 64 * 1024 * 1024),
    ("resource", "maxDocumentNodes", 1_000_000),
    ("resource", "maxDocumentDepth", 1_024),
    ("resource", "maxSchemaNodes", 10_000),
    ("resource", "maxSchemaExpressionBytes", 1024 * 1024),
    ("resource", "maxCollaborationMessageBytes", 64 * 1024 * 1024),
    ("resource", "maxEncodedStateBytes", 256 * 1024 * 1024),
    ("editing", "maxOperationsPerTransaction", 4_096),
    ("editing", "maxUndoGroups", 2_000),
    ("editing", "maxUndoRetainedUnits", 8_000_000),
    ("editing", "maxDerivedOutputBytes", 128 * 1024 * 1024),
    ("collaboration", "maxFramesPerMessage", 1_024),
    ("collaboration", "maxFrameBytes", 64 * 1024 * 1024),
    (
        "collaboration",
        "maxAggregateResponseBytes",
        64 * 1024 * 1024,
    ),
    ("collaboration", "maxAwarenessPeers", 10_000),
    ("collaboration", "maxAwarenessPeerBytes", 1024 * 1024),
    ("collaboration", "maxAwarenessBytes", 64 * 1024 * 1024),
    ("collaboration", "maxPendingOutboxMessages", 4_096),
    ("collaboration", "maxPendingOutboxBytes", 64 * 1024 * 1024),
    (
        "collaboration",
        "maxPendingDependencyUpdateBytes",
        64 * 1024 * 1024,
    ),
    ("collaboration", "maxPendingDependencyUpdateWork", 8_000_000),
];

fn create_config_with_limit(
    group: &str,
    field: &str,
    value: serde_json::Value,
) -> serde_json::Value {
    let group_fields = serde_json::Map::from_iter([(field.to_owned(), value)]);
    let limits = serde_json::Map::from_iter([(group.to_owned(), group_fields.into())]);
    json!({
        "initialization": { "type": "localEmpty" },
        "limits": limits,
    })
}

fn create_editor(config: serde_json::Value) -> String {
    let result = super::editor::editor_v2_create(config.to_string(), None);
    if let Some(error) = result.error {
        panic!("create failed unexpectedly: {error:?}");
    }
    let value: serde_json::Value =
        serde_json::from_str(result.value.as_deref().expect("create value")).unwrap();
    value["editorId"]
        .as_str()
        .expect("decimal editor id")
        .into()
}

fn assert_create_rejected(config: serde_json::Value) {
    let result = super::editor::editor_v2_create(config.to_string(), None);
    if let Some(value) = result.value {
        let value: serde_json::Value = serde_json::from_str(&value).unwrap();
        let editor_id = value["editorId"].as_str().unwrap().to_owned();
        let _ = super::editor::editor_v2_destroy(editor_id);
        panic!("create unexpectedly accepted the config");
    }
    assert!(result.error.is_some(), "rejection must carry an error");
}

fn create_error_from_json(config_json: String) -> FfiError {
    let result = super::editor::editor_v2_create(config_json, None);
    if let Some(value) = result.value {
        let value: serde_json::Value = serde_json::from_str(&value).unwrap();
        let editor_id = value["editorId"].as_str().unwrap().to_owned();
        let _ = super::editor::editor_v2_destroy(editor_id);
        panic!("create unexpectedly accepted the config");
    }
    result.error.expect("rejection must carry an error")
}

#[test]
fn create_installs_every_limit_override_in_the_created_session() {
    let config = json!({
        "initialization": { "type": "localEmpty" },
        "limits": {
            "resource": {
                "maxInputBytes": 64 * 1024 * 1024,
                "maxDocumentNodes": 1_000_000,
                "maxDocumentDepth": 1_024,
                "maxSchemaNodes": 10_000,
                "maxSchemaExpressionBytes": 1024 * 1024,
                "maxCollaborationMessageBytes": 64 * 1024 * 1024,
                "maxEncodedStateBytes": 256 * 1024 * 1024
            },
            "editing": {
                "maxOperationsPerTransaction": 4_096,
                "maxUndoGroups": 2_000,
                "maxUndoRetainedUnits": 8_000_000,
                "maxDerivedOutputBytes": 128 * 1024 * 1024
            },
            "collaboration": {
                "maxFramesPerMessage": 1_024,
                "maxFrameBytes": 64 * 1024 * 1024,
                "maxAggregateResponseBytes": 64 * 1024 * 1024,
                "maxAwarenessPeers": 10_000,
                "maxAwarenessPeerBytes": 1024 * 1024,
                "maxAwarenessBytes": 64 * 1024 * 1024,
                "maxPendingOutboxMessages": 4_096,
                "maxPendingOutboxBytes": 64 * 1024 * 1024,
                "maxPendingDependencyUpdateBytes": 64 * 1024 * 1024,
                "maxPendingDependencyUpdateWork": 8_000_000
            }
        }
    });
    let editor_id = create_editor(config);

    super::editor::with_editor(&editor_id, |session| {
        let resource = session.engine.resource_limits();
        assert_eq!(resource.max_input_bytes, 64 * 1024 * 1024);
        assert_eq!(resource.max_document_nodes, 1_000_000);
        assert_eq!(resource.max_document_depth, 1_024);
        assert_eq!(resource.max_schema_nodes, 10_000);
        assert_eq!(resource.max_schema_expression_bytes, 1024 * 1024);
        assert_eq!(resource.max_collaboration_message_bytes, 64 * 1024 * 1024);
        assert_eq!(resource.max_encoded_state_bytes, 256 * 1024 * 1024);

        let editing = session.engine.editing_limits();
        assert_eq!(editing.max_operations_per_transaction, 4_096);
        assert_eq!(editing.max_undo_groups, 2_000);
        assert_eq!(editing.max_undo_retained_units, 8_000_000);
        assert_eq!(editing.max_derived_output_bytes, 128 * 1024 * 1024);

        let collaboration = session.collaboration_limits();
        for (group, field, ceiling) in CREATE_LIMIT_CASES {
            if group == "collaboration" {
                assert_eq!(collaboration.value(field) as u64, ceiling, "{field}");
            }
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(
        super::editor::editor_v2_destroy(editor_id).value,
        Some(true)
    );
}

#[test]
fn create_limit_tables_reject_zero_and_one_over_and_accept_exact_ceiling() {
    for (group, field, ceiling) in CREATE_LIMIT_CASES {
        assert_create_rejected(create_config_with_limit(group, field, json!(0)));

        let editor_id = create_editor(create_config_with_limit(group, field, json!(ceiling)));
        assert_eq!(
            super::editor::editor_v2_destroy(editor_id).value,
            Some(true),
            "{group}.{field} exact ceiling"
        );

        assert_create_rejected(create_config_with_limit(group, field, json!(ceiling + 1)));
    }
}

#[test]
fn create_rejects_fractional_limit_json_for_every_limit_field() {
    for (group, field, _) in CREATE_LIMIT_CASES {
        assert_create_rejected(create_config_with_limit(group, field, json!(1.5)));
    }
}

#[test]
fn create_rejects_unknown_root_and_nested_group_fields() {
    for config in [
        json!({ "initialization": { "type": "localEmpty" }, "unknown": true }),
        json!({ "initialization": { "type": "localEmpty", "unknown": true } }),
        json!({
            "initialization": {
                "type": "localJson",
                "json": { "type": "doc" },
                "unknown": true
            }
        }),
        json!({
            "initialization": {
                "type": "localHtml",
                "html": "<p>x</p>",
                "unknown": true
            }
        }),
        json!({
            "initialization": {
                "type": "room",
                "documentId": "doc-1",
                "lineageId": "lineage-1",
                "unknown": true
            }
        }),
        json!({
            "initialization": { "type": "localEmpty" },
            "policy": { "unknown": true }
        }),
        json!({
            "initialization": { "type": "localEmpty" },
            "limits": { "unknown": {} }
        }),
        json!({
            "initialization": { "type": "localEmpty" },
            "limits": { "resource": { "unknown": 1 } }
        }),
        json!({
            "initialization": { "type": "localEmpty" },
            "limits": { "editing": { "unknown": 1 } }
        }),
        json!({
            "initialization": { "type": "localEmpty" },
            "limits": { "collaboration": { "unknown": 1 } }
        }),
    ] {
        assert_create_rejected(config);
    }
}

#[test]
fn create_rejects_explicit_null_for_every_optional_create_field() {
    let mut configs = vec![
        json!({ "initialization": { "type": "localEmpty" }, "schema": null }),
        json!({ "initialization": { "type": "localEmpty" }, "fragmentName": null }),
        json!({ "initialization": { "type": "localEmpty" }, "policy": null }),
        json!({ "initialization": { "type": "localEmpty" }, "limits": null }),
        json!({
            "initialization": {
                "type": "room",
                "documentId": "doc-1",
                "lineageId": "lineage-1",
                "snapshot": null
            }
        }),
    ];
    for field in ["maxLength", "readOnly", "inputFilter", "allowBase64Images"] {
        let mut config = json!({
            "initialization": { "type": "localEmpty" },
            "policy": {}
        });
        config["policy"][field] = serde_json::Value::Null;
        configs.push(config);
    }
    for group in ["resource", "editing", "collaboration"] {
        let mut config = json!({
            "initialization": { "type": "localEmpty" },
            "limits": {}
        });
        config["limits"][group] = serde_json::Value::Null;
        configs.push(config);
    }
    for (group, field, _) in CREATE_LIMIT_CASES {
        configs.push(create_config_with_limit(
            group,
            field,
            serde_json::Value::Null,
        ));
    }

    for config in configs {
        let error = create_error_from_json(config.to_string());
        assert_eq!(error.code, "CONFIG_INVALID", "config: {config}");
    }
}

#[test]
fn create_resolves_limits_before_materializing_initialization_payload() {
    let error = create_error_from_json(
        json!({
            "initialization": { "type": "localHtml", "html": { "not": "a string" } },
            "limits": { "resource": { "maxInputBytes": 0 } }
        })
        .to_string(),
    );
    assert_eq!(error.code, "INVALID_RESOURCE_LIMIT");
}

#[test]
fn local_empty_uses_the_schema_default_document_for_validated_admission() {
    let schema = json!({
        "nodes": [
            {"name": "doc", "content": "paragraph+", "role": "doc"},
            {"name": "paragraph", "content": "inline*", "group": "block", "role": "textBlock"},
            {"name": "text", "group": "inline", "role": "text"}
        ],
        "marks": []
    });
    let editor_id = create_editor(json!({
        "schema": schema,
        "initialization": {"type": "localEmpty"}
    }));

    let document = super::editor::editor_v2_get_document_json(editor_id.clone())
        .value
        .expect("default document JSON");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&document).unwrap(),
        json!({"type": "doc", "content": [{"type": "paragraph"}]})
    );
    assert_eq!(super::editor::editor_v2_destroy(editor_id).value, Some(true));
}

#[test]
fn create_pre_serde_retained_envelope_admission_is_exact() {
    const LIMIT: usize = 64 * 1024;
    let prefix = r#"{"initialization":{"type":"localEmpty"},"policy":{"inputFilter":""#;
    let suffix = r#""}}"#;
    let exact = format!(
        "{prefix}{}{suffix}",
        "x".repeat(LIMIT - prefix.len() - suffix.len())
    );
    assert_eq!(exact.len(), LIMIT);

    let result = super::editor::editor_v2_create(exact, None);
    let error = result.error.as_ref();
    assert!(error.is_none(), "exact retained envelope failed: {error:?}");
    let value: serde_json::Value =
        serde_json::from_str(result.value.as_deref().expect("create value")).unwrap();
    let editor_id = value["editorId"].as_str().unwrap().to_owned();
    assert_eq!(
        super::editor::editor_v2_destroy(editor_id).value,
        Some(true)
    );

    let one_over = format!(
        "{prefix}{}{suffix}",
        "x".repeat(LIMIT + 1 - prefix.len() - suffix.len())
    );
    let error = create_error_from_json(one_over);
    assert_eq!(error.code, "INPUT_LIMIT_EXCEEDED");
    assert_eq!(error.limit, Some((LIMIT as u64).to_string()));
    assert_eq!(error.actual, Some(((LIMIT + 1) as u64).to_string()));
}

#[test]
fn create_pre_serde_rejects_oversized_escaped_metadata() {
    const LIMIT: usize = 64 * 1024;
    let escaped = r#"\u0061"#.repeat((LIMIT / 6) + 8);
    let configs = [
        format!(r#"{{"initialization":{{"type":"localEmpty"}},"{escaped}":0}}"#),
        format!(r#"{{"initialization":{{"type":"localEmpty","{escaped}":0}}}}"#),
        format!(r#"{{"initialization":{{"type":"{escaped}"}}}}"#),
    ];

    for config in configs {
        assert!(config.len() > LIMIT);
        let error = create_error_from_json(config.clone());
        assert_eq!(
            error.code,
            "INPUT_LIMIT_EXCEEDED",
            "config length: {}",
            config.len()
        );
        assert_eq!(error.limit, Some((LIMIT as u64).to_string()));
        assert_eq!(error.actual, Some((config.len() as u64).to_string()));
    }
}

#[test]
fn create_local_json_depth_reaches_the_configured_semantic_limit() {
    const LIMIT: usize = 1_024;

    fn nested_blockquote_document(depth: usize) -> String {
        assert!(depth >= 2);
        let wrappers = depth - 2;
        let mut document = String::from(r#"{"type":"doc","content":["#);
        for _ in 0..wrappers {
            document.push_str(r#"{"type":"blockquote","content":["#);
        }
        document.push_str(r#"{"type":"paragraph"}"#);
        for _ in 0..wrappers {
            document.push_str("]}");
        }
        document.push_str("]}");
        document
    }

    fn config(document: &str) -> String {
        let mut config = String::from(r#"{"initialization":{"type":"localJson","json":"#);
        config.push_str(document);
        config.push_str(r#"},"limits":{"resource":{"maxDocumentDepth":"#);
        config.push_str(&LIMIT.to_string());
        config.push_str("}}}");
        config
    }

    let exact_result =
        super::editor::editor_v2_create(config(&nested_blockquote_document(LIMIT)), None);
    let exact_error = exact_result.error.as_ref().map(|error| {
        (
            error.domain.as_str(),
            error.code.as_str(),
            error.limit.clone(),
            error.actual.clone(),
        )
    });
    if exact_error.is_none() {
        let value: serde_json::Value =
            serde_json::from_str(exact_result.value.as_deref().expect("create value")).unwrap();
        let editor_id = value["editorId"].as_str().unwrap().to_owned();
        assert_eq!(
            super::editor::editor_v2_destroy(editor_id).value,
            Some(true)
        );
    }

    let one_over = create_error_from_json(config(&nested_blockquote_document(LIMIT + 1)));
    assert_eq!(
        (
            exact_error,
            (
                one_over.domain.as_str(),
                one_over.code.as_str(),
                one_over.limit,
                one_over.actual,
            ),
        ),
        (
            None,
            (
                "document",
                "DOCUMENT_LIMIT_EXCEEDED",
                Some((LIMIT as u64).to_string()),
                Some(((LIMIT + 1) as u64).to_string()),
            ),
        )
    );
}

#[test]
fn max_depth_document_lifecycle_is_stack_safe_on_a_constrained_thread() {
    const LIMIT: usize = 1_024;

    let result = std::thread::Builder::new()
        .name("max-depth-document-lifecycle".into())
        .stack_size(192 * 1024)
        .spawn(|| {
            use crate::schema::{presets::tiptap_schema, schema_fingerprint};
            use crate::yrs_engine::{
                DocumentScope, InitializationMode, TransactionOrigin, YrsDocumentEngine,
                YrsEngineConfig,
            };

            fn nested_blockquote_document(depth: usize) -> String {
                let wrappers = depth - 2;
                let mut document = String::from(r#"{"type":"doc","content":["#);
                for _ in 0..wrappers {
                    document.push_str(r#"{"type":"blockquote","content":["#);
                }
                document.push_str(r#"{"type":"paragraph"}"#);
                for _ in 0..wrappers {
                    document.push_str("]}");
                }
                document.push_str("]}");
                document
            }

            fn editor_id(result: FfiJsonResult) -> String {
                let value = result.value.expect("create should succeed");
                serde_json::from_str::<serde_json::Value>(&value).unwrap()["editorId"]
                    .as_str()
                    .unwrap()
                    .to_owned()
            }

            let document = nested_blockquote_document(LIMIT);
            let mut create = String::from(r#"{"initialization":{"type":"localJson","json":"#);
            create.push_str(&document);
            create.push_str(r#"},"limits":{"resource":{"maxDocumentDepth":"#);
            create.push_str(&LIMIT.to_string());
            create.push_str("}}}");
            let local = editor_id(super::editor::editor_v2_create(create, None));

            assert!(super::editor::editor_v2_get_document_json(local.clone())
                .value
                .is_some());
            assert!(super::editor::editor_v2_get_document_html(local.clone())
                .value
                .is_some());
            assert!(super::editor::editor_v2_get_content_snapshot(local.clone())
                .value
                .is_some());
            assert!(
                super::render::editor_v2_render_update(local.clone(), None, None)
                    .value
                    .is_some()
            );

            let replacement = format!(
                r#"{{"version":1,"requestId":"1","setJson":{document},"history":"resetAndClear"}}"#
            );
            assert!(
                super::editor::editor_v2_replace_document(local.clone(), replacement)
                    .value
                    .is_some()
            );

            let mut limits = ResourceLimits::default();
            limits.max_document_depth = LIMIT;
            let schema = tiptap_schema();
            let mut source = YrsDocumentEngine::new(YrsEngineConfig {
                schema: schema.clone(),
                fragment_name: "content".into(),
                initialization_mode: InitializationMode::LocalEmpty,
                resource_limits: limits,
                editing_limits: EditingLimits::default(),
                max_length: None,
                scope: Some(DocumentScope {
                    document_id: "depth-doc".into(),
                    lineage_id: "depth-lineage".into(),
                }),
            })
            .unwrap();
            source
                .import_json(&document, TransactionOrigin::DocumentImport)
                .unwrap();
            let snapshot = source.export_snapshot().unwrap();
            drop(source);

            let room_config = json!({
                "fragmentName": "content",
                "initialization": {
                    "type": "room",
                    "documentId": "depth-doc",
                    "lineageId": "depth-lineage",
                    "snapshot": {
                        "formatVersion": snapshot.format_version,
                        "documentId": &snapshot.document_id,
                        "lineageId": &snapshot.lineage_id,
                        "fragmentName": &snapshot.fragment_name,
                        "schemaFingerprint": schema_fingerprint(&schema),
                    }
                },
                "limits": { "resource": { "maxDocumentDepth": LIMIT } }
            });
            let room_create = super::editor::editor_v2_create(
                room_config.to_string(),
                Some(snapshot.encoded_state.clone()),
            );
            let room = editor_id(room_create);
            let exported = super::snapshot::editor_v2_snapshot_export(room.clone())
                .value
                .expect("room snapshot export should succeed");
            assert!(super::snapshot::editor_v2_snapshot_restore(
                room.clone(),
                exported.metadata_json,
                exported.encoded_state,
            )
            .value
            .is_some());

            assert_eq!(super::editor::editor_v2_destroy(room).value, Some(true));
            assert_eq!(super::editor::editor_v2_destroy(local).value, Some(true));
        })
        .expect("constrained-stack lifecycle thread should spawn")
        .join();

    assert!(
        result.is_ok(),
        "max-depth lifecycle must not panic or overflow"
    );
}

#[test]
fn deep_mark_attribute_render_grows_the_document_stack_on_a_mid_sized_thread() {
    const ATTRIBUTE_DEPTH: usize = 1_000;

    let result = std::thread::Builder::new()
        .name("deep-mark-attribute-render".into())
        .stack_size(512 * 1024)
        .spawn(|| {
            let mut attribute = String::new();
            for _ in 0..ATTRIBUTE_DEPTH {
                attribute.push_str(r#"{"nested":"#);
            }
            attribute.push_str("null");
            for _ in 0..ATTRIBUTE_DEPTH {
                attribute.push('}');
            }

            let mut config = String::from(
                r#"{"initialization":{"type":"localJson","json":{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"x","marks":[{"type":"link","attrs":{"href":"#,
            );
            config.push_str(&attribute);
            config.push_str(r#"}}"#);
            config.push_str(r#"]}"#);
            config.push_str(r#"]}"#);
            config.push_str(r#"]}"#);
            config.push_str(r#"},"limits":{"resource":{"maxDocumentDepth":1024}}}"#);

            let created = super::editor::editor_v2_create(config, None);
            let editor_id: String = serde_json::from_str::<serde_json::Value>(
                created.value.as_deref().unwrap_or_else(|| {
                    panic!("deep mark create failed: {:?}", created.error)
                }),
            )
            .unwrap()["editorId"]
                .as_str()
                .unwrap()
                .to_owned();
            let stack_reservation = [0_u8; 200 * 1024];
            std::hint::black_box(&stack_reservation);
            assert!(
                super::render::editor_v2_render_update(editor_id.clone(), None, None)
                    .value
                    .is_some()
            );
            std::hint::black_box(&stack_reservation);
            assert_eq!(super::editor::editor_v2_destroy(editor_id).value, Some(true));
        })
        .expect("mid-sized-stack render thread should spawn")
        .join();

    assert!(
        result.is_ok(),
        "deep mark-attribute render must not panic or overflow"
    );
}

#[test]
fn create_uses_configured_max_input_bytes_above_the_default_for_html() {
    let html = " ".repeat(ResourceLimits::default().max_input_bytes + 1);
    let config_json = format!(
        r#"{{"initialization":{{"type":"localHtml","html":"{html}"}},"limits":{{"resource":{{"maxInputBytes":{}}}}}}}"#,
        html.len()
    );
    let result = super::editor::editor_v2_create(config_json, None);
    let error = result.error.as_ref();
    assert!(error.is_none(), "configured create failed: {error:?}");
    let value: serde_json::Value =
        serde_json::from_str(result.value.as_deref().expect("create value")).unwrap();
    let editor_id = value["editorId"].as_str().unwrap().to_owned();
    assert_eq!(
        super::editor::editor_v2_destroy(editor_id).value,
        Some(true)
    );
}

#[test]
fn create_escaped_html_uses_decoded_bytes_and_allows_the_configured_hard_limit() {
    let escaped = "\0".repeat(32);
    let exact_id = create_editor(json!({
        "initialization": { "type": "localHtml", "html": escaped },
        "limits": { "resource": { "maxInputBytes": 32 } }
    }));
    assert_eq!(super::editor::editor_v2_destroy(exact_id).value, Some(true));

    let error = create_error_from_json(
        json!({
            "initialization": { "type": "localHtml", "html": escaped },
            "limits": { "resource": { "maxInputBytes": 31 } }
        })
        .to_string(),
    );
    assert_eq!(error.code, "INPUT_LIMIT_EXCEEDED");
    assert_eq!(error.limit, Some("31".into()));
    assert_eq!(error.actual, Some("32".into()));

    let large_escaped = "\0".repeat(22 * 1024 * 1024);
    let config_json = json!({
        "initialization": { "type": "localHtml", "html": large_escaped },
        "limits": { "resource": { "maxInputBytes": 64 * 1024 * 1024 } }
    })
    .to_string();
    assert!(config_json.len() > 128 * 1024 * 1024);
    let result = super::editor::editor_v2_create(config_json, None);
    let error = result.error.as_ref();
    assert!(error.is_none(), "configured escaped HTML failed: {error:?}");
    let value: serde_json::Value =
        serde_json::from_str(result.value.as_deref().expect("create value")).unwrap();
    let editor_id = value["editorId"].as_str().unwrap().to_owned();
    assert_eq!(
        super::editor::editor_v2_destroy(editor_id).value,
        Some(true)
    );
}

#[test]
fn create_rejects_local_json_null_before_document_import() {
    let error = create_error_from_json(
        json!({
            "initialization": { "type": "localJson", "json": null }
        })
        .to_string(),
    );
    assert_eq!(error.domain, "boundary");
    assert_eq!(error.code, "CONFIG_INVALID");
}

#[test]
fn create_constructs_a_configured_schema_exactly_once() {
    crate::schema::reset_schema_from_json_count_for_test();
    let editor_id = create_editor(json!({
        "schema": {
            "nodes": [
                { "name": "doc", "content": "paragraph", "role": "doc" },
                { "name": "paragraph", "content": "text*", "role": "textBlock" },
                { "name": "text", "role": "text" }
            ],
            "marks": []
        },
        "initialization": { "type": "localEmpty" }
    }));

    assert_eq!(crate::schema::take_schema_from_json_count_for_test(), 1);
    assert_eq!(
        super::editor::editor_v2_destroy(editor_id).value,
        Some(true)
    );
}

#[test]
fn create_rejects_removed_flat_policy_keys() {
    for (field, value) in [
        ("maxLength", json!(1)),
        ("readOnly", json!(true)),
        ("inputFilter", json!("x")),
        ("allowBase64Images", json!(true)),
    ] {
        let mut config = json!({ "initialization": { "type": "localEmpty" } });
        config[field] = value;
        assert_create_rejected(config);
    }
}

#[test]
fn freezes_document_and_transport_states() {
    assert_eq!(
        [
            DocumentState::LocalReady,
            DocumentState::AwaitRemote,
            DocumentState::RoomReady,
        ]
        .map(DocumentState::as_str),
        ["LocalReady", "AwaitRemote", "RoomReady"]
    );
    assert_eq!(
        [
            TransportState::Detached,
            TransportState::Disconnected,
            TransportState::Connecting,
            TransportState::Handshaking,
            TransportState::Synchronized,
            TransportState::Incompatible,
            TransportState::Destroying,
            TransportState::Destroyed,
        ]
        .map(TransportState::as_str),
        [
            "Detached",
            "Disconnected",
            "Connecting",
            "Handshaking",
            "Synchronized",
            "Incompatible",
            "Destroying",
            "Destroyed",
        ]
    );
}

#[test]
fn freezes_all_error_domains_and_operation_codes() {
    let fixture = shared_contract();
    assert_eq!(
        ERROR_DOMAINS,
        [
            "boundary",
            "document",
            "operation",
            "lifecycle",
            "snapshot",
            "transport",
        ]
    );
    assert_eq!(fixture["domains"], json!(ERROR_DOMAINS));
    assert_eq!(
        OPERATION_ERROR_CODES,
        [
            "ENGINE_NOT_READY",
            "REVISION_MISMATCH",
            "POSITION_INVALID",
            "TRANSACTION_INVALID",
            "OPERATION_INVALID",
            "OPERATION_LIMIT_EXCEEDED",
            "OPERATION_RESOURCE_EXHAUSTED",
            "DOCUMENT_INVALID",
            "DOCUMENT_LIMIT_EXCEEDED",
            "ENGINE_INVARIANT_FAILED",
        ]
    );
    assert_eq!(fixture["operationCodes"], json!(OPERATION_ERROR_CODES));
    assert_eq!(
        fixture["representativeCodes"],
        json!({
            "lifecycle": [
                "ENGINE_DESTROYING",
                "ENGINE_DESTROYED",
                "WHOLE_DOCUMENT_REPLACEMENT_CONNECTED"
            ],
            "snapshot": ["SNAPSHOT_RESTORE_CONNECTED"],
            "transport": ["TRANSPORT_PROTOCOL_INVALID"]
        })
    );
}

#[test]
fn legacy_error_conversions_share_the_complete_code_domain_mapping() {
    let operation_cases = [
        ("ENGINE_NOT_READY", ErrorDomain::Operation),
        ("REVISION_MISMATCH", ErrorDomain::Operation),
        ("POSITION_INVALID", ErrorDomain::Operation),
        ("TRANSACTION_INVALID", ErrorDomain::Operation),
        ("OPERATION_INVALID", ErrorDomain::Operation),
        ("OPERATION_LIMIT_EXCEEDED", ErrorDomain::Operation),
        ("OPERATION_RESOURCE_EXHAUSTED", ErrorDomain::Operation),
        ("DOCUMENT_INVALID", ErrorDomain::Document),
        ("DOCUMENT_LIMIT_EXCEEDED", ErrorDomain::Document),
        ("ENGINE_INVARIANT_FAILED", ErrorDomain::Operation),
    ];
    for (code, expected_domain) in operation_cases {
        let boundary = SessionError::from(BoundaryError {
            code,
            message: code.into(),
            limit: None,
            actual: None,
            details: None,
        });
        assert_eq!(
            boundary.domain, expected_domain,
            "boundary conversion: {code}"
        );

        let engine = SessionError::from(YrsEngineError::new(code, code));
        assert_eq!(engine.domain, expected_domain, "engine conversion: {code}");
    }

    for (code, expected_domain) in [
        ("CONFIG_INVALID", ErrorDomain::Boundary),
        ("DOCUMENT_PARSE_FAILED", ErrorDomain::Document),
        ("SCHEMA_INVALID", ErrorDomain::Document),
        ("MAX_LENGTH_EXCEEDED", ErrorDomain::Document),
        ("SESSION_NOT_FOUND", ErrorDomain::Boundary),
    ] {
        let boundary = SessionError::from(BoundaryError {
            code,
            message: code.into(),
            limit: None,
            actual: None,
            details: None,
        });
        assert_eq!(
            boundary.domain, expected_domain,
            "boundary conversion: {code}"
        );

        let engine = SessionError::from(YrsEngineError::new(code, code));
        assert_eq!(engine.domain, expected_domain, "engine conversion: {code}");
    }
}

#[test]
fn operation_errors_cross_session_conversion_with_frozen_codes_and_domains() {
    for (index, code) in OPERATION_ERROR_CODES.into_iter().enumerate() {
        let failure_class = if code == "OPERATION_RESOURCE_EXHAUSTED" {
            OperationFailureClass::AllocationOrReservation
        } else {
            OperationFailureClass::ExistingStableCode
        };
        let expected_domain = if matches!(code, "DOCUMENT_INVALID" | "DOCUMENT_LIMIT_EXCEEDED") {
            ErrorDomain::Document
        } else {
            ErrorDomain::Operation
        };
        let converted = SessionError::from_operation(
            OperationError {
                code,
                message: code.into(),
                request_id: 100 + index as u64,
                operation_index: None,
                limit: None,
                actual: None,
                details: None,
            },
            failure_class,
        );

        assert_eq!(converted.code, code, "code: {code}");
        assert_eq!(converted.domain, expected_domain, "domain: {code}");
    }
}

#[test]
fn lifecycle_snapshot_and_transport_domains_require_explicit_construction() {
    type ErrorConstructor = fn(YrsEngineError) -> SessionError;
    type ErrorDomainCase = (&'static str, ErrorDomain, ErrorConstructor);

    let cases: [ErrorDomainCase; 3] = [
        (
            "ENGINE_DESTROYED",
            ErrorDomain::Lifecycle,
            SessionError::lifecycle,
        ),
        (
            "SNAPSHOT_RESTORE_CONNECTED",
            ErrorDomain::Snapshot,
            SessionError::snapshot,
        ),
        (
            "TRANSPORT_PROTOCOL_INVALID",
            ErrorDomain::Transport,
            SessionError::transport,
        ),
    ];

    for (code, expected_domain, constructor) in cases {
        let legacy = SessionError::from(YrsEngineError::new(code, code));
        assert_eq!(
            legacy.domain,
            ErrorDomain::Boundary,
            "legacy conversion: {code}"
        );

        let explicit = constructor(YrsEngineError::new(code, code));
        assert_eq!(
            explicit.domain, expected_domain,
            "explicit construction: {code}"
        );
        assert_eq!(explicit.code, code);
    }
}

#[test]
fn ffi_results_enforce_exactly_one_value_or_error() {
    let error = FfiError::new(ErrorDomain::Boundary, "CONFIG_INVALID", "invalid config");

    let json = FfiJsonResult::ok("{}".into());
    assert_eq!(json.value, Some("{}".into()));
    assert_eq!(json.error, None);
    assert!(FfiJsonResult::try_new(None, None).is_err());
    assert!(FfiJsonResult::try_new(Some("{}".into()), Some(error.clone())).is_err());

    let unit = FfiUnitResult::ok();
    assert_eq!(unit.value, Some(true));
    assert_eq!(unit.error, None);
    assert!(FfiUnitResult::try_new(None, None).is_err());
    assert!(FfiUnitResult::try_new(Some(false), None).is_err());
    assert!(FfiUnitResult::try_new(Some(true), Some(error)).is_err());
}

#[test]
fn ffi_error_conversion_uses_decimal_request_ids_and_true_nullability() {
    let operation = OperationError {
        code: "OPERATION_LIMIT_EXCEEDED",
        message: "operation limit exceeded".into(),
        request_id: u64::MAX,
        operation_index: Some(7),
        limit: Some(256),
        actual: Some(257),
        details: Some(json!({ "field": "maxOperationsPerTransaction" })),
    };
    let rich = FfiError::from(SessionError::from_operation(
        operation,
        OperationFailureClass::ExistingStableCode,
    ));
    assert_eq!(rich.domain, "operation");
    assert_eq!(rich.request_id.as_deref(), Some("18446744073709551615"));
    assert_eq!(rich.operation_index.as_deref(), Some("7"));
    assert_eq!(rich.limit.as_deref(), Some("256"));
    assert_eq!(rich.actual.as_deref(), Some("257"));
    assert_eq!(
        rich.details_json.as_deref(),
        Some(r#"{"field":"maxOperationsPerTransaction"}"#)
    );

    let empty = FfiError::new(ErrorDomain::Lifecycle, "ENGINE_DESTROYED", "destroyed");
    assert_eq!(empty.request_id, None);
    assert_eq!(empty.operation_index, None);
    assert_eq!(empty.limit, None);
    assert_eq!(empty.actual, None);
    assert_eq!(empty.details_json, None);
}

#[test]
fn deterministic_limits_remap_but_allocation_failures_keep_resource_exhausted() {
    let resource_error = || OperationError {
        code: "OPERATION_RESOURCE_EXHAUSTED",
        message: "reservation failed".into(),
        request_id: 9,
        operation_index: Some(3),
        limit: Some(8),
        actual: Some(9),
        details: Some(json!({ "field": "work" })),
    };

    let operation = SessionError::from_operation(
        resource_error(),
        OperationFailureClass::DeterministicOperationLimit,
    );
    assert_eq!(operation.domain, ErrorDomain::Operation);
    assert_eq!(operation.code, "OPERATION_LIMIT_EXCEEDED");

    let document = SessionError::from_operation(
        resource_error(),
        OperationFailureClass::DeterministicDocumentLimit,
    );
    assert_eq!(document.domain, ErrorDomain::Document);
    assert_eq!(document.code, "DOCUMENT_LIMIT_EXCEEDED");

    let allocation = SessionError::from_operation(
        resource_error(),
        OperationFailureClass::AllocationOrReservation,
    );
    assert_eq!(allocation.domain, ErrorDomain::Operation);
    assert_eq!(allocation.code, "OPERATION_RESOURCE_EXHAUSTED");
}

#[test]
fn collaboration_limits_accept_the_ceiling_and_reject_zero_and_one_over() {
    let defaults = CollaborationLimits::default();
    let ceilings = CollaborationLimits::hard_ceiling();
    let fixture = shared_contract();
    assert_eq!(
        defaults.as_pairs_json(),
        fixture["collaborationLimits"]["defaults"]
    );
    assert_eq!(
        ceilings.as_pairs_json(),
        fixture["collaborationLimits"]["ceilings"]
    );
    defaults.validate().unwrap();
    ceilings.validate().unwrap();

    for field in CollaborationLimits::fields() {
        let mut zero = defaults.clone();
        zero.set_for_test(field, 0);
        let error = zero.validate().unwrap_err();
        assert_eq!(error.domain, ErrorDomain::Boundary, "{field}");
        assert_eq!(error.code, "INVALID_RESOURCE_LIMIT", "{field}");

        let mut one_over = defaults.clone();
        one_over.set_for_test(field, ceilings.value(field) + 1);
        let error = one_over.validate().unwrap_err();
        assert_eq!(error.code, "INVALID_RESOURCE_LIMIT", "{field}");
        assert_eq!(error.limit, Some(ceilings.value(field) as u64), "{field}");
        assert_eq!(
            error.actual,
            Some(ceilings.value(field) as u64 + 1),
            "{field}"
        );
    }
}

#[test]
fn session_config_reuses_existing_resolved_limit_types() {
    fn assert_resource_limits(_: &ResourceLimits) {}
    fn assert_editing_limits(_: &EditingLimits) {}
    fn assert_collaboration_limits(_: &CollaborationLimits) {}

    let config = crate::session::EditorSessionConfig::local_for_test();
    assert_resource_limits(&config.resource_limits);
    assert_editing_limits(&config.editing_limits);
    assert_collaboration_limits(&config.collaboration_limits);
    assert_eq!(config.fragment_name, "prosemirror");
    assert_eq!(config.input_filter, None);
}

#[test]
fn snapshot_export_result_enforces_exactly_one_value_or_error() {
    use super::types::{FfiSnapshotExport, FfiSnapshotExportResult};

    let export = FfiSnapshotExport {
        metadata_json: "{}".into(),
        encoded_state: vec![1, 2, 3],
    };
    let ok = FfiSnapshotExportResult::ok(export.clone());
    assert_eq!(ok.value, Some(export.clone()));
    assert_eq!(ok.error, None);
    assert!(FfiSnapshotExportResult::try_new(None, None).is_err());
    assert!(FfiSnapshotExportResult::try_new(
        Some(export),
        Some(FfiError::new(ErrorDomain::Snapshot, "SNAPSHOT_X", "x")),
    )
    .is_err());
    let err = FfiSnapshotExportResult::err(FfiError::new(
        ErrorDomain::Snapshot,
        "SNAPSHOT_RESTORE_CONNECTED",
        "connected",
    ));
    assert_eq!(err.value, None);
    assert_eq!(err.error.map(|error| error.domain), Some("snapshot".into()));
}
