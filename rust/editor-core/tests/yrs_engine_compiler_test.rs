use std::collections::HashMap;

use editor_core::boundary::ResourceLimits;
use editor_core::model::{Fragment, Mark, Node};
use editor_core::tiptap_schema;
use editor_core::yrs_engine::{
    Affinity, EditingLimits, EditorOffsetKind, HistoryPolicy, InitializationMode, OperationError,
    RevisionedPosition, RevisionedRange, SelectionIntent, TransactionOrigin, TypedOperation,
    TypedTransaction, YrsDocumentEngine, YrsEngineConfig,
};

#[test]
fn typed_preview_inputs_are_inert_public_values() {
    let engine = YrsDocumentEngine::new(YrsEngineConfig {
        schema: tiptap_schema(),
        fragment_name: "prosemirror".into(),
        initialization_mode: InitializationMode::LocalEmpty,
        resource_limits: ResourceLimits::default(),
        editing_limits: EditingLimits::default(),
        max_length: None,
        scope: None,
    })
    .unwrap();
    let before = engine.encoded_state().unwrap();
    let at = RevisionedPosition {
        offset: 0,
        kind: EditorOffsetKind::Scalar,
        affinity: Affinity::After,
    };
    let range = RevisionedRange { from: at, to: at };
    let mark = Mark::new("bold".into(), HashMap::new());
    let transaction = TypedTransaction {
        request_id: 9,
        base_document_revision: engine.revision(),
        origin: TransactionOrigin::LocalApi,
        operations: vec![
            TypedOperation::InsertText {
                at,
                text: "inert".into(),
                marks: vec![],
            },
            TypedOperation::DeleteRange { range },
            TypedOperation::ReplaceRange {
                range,
                content: Fragment::from(vec![Node::text("x".into(), vec![])]),
            },
            TypedOperation::AddMark {
                range,
                mark: mark.clone(),
            },
            TypedOperation::RemoveMark {
                range,
                mark_type: "bold".into(),
            },
            TypedOperation::ReplaceMark { range, mark },
        ],
        selection_intent: SelectionIntent::Preserve,
        history_policy: HistoryPolicy::Skip,
    };

    assert_eq!(transaction.operations.len(), 6);
    assert_eq!(engine.encoded_state().unwrap(), before);
    assert_eq!(engine.revision(), 0);

    let error = OperationError::operation_invalid(9, 4, "mark", "invalid mark");
    assert_eq!(error.code, "OPERATION_INVALID");
    assert_eq!(error.operation_index, Some(4));
    assert_eq!(
        serde_json::to_value(error).unwrap()["details"],
        serde_json::json!({ "field": "mark" })
    );
}
