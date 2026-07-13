use std::collections::HashMap;

use editor_core::boundary::ResourceLimits;
use editor_core::schema::{AttrSpec, NodeRole, NodeSpec, Schema};
use editor_core::serialize::{
    from_prosemirror_json_with_limits, to_html, to_prosemirror_json, FromHtmlOptions,
    UnknownTypeMode,
};
use editor_core::tiptap_schema;
use editor_core::yrs_engine::{
    DocumentScope, EngineCommit, InitializationMode, TransactionOrigin, YrsDocumentEngine,
    YrsEngineConfig,
};
use yrs::types::xml::XmlFragment;
use yrs::updates::decoder::Decode;
use yrs::{Doc, ReadTxn, Transact, Update};

#[derive(Debug, serde::Deserialize)]
struct FixtureCorpus {
    fixtures: Vec<DocumentFixture>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct DocumentFixture {
    name: String,
    schema: String,
    fragment_name: Option<String>,
    document: serde_json::Value,
}

fn fixtures() -> Vec<DocumentFixture> {
    serde_json::from_str::<FixtureCorpus>(include_str!("fixtures/yrs-documents.json"))
        .expect("Yrs fixture corpus must be valid JSON")
        .fixtures
}

fn local_config(schema: Schema) -> YrsEngineConfig {
    YrsEngineConfig {
        schema,
        fragment_name: "prosemirror".to_string(),
        initialization_mode: InitializationMode::LocalEmpty,
        resource_limits: ResourceLimits::default(),
        scope: None,
    }
}

fn custom_root_schema() -> Schema {
    Schema::from_json(&serde_json::json!({
        "nodes": [
            {
                "name": "article",
                "content": "body+",
                "role": "doc"
            },
            {
                "name": "body",
                "content": "inline*",
                "group": "body",
                "role": "textBlock",
                "htmlTag": "section"
            },
            {
                "name": "text",
                "group": "inline",
                "role": "text"
            }
        ],
        "marks": []
    }))
    .unwrap()
}

fn extended_schema() -> Schema {
    let base = tiptap_schema();
    let mut nodes: Vec<NodeSpec> = base.all_nodes().cloned().collect();
    nodes.extend([
        NodeSpec {
            name: "taskList".to_string(),
            content: editor_core::schema::content_rule::ContentRule::parse("taskItem+").unwrap(),
            group: Some("block".to_string()),
            attrs: HashMap::new(),
            role: NodeRole::List { ordered: false },
            html_tag: Some("ul".to_string()),
            is_void: false,
            allow_undeclared_attrs: false,
        },
        NodeSpec {
            name: "taskItem".to_string(),
            content: editor_core::schema::content_rule::ContentRule::parse("paragraph block*")
                .unwrap(),
            group: None,
            attrs: HashMap::from([(
                "checked".to_string(),
                AttrSpec {
                    default: Some(serde_json::Value::Bool(false)),
                    has_default: true,
                },
            )]),
            role: NodeRole::ListItem,
            html_tag: Some("li".to_string()),
            is_void: false,
            allow_undeclared_attrs: false,
        },
        NodeSpec {
            name: "mention".to_string(),
            content: editor_core::schema::content_rule::ContentRule::parse("").unwrap(),
            group: Some("inline".to_string()),
            attrs: HashMap::from([(
                "label".to_string(),
                AttrSpec {
                    default: Some(serde_json::Value::Null),
                    has_default: true,
                },
            )]),
            role: NodeRole::Inline,
            html_tag: None,
            is_void: true,
            allow_undeclared_attrs: true,
        },
    ]);
    let marks = base.all_marks().cloned().collect();
    Schema::new(nodes, marks)
}

fn schema_for(name: &str) -> Schema {
    match name {
        "tiptap" => tiptap_schema(),
        "customRoot" => custom_root_schema(),
        "extended" => extended_schema(),
        other => panic!("unknown fixture schema: {other}"),
    }
}

#[test]
fn shared_document_fixtures_match_legacy_json_and_html() {
    let fixtures = fixtures();
    let required_names = [
        "canonical-empty-tiptap-document",
        "canonical-empty-custom-root-document",
        "all-built-in-heading-levels",
        "all-built-in-marks",
        "blockquote-with-multiple-blocks",
        "bullet-list",
        "ordered-list-default-and-non-default-start",
        "task-list-default-and-checked-attrs",
        "nested-bullet-ordered-and-task-lists",
        "required-and-default-image-attrs",
        "inline-and-block-void-nodes",
        "mention-with-undeclared-application-attrs",
        "opaque-unknown-inline-node-with-nested-original-json",
        "opaque-unknown-block-node-with-nested-original-json",
        "emoji-and-zwj-sequences",
        "combining-mark-sequences",
        "right-to-left-text",
        "mixed-utf16-and-scalar-boundaries",
    ];
    assert_eq!(fixtures.len(), required_names.len());
    for name in required_names {
        assert!(
            fixtures.iter().any(|fixture| fixture.name == name),
            "required Yrs fixture is missing: {name}"
        );
    }

    for fixture in fixtures {
        let schema = schema_for(&fixture.schema);
        let legacy = from_prosemirror_json_with_limits(
            &fixture.document,
            &schema,
            UnknownTypeMode::Preserve,
            &ResourceLimits::default(),
        )
        .unwrap_or_else(|error| panic!("{} legacy parse failed: {error}", fixture.name));
        let expected_json = to_prosemirror_json(&legacy, &schema);
        let expected_html = to_html(&legacy, &schema);
        let mut config = local_config(schema);
        if let Some(fragment_name) = &fixture.fragment_name {
            config.fragment_name.clone_from(fragment_name);
        }
        let mut engine = YrsDocumentEngine::new(config).unwrap();
        engine
            .import_json(
                &fixture.document.to_string(),
                TransactionOrigin::DocumentImport,
            )
            .unwrap_or_else(|error| panic!("{} engine import failed: {error}", fixture.name));
        assert_eq!(
            engine.document_json().unwrap(),
            expected_json,
            "{}",
            fixture.name
        );
        assert_eq!(
            engine.document_html().unwrap(),
            expected_html,
            "{}",
            fixture.name
        );
    }
}

#[test]
fn opaque_inline_and_block_fixtures_survive_cross_engine_snapshots() {
    let opaque_fixtures = fixtures()
        .into_iter()
        .filter(|fixture| fixture.name.starts_with("opaque-unknown-"))
        .collect::<Vec<_>>();
    assert_eq!(opaque_fixtures.len(), 2);

    for fixture in opaque_fixtures {
        let fragment_name = fixture
            .fragment_name
            .clone()
            .unwrap_or_else(|| "prosemirror".to_string());
        let scoped_config = |schema| YrsEngineConfig {
            fragment_name: fragment_name.clone(),
            scope: Some(DocumentScope {
                document_id: format!("{}-document", fixture.name),
                lineage_id: "fixture-lineage".to_string(),
            }),
            ..local_config(schema)
        };
        let mut source =
            YrsDocumentEngine::new(scoped_config(schema_for(&fixture.schema))).unwrap();
        source
            .import_json(
                &fixture.document.to_string(),
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        let expected_json = source.document_json().unwrap();
        let expected_html = source.document_html().unwrap();
        let snapshot = source.export_snapshot().unwrap();
        let mut target =
            YrsDocumentEngine::new(scoped_config(schema_for(&fixture.schema))).unwrap();

        target.restore_snapshot(&snapshot).unwrap();

        assert_eq!(
            target.document_json().unwrap(),
            expected_json,
            "{}",
            fixture.name
        );
        assert_eq!(
            target.document_html().unwrap(),
            expected_html,
            "{}",
            fixture.name
        );
    }
}

fn arb_document() -> impl proptest::strategy::Strategy<Value = (u8, serde_json::Value)> {
    use proptest::prelude::*;

    (0_u8..7, "[A-Za-z0-9 😀éאב]{1,32}", 1_u8..5, any::<bool>()).prop_map(
        |(variant, text, ordinal, flag)| {
            let paragraph = || serde_json::json!({
                "type": "paragraph",
                "content": [{"type": "text", "text": text}]
            });
            let document = match variant {
                0 => serde_json::json!({
                    "type": "doc",
                    "content": [paragraph(), paragraph()]
                }),
                1 => serde_json::json!({
                    "type": "doc",
                    "content": [{
                        "type": "bulletList",
                        "content": [{
                            "type": "listItem",
                            "content": [paragraph(), {
                                "type": "orderedList",
                                "attrs": {"start": ordinal},
                                "content": [{"type": "listItem", "content": [paragraph()]}]
                            }]
                        }]
                    }]
                }),
                2 => serde_json::json!({
                    "type": "doc",
                    "content": [{
                        "type": "taskList",
                        "content": [{
                            "type": "taskItem",
                            "attrs": {"checked": flag},
                            "content": [paragraph()]
                        }]
                    }]
                }),
                3 => serde_json::json!({
                    "type": "doc",
                    "content": [{
                        "type": "paragraph",
                        "content": [{
                            "type": "text",
                            "text": text,
                            "marks": [
                                {"type": "bold"},
                                {"type": "italic"},
                                {"type": "link", "attrs": {"href": format!("https://example.test/{ordinal}")}}
                            ]
                        }]
                    }]
                }),
                4 => serde_json::json!({
                    "type": "doc",
                    "content": [
                        {"type": "paragraph", "content": [
                            {"type": "text", "text": text},
                            {"type": "hardBreak"}
                        ]},
                        {"type": "image", "attrs": {"src": format!("https://example.test/{ordinal}.png")}},
                        {"type": "horizontalRule"}
                    ]
                }),
                5 => serde_json::json!({
                    "type": "doc",
                    "content": [{
                        "type": "generatedCallout",
                        "attrs": {
                            "rank": ordinal,
                            "payload": [text, {"enabled": flag}]
                        },
                        "content": [{"type": "text", "text": "opaque child"}]
                    }]
                }),
                _ => serde_json::json!({
                    "type": "article",
                    "content": [{
                        "type": "body",
                        "content": [{"type": "text", "text": text}]
                    }]
                }),
            };
            (variant, document)
        },
    )
}

#[test]
fn bounded_valid_documents_round_trip_with_a_fixed_structural_corpus() {
    use proptest::test_runner::{Config, RngAlgorithm, TestRng, TestRunner};
    use std::cell::Cell;

    let config = Config {
        cases: 256,
        max_shrink_iters: 4_096,
        failure_persistence: None,
        ..Config::default()
    };
    let rng = TestRng::from_seed(RngAlgorithm::ChaCha, &[0x59; 32]);
    let mut runner = TestRunner::new_with_rng(config, rng);
    let seen: [Cell<bool>; 7] = std::array::from_fn(|_| Cell::new(false));

    runner
        .run(&arb_document(), |(variant, document)| {
            seen[usize::from(variant)].set(true);
            let schema = match variant {
                2 => extended_schema(),
                6 => custom_root_schema(),
                _ => tiptap_schema(),
            };
            let mut engine = YrsDocumentEngine::new(local_config(schema)).unwrap();
            engine
                .import_json(&document.to_string(), TransactionOrigin::DocumentImport)
                .unwrap();
            let first = engine.document_json().unwrap();
            engine
                .import_json(&first.to_string(), TransactionOrigin::DocumentImport)
                .unwrap();
            proptest::prop_assert_eq!(engine.document_json().unwrap(), first);
            Ok(())
        })
        .unwrap();

    assert!(
        seen.iter().all(Cell::get),
        "fixed corpus must cover all seven structural variants"
    );
}

#[derive(Debug, PartialEq)]
struct EngineAudit {
    ready: bool,
    client_id: u64,
    revision: u64,
    last_origin: Option<TransactionOrigin>,
    document_json: Option<serde_json::Value>,
    document_html: Option<String>,
    encoded_state: Vec<u8>,
    scope: Option<DocumentScope>,
    fragment_name: String,
    schema_fingerprint: String,
}

fn audit(engine: &YrsDocumentEngine) -> EngineAudit {
    EngineAudit {
        ready: engine.is_ready(),
        client_id: engine.client_id(),
        revision: engine.revision(),
        last_origin: engine.last_committed_origin(),
        document_json: engine.document_json(),
        document_html: engine.document_html(),
        encoded_state: engine.encoded_state().unwrap(),
        scope: engine.scope().cloned(),
        fragment_name: engine.fragment_name().to_string(),
        schema_fingerprint: engine.schema_fingerprint().to_string(),
    }
}

#[test]
fn legacy_mark_order_is_pinned_and_non_schema_order_is_rejected_atomically() {
    let schema = tiptap_schema();
    let input = serde_json::json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{
                "type": "text",
                "text": "marked",
                "marks": [{"type": "italic"}, {"type": "bold"}]
            }]
        }]
    });
    let legacy = from_prosemirror_json_with_limits(
        &input,
        &schema,
        UnknownTypeMode::Preserve,
        &ResourceLimits::default(),
    )
    .unwrap();
    assert_eq!(to_prosemirror_json(&legacy, &schema), input);
    assert_eq!(
        to_html(&legacy, &schema),
        "<p><em><strong>marked</strong></em></p>"
    );

    let mut engine = YrsDocumentEngine::new(local_config(schema)).unwrap();
    let before = audit(&engine);
    let error = engine
        .import_json(&input.to_string(), TransactionOrigin::DocumentImport)
        .unwrap_err();

    assert_eq!(error.code, "DOCUMENT_INVALID");
    assert_eq!(
        error.details,
        Some(serde_json::json!({"field": "marks", "reason": "nonCanonicalOrder"}))
    );
    assert_eq!(audit(&engine), before);
}

#[test]
fn duplicate_same_type_marks_are_rejected_before_yrs_write_atomically() {
    let schema = tiptap_schema();
    let input = serde_json::json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{
                "type": "text",
                "text": "links",
                "marks": [
                    {"type": "link", "attrs": {"href": "https://one.test"}},
                    {"type": "link", "attrs": {"href": "https://two.test"}}
                ]
            }]
        }]
    });
    let legacy = from_prosemirror_json_with_limits(
        &input,
        &schema,
        UnknownTypeMode::Preserve,
        &ResourceLimits::default(),
    )
    .unwrap();
    assert_eq!(to_prosemirror_json(&legacy, &schema), input);

    let mut engine = YrsDocumentEngine::new(local_config(schema)).unwrap();
    let before = audit(&engine);
    let error = engine
        .import_json(&input.to_string(), TransactionOrigin::DocumentImport)
        .unwrap_err();

    assert_eq!(error.code, "DOCUMENT_INVALID");
    assert_eq!(
        error.details,
        Some(serde_json::json!({"field": "marks", "markType": "link", "reason": "duplicateType"}))
    );
    assert_eq!(audit(&engine), before);
}

#[test]
fn local_empty_seeds_the_canonical_schema_document_at_revision_zero() {
    let engine = YrsDocumentEngine::new(local_config(tiptap_schema())).unwrap();
    let second_engine = YrsDocumentEngine::new(local_config(tiptap_schema())).unwrap();

    assert!(engine.is_ready());
    assert_eq!(engine.revision(), 0);
    assert_eq!(engine.last_committed_origin(), None);
    assert_eq!(
        engine.document_json(),
        Some(serde_json::json!({
            "type": "doc",
            "content": [{"type": "paragraph"}]
        }))
    );
    assert_eq!(engine.document_html().as_deref(), Some("<p></p>"));
    assert!(engine.document().is_some());
    assert!(!engine.encoded_state().unwrap().is_empty());
    assert_ne!(engine.client_id(), second_engine.client_id());
}

#[test]
fn await_remote_has_no_display_fallback_and_no_seeded_items() {
    let engine = YrsDocumentEngine::new(YrsEngineConfig {
        initialization_mode: InitializationMode::AwaitRemote,
        ..local_config(tiptap_schema())
    })
    .unwrap();

    assert!(!engine.is_ready());
    assert!(engine.document().is_none());
    assert!(engine.document_json().is_none());
    assert!(engine.document_html().is_none());
    assert_eq!(engine.revision(), 0);
    assert_eq!(engine.last_committed_origin(), None);
    assert!(engine.encoded_state().unwrap().is_empty());
}

#[test]
fn local_empty_respects_custom_roots_fragments_scope_and_limits() {
    let scope = DocumentScope {
        document_id: "document-7".to_string(),
        lineage_id: "lineage-3".to_string(),
    };
    let limits = ResourceLimits::default();
    let engine = YrsDocumentEngine::new(YrsEngineConfig {
        schema: custom_root_schema(),
        fragment_name: "article-content".to_string(),
        initialization_mode: InitializationMode::LocalEmpty,
        resource_limits: limits.clone(),
        scope: Some(scope.clone()),
    })
    .unwrap();

    let encoded_state = engine.encoded_state().unwrap();
    assert_eq!(
        engine.document_json(),
        Some(serde_json::json!({
            "type": "article",
            "content": [{"type": "body"}]
        }))
    );
    assert_eq!(
        engine.document_html().as_deref(),
        Some("<section></section>")
    );
    assert_eq!(engine.fragment_name(), "article-content");
    assert_eq!(engine.scope(), Some(&scope));
    assert_eq!(engine.resource_limits(), &limits);
    assert_eq!(engine.schema_fingerprint().len(), 64);

    let replay = Doc::new();
    replay
        .transact_mut()
        .apply_update(Update::decode_v1(&encoded_state).unwrap())
        .unwrap();
    let txn = replay.transact();
    let fragment = txn.get_xml_fragment("article-content").unwrap();
    assert_eq!(fragment.len(&txn), 1);
    assert!(txn.get_xml_fragment("prosemirror").is_none());
}

#[test]
fn local_empty_rejects_seeded_state_above_the_encoded_state_limit() {
    let mut config = local_config(tiptap_schema());
    config.resource_limits.max_encoded_state_bytes = 1;

    let error = match YrsDocumentEngine::new(config) {
        Ok(_) => panic!("canonical seeded state should exceed one encoded byte"),
        Err(error) => error,
    };

    assert_eq!(error.code, "INPUT_LIMIT_EXCEEDED");
    assert_eq!(error.limit, Some(1));
    assert!(error.actual.unwrap() > 1);
}

#[test]
fn changed_json_import_swaps_the_candidate_and_commits_once() {
    let mut engine = YrsDocumentEngine::new(local_config(tiptap_schema())).unwrap();
    let before = audit(&engine);

    let commit = engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"Hello"}]}]}"#,
            TransactionOrigin::LocalApi,
        )
        .unwrap();

    assert_eq!(
        commit,
        EngineCommit {
            changed: true,
            revision: 1,
        }
    );
    assert_eq!(engine.revision(), 1);
    assert_eq!(
        engine.last_committed_origin(),
        Some(TransactionOrigin::LocalApi)
    );
    assert_ne!(engine.client_id(), before.client_id);
    assert_ne!(engine.encoded_state().unwrap(), before.encoded_state);
    assert_eq!(engine.document_html().as_deref(), Some("<p>Hello</p>"));
    assert_eq!(
        engine.document_json(),
        Some(serde_json::json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{"type": "text", "text": "Hello"}]
            }]
        }))
    );
}

#[test]
fn no_op_json_import_preserves_revision_client_and_origin() {
    let mut engine = YrsDocumentEngine::new(local_config(tiptap_schema())).unwrap();
    engine
        .import_html(
            "<p>Committed</p>",
            &FromHtmlOptions::default(),
            TransactionOrigin::LocalApi,
        )
        .unwrap();
    let before = audit(&engine);
    let commit = engine
        .import_json(
            &engine.document_json().unwrap().to_string(),
            TransactionOrigin::DocumentImport,
        )
        .unwrap();

    assert_eq!(
        commit,
        EngineCommit {
            changed: false,
            revision: 1,
        }
    );
    assert_eq!(audit(&engine), before);
}

#[test]
fn json_and_html_multi_mark_imports_are_deterministic_canonical_no_ops() {
    let input = r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"marked","marks":[{"type":"bold"},{"type":"italic"}]}]}]}"#;

    for _ in 0..64 {
        let mut engine = YrsDocumentEngine::new(local_config(tiptap_schema())).unwrap();
        engine
            .import_json(input, TransactionOrigin::DocumentImport)
            .unwrap();
        let before = audit(&engine);
        let commit = engine
            .import_json(
                &engine.document_json().unwrap().to_string(),
                TransactionOrigin::LocalApi,
            )
            .unwrap();

        assert!(!commit.changed);
        assert_eq!(audit(&engine), before);
        assert_eq!(
            engine.document_json().unwrap()["content"][0]["content"][0]["marks"],
            serde_json::json!([{"type": "bold"}, {"type": "italic"}])
        );
    }

    let mut html_engine = YrsDocumentEngine::new(local_config(tiptap_schema())).unwrap();
    let mut json_engine = YrsDocumentEngine::new(local_config(tiptap_schema())).unwrap();
    let options = FromHtmlOptions::default();

    let html_commit = html_engine
        .import_html(
            "<p>Hello <strong>world</strong></p>",
            &options,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let json_commit = json_engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"Hello "},{"type":"text","text":"world","marks":[{"type":"bold"}]}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();

    assert_eq!(html_commit, json_commit);
    assert_eq!(html_engine.document(), json_engine.document());
    assert_eq!(html_engine.document_json(), json_engine.document_json());
    assert_eq!(html_engine.document_html(), json_engine.document_html());

    let before = audit(&html_engine);
    let error = html_engine
        .import_html(
            "<marquee>unknown</marquee>",
            &FromHtmlOptions {
                strict: true,
                allow_base64_images: false,
            },
            TransactionOrigin::LocalInput,
        )
        .unwrap_err();
    assert_eq!(error.code, "DOCUMENT_INVALID");
    assert_eq!(audit(&html_engine), before);

    for _ in 0..64 {
        let mut engine = YrsDocumentEngine::new(local_config(tiptap_schema())).unwrap();
        engine
            .import_html(
                "<p><strong><em>marked</em></strong></p>",
                &FromHtmlOptions::default(),
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        let before = audit(&engine);
        let canonical_html = engine.document_html().unwrap();
        let commit = engine
            .import_html(
                &canonical_html,
                &FromHtmlOptions::default(),
                TransactionOrigin::LocalApi,
            )
            .unwrap();

        assert!(!commit.changed);
        assert_eq!(audit(&engine), before);
        assert_eq!(
            engine.document_html().as_deref(),
            Some("<p><strong><em>marked</em></strong></p>")
        );
    }
}

#[test]
fn non_strict_html_import_preserves_opaque_tags_and_is_a_canonical_no_op() {
    let mut engine = YrsDocumentEngine::new(local_config(tiptap_schema())).unwrap();
    let expected = r#"<p><widget kind="warning">preserve me</widget></p>"#;

    engine
        .import_html(
            r#"<widget kind="warning">preserve me</widget>"#,
            &FromHtmlOptions::default(),
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    assert_eq!(engine.document_html().as_deref(), Some(expected));

    let before = audit(&engine);
    let commit = engine
        .import_html(
            expected,
            &FromHtmlOptions::default(),
            TransactionOrigin::LocalApi,
        )
        .unwrap();
    assert!(!commit.changed);
    assert_eq!(audit(&engine), before);

    let before = audit(&engine);
    let exported_json = engine.document_json().unwrap().to_string();
    let commit = engine
        .import_json(&exported_json, TransactionOrigin::RemoteSync)
        .unwrap();
    assert!(!commit.changed);
    assert_eq!(audit(&engine), before);
    assert_eq!(engine.document_html().as_deref(), Some(expected));
}

#[test]
fn public_json_cannot_forge_the_reserved_html_opaque_node() {
    let mut engine = YrsDocumentEngine::new(local_config(tiptap_schema())).unwrap();
    let before = audit(&engine);

    let error = engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"__opaque","attrs":{"forged":true}}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap_err();

    assert_eq!(error.code, "CODEC_INVARIANT_FAILED");
    assert_eq!(
        error.details,
        Some(serde_json::json!({ "phase": "candidateDerivation" }))
    );
    assert_eq!(audit(&engine), before);
}

#[test]
fn awaiting_engine_import_becomes_ready_with_one_commit() {
    let mut engine = YrsDocumentEngine::new(YrsEngineConfig {
        initialization_mode: InitializationMode::AwaitRemote,
        ..local_config(tiptap_schema())
    })
    .unwrap();
    let before_client_id = engine.client_id();

    let commit = engine
        .import_html(
            "<p>Remote</p>",
            &FromHtmlOptions::default(),
            TransactionOrigin::RemoteSync,
        )
        .unwrap();

    assert_eq!(
        commit,
        EngineCommit {
            changed: true,
            revision: 1,
        }
    );
    assert!(engine.is_ready());
    assert_ne!(engine.client_id(), before_client_id);
    assert_eq!(
        engine.last_committed_origin(),
        Some(TransactionOrigin::RemoteSync)
    );
    assert_eq!(engine.document_html().as_deref(), Some("<p>Remote</p>"));
}

#[test]
fn custom_root_imports_export_the_schema_doc_role() {
    let mut config = local_config(custom_root_schema());
    config.fragment_name = "article-content".to_string();
    let mut engine = YrsDocumentEngine::new(config).unwrap();

    let commit = engine
        .import_json(
            r#"{"type":"article","content":[{"type":"body","content":[{"type":"text","text":"Custom"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();

    assert!(commit.changed);
    assert_eq!(
        engine.document_html().as_deref(),
        Some("<section>Custom</section>")
    );
    assert_eq!(engine.document_json().unwrap()["type"], "article");

    let html_commit = engine
        .import_html(
            "<section>Updated</section>",
            &FromHtmlOptions::default(),
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    assert_eq!(html_commit.revision, 2);
    assert_eq!(engine.document_json().unwrap()["type"], "article");
    assert_eq!(
        engine.document_html().as_deref(),
        Some("<section>Updated</section>")
    );

    let replay = Doc::new();
    replay
        .transact_mut()
        .apply_update(Update::decode_v1(&engine.encoded_state().unwrap()).unwrap())
        .unwrap();
    let txn = replay.transact();
    assert!(txn.get_xml_fragment("article-content").is_some());
    assert!(txn.get_xml_fragment("prosemirror").is_none());
}

#[test]
fn opaque_json_import_preserves_the_legacy_canonical_payload() {
    let mut engine = YrsDocumentEngine::new(local_config(tiptap_schema())).unwrap();
    let opaque = serde_json::json!({
        "type": "doc",
        "content": [{
            "type": "callout",
            "attrs": {
                "kind": "warning",
                "metadata": [true, null, {"rank": 2}]
            },
            "content": [{"type": "text", "text": "preserve me"}]
        }]
    });

    let commit = engine
        .import_json(&opaque.to_string(), TransactionOrigin::DocumentImport)
        .unwrap();

    assert!(commit.changed);
    assert_eq!(engine.document_json(), Some(opaque));
}

#[test]
fn rejected_import_is_completely_atomic() {
    let mut engine = YrsDocumentEngine::new(local_config(tiptap_schema())).unwrap();
    let before = audit(&engine);
    let error = engine
        .import_json(r#"{"type":"paragraph"}"#, TransactionOrigin::DocumentImport)
        .unwrap_err();

    assert_eq!(error.code, "DOCUMENT_INVALID");
    assert_eq!(audit(&engine), before);
}

#[test]
fn bounded_and_malformed_imports_preserve_the_full_audit_state() {
    let mut config = local_config(tiptap_schema());
    config.resource_limits.max_input_bytes = 16;
    let mut engine = YrsDocumentEngine::new(config).unwrap();
    let before = audit(&engine);

    let error = engine
        .import_json(
            r#"{"type":"doc","content":[]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap_err();
    assert_eq!(error.code, "INPUT_LIMIT_EXCEEDED");
    assert_eq!(error.limit, Some(16));
    assert_eq!(audit(&engine), before);

    let error = engine
        .import_json("{", TransactionOrigin::DocumentImport)
        .unwrap_err();
    assert_eq!(error.code, "DOCUMENT_INVALID");
    assert_eq!(audit(&engine), before);

    let error = engine
        .import_html(
            "<p>this HTML input is too long</p>",
            &FromHtmlOptions::default(),
            TransactionOrigin::DocumentImport,
        )
        .unwrap_err();
    assert_eq!(error.code, "INPUT_LIMIT_EXCEEDED");
    assert_eq!(audit(&engine), before);
}

#[test]
fn traversal_and_encoded_output_import_limits_are_atomic() {
    let mut traversal_config = local_config(tiptap_schema());
    traversal_config.resource_limits.max_document_nodes = 2;
    let mut traversal_engine = YrsDocumentEngine::new(traversal_config).unwrap();
    let traversal_before = audit(&traversal_engine);
    let error = traversal_engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"three"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap_err();
    assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
    assert_eq!(audit(&traversal_engine), traversal_before);

    let mut encoded_config = local_config(tiptap_schema());
    encoded_config.initialization_mode = InitializationMode::AwaitRemote;
    encoded_config.resource_limits.max_encoded_state_bytes = 1;
    let mut encoded_engine = YrsDocumentEngine::new(encoded_config).unwrap();
    let encoded_before = audit(&encoded_engine);
    let oversized_text = "x".repeat(1_024);
    let input = serde_json::json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{"type": "text", "text": oversized_text}]
        }]
    })
    .to_string();
    let error = encoded_engine
        .import_json(&input, TransactionOrigin::DocumentImport)
        .unwrap_err();
    assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
    assert_eq!(
        error.details,
        Some(serde_json::json!({ "phase": "candidateDerivation" }))
    );
    assert_eq!(audit(&encoded_engine), encoded_before);
}
