use std::collections::HashMap;

use crate::boundary::ResourceLimits;
use crate::model::Node;
use crate::tiptap_schema;
use crate::yrs_engine::{
    Affinity, DocumentScope, EditingLimits, EditorOffsetKind, HistoryPolicy, InitializationMode,
    RevisionedPosition, SelectionIntent, TransactionOrigin, TypedOperation, TypedTransaction,
    YrsDocumentEngine, YrsEngineConfig,
};
use yrs::updates::decoder::Decode;
use yrs::Update;

fn engine(mode: InitializationMode) -> YrsDocumentEngine {
    YrsDocumentEngine::new(YrsEngineConfig {
        schema: tiptap_schema(),
        fragment_name: "prosemirror".into(),
        initialization_mode: mode,
        resource_limits: ResourceLimits::default(),
        editing_limits: EditingLimits::default(),
        max_length: None,
        scope: Some(DocumentScope {
            document_id: "structural-document".into(),
            lineage_id: "structural-lineage".into(),
        }),
    })
    .unwrap()
}

#[test]
fn public_atomic_entrypoint_applies_structural_plan_and_restores_standard_snapshot() {
    let mut source = engine(InitializationMode::LocalEmpty);
    let transaction = TypedTransaction {
        request_id: 21,
        base_document_revision: source.revision(),
        origin: TransactionOrigin::LocalCommand,
        operations: vec![TypedOperation::InsertNode {
            at: RevisionedPosition {
                offset: 1,
                kind: EditorOffsetKind::Scalar,
                affinity: Affinity::After,
            },
            node: Node::void("hardBreak".into(), HashMap::new()),
        }],
        selection_intent: SelectionIntent::UseOperationResult,
        history_policy: HistoryPolicy::Auto,
    };

    let commit = source.apply_typed_transaction(transaction).unwrap();

    assert!(commit.changed);
    assert_eq!(commit.document_revision, 1);
    assert_eq!(commit.state_revision, 1);
    assert_eq!(
        source.document_json().unwrap(),
        serde_json::json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{ "type": "hardBreak" }]
            }]
        })
    );

    let snapshot = source.export_snapshot().unwrap();
    let mut replica = engine(InitializationMode::AwaitRemote);
    replica.restore_snapshot(&snapshot).unwrap();
    assert_eq!(replica.document_json(), source.document_json());
    assert_eq!(replica.document_html(), source.document_html());
    let source_vector = Update::decode_v1(&snapshot.encoded_state)
        .unwrap()
        .state_vector();
    let replica_vector = Update::decode_v1(&replica.encoded_state().unwrap())
        .unwrap()
        .state_vector();
    assert_eq!(replica_vector, source_vector);
}
