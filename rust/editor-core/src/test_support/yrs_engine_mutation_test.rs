use crate::yrs_engine::{
    Affinity, EditorOffsetKind, HistoryPolicy, RevisionedPosition, SelectionIntent,
    TransactionOrigin, TypedOperation, TypedTransaction,
};

#[test]
fn text_mutation_wire_contract_remains_typed_and_revisioned() {
    let transaction = TypedTransaction {
        request_id: 9,
        base_document_revision: 4,
        origin: TransactionOrigin::LocalInput,
        operations: vec![TypedOperation::InsertText {
            at: RevisionedPosition {
                offset: 2,
                kind: EditorOffsetKind::Utf16,
                affinity: Affinity::After,
            },
            text: "x".into(),
            marks: vec![],
        }],
        selection_intent: SelectionIntent::UseOperationResult,
        history_policy: HistoryPolicy::Auto,
    };

    assert_eq!(transaction.request_id, 9);
    assert_eq!(transaction.base_document_revision, 4);
    assert_eq!(transaction.operations.len(), 1);
}
