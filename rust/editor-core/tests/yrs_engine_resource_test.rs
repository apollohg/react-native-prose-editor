use editor_core::boundary::ResourceLimits;
use editor_core::schema::Schema;
use editor_core::tiptap_schema;
use editor_core::yrs_engine::{
    DocumentScope, InitializationMode, TransactionOrigin, YrsDocumentEngine, YrsEngineConfig,
};
use yrs::encoding::read::Read;
use yrs::types::xml::{Xml, XmlElementPrelim, XmlFragment};
use yrs::updates::decoder::{Decoder, DecoderV1};
use yrs::{Doc, ReadTxn, StateVector, Transact, WriteTxn};

const SMALL_DECLARED_LENGTH_BOMB: &[u8] = &[1, 1, 1, 0, 3, 1, 0, 0x7f];

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
fn construction_rejects_scope_and_fragment_metadata_over_the_configured_budget() {
    for (field, configure) in [
        ("fragmentName", 0_u8),
        ("documentId", 1_u8),
        ("lineageId", 2_u8),
    ] {
        let mut candidate = config(
            tiptap_schema(),
            ResourceLimits {
                max_input_bytes: 4,
                ..ResourceLimits::default()
            },
            InitializationMode::LocalEmpty,
        );
        candidate.fragment_name = "f".into();
        candidate.scope.as_mut().unwrap().document_id = "d".into();
        candidate.scope.as_mut().unwrap().lineage_id = "l".into();
        match configure {
            0 => candidate.fragment_name = "12345".into(),
            1 => candidate.scope.as_mut().unwrap().document_id = "12345".into(),
            _ => candidate.scope.as_mut().unwrap().lineage_id = "12345".into(),
        }
        let error = match YrsDocumentEngine::new(candidate) {
            Ok(_) => panic!("{field}: oversized metadata should be rejected"),
            Err(error) => error,
        };
        assert_eq!(error.code, "INPUT_LIMIT_EXCEEDED", "{field}");
        assert_eq!(error.limit, Some(4), "{field}");
        assert_eq!(error.actual, Some(5), "{field}");
        assert_eq!(error.details, Some(serde_json::json!({"field": field})));
    }
}

fn config(
    schema: Schema,
    limits: ResourceLimits,
    initialization_mode: InitializationMode,
) -> YrsEngineConfig {
    YrsEngineConfig {
        schema,
        fragment_name: "prosemirror".to_string(),
        initialization_mode,
        resource_limits: limits,
        scope: Some(DocumentScope {
            document_id: "resource-doc".to_string(),
            lineage_id: "resource-lineage".to_string(),
        }),
    }
}

fn engine_with_limits(limits: ResourceLimits) -> YrsDocumentEngine {
    YrsDocumentEngine::new(config(
        tiptap_schema(),
        limits,
        InitializationMode::LocalEmpty,
    ))
    .unwrap()
}

fn assert_limit_error(
    error: &editor_core::yrs_engine::YrsEngineError,
    code: &'static str,
    limit: usize,
    actual: usize,
) {
    assert_eq!(error.code, code);
    assert_eq!(error.limit, Some(limit));
    assert_eq!(error.actual, Some(actual));
}

#[test]
fn input_bytes_accept_exact_and_one_above_limits_and_reject_one_below_atomically() {
    let input = r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"boundary"}]}]}"#;
    let actual = input.len();

    let mut rejected = engine_with_limits(ResourceLimits {
        max_input_bytes: actual - 1,
        ..ResourceLimits::default()
    });
    let before = audit(&rejected);
    let error = rejected
        .import_json(input, TransactionOrigin::DocumentImport)
        .unwrap_err();
    assert_limit_error(&error, "INPUT_LIMIT_EXCEEDED", actual - 1, actual);
    assert_eq!(audit(&rejected), before);

    for limit in [actual, actual + 1] {
        let mut accepted = engine_with_limits(ResourceLimits {
            max_input_bytes: limit,
            ..ResourceLimits::default()
        });
        accepted
            .import_json(input, TransactionOrigin::DocumentImport)
            .unwrap();
        assert_eq!(accepted.document_html().as_deref(), Some("<p>boundary</p>"));
    }
}

#[test]
fn document_nodes_accept_exact_and_one_above_limits_and_reject_one_below_atomically() {
    let input = r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"three"}]}]}"#;

    let mut rejected = engine_with_limits(ResourceLimits {
        max_document_nodes: 2,
        ..ResourceLimits::default()
    });
    let before = audit(&rejected);
    let error = rejected
        .import_json(input, TransactionOrigin::DocumentImport)
        .unwrap_err();
    assert_limit_error(&error, "DOCUMENT_LIMIT_EXCEEDED", 2, 3);
    assert_eq!(audit(&rejected), before);

    for limit in [3, 4] {
        let mut accepted = engine_with_limits(ResourceLimits {
            max_document_nodes: limit,
            ..ResourceLimits::default()
        });
        accepted
            .import_json(input, TransactionOrigin::DocumentImport)
            .unwrap();
        assert_eq!(accepted.document_html().as_deref(), Some("<p>three</p>"));
    }
}

#[test]
fn document_depth_accepts_exact_and_one_above_limits_and_rejects_one_below_atomically() {
    let input = r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"depth three"}]}]}"#;

    let mut rejected = engine_with_limits(ResourceLimits {
        max_document_depth: 2,
        ..ResourceLimits::default()
    });
    let before = audit(&rejected);
    let error = rejected
        .import_json(input, TransactionOrigin::DocumentImport)
        .unwrap_err();
    assert_limit_error(&error, "DOCUMENT_LIMIT_EXCEEDED", 2, 3);
    assert_eq!(audit(&rejected), before);

    for limit in [3, 4] {
        let mut accepted = engine_with_limits(ResourceLimits {
            max_document_depth: limit,
            ..ResourceLimits::default()
        });
        accepted
            .import_json(input, TransactionOrigin::DocumentImport)
            .unwrap();
        assert_eq!(
            accepted.document_html().as_deref(),
            Some("<p>depth three</p>")
        );
    }
}

#[test]
fn schema_node_and_expression_work_use_exact_configured_boundaries() {
    let schema_json = serde_json::json!({
        "nodes": [
            { "name": "doc", "content": "paragraph", "role": "doc" },
            { "name": "paragraph", "content": "text*", "group": "block", "role": "textBlock", "htmlTag": "p" },
            { "name": "text", "content": "", "group": "inline", "role": "text" }
        ],
        "marks": []
    });

    let node_error = Schema::from_json_with_limits(
        &schema_json,
        &ResourceLimits {
            max_schema_nodes: 2,
            ..ResourceLimits::default()
        },
    )
    .unwrap_err();
    assert_eq!(node_error.code, "SCHEMA_INVALID");
    assert_eq!(node_error.limit, Some(2));
    assert_eq!(node_error.actual, Some(3));

    let expression_bytes = "paragraph".len() + "text*".len();
    let expression_error = Schema::from_json_with_limits(
        &schema_json,
        &ResourceLimits {
            max_schema_expression_bytes: expression_bytes - 1,
            ..ResourceLimits::default()
        },
    )
    .unwrap_err();
    assert_eq!(expression_error.code, "SCHEMA_INVALID");
    assert_eq!(expression_error.limit, Some(expression_bytes - 1));
    assert_eq!(expression_error.actual, Some(expression_bytes));

    for (node_limit, expression_limit) in [(3, expression_bytes), (4, expression_bytes + 1)] {
        let limits = ResourceLimits {
            max_schema_nodes: node_limit,
            max_schema_expression_bytes: expression_limit,
            ..ResourceLimits::default()
        };
        let schema = Schema::from_json_with_limits(&schema_json, &limits).unwrap();
        let engine = YrsDocumentEngine::new(config(
            schema,
            limits.clone(),
            InitializationMode::LocalEmpty,
        ))
        .unwrap();
        assert_eq!(engine.resource_limits(), &limits);
        assert_eq!(engine.document_html().as_deref(), Some("<p></p>"));
    }
}

#[test]
fn encoded_state_accepts_exact_and_one_above_limits_and_rejects_one_below_atomically() {
    let mut source = engine_with_limits(ResourceLimits::default());
    source
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"encoded boundary"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let snapshot = source.export_snapshot().unwrap();
    let actual = snapshot.encoded_state.len();

    let mut rejected = YrsDocumentEngine::new(config(
        tiptap_schema(),
        ResourceLimits {
            max_encoded_state_bytes: actual - 1,
            ..ResourceLimits::default()
        },
        InitializationMode::AwaitRemote,
    ))
    .unwrap();
    let before = audit(&rejected);
    let error = rejected.restore_snapshot(&snapshot).unwrap_err();
    assert_limit_error(&error, "DOCUMENT_LIMIT_EXCEEDED", actual - 1, actual);
    assert_eq!(
        error.details,
        Some(serde_json::json!({ "field": "encodedState" }))
    );
    assert_eq!(audit(&rejected), before);

    for limit in [actual, actual + 1] {
        let mut accepted = YrsDocumentEngine::new(config(
            tiptap_schema(),
            ResourceLimits {
                max_encoded_state_bytes: limit,
                ..ResourceLimits::default()
            },
            InitializationMode::AwaitRemote,
        ))
        .unwrap();
        accepted.restore_snapshot(&snapshot).unwrap();
        assert_eq!(accepted.document_json(), source.document_json());
    }
}

#[test]
fn hostile_wide_deep_and_large_opaque_documents_are_rejected_atomically() {
    let wide = serde_json::json!({
        "type": "doc",
        "content": (0..1_000)
            .map(|_| serde_json::json!({ "type": "paragraph" }))
            .collect::<Vec<_>>()
    });
    let mut wide_engine = engine_with_limits(ResourceLimits {
        max_document_nodes: 1_000,
        ..ResourceLimits::default()
    });
    let before = audit(&wide_engine);
    let error = wide_engine
        .import_json(&wide.to_string(), TransactionOrigin::DocumentImport)
        .unwrap_err();
    assert_limit_error(&error, "DOCUMENT_LIMIT_EXCEEDED", 1_000, 1_001);
    assert_eq!(audit(&wide_engine), before);

    let deep_doc = Doc::new();
    {
        let mut transaction = deep_doc.transact_mut();
        let fragment = transaction.get_or_insert_xml_fragment("prosemirror");
        let mut parent =
            fragment.push_back(&mut transaction, XmlElementPrelim::empty("blockquote"));
        for _ in 1..300 {
            parent = parent.push_back(&mut transaction, XmlElementPrelim::empty("blockquote"));
        }
        parent.push_back(&mut transaction, XmlElementPrelim::empty("paragraph"));
    }
    let mut deep_engine = engine_with_limits(ResourceLimits::default());
    let before = audit(&deep_engine);
    let mut deep_snapshot = deep_engine.export_snapshot().unwrap();
    deep_snapshot.encoded_state = deep_doc
        .transact()
        .encode_state_as_update_v1(&StateVector::default());
    let error = deep_engine.restore_snapshot(&deep_snapshot).unwrap_err();
    assert_limit_error(&error, "DOCUMENT_LIMIT_EXCEEDED", 256, 257);
    assert_eq!(audit(&deep_engine), before);

    let opaque = serde_json::json!({
        "type": "doc",
        "content": [{
            "type": "opaqueBlock",
            "attrs": {
                "original": {
                    "payload": "x".repeat(8_192),
                    "nested": [{ "flags": [true, false, null] }]
                }
            }
        }]
    })
    .to_string();
    let limits = ResourceLimits {
        max_encoded_state_bytes: 1,
        ..ResourceLimits::default()
    };
    assert!(opaque.len() < limits.max_input_bytes);
    let mut opaque_engine = YrsDocumentEngine::new(config(
        tiptap_schema(),
        limits,
        InitializationMode::AwaitRemote,
    ))
    .unwrap();
    let before = audit(&opaque_engine);
    let error = opaque_engine
        .import_json(&opaque, TransactionOrigin::DocumentImport)
        .unwrap_err();
    assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
    assert_eq!(error.limit, Some(1));
    assert!(error.actual.is_some_and(|actual| actual > 1));
    assert_eq!(
        error.details,
        Some(serde_json::json!({ "phase": "candidateDerivation" }))
    );
    assert_eq!(audit(&opaque_engine), before);
}

#[test]
fn declared_length_bomb_reaches_binary_payload_length() {
    let mut decoder = DecoderV1::from(SMALL_DECLARED_LENGTH_BOMB);

    assert_eq!(decoder.read_var::<u32>().unwrap(), 1, "client count");
    assert_eq!(decoder.read_var::<u32>().unwrap(), 1, "block count");
    assert_eq!(decoder.read_client().unwrap().get(), 1, "client ID");
    assert_eq!(decoder.read_var::<u32>().unwrap(), 0, "client clock");
    assert_eq!(decoder.read_info().unwrap(), 3, "binary content tag");
    assert!(decoder.read_parent_info().unwrap(), "named parent");
    assert_eq!(decoder.read_string().unwrap(), "", "empty parent name");
    assert_eq!(decoder.read_len().unwrap(), 127, "declared binary length");
    assert!(
        decoder.read_exact(127).is_err(),
        "binary payload is truncated"
    );
}

#[test]
fn malformed_update_v1_seed_corpus_never_unwinds_and_is_fully_atomic() {
    let seeds: &[(&str, &[u8])] = &[
        ("empty", &[]),
        ("truncated-update", &[1]),
        (
            "overlong-varint",
            &[
                0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x00,
            ],
        ),
        (
            "invalid-client-clock",
            &[1, 1, 1, 0xff, 0xff, 0xff, 0xff, 0x7f],
        ),
        ("small-declared-length-bomb", SMALL_DECLARED_LENGTH_BOMB),
        ("truncated-delete-set", &[0, 1, 1]),
    ];

    assert_eq!(seeds.len(), 6);
    for (index, (name, encoded_state)) in seeds.iter().enumerate() {
        assert!(
            seeds[..index]
                .iter()
                .all(|(prior_name, prior_seed)| prior_name != name && prior_seed != encoded_state),
            "{name}: malformed seed names and bytes must be distinct"
        );
        let mut engine = engine_with_limits(ResourceLimits::default());
        let before = audit(&engine);
        let mut snapshot = engine.export_snapshot().unwrap();
        snapshot.encoded_state = encoded_state.to_vec();

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            engine.restore_snapshot(&snapshot)
        }));
        let error = result
            .unwrap_or_else(|_| panic!("{name}: snapshot restore unwound"))
            .unwrap_err();
        assert_eq!(error.code, "COLLABORATION_DECODE_FAILED", "{name}");
        assert_eq!(error.limit, None, "{name}");
        assert_eq!(error.actual, None, "{name}");
        let details = error.details.as_ref().expect("preflight details");
        assert_eq!(details["field"], "encodedState", "{name}");
        assert_eq!(details["phase"], "updatePreflight", "{name}");
        assert_eq!(audit(&engine), before, "{name}");
    }
}

#[test]
fn structural_preflight_rejections_are_structured_and_atomic() {
    let seeds = [
        ("array", vec![1, 1, 1, 0, 8, 1, 0, 1, 117, 127]),
        ("map", vec![1, 1, 1, 0, 8, 1, 0, 1, 118, 127]),
        ("item-count", vec![1, 1, 1, 0, 8, 1, 0, 127]),
        ("delete-ranges", vec![0, 1, 1, 127]),
    ];
    for (name, encoded_state) in seeds {
        let mut engine = engine_with_limits(ResourceLimits::default());
        let before = audit(&engine);
        let mut snapshot = engine.export_snapshot().unwrap();
        snapshot.encoded_state = encoded_state;

        let error = engine.restore_snapshot(&snapshot).unwrap_err();

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
        assert_eq!(audit(&engine), before, "{name}");
    }

    let mut nested_any = vec![1, 1, 1, 0, 8, 1, 0, 1];
    for _ in 0..8 {
        nested_any.extend_from_slice(&[117, 1]);
    }
    nested_any.extend_from_slice(&[126, 0]);
    let mut engine = engine_with_limits(ResourceLimits {
        max_document_depth: 8,
        ..ResourceLimits::default()
    });
    let before = audit(&engine);
    let mut snapshot = engine.export_snapshot().unwrap();
    snapshot.encoded_state = nested_any;

    let error = engine.restore_snapshot(&snapshot).unwrap_err();

    assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
    assert_eq!(error.limit, Some(8));
    assert_eq!(error.actual, Some(9));
    assert_eq!(audit(&engine), before);
}

#[test]
fn json_content_work_rejection_is_structured_and_atomic() {
    let json = b"[null,null,null]";
    let mut encoded_state = vec![1, 1, 1, 0, 5, 1, 0, json.len() as u8];
    encoded_state.extend_from_slice(json);
    encoded_state.push(0);
    let mut engine = engine_with_limits(ResourceLimits {
        max_document_nodes: 5,
        ..ResourceLimits::default()
    });
    let before = audit(&engine);
    let mut snapshot = engine.export_snapshot().unwrap();
    snapshot.encoded_state = encoded_state;

    let error = engine.restore_snapshot(&snapshot).unwrap_err();

    assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
    assert_eq!(error.limit, Some(5));
    assert_eq!(error.actual, Some(6));
    assert_eq!(
        error.details,
        Some(serde_json::json!({
            "field": "encodedState",
            "phase": "updatePreflight",
            "dimension": "work"
        }))
    );
    assert_eq!(audit(&engine), before);
}

#[test]
fn snapshot_any_materialization_output_limit_is_structured_and_atomic() {
    let limits = ResourceLimits {
        max_input_bytes: 70,
        ..ResourceLimits::default()
    };
    let mut engine = YrsDocumentEngine::new(YrsEngineConfig {
        schema: tiptap_schema(),
        fragment_name: "p".into(),
        initialization_mode: InitializationMode::LocalEmpty,
        resource_limits: limits,
        scope: Some(DocumentScope {
            document_id: "d".into(),
            lineage_id: "l".into(),
        }),
    })
    .unwrap();
    let hostile = Doc::new();
    {
        let mut txn = hostile.transact_mut();
        let fragment = txn.get_or_insert_xml_fragment("p");
        let paragraph = fragment.push_back(&mut txn, XmlElementPrelim::empty("paragraph"));
        paragraph.insert_attribute(&mut txn, "payload", "x".repeat(100));
    }
    let before = audit(&engine);
    let mut snapshot = engine.export_snapshot().unwrap();
    snapshot.encoded_state = hostile
        .transact()
        .encode_state_as_update_v1(&StateVector::default());

    let error = engine.restore_snapshot(&snapshot).unwrap_err();

    assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
    assert_eq!(error.limit, Some(70));
    assert!(error.actual.is_some_and(|actual| actual > 70));
    assert_eq!(
        error.details,
        Some(serde_json::json!({
            "field": "encodedState",
            "phase": "candidateMaterialization",
            "dimension": "outputBytes"
        }))
    );
    assert_eq!(audit(&engine), before);
}
