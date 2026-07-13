use editor_core::boundary::ResourceLimits;
use editor_core::tiptap_schema;
use editor_core::yrs_engine::{
    DocumentScope, EngineCommit, InitializationMode, TransactionOrigin, YrsDocumentEngine,
    YrsEngineConfig, SNAPSHOT_FORMAT_VERSION,
};
use yrs::types::text::Text;
use yrs::types::xml::{XmlFragment, XmlTextPrelim};
use yrs::updates::decoder::Decode;
use yrs::{Doc, OffsetKind, Options, ReadTxn, StateVector, Transact, Update, WriteTxn};

fn engine_config(
    document_id: &str,
    lineage_id: &str,
    fragment_name: &str,
    resource_limits: ResourceLimits,
) -> YrsEngineConfig {
    YrsEngineConfig {
        schema: tiptap_schema(),
        fragment_name: fragment_name.into(),
        initialization_mode: InitializationMode::LocalEmpty,
        resource_limits,
        scope: Some(DocumentScope {
            document_id: document_id.into(),
            lineage_id: lineage_id.into(),
        }),
    }
}

fn scoped_engine(document_id: &str, lineage_id: &str) -> YrsDocumentEngine {
    YrsDocumentEngine::new(engine_config(
        document_id,
        lineage_id,
        "prosemirror",
        ResourceLimits::default(),
    ))
    .unwrap()
}

fn populated_scoped_engine() -> YrsDocumentEngine {
    let mut engine = scoped_engine("doc-a", "lineage-a");
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"snapshot text"}]}]}"#,
            TransactionOrigin::LocalApi,
        )
        .unwrap();
    engine
}

#[derive(Debug, PartialEq)]
struct EngineAudit {
    ready: bool,
    client_id: u64,
    revision: u64,
    last_origin: Option<TransactionOrigin>,
    document_json: Option<serde_json::Value>,
    encoded_state: Vec<u8>,
}

fn audit(engine: &YrsDocumentEngine) -> EngineAudit {
    EngineAudit {
        ready: engine.is_ready(),
        client_id: engine.client_id(),
        revision: engine.revision(),
        last_origin: engine.last_committed_origin(),
        document_json: engine.document_json(),
        encoded_state: engine.encoded_state().unwrap(),
    }
}

fn state_vector(encoded_state: &[u8]) -> StateVector {
    let doc = Doc::new();
    if !encoded_state.is_empty() {
        doc.transact_mut()
            .apply_update(Update::decode_v1(encoded_state).unwrap())
            .unwrap();
    }
    let state_vector = doc.transact().state_vector();
    state_vector
}

fn assert_field(error: &editor_core::yrs_engine::YrsEngineError, field: &str) {
    assert_eq!(error.details, Some(serde_json::json!({ "field": field })));
}

#[test]
fn export_requires_a_ready_scoped_document() {
    let unscoped = YrsDocumentEngine::new(YrsEngineConfig {
        scope: None,
        ..engine_config(
            "ignored",
            "ignored",
            "prosemirror",
            ResourceLimits::default(),
        )
    })
    .unwrap();
    let error = unscoped.export_snapshot().unwrap_err();
    assert_eq!(error.code, "SNAPSHOT_SCOPE_MISMATCH");
    assert_field(&error, "documentId");

    let awaiting = YrsDocumentEngine::new(YrsEngineConfig {
        initialization_mode: InitializationMode::AwaitRemote,
        ..engine_config(
            "doc-a",
            "lineage-a",
            "prosemirror",
            ResourceLimits::default(),
        )
    })
    .unwrap();
    let error = awaiting.export_snapshot().unwrap_err();
    assert_eq!(error.code, "DOCUMENT_INVALID");
    assert_field(&error, "encodedState");
}

#[test]
fn export_snapshot_uses_the_exact_scope_codec_and_update_v1_state() {
    let source = populated_scoped_engine();
    let snapshot = source.export_snapshot().unwrap();

    assert_eq!(snapshot.format_version, SNAPSHOT_FORMAT_VERSION);
    assert_eq!(snapshot.document_id, "doc-a");
    assert_eq!(snapshot.lineage_id, "lineage-a");
    assert_eq!(snapshot.fragment_name, "prosemirror");
    assert_eq!(snapshot.schema_fingerprint, source.schema_fingerprint());
    assert_eq!(snapshot.encoded_state, source.encoded_state().unwrap());
    Update::decode_v1(&snapshot.encoded_state).unwrap();
}

#[test]
fn unsupported_format_wins_before_every_other_manifest_failure() {
    let mut engine = scoped_engine("doc-a", "lineage-a");
    let before = audit(&engine);
    let mut snapshot = engine.export_snapshot().unwrap();
    snapshot.format_version = 2;
    snapshot.document_id = "doc-b".into();
    snapshot.lineage_id = "lineage-b".into();
    snapshot.fragment_name = "other".into();
    snapshot.schema_fingerprint = "wrong".into();
    snapshot.encoded_state = vec![0xff, 0xff, 0xff];

    let error = engine.restore_snapshot(&snapshot).unwrap_err();
    assert_eq!(error.code, "SNAPSHOT_VERSION_UNSUPPORTED");
    assert_field(&error, "formatVersion");
    assert_eq!(audit(&engine), before);
}

#[test]
fn document_scope_mismatch_wins_over_malformed_encoded_state() {
    let mut engine = scoped_engine("doc-a", "lineage-a");
    let before = audit(&engine);
    let mut snapshot = engine.export_snapshot().unwrap();
    snapshot.document_id = "doc-b".into();
    snapshot.encoded_state = vec![0xff, 0xff, 0xff];

    let error = engine.restore_snapshot(&snapshot).unwrap_err();
    assert_eq!(error.code, "SNAPSHOT_SCOPE_MISMATCH");
    assert_field(&error, "documentId");
    assert_eq!(audit(&engine), before);
}

#[test]
fn metadata_mismatch_wins_over_malformed_encoded_state() {
    let mut engine = scoped_engine("doc-a", "lineage-a");
    let before = audit(&engine);
    let mut snapshot = engine.export_snapshot().unwrap();
    snapshot.lineage_id = "lineage-b".into();
    snapshot.encoded_state = vec![0xff, 0xff, 0xff];
    let error = engine.restore_snapshot(&snapshot).unwrap_err();
    assert_eq!(error.code, "SNAPSHOT_LINEAGE_MISMATCH");
    assert_field(&error, "lineageId");
    assert_eq!(audit(&engine), before);
}

#[test]
fn fragment_mismatch_wins_over_malformed_encoded_state() {
    let mut engine = scoped_engine("doc-a", "lineage-a");
    let before = audit(&engine);
    let mut snapshot = engine.export_snapshot().unwrap();
    snapshot.fragment_name = "other".into();
    snapshot.encoded_state = vec![0xff, 0xff, 0xff];

    let error = engine.restore_snapshot(&snapshot).unwrap_err();
    assert_eq!(error.code, "SNAPSHOT_FRAGMENT_MISMATCH");
    assert_field(&error, "fragmentName");
    assert_eq!(audit(&engine), before);
}

#[test]
fn schema_mismatch_wins_over_malformed_encoded_state() {
    let mut engine = scoped_engine("doc-a", "lineage-a");
    let before = audit(&engine);
    let mut snapshot = engine.export_snapshot().unwrap();
    snapshot.schema_fingerprint = "wrong".into();
    snapshot.encoded_state = vec![0xff, 0xff, 0xff];

    let error = engine.restore_snapshot(&snapshot).unwrap_err();
    assert_eq!(error.code, "SNAPSHOT_SCHEMA_MISMATCH");
    assert_field(&error, "schemaFingerprint");
    assert_eq!(audit(&engine), before);
}

#[test]
fn unscoped_restore_is_rejected_before_encoded_state_decode() {
    let source = populated_scoped_engine();
    let mut snapshot = source.export_snapshot().unwrap();
    snapshot.encoded_state = vec![0xff, 0xff, 0xff];
    let mut target = YrsDocumentEngine::new(YrsEngineConfig {
        scope: None,
        ..engine_config(
            "ignored",
            "ignored",
            "prosemirror",
            ResourceLimits::default(),
        )
    })
    .unwrap();
    let before = audit(&target);

    let error = target.restore_snapshot(&snapshot).unwrap_err();
    assert_eq!(error.code, "SNAPSHOT_SCOPE_MISMATCH");
    assert_field(&error, "documentId");
    assert_eq!(audit(&target), before);
}

#[test]
fn metadata_and_encoded_state_limits_precede_decode() {
    let source = populated_scoped_engine();
    let snapshot = source.export_snapshot().unwrap();

    let metadata_limits = ResourceLimits {
        max_input_bytes: 1,
        ..ResourceLimits::default()
    };
    let mut metadata_target = YrsDocumentEngine::new(engine_config(
        "doc-a",
        "lineage-a",
        "prosemirror",
        metadata_limits,
    ))
    .unwrap();
    let metadata_before = audit(&metadata_target);
    let mut malformed = snapshot.clone();
    malformed.encoded_state = vec![0xff, 0xff, 0xff];
    let error = metadata_target.restore_snapshot(&malformed).unwrap_err();
    assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
    assert_field(&error, "metadata");
    assert_eq!(error.limit, Some(1));
    assert_eq!(audit(&metadata_target), metadata_before);

    let encoded_limit = 1_024;
    let encoded_limits = ResourceLimits {
        max_encoded_state_bytes: encoded_limit,
        ..ResourceLimits::default()
    };
    let mut encoded_target = YrsDocumentEngine::new(engine_config(
        "doc-a",
        "lineage-a",
        "prosemirror",
        encoded_limits,
    ))
    .unwrap();
    let encoded_before = audit(&encoded_target);
    let mut oversized = snapshot;
    oversized.encoded_state = vec![0xff; encoded_limit + 1];
    let error = encoded_target.restore_snapshot(&oversized).unwrap_err();
    assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
    assert_field(&error, "encodedState");
    assert_eq!(error.limit, Some(encoded_limit));
    assert_eq!(error.actual, Some(encoded_limit + 1));
    assert_eq!(audit(&encoded_target), encoded_before);
}

#[test]
fn malformed_encoded_state_is_panic_contained_and_atomic() {
    let mut engine = scoped_engine("doc-a", "lineage-a");
    let before = audit(&engine);
    let mut snapshot = engine.export_snapshot().unwrap();
    snapshot.encoded_state = vec![0xff, 0xff, 0xff];

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        engine.restore_snapshot(&snapshot)
    }));
    let error = result
        .expect("snapshot restore must not unwind")
        .unwrap_err();
    assert_eq!(error.code, "COLLABORATION_DECODE_FAILED");
    assert_field(&error, "encodedState");
    assert_eq!(audit(&engine), before);
}

#[test]
fn awaiting_empty_state_does_not_bypass_snapshot_decode_as_a_no_op() {
    let source = scoped_engine("doc-a", "lineage-a");
    let mut snapshot = source.export_snapshot().unwrap();
    snapshot.encoded_state.clear();
    let mut awaiting = YrsDocumentEngine::new(YrsEngineConfig {
        initialization_mode: InitializationMode::AwaitRemote,
        ..engine_config(
            "doc-a",
            "lineage-a",
            "prosemirror",
            ResourceLimits::default(),
        )
    })
    .unwrap();
    let before = audit(&awaiting);

    let error = awaiting.restore_snapshot(&snapshot).unwrap_err();

    assert_eq!(error.code, "COLLABORATION_DECODE_FAILED");
    assert_field(&error, "encodedState");
    assert_eq!(audit(&awaiting), before);
}

#[test]
fn missing_fragment_and_invalid_document_are_atomic() {
    let mut engine = scoped_engine("doc-a", "lineage-a");

    let wrong_fragment_doc = Doc::new();
    {
        let mut txn = wrong_fragment_doc.transact_mut();
        let fragment = txn.get_or_insert_xml_fragment("other");
        fragment.push_back(&mut txn, XmlTextPrelim::new("wrong fragment"));
    }
    let mut missing_fragment = engine.export_snapshot().unwrap();
    missing_fragment.encoded_state = wrong_fragment_doc
        .transact()
        .encode_state_as_update_v1(&StateVector::default());
    let before = audit(&engine);
    let error = engine.restore_snapshot(&missing_fragment).unwrap_err();
    assert_eq!(error.code, "CODEC_INVARIANT_FAILED");
    assert_field(&error, "fragmentName");
    assert_eq!(audit(&engine), before);

    let invalid_doc = Doc::new();
    {
        let mut txn = invalid_doc.transact_mut();
        let fragment = txn.get_or_insert_xml_fragment("prosemirror");
        fragment.push_back(&mut txn, XmlTextPrelim::new("root text is not a block"));
    }
    let mut invalid = engine.export_snapshot().unwrap();
    invalid.encoded_state = invalid_doc
        .transact()
        .encode_state_as_update_v1(&StateVector::default());
    let error = engine.restore_snapshot(&invalid).unwrap_err();
    assert_eq!(error.code, "DOCUMENT_INVALID");
    assert_field(&error, "encodedState");
    assert_eq!(audit(&engine), before);
}

#[test]
fn derived_document_limits_are_atomic() {
    let source = populated_scoped_engine();
    let snapshot = source.export_snapshot().unwrap();
    let limits = ResourceLimits {
        max_document_nodes: 2,
        ..ResourceLimits::default()
    };
    let mut target =
        YrsDocumentEngine::new(engine_config("doc-a", "lineage-a", "prosemirror", limits)).unwrap();
    let before = audit(&target);

    let error = target.restore_snapshot(&snapshot).unwrap_err();
    assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
    assert_field(&error, "encodedState");
    assert_eq!(audit(&target), before);
}

#[test]
fn identical_snapshot_restore_is_a_full_audit_no_op() {
    let mut engine = populated_scoped_engine();
    let snapshot = engine.export_snapshot().unwrap();
    let before = audit(&engine);

    let commit = engine.restore_snapshot(&snapshot).unwrap();

    assert_eq!(
        commit,
        EngineCommit {
            changed: false,
            revision: before.revision,
        }
    );
    assert_eq!(audit(&engine), before);
}

#[test]
fn changed_restore_swaps_once_and_records_snapshot_origin() {
    let source = populated_scoped_engine();
    let snapshot = source.export_snapshot().unwrap();
    let mut target = scoped_engine("doc-a", "lineage-a");
    target
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"target"}]}]}"#,
            TransactionOrigin::LocalApi,
        )
        .unwrap();
    let before = audit(&target);

    let commit = target.restore_snapshot(&snapshot).unwrap();

    assert_eq!(
        commit,
        EngineCommit {
            changed: true,
            revision: before.revision + 1,
        }
    );
    assert_eq!(target.revision(), before.revision + 1);
    assert_eq!(
        target.last_committed_origin(),
        Some(TransactionOrigin::SnapshotRestore)
    );
    assert_eq!(target.document_json(), source.document_json());
    assert_ne!(target.client_id(), before.client_id);
}

#[test]
fn restore_uses_a_fresh_local_client_identity() {
    let source = populated_scoped_engine();
    let snapshot = source.export_snapshot().unwrap();
    let mut target = scoped_engine(&snapshot.document_id, &snapshot.lineage_id);
    let target_before = target.client_id();
    target.restore_snapshot(&snapshot).unwrap();
    assert_ne!(target.client_id(), source.client_id());
    assert_ne!(target.client_id(), target_before);
    assert_eq!(target.document_json(), source.document_json());
}

#[test]
fn round_trip_preserves_durable_state_vector_without_claiming_the_fresh_identity() {
    let source = populated_scoped_engine();
    let snapshot = source.export_snapshot().unwrap();
    let mut restored = scoped_engine("doc-a", "lineage-a");
    restored.restore_snapshot(&snapshot).unwrap();

    let source_vector = state_vector(&source.encoded_state().unwrap());
    let restored_vector = state_vector(&restored.encoded_state().unwrap());
    assert_eq!(restored_vector, source_vector);
    assert!(!restored_vector.contains_client(&restored.client_id()));

    let local_doc = Doc::with_options(Options {
        client_id: restored.client_id(),
        offset_kind: OffsetKind::Utf16,
        ..Options::default()
    });
    local_doc
        .transact_mut()
        .apply_update(Update::decode_v1(&restored.encoded_state().unwrap()).unwrap())
        .unwrap();
    assert!(!local_doc
        .transact()
        .state_vector()
        .contains_client(&restored.client_id()));
    {
        let mut txn = local_doc.transact_mut();
        let text = txn.get_or_insert_text("later-local-content");
        text.insert(&mut txn, 0, "x");
    }
    assert!(local_doc
        .transact()
        .state_vector()
        .contains_client(&restored.client_id()));
}

#[test]
fn snapshots_transport_opaque_content_and_custom_fragments_across_engines() {
    let mut source = YrsDocumentEngine::new(engine_config(
        "doc-custom",
        "lineage-custom",
        "article-content",
        ResourceLimits::default(),
    ))
    .unwrap();
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
    source
        .import_json(&opaque.to_string(), TransactionOrigin::DocumentImport)
        .unwrap();
    let snapshot = source.export_snapshot().unwrap();
    let mut target = YrsDocumentEngine::new(engine_config(
        "doc-custom",
        "lineage-custom",
        "article-content",
        ResourceLimits::default(),
    ))
    .unwrap();

    target.restore_snapshot(&snapshot).unwrap();

    assert_eq!(target.document_json(), Some(opaque));
    assert_eq!(target.fragment_name(), "article-content");
    let replay = Doc::new();
    replay
        .transact_mut()
        .apply_update(Update::decode_v1(&target.encoded_state().unwrap()).unwrap())
        .unwrap();
    let txn = replay.transact();
    assert!(txn.get_xml_fragment("article-content").is_some());
    assert!(txn.get_xml_fragment("prosemirror").is_none());
}
