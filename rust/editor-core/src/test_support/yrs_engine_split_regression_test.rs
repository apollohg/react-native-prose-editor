use crate::boundary::ResourceLimits;
use crate::tiptap_schema;
use crate::yrs_engine::{
    Affinity, EditingLimits, EditorOffsetKind, HistoryPolicy, InitializationMode,
    ResolvedSelection, RevisionedPosition, RevisionedRange, SelectionInput, SelectionIntent,
    TransactionOrigin, TypedCommand, TypedOperation, TypedTransaction, TypedTransactionResult,
    YrsDocumentEngine, YrsEngineConfig,
};
use yrs::updates::decoder::Decode;
use yrs::{Doc, OffsetKind, Options, ReadTxn, StateVector, Transact, Update};

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

fn point(offset: u32, affinity: Affinity) -> RevisionedPosition {
    RevisionedPosition {
        offset,
        kind: EditorOffsetKind::Scalar,
        affinity,
    }
}

fn apply(engine: &mut YrsDocumentEngine, request_id: u64, operations: Vec<TypedOperation>) {
    engine
        .apply_typed_transaction(TypedTransaction {
            request_id,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalApi,
            operations,
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Skip,
        })
        .unwrap();
}

fn import_marked_text(engine: &mut YrsDocumentEngine) {
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"ab","marks":[{"type":"bold"}]}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
}

#[test]
fn split_at_start_after_deleting_marked_prefix() {
    let mut engine = engine();
    import_marked_text(&mut engine);

    apply(
        &mut engine,
        1,
        vec![TypedOperation::DeleteRange {
            range: RevisionedRange {
                from: point(0, Affinity::After),
                to: point(1, Affinity::After),
            },
        }],
    );
    apply(
        &mut engine,
        2,
        vec![TypedOperation::SplitBlock {
            at: point(0, Affinity::After),
            node_type: "paragraph".into(),
            attrs: Default::default(),
        }],
    );

    assert_eq!(
        engine.document_json(),
        Some(serde_json::json!({
            "type": "doc",
            "content": [
                {"type": "paragraph"},
                {"type": "paragraph", "content": [
                    {"type": "text", "text": "b", "marks": [{"type": "bold"}]}
                ]}
            ]
        }))
    );
}

#[test]
fn split_at_start_of_untouched_text() {
    let mut engine = engine();
    import_marked_text(&mut engine);

    apply(
        &mut engine,
        1,
        vec![TypedOperation::SplitBlock {
            at: point(0, Affinity::After),
            node_type: "paragraph".into(),
            attrs: Default::default(),
        }],
    );

    assert_eq!(
        engine.document_json(),
        Some(serde_json::json!({
            "type": "doc",
            "content": [
                {"type": "paragraph"},
                {"type": "paragraph", "content": [
                    {"type": "text", "text": "ab", "marks": [{"type": "bold"}]}
                ]}
            ]
        }))
    );
}

#[test]
fn split_at_start_then_insert_can_target_either_side() {
    for (affinity, expected) in [
        (
            Affinity::Before,
            serde_json::json!({
                "type": "doc",
                "content": [
                    {"type": "paragraph", "content": [{"type": "text", "text": "x"}]},
                    {"type": "paragraph", "content": [
                        {"type": "text", "text": "ab", "marks": [{"type": "bold"}]}
                    ]}
                ]
            }),
        ),
        (
            Affinity::After,
            serde_json::json!({
                "type": "doc",
                "content": [
                    {"type": "paragraph"},
                    {"type": "paragraph", "content": [
                        {"type": "text", "text": "x"},
                        {"type": "text", "text": "ab", "marks": [{"type": "bold"}]}
                    ]}
                ]
            }),
        ),
    ] {
        let mut engine = engine();
        import_marked_text(&mut engine);
        apply(
            &mut engine,
            1,
            vec![
                TypedOperation::SplitBlock {
                    at: point(0, Affinity::After),
                    node_type: "paragraph".into(),
                    attrs: Default::default(),
                },
                TypedOperation::InsertText {
                    at: point(0, affinity),
                    text: "x".into(),
                    marks: vec![],
                },
            ],
        );
        assert_eq!(engine.document_json(), Some(expected));
    }
}

#[test]
fn delete_then_split_at_start_in_one_envelope() {
    let mut engine = engine();
    import_marked_text(&mut engine);
    apply(
        &mut engine,
        1,
        vec![
            TypedOperation::DeleteRange {
                range: RevisionedRange {
                    from: point(0, Affinity::After),
                    to: point(1, Affinity::After),
                },
            },
            TypedOperation::SplitBlock {
                at: point(0, Affinity::After),
                node_type: "paragraph".into(),
                attrs: Default::default(),
            },
        ],
    );
    assert_eq!(
        engine.document_json(),
        Some(serde_json::json!({
            "type": "doc",
            "content": [
                {"type": "paragraph"},
                {"type": "paragraph", "content": [
                    {"type": "text", "text": "b", "marks": [{"type": "bold"}]}
                ]}
            ]
        }))
    );
}

#[test]
fn split_before_first_text_after_atom_keeps_a_left_insertion_gap() {
    let mut engine = engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"hardBreak"},{"type":"text","text":"b"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    apply(
        &mut engine,
        1,
        vec![
            TypedOperation::SplitBlock {
                at: point(1, Affinity::After),
                node_type: "paragraph".into(),
                attrs: Default::default(),
            },
            TypedOperation::InsertText {
                at: point(1, Affinity::Before),
                text: "x".into(),
                marks: vec![],
            },
        ],
    );
    assert_eq!(
        engine.document_json(),
        Some(serde_json::json!({
            "type": "doc",
            "content": [
                {"type": "paragraph", "content": [
                    {"type": "hardBreak"},
                    {"type": "text", "text": "x"}
                ]},
                {"type": "paragraph", "content": [{"type": "text", "text": "b"}]}
            ]
        }))
    );
}

// ---------------------------------------------------------------------------
// Return-at-end-of-block (E1): the v2 lowering must accept a SplitBlock whose
// boundary is the very end of a block (empty suffix) and materialize the
// right sibling from the compiler preview.
// ---------------------------------------------------------------------------

fn import(engine: &mut YrsDocumentEngine, json: &str) {
    engine
        .import_json(json, TransactionOrigin::DocumentImport)
        .unwrap();
}

fn split_paragraph(at: u32) -> TypedOperation {
    TypedOperation::SplitBlock {
        at: point(at, Affinity::After),
        node_type: "paragraph".into(),
        attrs: Default::default(),
    }
}

fn insert_text(at: u32, text: &str) -> TypedOperation {
    TypedOperation::InsertText {
        at: point(at, Affinity::After),
        text: text.into(),
        marks: vec![],
    }
}

fn apply_input(
    engine: &mut YrsDocumentEngine,
    request_id: u64,
    operations: Vec<TypedOperation>,
) -> TypedTransactionResult {
    engine
        .apply_typed_transaction_with_result(TypedTransaction {
            request_id,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalInput,
            operations,
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Boundary,
        })
        .unwrap()
}

fn assert_collapsed_text_selection(result: &TypedTransactionResult, scalar: u32) {
    let ResolvedSelection::Text { anchor, head } = result.selection else {
        panic!("expected a text selection, got {:?}", result.selection);
    };
    assert_eq!(anchor, head, "split selection must be collapsed");
    assert_eq!(
        head.scalar, scalar,
        "selection must land at the start of the new block"
    );
}

#[test]
fn split_at_end_of_text_block_creates_empty_right_block_and_accepts_input() {
    let mut engine = engine();
    import(
        &mut engine,
        r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"ab"}]}]}"#,
    );

    // Caret at the very end of "ab" (scalar 2): Return-at-EOL.
    let split = apply_input(&mut engine, 1, vec![split_paragraph(2)]);
    assert!(split.changed);
    assert_eq!(
        engine.document_json(),
        Some(serde_json::json!({
            "type": "doc",
            "content": [
                {"type": "paragraph", "content": [{"type": "text", "text": "ab"}]},
                {"type": "paragraph"}
            ]
        }))
    );
    // Standard Return-at-EOL: selection at the start of the new block.
    // An empty block occupies one rendered scalar and the engine maps its
    // only cursor position to the end of that range ("ab" = 2, block
    // separator = 1, empty-block cursor = 4).
    assert_collapsed_text_selection(&split, 4);

    // Subsequent typed input lands in the new block.
    let insert = apply_input(&mut engine, 2, vec![insert_text(4, "x")]);
    assert!(insert.changed);
    assert_eq!(
        engine.document_json(),
        Some(serde_json::json!({
            "type": "doc",
            "content": [
                {"type": "paragraph", "content": [{"type": "text", "text": "ab"}]},
                {"type": "paragraph", "content": [{"type": "text", "text": "x"}]}
            ]
        }))
    );

    // Undo restores one block step by step; redo re-applies both steps.
    assert!(engine.can_undo());
    engine.undo(3).unwrap();
    assert_eq!(
        engine.document_json(),
        Some(serde_json::json!({
            "type": "doc",
            "content": [
                {"type": "paragraph", "content": [{"type": "text", "text": "ab"}]},
                {"type": "paragraph"}
            ]
        }))
    );
    engine.undo(4).unwrap();
    assert_eq!(
        engine.document_json(),
        Some(serde_json::json!({
            "type": "doc",
            "content": [
                {"type": "paragraph", "content": [{"type": "text", "text": "ab"}]}
            ]
        }))
    );
    assert!(engine.can_redo());
    engine.redo(5).unwrap();
    assert_eq!(
        engine.document_json(),
        Some(serde_json::json!({
            "type": "doc",
            "content": [
                {"type": "paragraph", "content": [{"type": "text", "text": "ab"}]},
                {"type": "paragraph"}
            ]
        }))
    );
    engine.redo(6).unwrap();
    assert_eq!(
        engine.document_json(),
        Some(serde_json::json!({
            "type": "doc",
            "content": [
                {"type": "paragraph", "content": [{"type": "text", "text": "ab"}]},
                {"type": "paragraph", "content": [{"type": "text", "text": "x"}]}
            ]
        }))
    );
}

#[test]
fn split_at_end_of_empty_paragraph_creates_two_empty_blocks() {
    let mut engine = engine();
    import(
        &mut engine,
        r#"{"type":"doc","content":[{"type":"paragraph"}]}"#,
    );

    // Caret at scalar 0 of an empty block is also an end-of-block split.
    let split = apply_input(&mut engine, 1, vec![split_paragraph(0)]);
    assert!(split.changed);
    assert_eq!(
        engine.document_json(),
        Some(serde_json::json!({
            "type": "doc",
            "content": [{"type": "paragraph"}, {"type": "paragraph"}]
        }))
    );
    // Two empty blocks: the second block starts at scalar 2 (empty block = 1,
    // separator = 1) and its cursor maps to scalar 3.
    assert_collapsed_text_selection(&split, 3);

    let insert = apply_input(&mut engine, 2, vec![insert_text(3, "x")]);
    assert!(insert.changed);
    assert_eq!(
        engine.document_json(),
        Some(serde_json::json!({
            "type": "doc",
            "content": [
                {"type": "paragraph"},
                {"type": "paragraph", "content": [{"type": "text", "text": "x"}]}
            ]
        }))
    );
}

#[test]
fn split_at_end_of_list_item_paragraph_creates_sibling_list_item() {
    let mut engine = engine();
    import(
        &mut engine,
        r#"{"type":"doc","content":[{"type":"bulletList","content":[{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"ab"}]}]}]}]}"#,
    );

    // The rendered "• " marker occupies two scalars, so the caret at the very
    // end of "ab" sits at scalar 4.
    let split = apply_input(&mut engine, 1, vec![split_paragraph(4)]);
    assert!(split.changed);
    assert_eq!(
        engine.document_json(),
        Some(serde_json::json!({
            "type": "doc",
            "content": [{
                "type": "bulletList",
                "content": [
                    {"type": "listItem", "content": [
                        {"type": "paragraph", "content": [{"type": "text", "text": "ab"}]}
                    ]},
                    {"type": "listItem", "content": [{"type": "paragraph"}]}
                ]
            }]
        }))
    );
    // New item cursor: 2 (marker) + 2 ("ab") + 1 (separator) + 2 (new marker)
    // + 1 (empty-block cursor) = 8.
    assert_collapsed_text_selection(&split, 8);

    let insert = apply_input(&mut engine, 2, vec![insert_text(8, "x")]);
    assert!(insert.changed);
    assert_eq!(
        engine.document_json(),
        Some(serde_json::json!({
            "type": "doc",
            "content": [{
                "type": "bulletList",
                "content": [
                    {"type": "listItem", "content": [
                        {"type": "paragraph", "content": [{"type": "text", "text": "ab"}]}
                    ]},
                    {"type": "listItem", "content": [
                        {"type": "paragraph", "content": [{"type": "text", "text": "x"}]}
                    ]}
                ]
            }]
        }))
    );
}

#[test]
fn split_at_end_of_blockquote_paragraph_creates_sibling_inside_quote() {
    let mut engine = engine();
    import(
        &mut engine,
        r#"{"type":"doc","content":[{"type":"blockquote","content":[{"type":"paragraph","content":[{"type":"text","text":"ab"}]}]}]}"#,
    );

    let split = apply_input(&mut engine, 1, vec![split_paragraph(2)]);
    assert!(split.changed);
    assert_eq!(
        engine.document_json(),
        Some(serde_json::json!({
            "type": "doc",
            "content": [{
                "type": "blockquote",
                "content": [
                    {"type": "paragraph", "content": [{"type": "text", "text": "ab"}]},
                    {"type": "paragraph"}
                ]
            }]
        }))
    );
    assert_collapsed_text_selection(&split, 4);
}

#[test]
fn split_block_on_empty_blockquote_paragraph_exits_quote() {
    // Ported from the deleted legacy `code_review_fixes_test` suite (Task 18D):
    // the only dedicated pin of `command_planner::plan_empty_blockquote_exit`.
    // Return (TypedCommand::SplitBlock, the production Return-key path) with the
    // caret in an empty blockquote paragraph must lift the caret out of the
    // quote into a sibling paragraph instead of splitting inside it.
    let mut engine = engine();
    import(
        &mut engine,
        r#"{"type":"doc","content":[{"type":"blockquote","content":[{"type":"paragraph","content":[{"type":"text","text":"Hello"}]},{"type":"paragraph"}]}]}"#,
    );

    // Caret in the empty paragraph: "Hello" = 5 scalars, block separator = 1,
    // empty-block cursor = 7. Selection normalization places the caret at the
    // doc position that resolves into the empty paragraph.
    engine
        .apply_typed_transaction(TypedTransaction {
            request_id: 1,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: vec![],
            selection_intent: SelectionIntent::Set(SelectionInput::Text {
                anchor: point(7, Affinity::Before),
                head: point(7, Affinity::Before),
            }),
            history_policy: HistoryPolicy::Skip,
        })
        .unwrap();

    let split = engine
        .apply_command(2, TypedCommand::SplitBlock)
        .unwrap()
        .expect("split command must apply");
    assert!(split.changed);
    assert_eq!(
        engine.document_json(),
        Some(serde_json::json!({
            "type": "doc",
            "content": [
                {
                    "type": "blockquote",
                    "content": [
                        {"type": "paragraph", "content": [{"type": "text", "text": "Hello"}]}
                    ]
                },
                {"type": "paragraph"}
            ]
        }))
    );

    // The caret lands in the new sibling paragraph outside the quote, so typed
    // input continues outside the blockquote.
    let insert = engine
        .apply_command(3, TypedCommand::InsertText { text: "x".into() })
        .unwrap()
        .expect("insert command must apply");
    assert!(insert.changed);
    assert_eq!(
        engine.document_json(),
        Some(serde_json::json!({
            "type": "doc",
            "content": [
                {
                    "type": "blockquote",
                    "content": [
                        {"type": "paragraph", "content": [{"type": "text", "text": "Hello"}]}
                    ]
                },
                {"type": "paragraph", "content": [{"type": "text", "text": "x"}]}
            ]
        }))
    );
}

#[test]
fn split_at_end_update_applies_cleanly_to_a_raw_yrs_replica() {
    let scope = || {
        Some(crate::yrs_engine::DocumentScope {
            document_id: "e1-split-document".into(),
            lineage_id: "e1-split-lineage".into(),
        })
    };
    let mut engine = YrsDocumentEngine::new(YrsEngineConfig {
        schema: tiptap_schema(),
        fragment_name: "prosemirror".into(),
        initialization_mode: InitializationMode::LocalEmpty,
        resource_limits: ResourceLimits::default(),
        editing_limits: EditingLimits::default(),
        max_length: None,
        scope: scope(),
    })
    .unwrap();
    import(
        &mut engine,
        r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"ab"}]}]}"#,
    );
    let base_encoded = engine.encoded_state().unwrap();

    apply_input(&mut engine, 1, vec![split_paragraph(2)]);
    apply_input(&mut engine, 2, vec![insert_text(4, "x")]);

    // The produced update-v1: diff the engine's post-split state against the
    // pre-split base, encoded from a raw yrs replica of the engine state.
    let engine_raw = Doc::with_options(Options {
        client_id: yrs::ClientID::new(70_002),
        offset_kind: OffsetKind::Utf16,
        ..Options::default()
    });
    engine_raw.get_or_insert_xml_fragment("prosemirror");
    engine_raw
        .transact_mut()
        .apply_update(Update::decode_v1(&engine.encoded_state().unwrap()).unwrap())
        .unwrap();

    // Independent raw yrs replica: base state, then the produced update-v1.
    let raw = Doc::with_options(Options {
        client_id: yrs::ClientID::new(70_001),
        offset_kind: OffsetKind::Utf16,
        ..Options::default()
    });
    raw.get_or_insert_xml_fragment("prosemirror");
    raw.transact_mut()
        .apply_update(Update::decode_v1(&base_encoded).unwrap())
        .unwrap();
    let diff = {
        let base_txn = raw.transact();
        let base_vector = base_txn.state_vector();
        engine_raw
            .transact()
            .encode_state_as_update_v1(&base_vector)
    };
    raw.transact_mut()
        .apply_update(Update::decode_v1(&diff).unwrap())
        .unwrap();

    // Convergence: the merged replica holds exactly the engine's item set —
    // no duplicated structure, nothing extra.
    assert_eq!(
        raw.transact().state_vector(),
        engine_raw.transact().state_vector(),
        "merged raw replica must converge to the engine's exact state"
    );

    // The merged replica hydrates to the same two-block document.
    let merged_update = {
        let txn = raw.transact();
        txn.encode_state_as_update_v1(&StateVector::default())
    };
    let mut snapshot = engine.export_snapshot().unwrap();
    snapshot.encoded_state = merged_update;
    let mut replica = YrsDocumentEngine::new(YrsEngineConfig {
        schema: tiptap_schema(),
        fragment_name: "prosemirror".into(),
        initialization_mode: InitializationMode::AwaitRemote,
        resource_limits: ResourceLimits::default(),
        editing_limits: EditingLimits::default(),
        max_length: None,
        scope: scope(),
    })
    .unwrap();
    replica.restore_snapshot(&snapshot).unwrap();
    assert_eq!(replica.document_json(), engine.document_json());
    assert_eq!(
        replica.document_json(),
        Some(serde_json::json!({
            "type": "doc",
            "content": [
                {"type": "paragraph", "content": [{"type": "text", "text": "ab"}]},
                {"type": "paragraph", "content": [{"type": "text", "text": "x"}]}
            ]
        }))
    );
}
