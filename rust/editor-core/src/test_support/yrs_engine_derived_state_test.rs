use crate::boundary::ResourceLimits;
use crate::model::{Fragment, Mark, Node};
use crate::position::PositionMap;
use crate::tiptap_schema;
use crate::yrs_engine::{
    Affinity, DocumentScope, EditingLimits, EditorOffsetKind, HistoryPolicy, InitializationMode,
    ResolvedPoint, ResolvedSelection, RevisionedPosition, RevisionedRange, SelectionInput,
    SelectionIntent, TransactionOrigin, TypedOperation, TypedTransaction, YrsDocumentEngine,
    YrsEngineConfig,
};

fn engine() -> YrsDocumentEngine {
    YrsDocumentEngine::new(YrsEngineConfig {
        schema: tiptap_schema(),
        fragment_name: "prosemirror".into(),
        initialization_mode: InitializationMode::LocalEmpty,
        resource_limits: ResourceLimits::default(),
        editing_limits: EditingLimits::default(),
        max_length: None,
        scope: None,
    })
    .unwrap()
}

fn scoped_engine(mode: InitializationMode) -> YrsDocumentEngine {
    YrsDocumentEngine::new(YrsEngineConfig {
        schema: tiptap_schema(),
        fragment_name: "prosemirror".into(),
        initialization_mode: mode,
        resource_limits: ResourceLimits::default(),
        editing_limits: EditingLimits::default(),
        max_length: None,
        scope: Some(DocumentScope {
            document_id: "derived-doc".into(),
            lineage_id: "derived-lineage".into(),
        }),
    })
    .unwrap()
}

fn point(offset: u32, kind: EditorOffsetKind, affinity: Affinity) -> RevisionedPosition {
    RevisionedPosition {
        offset,
        kind,
        affinity,
    }
}

fn transaction(
    engine: &YrsDocumentEngine,
    request_id: u64,
    operations: Vec<TypedOperation>,
    selection_intent: SelectionIntent,
) -> TypedTransaction {
    TypedTransaction {
        request_id,
        base_document_revision: engine.revision(),
        origin: TransactionOrigin::LocalApi,
        operations,
        selection_intent,
        history_policy: HistoryPolicy::Skip,
    }
}

fn import(engine: &mut YrsDocumentEngine, value: serde_json::Value) {
    engine
        .import_json(&value.to_string(), TransactionOrigin::DocumentImport)
        .unwrap();
}

fn assert_incremental_matches_full(engine: &YrsDocumentEngine) {
    let document = engine.document().unwrap();
    let incremental = engine.position_map().unwrap();
    let full = PositionMap::build(document, &tiptap_schema());
    assert_eq!(incremental.block_count(), full.block_count());
    assert_eq!(incremental.total_scalars(), full.total_scalars());
    for index in 0..full.block_count() {
        let incremental_block = incremental.block(index).unwrap();
        let full_block = full.block(index).unwrap();
        assert_eq!(
            incremental_block.doc_start, full_block.doc_start,
            "block {index} doc_start"
        );
        assert_eq!(
            incremental_block.doc_end, full_block.doc_end,
            "block {index} doc_end"
        );
        assert_eq!(
            incremental_block.scalar_start, full_block.scalar_start,
            "block {index} scalar_start"
        );
        assert_eq!(
            incremental_block.scalar_len, full_block.scalar_len,
            "block {index} scalar_len"
        );
        assert_eq!(
            incremental_block.scalar_prefix_len, full_block.scalar_prefix_len,
            "block {index} scalar_prefix_len"
        );
        assert_eq!(
            incremental_block.rendered_break_after, full_block.rendered_break_after,
            "block {index} rendered_break_after"
        );
        assert_eq!(
            incremental_block.node_path, full_block.node_path,
            "block {index} node_path"
        );
        assert_eq!(
            incremental_block.is_void_block, full_block.is_void_block,
            "block {index} is_void_block"
        );
    }
    for offset in 0..=incremental.total_scalars() {
        assert_eq!(
            incremental.scalar_to_doc(offset, document),
            full.scalar_to_doc(offset, document),
            "scalar offset {offset}"
        );
    }
    for position in 0..=document.content_size() {
        assert_eq!(
            incremental.doc_to_scalar(position, document),
            full.doc_to_scalar(position, document),
            "document position {position}"
        );
    }
}

#[test]
fn local_empty_initializes_a_normalized_first_cursor() {
    let engine = engine();
    assert_eq!(
        engine.resolved_selection(),
        Some(&ResolvedSelection::Text {
            anchor: ResolvedPoint {
                document: 1,
                scalar: 1,
                utf16: 1,
            },
            head: ResolvedPoint {
                document: 1,
                scalar: 1,
                utf16: 1,
            },
        })
    );
    assert!(engine.relative_selection().is_some());
    assert_incremental_matches_full(&engine);
}

#[test]
fn selection_only_text_ranges_resolve_forward_backward_scalar_and_utf16() {
    let mut engine = engine();
    import(
        &mut engine,
        serde_json::json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{ "type": "text", "text": "A😀B" }]
            }]
        }),
    );
    let encoded = engine.encoded_state().unwrap();
    let document_revision = engine.revision();
    let state_revision = engine.state_revision();

    let forward = transaction(
        &engine,
        1,
        vec![],
        SelectionIntent::Set(SelectionInput::Text {
            anchor: point(1, EditorOffsetKind::Scalar, Affinity::Before),
            head: point(3, EditorOffsetKind::Utf16, Affinity::After),
        }),
    );
    let commit = engine.apply_typed_transaction(forward).unwrap();
    assert!(commit.changed);
    assert_eq!(commit.document_revision, document_revision);
    assert_eq!(commit.state_revision, state_revision + 1);
    assert_eq!(engine.encoded_state().unwrap(), encoded);
    assert_eq!(
        engine.resolved_selection(),
        Some(&ResolvedSelection::Text {
            anchor: ResolvedPoint {
                document: 2,
                scalar: 1,
                utf16: 1,
            },
            head: ResolvedPoint {
                document: 3,
                scalar: 2,
                utf16: 3,
            },
        })
    );

    let backward = transaction(
        &engine,
        2,
        vec![],
        SelectionIntent::Set(SelectionInput::Text {
            anchor: point(3, EditorOffsetKind::Utf16, Affinity::After),
            head: point(1, EditorOffsetKind::Scalar, Affinity::Before),
        }),
    );
    let commit = engine.apply_typed_transaction(backward).unwrap();
    assert!(commit.changed);
    assert_eq!(commit.document_revision, document_revision);
    assert_eq!(commit.state_revision, state_revision + 2);
    assert_eq!(engine.encoded_state().unwrap(), encoded);
    let ResolvedSelection::Text { anchor, head } = engine.resolved_selection().unwrap() else {
        panic!("expected text selection")
    };
    assert_eq!((anchor.document, head.document), (3, 2));

    let no_op = transaction(
        &engine,
        3,
        vec![],
        SelectionIntent::Set(SelectionInput::Text {
            anchor: point(3, EditorOffsetKind::Utf16, Affinity::After),
            head: point(1, EditorOffsetKind::Scalar, Affinity::Before),
        }),
    );
    let commit = engine.apply_typed_transaction(no_op).unwrap();
    assert!(!commit.changed);
    assert_eq!(commit.state_revision, state_revision + 2);
}

#[test]
fn node_all_empty_and_void_selections_are_derived_without_writing_yrs() {
    let mut engine = engine();
    import(
        &mut engine,
        serde_json::json!({
            "type": "doc",
            "content": [
                { "type": "paragraph" },
                { "type": "horizontalRule" },
                { "type": "paragraph", "content": [{ "type": "text", "text": "x" }] }
            ]
        }),
    );
    let encoded = engine.encoded_state().unwrap();
    let revision = engine.revision();

    let node = transaction(
        &engine,
        4,
        vec![],
        SelectionIntent::Set(SelectionInput::Node {
            at: point(2, EditorOffsetKind::Scalar, Affinity::After),
        }),
    );
    assert!(engine.apply_typed_transaction(node).unwrap().changed);
    let ResolvedSelection::Node { at } = engine.resolved_selection().unwrap() else {
        panic!("expected node selection")
    };
    assert_eq!(at.scalar, 2);

    let all = transaction(
        &engine,
        5,
        vec![],
        SelectionIntent::Set(SelectionInput::All),
    );
    assert!(engine.apply_typed_transaction(all).unwrap().changed);
    assert_eq!(engine.resolved_selection(), Some(&ResolvedSelection::All));
    assert_eq!(engine.encoded_state().unwrap(), encoded);
    assert_eq!(engine.revision(), revision);

    let invalid_text_node = transaction(
        &engine,
        51,
        vec![],
        SelectionIntent::Set(SelectionInput::Node {
            at: point(4, EditorOffsetKind::Scalar, Affinity::Before),
        }),
    );
    assert_eq!(
        engine
            .apply_typed_transaction(invalid_text_node)
            .unwrap_err()
            .code,
        "POSITION_INVALID"
    );
}

#[test]
fn initialization_prefers_textblocks_and_falls_back_to_a_void_node() {
    let mut leading_void = engine();
    import(
        &mut leading_void,
        serde_json::json!({"type":"doc","content":[{"type":"horizontalRule"},{"type":"paragraph","content":[{"type":"text","text":"x"}]}]}),
    );
    let ResolvedSelection::Text { anchor, head } = leading_void.resolved_selection().unwrap()
    else {
        panic!("expected text cursor after leading void")
    };
    assert_eq!((anchor.document, head.document), (2, 2));

    let mut only_void = engine();
    import(
        &mut only_void,
        serde_json::json!({"type":"doc","content":[{"type":"horizontalRule"}]}),
    );
    assert!(matches!(
        only_void.resolved_selection(),
        Some(ResolvedSelection::Node { .. })
    ));

    let mut inline_void = engine();
    import(
        &mut inline_void,
        serde_json::json!({"type":"doc","content":[{"type":"paragraph","content":[{"type":"hardBreak"}]}]}),
    );
    assert!(matches!(
        inline_void.resolved_selection(),
        Some(ResolvedSelection::Text { .. })
    ));
}

#[test]
fn structural_set_fallback_keeps_exact_affinity() {
    let mut engine = engine();
    import(
        &mut engine,
        serde_json::json!({"type":"doc","content":[{"type":"bulletList","content":[{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"one"}]}]}]},{"type":"paragraph","content":[{"type":"text","text":"two"}]}]}),
    );
    let unwrap = transaction(
        &engine,
        61,
        vec![TypedOperation::UnwrapFromList {
            at: point(1, EditorOffsetKind::Scalar, Affinity::Before),
        }],
        SelectionIntent::Set(SelectionInput::Text {
            anchor: point(1, EditorOffsetKind::Scalar, Affinity::Before),
            head: point(1, EditorOffsetKind::Scalar, Affinity::Before),
        }),
    );
    engine.apply_typed_transaction(unwrap).unwrap();
    let crate::yrs_engine::RelativeSelection::Text { anchor, head } =
        engine.relative_selection().unwrap()
    else {
        panic!("expected text selection")
    };
    assert_eq!(anchor.affinity, Affinity::Before);
    assert_eq!(head.affinity, Affinity::Before);
}

#[test]
fn inline_delete_keeps_exact_set_affinity_on_the_surviving_text_branch() {
    let mut engine = engine();
    import(
        &mut engine,
        serde_json::json!({"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}),
    );
    let revision = engine.revision();
    let state_revision = engine.state_revision();
    let delete_and_set = transaction(
        &engine,
        62,
        vec![TypedOperation::DeleteRange {
            range: RevisionedRange {
                from: point(1, EditorOffsetKind::Scalar, Affinity::Before),
                to: point(3, EditorOffsetKind::Scalar, Affinity::Before),
            },
        }],
        SelectionIntent::Set(SelectionInput::Text {
            anchor: point(2, EditorOffsetKind::Scalar, Affinity::After),
            head: point(2, EditorOffsetKind::Scalar, Affinity::After),
        }),
    );
    let commit = engine.apply_typed_transaction(delete_and_set).unwrap();
    assert!(commit.changed);
    assert_eq!(commit.document_revision, revision + 1);
    assert_eq!(commit.state_revision, state_revision + 1);
    let crate::yrs_engine::RelativeSelection::Text { anchor, head } =
        engine.relative_selection().unwrap()
    else {
        panic!("expected text selection")
    };
    assert_eq!(anchor.affinity, Affinity::After);
    assert_eq!(head.affinity, Affinity::After);
    assert_eq!(anchor.sticky, head.sticky);
    let ResolvedSelection::Text { anchor, head } = engine.resolved_selection().unwrap() else {
        panic!("expected resolved text selection")
    };
    assert_eq!(anchor, head);
    assert_eq!(anchor.scalar, 1);
}

#[test]
fn exact_after_selection_crosses_a_fragmented_live_text_boundary() {
    let mut engine = engine();
    import(
        &mut engine,
        serde_json::json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [
                    { "type": "text", "text": "a", "marks": [{ "type": "bold" }] },
                    { "type": "text", "text": "b", "marks": [{ "type": "italic" }] }
                ]
            }]
        }),
    );
    let remove_marks = transaction(
        &engine,
        63,
        vec![
            TypedOperation::RemoveMark {
                range: RevisionedRange {
                    from: point(0, EditorOffsetKind::Scalar, Affinity::Before),
                    to: point(1, EditorOffsetKind::Scalar, Affinity::Before),
                },
                mark_type: "bold".into(),
            },
            TypedOperation::RemoveMark {
                range: RevisionedRange {
                    from: point(1, EditorOffsetKind::Scalar, Affinity::Before),
                    to: point(2, EditorOffsetKind::Scalar, Affinity::Before),
                },
                mark_type: "italic".into(),
            },
        ],
        SelectionIntent::Preserve,
    );
    engine.apply_typed_transaction(remove_marks).unwrap();

    let set_boundary = transaction(
        &engine,
        64,
        vec![],
        SelectionIntent::Set(SelectionInput::Text {
            anchor: point(1, EditorOffsetKind::Scalar, Affinity::After),
            head: point(1, EditorOffsetKind::Scalar, Affinity::After),
        }),
    );
    engine.apply_typed_transaction(set_boundary).unwrap();
    let crate::yrs_engine::RelativeSelection::Text { anchor, head } =
        engine.relative_selection().unwrap()
    else {
        panic!("expected text selection")
    };
    assert_eq!(anchor.affinity, Affinity::After);
    assert_eq!(head.affinity, Affinity::After);
    let ResolvedSelection::Text { anchor, head } = engine.resolved_selection().unwrap() else {
        panic!("expected resolved text selection")
    };
    assert_eq!((anchor.scalar, head.scalar), (1, 1));
}

#[test]
fn changing_only_affinity_is_state_and_repeating_it_is_a_no_op() {
    let mut engine = engine();
    import(
        &mut engine,
        serde_json::json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{ "type": "text", "text": "abc" }]
            }]
        }),
    );
    let encoded = engine.encoded_state().unwrap();
    let document_revision = engine.revision();

    let set = |engine: &YrsDocumentEngine, request_id, affinity| {
        transaction(
            engine,
            request_id,
            vec![],
            SelectionIntent::Set(SelectionInput::Text {
                anchor: point(1, EditorOffsetKind::Scalar, affinity),
                head: point(1, EditorOffsetKind::Scalar, affinity),
            }),
        )
    };
    engine
        .apply_typed_transaction(set(&engine, 10, Affinity::Before))
        .unwrap();
    let resolved = engine.resolved_selection().cloned();
    let before_relative = engine.relative_selection().cloned();
    let state_revision = engine.state_revision();

    let changed = engine
        .apply_typed_transaction(set(&engine, 11, Affinity::After))
        .unwrap();
    assert!(changed.changed);
    assert_eq!(engine.resolved_selection().cloned(), resolved);
    assert_ne!(engine.relative_selection().cloned(), before_relative);
    assert_eq!(engine.state_revision(), state_revision + 1);
    assert_eq!(engine.revision(), document_revision);
    assert_eq!(engine.encoded_state().unwrap(), encoded);

    let repeated = engine
        .apply_typed_transaction(set(&engine, 12, Affinity::After))
        .unwrap();
    assert!(!repeated.changed);
    assert_eq!(engine.state_revision(), state_revision + 1);
}

#[test]
fn deleted_relative_anchor_resolves_deterministically_after_edit() {
    let mut engine = engine();
    import(
        &mut engine,
        serde_json::json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{ "type": "text", "text": "abcdef" }]
            }]
        }),
    );
    let set = transaction(
        &engine,
        6,
        vec![],
        SelectionIntent::Set(SelectionInput::Text {
            anchor: point(3, EditorOffsetKind::Scalar, Affinity::Before),
            head: point(3, EditorOffsetKind::Scalar, Affinity::After),
        }),
    );
    engine.apply_typed_transaction(set).unwrap();

    let delete = transaction(
        &engine,
        7,
        vec![TypedOperation::DeleteRange {
            range: RevisionedRange {
                from: point(1, EditorOffsetKind::Scalar, Affinity::Before),
                to: point(5, EditorOffsetKind::Scalar, Affinity::After),
            },
        }],
        SelectionIntent::Preserve,
    );
    engine.apply_typed_transaction(delete).unwrap();
    let ResolvedSelection::Text { anchor, head } = engine.resolved_selection().unwrap() else {
        panic!("expected text selection")
    };
    assert_eq!((anchor.scalar, head.scalar), (1, 1));
    assert_incremental_matches_full(&engine);
}

#[test]
fn preserved_node_selection_normalizes_after_its_atom_is_deleted() {
    let mut engine = engine();
    import(
        &mut engine,
        serde_json::json!({
            "type": "doc",
            "content": [
                { "type": "horizontalRule" },
                { "type": "paragraph", "content": [{ "type": "text", "text": "x" }] }
            ]
        }),
    );
    let select_node = transaction(
        &engine,
        70,
        vec![],
        SelectionIntent::Set(SelectionInput::Node {
            at: point(0, EditorOffsetKind::Scalar, Affinity::Before),
        }),
    );
    engine.apply_typed_transaction(select_node).unwrap();
    let node_relative = engine.relative_selection().cloned();

    let unrelated_insert = transaction(
        &engine,
        71,
        vec![TypedOperation::InsertText {
            at: point(2, EditorOffsetKind::Scalar, Affinity::Before),
            text: "y".into(),
            marks: vec![],
        }],
        SelectionIntent::Preserve,
    );
    engine.apply_typed_transaction(unrelated_insert).unwrap();
    assert!(matches!(
        engine.resolved_selection(),
        Some(ResolvedSelection::Node { .. })
    ));
    assert_eq!(engine.relative_selection().cloned(), node_relative);

    let delete_atom = transaction(
        &engine,
        72,
        vec![TypedOperation::DeleteRange {
            range: RevisionedRange {
                from: point(0, EditorOffsetKind::Scalar, Affinity::Before),
                to: point(1, EditorOffsetKind::Scalar, Affinity::After),
            },
        }],
        SelectionIntent::Preserve,
    );
    engine.apply_typed_transaction(delete_atom).unwrap();
    assert!(matches!(
        engine.relative_selection(),
        Some(crate::yrs_engine::RelativeSelection::Text { .. })
    ));
    let ResolvedSelection::Text { anchor, head } = engine.resolved_selection().unwrap() else {
        panic!("deleted node selection must normalize to a text cursor")
    };
    assert_eq!(anchor, head);
}

include!("yrs_engine_derived_state_test/incremental_and_results.rs");
