use editor_core::boundary::ResourceLimits;
use editor_core::model::Mark;
use editor_core::tiptap_schema;
use editor_core::yrs_engine::{
    Affinity, DocumentScope, EditingLimits, EditorOffsetKind, HistoryPolicy, InitializationMode,
    RevisionedPosition, SelectionIntent, TransactionOrigin, TypedOperation, TypedTransaction,
    YrsDocumentEngine, YrsEngineConfig,
};
use yrs::updates::decoder::Decode;
use yrs::{Doc, ReadTxn, StateVector, Transact, Update};

#[derive(Debug, PartialEq)]
struct RejectedTransactionAudit {
    encoded_state: Vec<u8>,
    state_vector: StateVector,
    canonical_json: Option<serde_json::Value>,
    html: Option<String>,
    document_revision: u64,
    state_revision: u64,
    client_id: u64,
    durable_client_ids: Vec<u64>,
    relative_selection: Option<editor_core::yrs_engine::RelativeSelection>,
    resolved_selection: Option<editor_core::yrs_engine::ResolvedSelection>,
    stored_marks: Option<()>,
    render_cache: Vec<()>,
    history_depth: usize,
    last_origin: Option<TransactionOrigin>,
    document_id: String,
    lineage_id: String,
    fragment_name: String,
    schema_fingerprint: String,
}

fn state_vector(encoded_state: &[u8]) -> StateVector {
    let doc = Doc::new();
    if !encoded_state.is_empty() {
        let mut txn = doc.transact_mut();
        txn.apply_update(Update::decode_v1(encoded_state).unwrap())
            .unwrap();
    }
    let txn = doc.transact();
    txn.state_vector()
}

fn audit(engine: &YrsDocumentEngine) -> RejectedTransactionAudit {
    let encoded_state = engine.encoded_state().unwrap();
    let state_vector = state_vector(&encoded_state);
    let mut durable_client_ids: Vec<_> = state_vector
        .iter()
        .map(|(client, _)| client.get())
        .collect();
    durable_client_ids.sort_unstable();
    let scope = engine.scope().unwrap();
    RejectedTransactionAudit {
        state_vector,
        encoded_state,
        canonical_json: engine.document_json(),
        html: engine.document_html(),
        document_revision: engine.revision(),
        state_revision: engine.state_revision(),
        client_id: engine.client_id(),
        durable_client_ids,
        relative_selection: engine.relative_selection().cloned(),
        resolved_selection: engine.resolved_selection().cloned(),
        stored_marks: None,
        render_cache: Vec::new(),
        history_depth: 0,
        last_origin: engine.last_committed_origin(),
        document_id: scope.document_id.clone(),
        lineage_id: scope.lineage_id.clone(),
        fragment_name: engine.fragment_name().to_owned(),
        schema_fingerprint: engine.schema_fingerprint().to_owned(),
    }
}

fn engine(mode: InitializationMode) -> YrsDocumentEngine {
    YrsDocumentEngine::new(YrsEngineConfig {
        schema: tiptap_schema(),
        fragment_name: "prosemirror".into(),
        initialization_mode: mode,
        resource_limits: ResourceLimits::default(),
        editing_limits: EditingLimits::default(),
        max_length: None,
        scope: Some(DocumentScope {
            document_id: "document-7".into(),
            lineage_id: "lineage-7".into(),
        }),
    })
    .unwrap()
}

fn transaction(
    engine: &YrsDocumentEngine,
    request_id: u64,
    operations: Vec<TypedOperation>,
) -> TypedTransaction {
    TypedTransaction {
        request_id,
        base_document_revision: engine.revision(),
        origin: TransactionOrigin::LocalApi,
        operations,
        selection_intent: SelectionIntent::Preserve,
        history_policy: HistoryPolicy::Skip,
    }
}

fn point(offset: u32) -> RevisionedPosition {
    RevisionedPosition {
        offset,
        kind: EditorOffsetKind::Scalar,
        affinity: Affinity::After,
    }
}

#[test]
fn durable_commit_changes_both_revisions_once_and_complete_no_op_changes_neither() {
    let mut engine = engine(InitializationMode::LocalEmpty);
    let commit = engine
        .apply_typed_transaction(transaction(
            &engine,
            7,
            vec![TypedOperation::InsertText {
                at: point(1),
                text: "x".into(),
                marks: vec![],
            }],
        ))
        .unwrap();

    assert!(commit.changed);
    assert_eq!(commit.document_revision, 1);
    assert_eq!(commit.state_revision, 1);
    assert_eq!(commit.request_id, 7);
    assert_eq!(commit.origin, TransactionOrigin::LocalApi);
    assert_eq!(
        engine.document_json().unwrap()["content"][0]["content"][0]["text"],
        "x"
    );

    let before = audit(&engine);
    let no_op = engine
        .apply_typed_transaction(transaction(&engine, 8, vec![]))
        .unwrap();
    assert!(!no_op.changed);
    assert_eq!(no_op.document_revision, 1);
    assert_eq!(no_op.state_revision, 1);
    assert_eq!(audit(&engine), before);
}

#[test]
fn stale_and_awaiting_transactions_reject_without_changing_any_audited_state() {
    let mut ready = engine(InitializationMode::LocalEmpty);
    let mut stale = transaction(
        &ready,
        9,
        vec![TypedOperation::InsertText {
            at: point(1),
            text: "stale".into(),
            marks: vec![],
        }],
    );
    stale.base_document_revision = ready.revision() + 1;
    let before = audit(&ready);
    let error = ready.apply_typed_transaction(stale).unwrap_err();
    assert_eq!(error.code, "REVISION_MISMATCH");
    assert_eq!(audit(&ready), before);

    let mut awaiting = engine(InitializationMode::AwaitRemote);
    let before = audit(&awaiting);
    let error = awaiting
        .apply_typed_transaction(transaction(&awaiting, 10, vec![]))
        .unwrap_err();
    assert_eq!(error.code, "ENGINE_NOT_READY");
    assert_eq!(audit(&awaiting), before);
}

#[test]
fn validation_and_resource_failure_classes_preserve_the_complete_audit() {
    let mut invalid_position = engine(InitializationMode::LocalEmpty);
    let tx = transaction(
        &invalid_position,
        11,
        vec![TypedOperation::InsertText {
            at: point(99),
            text: "x".into(),
            marks: vec![],
        }],
    );
    let before = audit(&invalid_position);
    let error = invalid_position.apply_typed_transaction(tx).unwrap_err();
    assert_eq!(error.code, "POSITION_INVALID");
    assert_eq!(audit(&invalid_position), before);

    let mut invalid_origin = engine(InitializationMode::LocalEmpty);
    let mut tx = transaction(&invalid_origin, 12, vec![]);
    tx.origin = TransactionOrigin::RemoteSync;
    let before = audit(&invalid_origin);
    let error = invalid_origin.apply_typed_transaction(tx).unwrap_err();
    assert_eq!(error.code, "TRANSACTION_INVALID");
    assert_eq!(audit(&invalid_origin), before);

    let mut invalid_operation = engine(InitializationMode::LocalEmpty);
    let tx = transaction(
        &invalid_operation,
        13,
        vec![TypedOperation::InsertText {
            at: point(1),
            text: "x".into(),
            marks: vec![Mark::new("unknown".into(), Default::default())],
        }],
    );
    let before = audit(&invalid_operation);
    let error = invalid_operation.apply_typed_transaction(tx).unwrap_err();
    assert_eq!(error.code, "OPERATION_INVALID");
    assert_eq!(audit(&invalid_operation), before);

    let mut operation_limited = YrsDocumentEngine::new(YrsEngineConfig {
        schema: tiptap_schema(),
        fragment_name: "prosemirror".into(),
        initialization_mode: InitializationMode::LocalEmpty,
        resource_limits: ResourceLimits::default(),
        editing_limits: EditingLimits {
            max_operations_per_transaction: 1,
            ..EditingLimits::default()
        },
        max_length: None,
        scope: Some(DocumentScope {
            document_id: "document-7".into(),
            lineage_id: "lineage-7".into(),
        }),
    })
    .unwrap();
    let operation = TypedOperation::InsertText {
        at: point(1),
        text: "x".into(),
        marks: vec![],
    };
    let tx = transaction(&operation_limited, 14, vec![operation.clone(), operation]);
    let before = audit(&operation_limited);
    let error = operation_limited.apply_typed_transaction(tx).unwrap_err();
    assert_eq!(error.code, "OPERATION_LIMIT_EXCEEDED");
    assert_eq!(audit(&operation_limited), before);

    let mut document_limited = YrsDocumentEngine::new(YrsEngineConfig {
        schema: tiptap_schema(),
        fragment_name: "prosemirror".into(),
        initialization_mode: InitializationMode::LocalEmpty,
        resource_limits: ResourceLimits::default(),
        editing_limits: EditingLimits::default(),
        max_length: Some(0),
        scope: Some(DocumentScope {
            document_id: "document-7".into(),
            lineage_id: "lineage-7".into(),
        }),
    })
    .unwrap();
    let tx = transaction(
        &document_limited,
        15,
        vec![TypedOperation::InsertText {
            at: point(1),
            text: "x".into(),
            marks: vec![],
        }],
    );
    let before = audit(&document_limited);
    let error = document_limited.apply_typed_transaction(tx).unwrap_err();
    assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
    assert_eq!(audit(&document_limited), before);
}
