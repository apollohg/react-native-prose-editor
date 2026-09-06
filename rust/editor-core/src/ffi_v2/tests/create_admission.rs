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
    assert_eq!(
        super::editor::editor_v2_destroy(editor_id).value,
        Some(true)
    );
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
            use crate::schema::{presets::default_schema, schema_fingerprint};
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
            let schema = default_schema();
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
