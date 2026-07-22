use std::collections::HashMap;

use crate::boundary::ResourceLimits;
use crate::model::{Fragment, Mark, Node};
use crate::tiptap_schema;
use crate::yrs_engine::{
    Affinity, EditingLimitOverrides, EditingLimits, EditorOffsetKind, HistoryPolicy,
    InitializationMode, OperationError, RevisionedPosition, RevisionedRange, SelectionInput,
    SelectionIntent, TransactionCommit, TransactionOrigin, TypedOperation, TypedTransaction,
    YrsDocumentEngine, YrsEngineConfig, HARD_MAX_DERIVED_OUTPUT_BYTES,
    HARD_MAX_OPERATIONS_PER_TRANSACTION, HARD_MAX_UNDO_GROUPS, HARD_MAX_UNDO_RETAINED_UNITS,
};
use serde_json::json;

#[test]
fn editing_limits_have_approved_defaults_and_hard_ceilings() {
    let defaults = EditingLimits::default();
    assert_eq!(defaults.max_operations_per_transaction, 256);
    assert_eq!(defaults.max_undo_groups, 500);
    assert_eq!(defaults.max_undo_retained_units, 1_000_000);
    assert_eq!(defaults.max_derived_output_bytes, 32 * 1024 * 1024);

    assert_eq!(HARD_MAX_OPERATIONS_PER_TRANSACTION, 4_096);
    assert_eq!(HARD_MAX_UNDO_GROUPS, 2_000);
    assert_eq!(HARD_MAX_UNDO_RETAINED_UNITS, 8_000_000);
    assert_eq!(HARD_MAX_DERIVED_OUTPUT_BYTES, 128 * 1024 * 1024);

    assert_eq!(
        EditingLimits::resolve(EditingLimitOverrides {
            max_operations_per_transaction: Some(HARD_MAX_OPERATIONS_PER_TRANSACTION),
            max_undo_groups: Some(HARD_MAX_UNDO_GROUPS),
            max_undo_retained_units: Some(HARD_MAX_UNDO_RETAINED_UNITS),
            max_derived_output_bytes: Some(HARD_MAX_DERIVED_OUTPUT_BYTES),
        })
        .unwrap(),
        EditingLimits {
            max_operations_per_transaction: HARD_MAX_OPERATIONS_PER_TRANSACTION,
            max_undo_groups: HARD_MAX_UNDO_GROUPS,
            max_undo_retained_units: HARD_MAX_UNDO_RETAINED_UNITS,
            max_derived_output_bytes: HARD_MAX_DERIVED_OUTPUT_BYTES,
        }
    );
}

#[test]
fn editing_limits_reject_zero_and_one_over_with_field_details() {
    let invalid = [
        (
            EditingLimitOverrides {
                max_operations_per_transaction: Some(0),
                ..EditingLimitOverrides::default()
            },
            "maxOperationsPerTransaction",
        ),
        (
            EditingLimitOverrides {
                max_operations_per_transaction: Some(HARD_MAX_OPERATIONS_PER_TRANSACTION + 1),
                ..EditingLimitOverrides::default()
            },
            "maxOperationsPerTransaction",
        ),
        (
            EditingLimitOverrides {
                max_undo_groups: Some(0),
                ..EditingLimitOverrides::default()
            },
            "maxUndoGroups",
        ),
        (
            EditingLimitOverrides {
                max_undo_groups: Some(HARD_MAX_UNDO_GROUPS + 1),
                ..EditingLimitOverrides::default()
            },
            "maxUndoGroups",
        ),
        (
            EditingLimitOverrides {
                max_undo_retained_units: Some(0),
                ..EditingLimitOverrides::default()
            },
            "maxUndoRetainedUnits",
        ),
        (
            EditingLimitOverrides {
                max_undo_retained_units: Some(HARD_MAX_UNDO_RETAINED_UNITS + 1),
                ..EditingLimitOverrides::default()
            },
            "maxUndoRetainedUnits",
        ),
        (
            EditingLimitOverrides {
                max_derived_output_bytes: Some(0),
                ..EditingLimitOverrides::default()
            },
            "maxDerivedOutputBytes",
        ),
        (
            EditingLimitOverrides {
                max_derived_output_bytes: Some(HARD_MAX_DERIVED_OUTPUT_BYTES + 1),
                ..EditingLimitOverrides::default()
            },
            "maxDerivedOutputBytes",
        ),
    ];

    for (overrides, field) in invalid {
        let error = EditingLimits::resolve(overrides).unwrap_err();
        assert_eq!(error.code, "INVALID_RESOURCE_LIMIT");
        assert_eq!(error.details, Some(json!({ "field": field })));
    }
}

#[test]
fn engine_config_stores_editing_limits_and_preserves_explicit_zero_max_length() {
    let editing_limits = EditingLimits::resolve(EditingLimitOverrides {
        max_operations_per_transaction: Some(17),
        ..EditingLimitOverrides::default()
    })
    .unwrap();
    let engine = YrsDocumentEngine::new(YrsEngineConfig {
        schema: tiptap_schema(),
        fragment_name: "prosemirror".into(),
        initialization_mode: InitializationMode::LocalEmpty,
        resource_limits: ResourceLimits::default(),
        editing_limits: editing_limits.clone(),
        max_length: Some(0),
        scope: None,
    })
    .unwrap();

    assert_eq!(engine.editing_limits(), &editing_limits);
    assert_eq!(engine.max_length(), Some(0));
}

#[test]
fn engine_config_revalidates_raw_editing_limits_before_creating_a_document() {
    let exact = EditingLimits {
        max_operations_per_transaction: HARD_MAX_OPERATIONS_PER_TRANSACTION,
        max_undo_groups: HARD_MAX_UNDO_GROUPS,
        max_undo_retained_units: HARD_MAX_UNDO_RETAINED_UNITS,
        max_derived_output_bytes: HARD_MAX_DERIVED_OUTPUT_BYTES,
    };
    let exact_engine = YrsDocumentEngine::new(YrsEngineConfig {
        schema: tiptap_schema(),
        fragment_name: "prosemirror".into(),
        initialization_mode: InitializationMode::LocalEmpty,
        resource_limits: ResourceLimits::default(),
        editing_limits: exact.clone(),
        max_length: None,
        scope: None,
    })
    .unwrap();
    assert_eq!(exact_engine.editing_limits(), &exact);

    let invalid = [
        (
            EditingLimits {
                max_operations_per_transaction: 0,
                ..EditingLimits::default()
            },
            "maxOperationsPerTransaction",
            0,
            HARD_MAX_OPERATIONS_PER_TRANSACTION,
        ),
        (
            EditingLimits {
                max_operations_per_transaction: HARD_MAX_OPERATIONS_PER_TRANSACTION + 1,
                ..EditingLimits::default()
            },
            "maxOperationsPerTransaction",
            HARD_MAX_OPERATIONS_PER_TRANSACTION + 1,
            HARD_MAX_OPERATIONS_PER_TRANSACTION,
        ),
        (
            EditingLimits {
                max_undo_groups: 0,
                ..EditingLimits::default()
            },
            "maxUndoGroups",
            0,
            HARD_MAX_UNDO_GROUPS,
        ),
        (
            EditingLimits {
                max_undo_groups: HARD_MAX_UNDO_GROUPS + 1,
                ..EditingLimits::default()
            },
            "maxUndoGroups",
            HARD_MAX_UNDO_GROUPS + 1,
            HARD_MAX_UNDO_GROUPS,
        ),
        (
            EditingLimits {
                max_undo_retained_units: 0,
                ..EditingLimits::default()
            },
            "maxUndoRetainedUnits",
            0,
            usize::try_from(HARD_MAX_UNDO_RETAINED_UNITS).unwrap(),
        ),
        (
            EditingLimits {
                max_undo_retained_units: HARD_MAX_UNDO_RETAINED_UNITS + 1,
                ..EditingLimits::default()
            },
            "maxUndoRetainedUnits",
            usize::try_from(HARD_MAX_UNDO_RETAINED_UNITS + 1).unwrap(),
            usize::try_from(HARD_MAX_UNDO_RETAINED_UNITS).unwrap(),
        ),
        (
            EditingLimits {
                max_derived_output_bytes: 0,
                ..EditingLimits::default()
            },
            "maxDerivedOutputBytes",
            0,
            HARD_MAX_DERIVED_OUTPUT_BYTES,
        ),
        (
            EditingLimits {
                max_derived_output_bytes: HARD_MAX_DERIVED_OUTPUT_BYTES + 1,
                ..EditingLimits::default()
            },
            "maxDerivedOutputBytes",
            HARD_MAX_DERIVED_OUTPUT_BYTES + 1,
            HARD_MAX_DERIVED_OUTPUT_BYTES,
        ),
    ];

    for (editing_limits, field, actual, limit) in invalid {
        let error = YrsDocumentEngine::new(YrsEngineConfig {
            schema: tiptap_schema(),
            fragment_name: "prosemirror".into(),
            initialization_mode: InitializationMode::LocalEmpty,
            resource_limits: ResourceLimits::default(),
            editing_limits,
            max_length: None,
            scope: None,
        })
        .err()
        .unwrap();
        assert_eq!(error.code, "INVALID_RESOURCE_LIMIT");
        assert_eq!(error.details, Some(json!({ "field": field })));
        assert_eq!(error.actual, Some(actual));
        assert_eq!(error.limit, Some(limit));
    }
}

#[test]
fn engine_config_revalidates_raw_resource_limits_before_creating_a_document() {
    let exact = ResourceLimits {
        max_input_bytes: 64 * 1024 * 1024,
        max_document_nodes: 1_000_000,
        max_document_depth: 1_024,
        max_schema_nodes: 10_000,
        max_schema_expression_bytes: 1024 * 1024,
        max_collaboration_message_bytes: 64 * 1024 * 1024,
        max_encoded_state_bytes: 256 * 1024 * 1024,
    };
    let exact_engine = YrsDocumentEngine::new(YrsEngineConfig {
        schema: tiptap_schema(),
        fragment_name: "prosemirror".into(),
        initialization_mode: InitializationMode::LocalEmpty,
        resource_limits: exact.clone(),
        editing_limits: EditingLimits::default(),
        max_length: None,
        scope: None,
    })
    .unwrap();
    assert_eq!(exact_engine.resource_limits(), &exact);

    let mut invalid = Vec::new();
    for (field, limit) in [
        ("maxInputBytes", 64 * 1024 * 1024),
        ("maxDocumentNodes", 1_000_000),
        ("maxDocumentDepth", 1_024),
        ("maxSchemaNodes", 10_000),
        ("maxSchemaExpressionBytes", 1024 * 1024),
        ("maxCollaborationMessageBytes", 64 * 1024 * 1024),
        ("maxEncodedStateBytes", 256 * 1024 * 1024),
    ] {
        for actual in [0, limit + 1] {
            let mut limits = exact.clone();
            match field {
                "maxInputBytes" => limits.max_input_bytes = actual,
                "maxDocumentNodes" => limits.max_document_nodes = actual,
                "maxDocumentDepth" => limits.max_document_depth = actual,
                "maxSchemaNodes" => limits.max_schema_nodes = actual,
                "maxSchemaExpressionBytes" => limits.max_schema_expression_bytes = actual,
                "maxCollaborationMessageBytes" => {
                    limits.max_collaboration_message_bytes = actual;
                }
                "maxEncodedStateBytes" => limits.max_encoded_state_bytes = actual,
                _ => unreachable!(),
            }
            invalid.push((limits, field, actual, limit));
        }
    }

    for (resource_limits, field, actual, limit) in invalid {
        let error = YrsDocumentEngine::new(YrsEngineConfig {
            schema: tiptap_schema(),
            fragment_name: "prosemirror".into(),
            initialization_mode: InitializationMode::LocalEmpty,
            resource_limits,
            editing_limits: EditingLimits::default(),
            max_length: None,
            scope: None,
        })
        .err()
        .unwrap();
        assert_eq!(error.code, "INVALID_RESOURCE_LIMIT");
        assert_eq!(error.details, Some(json!({ "field": field })));
        assert_eq!(error.actual, Some(actual));
        assert_eq!(error.limit, Some(limit));
    }
}

#[test]
fn operation_errors_have_only_the_nine_stable_codes() {
    let errors = [
        OperationError::engine_not_ready(41),
        OperationError::revision_mismatch(41, 7, 8),
        OperationError::position_invalid(41, 2, "at", "position is outside the document"),
        OperationError::transaction_invalid(41, "operations", "transaction is empty"),
        OperationError::operation_invalid(41, 2, "text", "text must not be empty"),
        OperationError::operation_limit_exceeded(
            41,
            Some(2),
            "maxOperationsPerTransaction",
            256,
            257,
        ),
        OperationError::document_invalid(41, Some(2), "content", "document is invalid"),
        OperationError::document_limit_exceeded(41, Some(2), "maxDerivedOutputBytes", 1024, 1025),
        OperationError::engine_invariant_failed(41, Some(2), "mutation plan was inconsistent"),
    ];

    assert_eq!(
        errors.map(|error| error.code),
        [
            "ENGINE_NOT_READY",
            "REVISION_MISMATCH",
            "POSITION_INVALID",
            "TRANSACTION_INVALID",
            "OPERATION_INVALID",
            "OPERATION_LIMIT_EXCEEDED",
            "DOCUMENT_INVALID",
            "DOCUMENT_LIMIT_EXCEEDED",
            "ENGINE_INVARIANT_FAILED",
        ]
    );
}

#[test]
fn operation_errors_serialize_camel_case_structured_details() {
    let error = OperationError::revision_mismatch(41, 7, 8);
    assert_eq!(
        serde_json::to_value(error).unwrap(),
        json!({
            "code": "REVISION_MISMATCH",
            "message": "document revision mismatch: expected 7, actual 8",
            "requestId": 41,
            "details": {
                "expectedRevision": "7",
                "actualRevision": "8",
            }
        })
    );

    let error = OperationError::operation_limit_exceeded(
        41,
        Some(2),
        "maxOperationsPerTransaction",
        256,
        257,
    );
    assert_eq!(
        serde_json::to_value(error).unwrap(),
        json!({
            "code": "OPERATION_LIMIT_EXCEEDED",
            "message": "maxOperationsPerTransaction exceeds limit 256: 257",
            "requestId": 41,
            "operationIndex": 2,
            "limit": 256,
            "actual": 257,
            "details": { "field": "maxOperationsPerTransaction" }
        })
    );
}

#[test]
fn typed_operation_contract_contains_every_approved_variant() {
    let at = RevisionedPosition {
        offset: 3,
        kind: EditorOffsetKind::Scalar,
        affinity: Affinity::After,
    };
    let other = RevisionedPosition {
        offset: 5,
        kind: EditorOffsetKind::Utf16,
        affinity: Affinity::Before,
    };
    let range = RevisionedRange {
        from: at,
        to: other,
    };
    let attrs = HashMap::from([("level".to_string(), json!(2))]);
    let mark = Mark::new("bold".into(), HashMap::new());
    let node = Node::void("hardBreak".into(), HashMap::new());

    let operations = vec![
        TypedOperation::InsertText {
            at,
            text: "hello".into(),
            marks: vec![mark.clone()],
        },
        TypedOperation::DeleteRange { range },
        TypedOperation::ReplaceRange {
            range,
            content: Fragment::from(vec![node.clone()]),
        },
        TypedOperation::AddMark {
            range,
            mark: mark.clone(),
        },
        TypedOperation::RemoveMark {
            range,
            mark_type: "bold".into(),
        },
        TypedOperation::ReplaceMark {
            range,
            mark: mark.clone(),
        },
        TypedOperation::SplitBlock {
            at,
            node_type: "paragraph".into(),
            attrs: attrs.clone(),
        },
        TypedOperation::JoinBlocks { at },
        TypedOperation::WrapInList {
            range,
            list_type: "bulletList".into(),
            item_type: "listItem".into(),
            attrs: attrs.clone(),
            item_attrs: attrs.clone(),
        },
        TypedOperation::UnwrapFromList { at },
        TypedOperation::IndentListItem { at },
        TypedOperation::OutdentListItem { at },
        TypedOperation::InsertNode { at, node },
        TypedOperation::UpdateNodeAttrs { at, attrs },
    ];

    let transaction = TypedTransaction {
        request_id: 41,
        base_document_revision: 7,
        origin: TransactionOrigin::LocalApi,
        operations,
        selection_intent: SelectionIntent::Set(SelectionInput::Text {
            anchor: at,
            head: other,
        }),
        history_policy: HistoryPolicy::Boundary,
    };
    assert_eq!(transaction.operations.len(), 14);

    let commit = TransactionCommit {
        request_id: transaction.request_id,
        changed: true,
        document_revision: 8,
        state_revision: 9,
        origin: transaction.origin,
    };
    assert_eq!(commit.document_revision, 8);
}
