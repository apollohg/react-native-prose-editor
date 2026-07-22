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

#[test]
fn incremental_and_fallback_position_updates_match_full_builds() {
    let mut engine = engine();
    import(
        &mut engine,
        serde_json::json!({
            "type": "doc",
            "content": [
                { "type": "paragraph", "content": [{ "type": "text", "text": "one" }] },
                { "type": "paragraph", "content": [{ "type": "text", "text": "two" }] }
            ]
        }),
    );
    assert_incremental_matches_full(&engine);

    let insert = transaction(
        &engine,
        8,
        vec![TypedOperation::InsertText {
            at: point(2, EditorOffsetKind::Scalar, Affinity::After),
            text: "😀".into(),
            marks: vec![],
        }],
        SelectionIntent::UseOperationResult,
    );
    engine.apply_typed_transaction(insert).unwrap();
    assert_incremental_matches_full(&engine);

    let add_mark = transaction(
        &engine,
        81,
        vec![TypedOperation::AddMark {
            range: RevisionedRange {
                from: point(0, EditorOffsetKind::Scalar, Affinity::Before),
                to: point(2, EditorOffsetKind::Scalar, Affinity::Before),
            },
            mark: Mark::new("bold".into(), Default::default()),
        }],
        SelectionIntent::Preserve,
    );
    engine.apply_typed_transaction(add_mark).unwrap();
    assert_incremental_matches_full(&engine);

    let delete = transaction(
        &engine,
        82,
        vec![TypedOperation::DeleteRange {
            range: RevisionedRange {
                from: point(1, EditorOffsetKind::Scalar, Affinity::Before),
                to: point(2, EditorOffsetKind::Scalar, Affinity::Before),
            },
        }],
        SelectionIntent::Preserve,
    );
    engine.apply_typed_transaction(delete).unwrap();
    assert_incremental_matches_full(&engine);

    let replace = transaction(
        &engine,
        83,
        vec![TypedOperation::ReplaceRange {
            range: RevisionedRange {
                from: point(0, EditorOffsetKind::Scalar, Affinity::Before),
                to: point(1, EditorOffsetKind::Scalar, Affinity::Before),
            },
            content: Fragment::from(vec![Node::text("X".into(), vec![])]),
        }],
        SelectionIntent::Preserve,
    );
    engine.apply_typed_transaction(replace).unwrap();
    assert_incremental_matches_full(&engine);

    let split = transaction(
        &engine,
        9,
        vec![TypedOperation::SplitBlock {
            at: point(2, EditorOffsetKind::Scalar, Affinity::After),
            node_type: "paragraph".into(),
            attrs: Default::default(),
        }],
        SelectionIntent::UseOperationResult,
    );
    engine.apply_typed_transaction(split).unwrap();
    assert_incremental_matches_full(&engine);

    let insert_node = transaction(
        &engine,
        84,
        vec![TypedOperation::InsertNode {
            at: point(1, EditorOffsetKind::Scalar, Affinity::Before),
            node: Node::void("hardBreak".into(), Default::default()),
        }],
        SelectionIntent::Preserve,
    );
    engine.apply_typed_transaction(insert_node).unwrap();
    assert_incremental_matches_full(&engine);
}

#[test]
fn operation_result_selection_is_total_at_text_end_and_for_ranges() {
    let mut engine = engine();
    import(
        &mut engine,
        serde_json::json!({"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"A😀B"}]}]}),
    );
    let insert = transaction(
        &engine,
        20,
        vec![TypedOperation::InsertText {
            at: point(3, EditorOffsetKind::Scalar, Affinity::After),
            text: "!".into(),
            marks: vec![],
        }],
        SelectionIntent::UseOperationResult,
    );
    engine.apply_typed_transaction(insert).unwrap();
    let ResolvedSelection::Text { anchor, head } = engine.resolved_selection().unwrap() else {
        panic!("expected cursor")
    };
    assert_eq!((anchor.scalar, anchor.utf16), (4, 5));
    assert_eq!(anchor, head);

    let mark = transaction(
        &engine,
        21,
        vec![TypedOperation::AddMark {
            range: RevisionedRange {
                from: point(0, EditorOffsetKind::Scalar, Affinity::After),
                to: point(4, EditorOffsetKind::Scalar, Affinity::After),
            },
            mark: Mark::new("bold".into(), Default::default()),
        }],
        SelectionIntent::UseOperationResult,
    );
    engine.apply_typed_transaction(mark).unwrap();
    let ResolvedSelection::Text { anchor, head } = engine.resolved_selection().unwrap() else {
        panic!("expected range")
    };
    assert_eq!((anchor.scalar, head.scalar), (0, 4));
    assert_incremental_matches_full(&engine);

    let encoded = engine.encoded_state().unwrap();
    let revision = engine.revision();
    let state_revision = engine.state_revision();
    let selection_only = transaction(
        &engine,
        211,
        vec![TypedOperation::DeleteRange {
            range: RevisionedRange {
                from: point(1, EditorOffsetKind::Scalar, Affinity::Before),
                to: point(1, EditorOffsetKind::Scalar, Affinity::Before),
            },
        }],
        SelectionIntent::UseOperationResult,
    );
    let commit = engine.apply_typed_transaction(selection_only).unwrap();
    assert!(commit.changed);
    assert_eq!(engine.encoded_state().unwrap(), encoded);
    assert_eq!(engine.revision(), revision);
    assert_eq!(engine.state_revision(), state_revision + 1);

    import(
        &mut engine,
        serde_json::json!({
            "type": "doc",
            "content": [{
                "type": "image",
                "attrs": {
                    "src": "asset://same",
                    "alt": null,
                    "title": null,
                    "width": null,
                    "height": null
                }
            }]
        }),
    );
    let select_all = transaction(
        &engine,
        212,
        vec![],
        SelectionIntent::Set(SelectionInput::All),
    );
    engine.apply_typed_transaction(select_all).unwrap();
    let encoded = engine.encoded_state().unwrap();
    let revision = engine.revision();
    let state_revision = engine.state_revision();
    let update_node = transaction(
        &engine,
        213,
        vec![TypedOperation::UpdateNodeAttrs {
            at: point(0, EditorOffsetKind::Scalar, Affinity::Before),
            attrs: serde_json::from_value(serde_json::json!({
                "src": "asset://same",
                "alt": null,
                "title": null,
                "width": null,
                "height": null
            }))
            .unwrap(),
        }],
        SelectionIntent::UseOperationResult,
    );
    let commit = engine.apply_typed_transaction(update_node).unwrap();
    assert!(commit.changed);
    assert_eq!(engine.encoded_state().unwrap(), encoded);
    assert_eq!(engine.revision(), revision);
    assert_eq!(engine.state_revision(), state_revision + 1);
    assert!(matches!(
        engine.resolved_selection(),
        Some(ResolvedSelection::Node { .. })
    ));
}

#[test]
fn update_node_attrs_operation_result_selects_only_void_nodes() {
    let mut container = engine();
    import(
        &mut container,
        serde_json::json!({
            "type": "doc",
            "content": [{
                "type": "orderedList",
                "attrs": { "start": 1 },
                "content": [{
                    "type": "listItem",
                    "content": [{
                        "type": "paragraph",
                        "content": [{ "type": "text", "text": "one" }]
                    }]
                }]
            }]
        }),
    );
    let select_all = transaction(
        &container,
        214,
        vec![],
        SelectionIntent::Set(SelectionInput::All),
    );
    container.apply_typed_transaction(select_all).unwrap();
    let unchanged_container = transaction(
        &container,
        215,
        vec![TypedOperation::UpdateNodeAttrs {
            at: point(0, EditorOffsetKind::Scalar, Affinity::Before),
            attrs: serde_json::from_value(serde_json::json!({ "start": 1 })).unwrap(),
        }],
        SelectionIntent::UseOperationResult,
    );
    let commit = container
        .apply_typed_transaction(unchanged_container)
        .unwrap();
    assert!(!commit.changed);
    assert_eq!(
        container.resolved_selection(),
        Some(&ResolvedSelection::All)
    );

    let update_container = transaction(
        &container,
        217,
        vec![TypedOperation::UpdateNodeAttrs {
            at: point(0, EditorOffsetKind::Scalar, Affinity::Before),
            attrs: serde_json::from_value(serde_json::json!({ "start": 2 })).unwrap(),
        }],
        SelectionIntent::UseOperationResult,
    );
    let commit = container.apply_typed_transaction(update_container).unwrap();
    assert!(commit.changed);
    assert_eq!(
        container.resolved_selection(),
        Some(&ResolvedSelection::All)
    );
}

#[test]
fn nonvoid_attrs_operation_result_maps_selection_through_prior_edits() {
    let mut engine = engine();
    import(
        &mut engine,
        serde_json::json!({
            "type": "doc",
            "content": [
                {
                    "type": "bulletList",
                    "content": [{
                        "type": "listItem",
                        "content": [{
                            "type": "paragraph",
                            "content": [{ "type": "text", "text": "one" }]
                        }]
                    }]
                },
                {
                    "type": "orderedList",
                    "attrs": { "start": 1 },
                    "content": [{
                        "type": "listItem",
                        "content": [{
                            "type": "paragraph",
                            "content": [{ "type": "text", "text": "two" }]
                        }]
                    }]
                }
            ]
        }),
    );
    let total_scalars = engine.position_map().unwrap().total_scalars();
    let set_cursor = transaction(
        &engine,
        218,
        vec![],
        SelectionIntent::Set(SelectionInput::Text {
            anchor: point(3, EditorOffsetKind::Scalar, Affinity::Before),
            head: point(3, EditorOffsetKind::Scalar, Affinity::Before),
        }),
    );
    engine.apply_typed_transaction(set_cursor).unwrap();
    let crate::yrs_engine::RelativeSelection::Text {
        anchor: relative_before,
        head: relative_before_head,
    } = engine.relative_selection().unwrap().clone()
    else {
        panic!("expected initial relative text cursor")
    };
    assert_eq!(relative_before, relative_before_head);

    let edit_then_attrs = transaction(
        &engine,
        219,
        vec![
            TypedOperation::UnwrapFromList {
                at: point(3, EditorOffsetKind::Scalar, Affinity::Before),
            },
            TypedOperation::UpdateNodeAttrs {
                at: point(
                    total_scalars - 1,
                    EditorOffsetKind::Scalar,
                    Affinity::Before,
                ),
                attrs: serde_json::from_value(serde_json::json!({ "start": 2 })).unwrap(),
            },
        ],
        SelectionIntent::UseOperationResult,
    );
    engine.apply_typed_transaction(edit_then_attrs).unwrap();
    let crate::yrs_engine::RelativeSelection::Text {
        anchor: relative_after,
        head: relative_after_head,
    } = engine.relative_selection().unwrap().clone()
    else {
        panic!("expected rematerialized relative text cursor")
    };
    assert_eq!(relative_after, relative_after_head);
    assert_eq!(relative_after.affinity, Affinity::Before);
    assert_ne!(
        relative_after.sticky, relative_before.sticky,
        "unwrap must delete the selected branch and exercise the mapped fallback"
    );
    let ResolvedSelection::Text { anchor, head } = engine.resolved_selection().unwrap() else {
        panic!("expected mapped text cursor")
    };
    assert_eq!(anchor, head);
    assert_eq!((anchor.document, anchor.scalar, anchor.utf16), (2, 1, 1));
}

#[test]
fn envelope_errors_precede_unrepresentable_explicit_selection() {
    let mut engine = engine();
    import(
        &mut engine,
        serde_json::json!({"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"x"}]}]}),
    );
    let before = engine.encoded_state().unwrap();
    let mut stale = transaction(
        &engine,
        22,
        vec![],
        SelectionIntent::Set(SelectionInput::Text {
            anchor: point(u32::MAX, EditorOffsetKind::Utf16, Affinity::After),
            head: point(u32::MAX, EditorOffsetKind::Utf16, Affinity::After),
        }),
    );
    stale.base_document_revision += 1;
    assert_eq!(
        engine.apply_typed_transaction(stale).unwrap_err().code,
        "REVISION_MISMATCH"
    );
    assert_eq!(engine.encoded_state().unwrap(), before);

    let mut invalid_origin = transaction(
        &engine,
        23,
        vec![],
        SelectionIntent::Set(SelectionInput::Text {
            anchor: point(u32::MAX, EditorOffsetKind::Utf16, Affinity::After),
            head: point(u32::MAX, EditorOffsetKind::Utf16, Affinity::After),
        }),
    );
    invalid_origin.origin = TransactionOrigin::RemoteSync;
    assert_eq!(
        engine
            .apply_typed_transaction(invalid_origin)
            .unwrap_err()
            .code,
        "TRANSACTION_INVALID"
    );
    assert_eq!(engine.encoded_state().unwrap(), before);
}

#[test]
fn awaiting_restore_and_import_lifecycle_install_or_preserve_derived_state() {
    let mut source = scoped_engine(InitializationMode::LocalEmpty);
    import(
        &mut source,
        serde_json::json!({"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"remote"}]}]}),
    );
    let snapshot = source.export_snapshot().unwrap();

    let mut target = scoped_engine(InitializationMode::AwaitRemote);
    assert!(!target.is_ready());
    assert!(target.position_map().is_none());
    assert!(target.relative_selection().is_none());
    assert!(target.resolved_selection().is_none());

    let mut direct_import = scoped_engine(InitializationMode::AwaitRemote);
    import(
        &mut direct_import,
        serde_json::json!({"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"direct"}]}]}),
    );
    assert!(direct_import.is_ready());
    assert!(direct_import.resolved_selection().is_some());

    target.restore_snapshot(&snapshot).unwrap();
    assert!(target.is_ready());
    assert!(target.resolved_selection().is_some());
    assert_incremental_matches_full(&target);
    let restore_revision = target.revision();
    let restore_state_revision = target.state_revision();

    let select = transaction(
        &target,
        30,
        vec![],
        SelectionIntent::Set(SelectionInput::Text {
            anchor: point(2, EditorOffsetKind::Scalar, Affinity::Before),
            head: point(2, EditorOffsetKind::Scalar, Affinity::Before),
        }),
    );
    target.apply_typed_transaction(select).unwrap();
    let selected = target.relative_selection().cloned();
    let selected_state_revision = target.state_revision();

    let unchanged_snapshot = target.restore_snapshot(&snapshot).unwrap();
    assert!(!unchanged_snapshot.changed);
    assert_eq!(target.relative_selection().cloned(), selected);
    assert_eq!(target.revision(), restore_revision);
    assert_eq!(target.state_revision(), selected_state_revision);

    let current_json = target.document_json().unwrap().to_string();
    let unchanged_import = target
        .import_json(&current_json, TransactionOrigin::DocumentImport)
        .unwrap();
    assert!(!unchanged_import.changed);
    assert_eq!(target.relative_selection().cloned(), selected);
    assert_eq!(target.state_revision(), selected_state_revision);

    import(
        &mut target,
        serde_json::json!({"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"replacement"}]}]}),
    );
    assert_eq!(target.revision(), restore_revision + 1);
    assert_eq!(target.state_revision(), selected_state_revision + 1);
    assert_ne!(target.relative_selection().cloned(), selected);
    assert!(target.state_revision() > restore_state_revision);
    assert_incremental_matches_full(&target);
}
