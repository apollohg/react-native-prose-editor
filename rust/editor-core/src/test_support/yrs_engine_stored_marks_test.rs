use std::collections::HashMap;

use crate::boundary::ResourceLimits;
use crate::model::Mark;
use crate::tiptap_schema;
use crate::yrs_engine::{
    Affinity, DocumentScope, EditingLimits, EditorOffsetKind, HistoryPolicy, InitializationMode,
    RevisionedPosition, RevisionedRange, SelectionInput, SelectionIntent, TransactionOrigin,
    TypedOperation, TypedTransaction, YrsDocumentEngine, YrsEngineConfig,
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

fn scoped_engine() -> YrsDocumentEngine {
    YrsDocumentEngine::new(YrsEngineConfig {
        schema: tiptap_schema(),
        fragment_name: "prosemirror".into(),
        initialization_mode: InitializationMode::LocalEmpty,
        resource_limits: ResourceLimits::default(),
        editing_limits: EditingLimits::default(),
        max_length: None,
        scope: Some(DocumentScope {
            document_id: "stored-doc".into(),
            lineage_id: "stored-lineage".into(),
        }),
    })
    .unwrap()
}

fn point(offset: u32) -> RevisionedPosition {
    RevisionedPosition {
        offset,
        kind: EditorOffsetKind::Scalar,
        affinity: Affinity::After,
    }
}

fn utf16_point(offset: u32) -> RevisionedPosition {
    RevisionedPosition {
        offset,
        kind: EditorOffsetKind::Utf16,
        affinity: Affinity::After,
    }
}

fn before_point(offset: u32) -> RevisionedPosition {
    RevisionedPosition {
        offset,
        kind: EditorOffsetKind::Scalar,
        affinity: Affinity::Before,
    }
}

fn affinity_point(offset: u32, affinity: Affinity) -> RevisionedPosition {
    RevisionedPosition {
        offset,
        kind: EditorOffsetKind::Scalar,
        affinity,
    }
}

fn range(from: u32, to: u32) -> RevisionedRange {
    RevisionedRange {
        from: point(from),
        to: point(to),
    }
}

fn mark(mark_type: &str) -> Mark {
    Mark::new(mark_type.into(), HashMap::new())
}

fn link(href: &str) -> Mark {
    Mark::new(
        "link".into(),
        HashMap::from([("href".into(), serde_json::json!(href))]),
    )
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
        origin: TransactionOrigin::LocalInput,
        operations,
        selection_intent,
        history_policy: HistoryPolicy::Skip,
    }
}

fn apply(
    engine: &mut YrsDocumentEngine,
    request_id: u64,
    operations: Vec<TypedOperation>,
    selection_intent: SelectionIntent,
) -> crate::yrs_engine::TransactionCommit {
    engine
        .apply_typed_transaction(transaction(
            engine,
            request_id,
            operations,
            selection_intent,
        ))
        .unwrap()
}

fn import_marked_text(engine: &mut YrsDocumentEngine) {
    engine
        .import_json(
            &serde_json::json!({
                "type": "doc",
                "content": [{
                    "type": "paragraph",
                    "content": [{
                        "type": "text",
                        "text": "ab",
                        "marks": [{ "type": "bold" }]
                    }]
                }]
            })
            .to_string(),
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    apply(
        engine,
        1,
        vec![],
        SelectionIntent::Set(SelectionInput::Text {
            anchor: point(1),
            head: point(1),
        }),
    );
}

fn import_plain_text(engine: &mut YrsDocumentEngine, text: &str) {
    engine
        .import_json(
            &serde_json::json!({
                "type": "doc",
                "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": text }] }]
            })
            .to_string(),
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    apply(
        engine,
        1,
        vec![],
        SelectionIntent::Set(SelectionInput::Text {
            anchor: point(1),
            head: point(1),
        }),
    );
}

fn mark_types(engine: &YrsDocumentEngine) -> Option<Vec<&str>> {
    engine
        .stored_marks()
        .map(|marks| marks.iter().map(Mark::mark_type).collect())
}

#[test]
fn collapsed_mark_operations_are_state_only_and_schema_ranked() {
    let mut engine = engine();
    import_marked_text(&mut engine);
    let encoded = engine.encoded_state().unwrap();
    let document_revision = engine.revision();
    let state_revision = engine.state_revision();

    let add = apply(
        &mut engine,
        2,
        vec![TypedOperation::AddMark {
            range: range(1, 1),
            mark: mark("italic"),
        }],
        SelectionIntent::Preserve,
    );
    assert!(add.changed);
    assert_eq!(add.document_revision, document_revision);
    assert_eq!(add.state_revision, state_revision + 1);
    assert_eq!(engine.encoded_state().unwrap(), encoded);
    assert_eq!(mark_types(&engine), Some(vec!["bold", "italic"]));

    apply(
        &mut engine,
        3,
        vec![TypedOperation::RemoveMark {
            range: range(1, 1),
            mark_type: "bold".into(),
        }],
        SelectionIntent::Preserve,
    );
    assert_eq!(mark_types(&engine), Some(vec!["italic"]));

    apply(
        &mut engine,
        4,
        vec![TypedOperation::ReplaceMark {
            range: range(1, 1),
            mark: link("https://example.com"),
        }],
        SelectionIntent::Preserve,
    );
    assert_eq!(mark_types(&engine), Some(vec!["italic", "link"]));
    assert_eq!(
        engine.stored_marks().unwrap()[1].attrs()["href"],
        serde_json::json!("https://example.com")
    );
}

#[test]
fn zero_width_mark_away_from_the_current_caret_remains_a_semantic_no_op() {
    let mut engine = engine();
    import_marked_text(&mut engine);
    let before = (
        engine.encoded_state().unwrap(),
        engine.revision(),
        engine.state_revision(),
        engine.stored_marks().map(<[Mark]>::to_vec),
    );
    let commit = apply(
        &mut engine,
        2,
        vec![TypedOperation::AddMark {
            range: range(0, 0),
            mark: mark("italic"),
        }],
        SelectionIntent::Preserve,
    );
    assert!(!commit.changed);
    assert_eq!(
        before,
        (
            engine.encoded_state().unwrap(),
            engine.revision(),
            engine.state_revision(),
            engine.stored_marks().map(<[Mark]>::to_vec),
        )
    );
}

#[test]
fn explicit_plain_insert_never_infers_stored_or_document_marks() {
    let mut engine = engine();
    import_marked_text(&mut engine);
    apply(
        &mut engine,
        2,
        vec![TypedOperation::AddMark {
            range: range(1, 1),
            mark: mark("italic"),
        }],
        SelectionIntent::Preserve,
    );

    apply(
        &mut engine,
        3,
        vec![TypedOperation::InsertText {
            at: point(1),
            text: "x".into(),
            marks: vec![],
        }],
        SelectionIntent::UseOperationResult,
    );
    let json = engine.document_json().unwrap();
    let content = json["content"][0]["content"].as_array().unwrap();
    assert!(content
        .iter()
        .any(|node| node["text"] == "x" && node.get("marks").is_none()));
    assert_eq!(engine.stored_marks(), None);
}

#[test]
fn compatible_sequential_typing_preserves_stored_marks_but_mismatch_clears() {
    let mut engine = engine();
    import_marked_text(&mut engine);
    apply(
        &mut engine,
        2,
        vec![TypedOperation::RemoveMark {
            range: range(1, 1),
            mark_type: "bold".into(),
        }],
        SelectionIntent::Preserve,
    );
    assert_eq!(engine.stored_marks(), Some([].as_slice()));

    apply(
        &mut engine,
        3,
        vec![TypedOperation::InsertText {
            at: point(1),
            text: "x".into(),
            marks: vec![],
        }],
        SelectionIntent::UseOperationResult,
    );
    assert_eq!(engine.stored_marks(), Some([].as_slice()));

    apply(
        &mut engine,
        4,
        vec![TypedOperation::InsertText {
            at: point(1),
            text: "y".into(),
            marks: vec![mark("bold")],
        }],
        SelectionIntent::Preserve,
    );
    assert_eq!(engine.stored_marks(), None);
}

#[test]
fn changed_selection_and_range_formatting_clear_stored_marks() {
    let mut engine = engine();
    import_marked_text(&mut engine);
    apply(
        &mut engine,
        2,
        vec![TypedOperation::RemoveMark {
            range: range(1, 1),
            mark_type: "bold".into(),
        }],
        SelectionIntent::Preserve,
    );
    assert!(engine.stored_marks().is_some());

    apply(
        &mut engine,
        3,
        vec![],
        SelectionIntent::Set(SelectionInput::Text {
            anchor: point(1),
            head: point(0),
        }),
    );
    assert_eq!(engine.stored_marks(), None);

    apply(
        &mut engine,
        4,
        vec![TypedOperation::AddMark {
            range: range(0, 2),
            mark: mark("italic"),
        }],
        SelectionIntent::Preserve,
    );
    assert_eq!(engine.stored_marks(), None);
}

#[test]
fn invalid_stored_mark_attrs_reject_atomically() {
    let mut engine = engine();
    import_marked_text(&mut engine);
    let before = (
        engine.encoded_state().unwrap(),
        engine.revision(),
        engine.state_revision(),
        engine.stored_marks().map(<[Mark]>::to_vec),
    );
    let invalid = Mark::new(
        "link".into(),
        HashMap::from([("unknown".into(), serde_json::json!(true))]),
    );
    let error = engine
        .apply_typed_transaction(transaction(
            &engine,
            2,
            vec![TypedOperation::AddMark {
                range: range(1, 1),
                mark: invalid,
            }],
            SelectionIntent::Preserve,
        ))
        .unwrap_err();
    assert_eq!(error.code, "OPERATION_INVALID");
    assert_eq!(
        before,
        (
            engine.encoded_state().unwrap(),
            engine.revision(),
            engine.state_revision(),
            engine.stored_marks().map(<[Mark]>::to_vec),
        )
    );
}

#[test]
fn changed_import_clears_but_identical_and_rejected_imports_preserve() {
    let mut engine = engine();
    import_marked_text(&mut engine);
    apply(
        &mut engine,
        2,
        vec![TypedOperation::RemoveMark {
            range: range(1, 1),
            mark_type: "bold".into(),
        }],
        SelectionIntent::Preserve,
    );
    let same = engine.document_json().unwrap();
    engine
        .import_json(&same.to_string(), TransactionOrigin::DocumentImport)
        .unwrap();
    assert_eq!(engine.stored_marks(), Some([].as_slice()));

    assert!(engine
        .import_json("{not-json", TransactionOrigin::DocumentImport)
        .is_err());
    assert_eq!(engine.stored_marks(), Some([].as_slice()));

    engine
        .import_json(
            &serde_json::json!({
                "type": "doc",
                "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "changed" }] }]
            })
            .to_string(),
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    assert_eq!(engine.stored_marks(), None);
}

#[test]
fn identical_stored_operation_and_identical_selection_are_complete_no_ops() {
    let mut engine = engine();
    import_marked_text(&mut engine);
    apply(
        &mut engine,
        2,
        vec![TypedOperation::AddMark {
            range: range(1, 1),
            mark: mark("italic"),
        }],
        SelectionIntent::Preserve,
    );
    let before = (
        engine.encoded_state().unwrap(),
        engine.revision(),
        engine.state_revision(),
        engine.stored_marks().unwrap().to_vec(),
    );

    let identical_mark = apply(
        &mut engine,
        3,
        vec![TypedOperation::AddMark {
            range: range(1, 1),
            mark: mark("italic"),
        }],
        SelectionIntent::Preserve,
    );
    assert!(!identical_mark.changed);
    let identical_selection = apply(
        &mut engine,
        4,
        vec![],
        SelectionIntent::Set(SelectionInput::Text {
            anchor: point(1),
            head: point(1),
        }),
    );
    assert!(!identical_selection.changed);
    assert_eq!(
        before,
        (
            engine.encoded_state().unwrap(),
            engine.revision(),
            engine.state_revision(),
            engine.stored_marks().unwrap().to_vec(),
        )
    );
}

#[test]
fn moved_caret_and_all_selection_clear_stored_marks_once() {
    let mut engine = engine();
    import_marked_text(&mut engine);
    apply(
        &mut engine,
        2,
        vec![TypedOperation::RemoveMark {
            range: range(1, 1),
            mark_type: "bold".into(),
        }],
        SelectionIntent::Preserve,
    );
    let revision = engine.revision();
    let state_revision = engine.state_revision();
    let moved = apply(
        &mut engine,
        3,
        vec![],
        SelectionIntent::Set(SelectionInput::Text {
            anchor: point(0),
            head: point(0),
        }),
    );
    assert!(moved.changed);
    assert_eq!(moved.document_revision, revision);
    assert_eq!(moved.state_revision, state_revision + 1);
    assert_eq!(engine.stored_marks(), None);

    let all = apply(
        &mut engine,
        4,
        vec![],
        SelectionIntent::Set(SelectionInput::All),
    );
    assert!(all.changed);
    assert_eq!(engine.stored_marks(), None);
}

#[test]
fn nonzero_range_format_clears_existing_stored_marks_directly() {
    for operation in [
        TypedOperation::AddMark {
            range: range(0, 2),
            mark: mark("italic"),
        },
        TypedOperation::RemoveMark {
            range: range(0, 2),
            mark_type: "bold".into(),
        },
        TypedOperation::ReplaceMark {
            range: range(0, 2),
            mark: mark("italic"),
        },
    ] {
        let mut engine = engine();
        import_marked_text(&mut engine);
        apply(
            &mut engine,
            2,
            vec![TypedOperation::RemoveMark {
                range: range(1, 1),
                mark_type: "bold".into(),
            }],
            SelectionIntent::Preserve,
        );
        assert!(engine.stored_marks().is_some());
        apply(&mut engine, 3, vec![operation], SelectionIntent::Preserve);
        assert_eq!(engine.stored_marks(), None);
    }
}

#[test]
fn mixed_scalar_utf16_collapsed_range_uses_one_emoji_caret() {
    let mut engine = engine();
    engine
        .import_json(
            &serde_json::json!({
                "type": "doc",
                "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "A😀B" }] }]
            })
            .to_string(),
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    apply(
        &mut engine,
        1,
        vec![],
        SelectionIntent::Set(SelectionInput::Text {
            anchor: point(2),
            head: point(2),
        }),
    );
    apply(
        &mut engine,
        2,
        vec![TypedOperation::AddMark {
            range: RevisionedRange {
                from: point(2),
                to: utf16_point(3),
            },
            mark: mark("italic"),
        }],
        SelectionIntent::Preserve,
    );
    assert_eq!(mark_types(&engine), Some(vec!["italic"]));
}

#[test]
fn canonical_equivalent_explicit_typing_preserves_ranked_stored_marks() {
    let mut engine = engine();
    import_marked_text(&mut engine);
    apply(
        &mut engine,
        2,
        vec![TypedOperation::AddMark {
            range: range(1, 1),
            mark: mark("italic"),
        }],
        SelectionIntent::Preserve,
    );
    apply(
        &mut engine,
        3,
        vec![TypedOperation::InsertText {
            at: point(1),
            text: "x".into(),
            marks: vec![mark("italic"), mark("bold")],
        }],
        SelectionIntent::UseOperationResult,
    );
    assert_eq!(mark_types(&engine), Some(vec!["bold", "italic"]));
}

#[test]
fn compatible_adjacent_delete_and_caret_split_preserve_but_away_structural_operations_clear() {
    let mut deletion = engine();
    import_marked_text(&mut deletion);
    apply(
        &mut deletion,
        2,
        vec![TypedOperation::AddMark {
            range: range(1, 1),
            mark: mark("italic"),
        }],
        SelectionIntent::Preserve,
    );
    apply(
        &mut deletion,
        3,
        vec![TypedOperation::DeleteRange { range: range(0, 1) }],
        SelectionIntent::UseOperationResult,
    );
    assert_eq!(mark_types(&deletion), Some(vec!["bold", "italic"]));

    let mut structural = engine();
    import_marked_text(&mut structural);
    apply(
        &mut structural,
        2,
        vec![TypedOperation::AddMark {
            range: range(1, 1),
            mark: mark("italic"),
        }],
        SelectionIntent::Preserve,
    );
    apply(
        &mut structural,
        3,
        vec![TypedOperation::SplitBlock {
            at: before_point(1),
            node_type: "paragraph".into(),
            attrs: HashMap::new(),
        }],
        SelectionIntent::Preserve,
    );
    // A split *at* the caret is Return: the active formatting continues onto
    // the new block, so the stored set survives.
    assert_eq!(mark_types(&structural), Some(vec!["bold", "italic"]));

    let mut structural_away = engine();
    import_marked_text(&mut structural_away);
    apply(
        &mut structural_away,
        2,
        vec![TypedOperation::AddMark {
            range: range(1, 1),
            mark: mark("italic"),
        }],
        SelectionIntent::Preserve,
    );
    apply(
        &mut structural_away,
        3,
        vec![TypedOperation::SplitBlock {
            at: before_point(0),
            node_type: "paragraph".into(),
            attrs: HashMap::new(),
        }],
        SelectionIntent::Preserve,
    );
    // A split away from the caret is an ordinary structural edit and still
    // clears the stored set.
    assert_eq!(structural_away.stored_marks(), None);

    let mut structural_delete = engine();
    structural_delete
        .import_json(
            &serde_json::json!({
                "type": "doc",
                "content": [{
                    "type": "paragraph",
                    "content": [
                        {"type": "text", "text": "a", "marks": [{"type": "bold"}]},
                        {"type": "hardBreak"},
                        {"type": "text", "text": "b"}
                    ]
                }]
            })
            .to_string(),
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    apply(
        &mut structural_delete,
        1,
        vec![],
        SelectionIntent::Set(SelectionInput::Text {
            anchor: before_point(1),
            head: before_point(1),
        }),
    );
    apply(
        &mut structural_delete,
        2,
        vec![TypedOperation::AddMark {
            range: RevisionedRange {
                from: before_point(1),
                to: before_point(1),
            },
            mark: mark("italic"),
        }],
        SelectionIntent::Preserve,
    );
    assert!(structural_delete.stored_marks().is_some());
    apply(
        &mut structural_delete,
        3,
        vec![TypedOperation::DeleteRange {
            range: RevisionedRange {
                from: before_point(1),
                to: before_point(2),
            },
        }],
        SelectionIntent::UseOperationResult,
    );
    assert_eq!(structural_delete.stored_marks(), None);
}

#[test]
fn later_away_operation_result_cannot_reuse_an_earlier_input_exemption() {
    let mut engine = engine();
    import_marked_text(&mut engine);
    apply(
        &mut engine,
        2,
        vec![TypedOperation::AddMark {
            range: range(1, 1),
            mark: mark("italic"),
        }],
        SelectionIntent::Preserve,
    );
    assert_eq!(mark_types(&engine), Some(vec!["bold", "italic"]));

    apply(
        &mut engine,
        3,
        vec![
            TypedOperation::InsertText {
                at: point(1),
                text: "x".into(),
                marks: vec![mark("bold"), mark("italic")],
            },
            TypedOperation::DeleteRange { range: range(0, 0) },
        ],
        SelectionIntent::UseOperationResult,
    );

    assert_eq!(engine.stored_marks(), None);
}

#[test]
fn no_op_result_at_the_fully_mapped_caret_preserves_stored_marks() {
    let mut engine = engine();
    import_marked_text(&mut engine);
    apply(
        &mut engine,
        2,
        vec![TypedOperation::AddMark {
            range: range(1, 1),
            mark: mark("italic"),
        }],
        SelectionIntent::Preserve,
    );

    apply(
        &mut engine,
        3,
        vec![
            TypedOperation::InsertText {
                at: point(1),
                text: "x".into(),
                marks: vec![mark("bold"), mark("italic")],
            },
            TypedOperation::DeleteRange { range: range(1, 1) },
        ],
        SelectionIntent::UseOperationResult,
    );

    assert_eq!(mark_types(&engine), Some(vec!["bold", "italic"]));
}

#[test]
fn caret_mark_result_after_compatible_input_keeps_the_updated_stored_set() {
    for (final_operation, expected) in [
        (
            TypedOperation::AddMark {
                range: range(1, 1),
                mark: mark("strike"),
            },
            vec!["bold", "italic", "strike"],
        ),
        (
            TypedOperation::RemoveMark {
                range: range(1, 1),
                mark_type: "italic".into(),
            },
            vec!["bold"],
        ),
        (
            TypedOperation::ReplaceMark {
                range: range(1, 1),
                mark: link("https://example.com/after-input"),
            },
            vec!["bold", "italic", "link"],
        ),
    ] {
        let mut engine = engine();
        import_marked_text(&mut engine);
        apply(
            &mut engine,
            2,
            vec![TypedOperation::AddMark {
                range: range(1, 1),
                mark: mark("italic"),
            }],
            SelectionIntent::Preserve,
        );

        apply(
            &mut engine,
            3,
            vec![
                TypedOperation::InsertText {
                    at: point(1),
                    text: "x".into(),
                    marks: vec![mark("bold"), mark("italic")],
                },
                final_operation,
            ],
            SelectionIntent::UseOperationResult,
        );

        assert_eq!(mark_types(&engine), Some(expected));
    }
}

#[test]
fn net_stored_mark_state_in_one_envelope_controls_revision() {
    let mut marked_engine = engine();
    import_marked_text(&mut marked_engine);
    apply(
        &mut marked_engine,
        2,
        vec![TypedOperation::RemoveMark {
            range: range(1, 1),
            mark_type: "bold".into(),
        }],
        SelectionIntent::Preserve,
    );
    let state_revision = marked_engine.state_revision();
    let unchanged = apply(
        &mut marked_engine,
        3,
        vec![
            TypedOperation::AddMark {
                range: range(1, 1),
                mark: mark("bold"),
            },
            TypedOperation::RemoveMark {
                range: range(1, 1),
                mark_type: "bold".into(),
            },
        ],
        SelectionIntent::Preserve,
    );
    assert!(!unchanged.changed);
    assert_eq!(unchanged.state_revision, state_revision);
    assert_eq!(marked_engine.stored_marks(), Some([].as_slice()));

    let mut plain = engine();
    let changed = apply(
        &mut plain,
        1,
        vec![
            TypedOperation::AddMark {
                range: range(1, 1),
                mark: mark("bold"),
            },
            TypedOperation::RemoveMark {
                range: range(1, 1),
                mark_type: "bold".into(),
            },
        ],
        SelectionIntent::Preserve,
    );
    assert!(changed.changed);
    assert_eq!(plain.stored_marks(), Some([].as_slice()));
}

#[test]
fn duplicate_and_unknown_mark_inputs_reject_with_operation_attribution() {
    let mut engine = engine();
    import_marked_text(&mut engine);
    let before = (
        engine.encoded_state().unwrap(),
        engine.revision(),
        engine.state_revision(),
        engine.stored_marks().map(<[Mark]>::to_vec),
    );
    let duplicate = engine
        .apply_typed_transaction(transaction(
            &engine,
            2,
            vec![TypedOperation::InsertText {
                at: point(1),
                text: "x".into(),
                marks: vec![mark("bold"), mark("bold")],
            }],
            SelectionIntent::UseOperationResult,
        ))
        .unwrap_err();
    assert_eq!(duplicate.code, "OPERATION_INVALID");
    assert_eq!(duplicate.operation_index, Some(0));
    assert_eq!(
        before,
        (
            engine.encoded_state().unwrap(),
            engine.revision(),
            engine.state_revision(),
            engine.stored_marks().map(<[Mark]>::to_vec),
        )
    );

    let unknown = engine
        .apply_typed_transaction(transaction(
            &engine,
            3,
            vec![TypedOperation::RemoveMark {
                range: range(1, 1),
                mark_type: "mystery".into(),
            }],
            SelectionIntent::Preserve,
        ))
        .unwrap_err();
    assert_eq!(unknown.code, "OPERATION_INVALID");
    assert_eq!(unknown.operation_index, Some(0));
}

#[test]
fn inherited_identical_add_and_replace_preserve_none_without_revision() {
    let mut engine = engine();
    import_marked_text(&mut engine);
    assert_eq!(engine.stored_marks(), None);
    let state_revision = engine.state_revision();

    for (request_id, operation) in [
        (
            2,
            TypedOperation::AddMark {
                range: range(1, 1),
                mark: mark("bold"),
            },
        ),
        (
            3,
            TypedOperation::ReplaceMark {
                range: range(1, 1),
                mark: mark("bold"),
            },
        ),
    ] {
        let commit = apply(
            &mut engine,
            request_id,
            vec![operation],
            SelectionIntent::Preserve,
        );
        assert!(!commit.changed);
        assert_eq!(commit.state_revision, state_revision);
        assert_eq!(engine.stored_marks(), None);
    }
}

#[test]
fn add_same_type_with_different_attrs_rejects_for_caret_and_range() {
    let mut engine = engine();
    engine
        .import_json(
            &serde_json::json!({
                "type": "doc",
                "content": [{
                    "type": "paragraph",
                    "content": [{
                        "type": "text",
                        "text": "ab",
                        "marks": [{ "type": "link", "attrs": { "href": "https://old" } }]
                    }]
                }]
            })
            .to_string(),
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    apply(
        &mut engine,
        1,
        vec![],
        SelectionIntent::Set(SelectionInput::Text {
            anchor: point(1),
            head: point(1),
        }),
    );
    let before = (
        engine.encoded_state().unwrap(),
        engine.revision(),
        engine.state_revision(),
        engine.stored_marks().map(<[Mark]>::to_vec),
    );
    for (request_id, target) in [(2, range(1, 1)), (3, range(0, 2))] {
        let error = engine
            .apply_typed_transaction(transaction(
                &engine,
                request_id,
                vec![TypedOperation::AddMark {
                    range: target,
                    mark: link("https://new"),
                }],
                SelectionIntent::Preserve,
            ))
            .unwrap_err();
        assert_eq!(error.code, "OPERATION_INVALID");
        assert_eq!(error.operation_index, Some(0));
        assert_eq!(
            before,
            (
                engine.encoded_state().unwrap(),
                engine.revision(),
                engine.state_revision(),
                engine.stored_marks().map(<[Mark]>::to_vec),
            )
        );
    }
}

#[test]
fn sequential_preview_marks_feed_later_collapsed_mark_operation() {
    let mut engine = engine();
    import_plain_text(&mut engine, "ab");
    apply(
        &mut engine,
        2,
        vec![
            TypedOperation::AddMark {
                range: range(0, 2),
                mark: mark("bold"),
            },
            TypedOperation::AddMark {
                range: range(1, 1),
                mark: mark("italic"),
            },
        ],
        SelectionIntent::Preserve,
    );
    assert_eq!(mark_types(&engine), Some(vec!["bold", "italic"]));
}

#[test]
fn use_result_away_and_noncollapsed_results_clear_existing_stored_marks() {
    for operation in [
        TypedOperation::AddMark {
            range: range(0, 0),
            mark: mark("italic"),
        },
        TypedOperation::AddMark {
            range: range(0, 2),
            mark: mark("bold"),
        },
        TypedOperation::DeleteRange { range: range(0, 0) },
    ] {
        let mut engine = engine();
        import_marked_text(&mut engine);
        apply(
            &mut engine,
            2,
            vec![TypedOperation::RemoveMark {
                range: range(1, 1),
                mark_type: "bold".into(),
            }],
            SelectionIntent::Preserve,
        );
        assert!(engine.stored_marks().is_some());
        apply(
            &mut engine,
            3,
            vec![operation],
            SelectionIntent::UseOperationResult,
        );
        assert_eq!(engine.stored_marks(), None);
    }
}

#[test]
fn opposite_affinity_after_prior_insert_does_not_mutate_stored_marks() {
    let mut engine = engine();
    apply(
        &mut engine,
        1,
        vec![TypedOperation::RemoveMark {
            range: range(1, 1),
            mark_type: "bold".into(),
        }],
        SelectionIntent::Preserve,
    );
    assert_eq!(engine.stored_marks(), Some([].as_slice()));
    let caret_affinity = match engine.relative_selection().unwrap() {
        crate::yrs_engine::RelativeSelection::Text { head, .. } => head.affinity,
        _ => panic!("expected collapsed text selection"),
    };
    let opposite = match caret_affinity {
        Affinity::Before => Affinity::After,
        Affinity::After => Affinity::Before,
    };
    apply(
        &mut engine,
        2,
        vec![
            TypedOperation::InsertText {
                at: affinity_point(1, caret_affinity),
                text: "x".into(),
                marks: vec![],
            },
            TypedOperation::AddMark {
                range: RevisionedRange {
                    from: affinity_point(1, opposite),
                    to: affinity_point(1, opposite),
                },
                mark: mark("italic"),
            },
        ],
        SelectionIntent::Preserve,
    );
    assert_eq!(engine.stored_marks(), Some([].as_slice()));
}

#[test]
fn no_op_structural_operation_plus_compatible_insert_preserves_stored_marks() {
    let mut engine = engine();
    engine
        .import_json(
            &serde_json::json!({
                "type": "doc",
                "content": [{
                    "type": "orderedList",
                    "attrs": { "start": 1 },
                    "content": [{
                        "type": "listItem",
                        "content": [{
                            "type": "paragraph",
                            "content": [{
                                "type": "text",
                                "text": "ab",
                                "marks": [{ "type": "bold" }]
                            }]
                        }]
                    }]
                }]
            })
            .to_string(),
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let block = engine.position_map().unwrap().block(0).unwrap();
    let caret = block.scalar_start + block.scalar_prefix_len + 1;
    apply(
        &mut engine,
        1,
        vec![],
        SelectionIntent::Set(SelectionInput::Text {
            anchor: point(caret),
            head: point(caret),
        }),
    );
    apply(
        &mut engine,
        2,
        vec![TypedOperation::AddMark {
            range: range(caret, caret),
            mark: mark("italic"),
        }],
        SelectionIntent::Preserve,
    );
    let caret_affinity = match engine.relative_selection().unwrap() {
        crate::yrs_engine::RelativeSelection::Text { head, .. } => head.affinity,
        _ => panic!("expected collapsed text selection"),
    };
    apply(
        &mut engine,
        3,
        vec![
            TypedOperation::UpdateNodeAttrs {
                at: before_point(0),
                attrs: serde_json::from_value(serde_json::json!({ "start": 1 })).unwrap(),
            },
            TypedOperation::InsertText {
                at: affinity_point(caret, caret_affinity),
                text: "x".into(),
                marks: vec![mark("bold"), mark("italic")],
            },
        ],
        SelectionIntent::UseOperationResult,
    );
    assert_eq!(mark_types(&engine), Some(vec!["bold", "italic"]));
}

#[test]
fn matching_marks_inserted_away_from_caret_clear_stored_marks() {
    let mut engine = engine();
    import_marked_text(&mut engine);
    apply(
        &mut engine,
        2,
        vec![TypedOperation::AddMark {
            range: range(1, 1),
            mark: mark("italic"),
        }],
        SelectionIntent::Preserve,
    );
    apply(
        &mut engine,
        3,
        vec![TypedOperation::InsertText {
            at: point(0),
            text: "x".into(),
            marks: vec![mark("bold"), mark("italic")],
        }],
        SelectionIntent::Preserve,
    );
    assert_eq!(engine.stored_marks(), None);
}

#[test]
fn snapshot_restore_is_document_scoped_and_never_transfers_local_stored_marks() {
    let mut target = scoped_engine();
    import_marked_text(&mut target);
    let exact = target.export_snapshot().unwrap();
    apply(
        &mut target,
        2,
        vec![TypedOperation::RemoveMark {
            range: range(1, 1),
            mark_type: "bold".into(),
        }],
        SelectionIntent::Preserve,
    );
    assert_eq!(target.stored_marks(), Some([].as_slice()));
    let exact_commit = target.restore_snapshot(&exact).unwrap();
    assert!(!exact_commit.changed);
    assert_eq!(target.stored_marks(), Some([].as_slice()));

    let mut rejected = exact.clone();
    rejected.format_version += 1;
    assert!(target.restore_snapshot(&rejected).is_err());
    assert_eq!(target.stored_marks(), Some([].as_slice()));

    let mut donor = scoped_engine();
    import_plain_text(&mut donor, "changed");
    let changed = donor.export_snapshot().unwrap();
    let changed_commit = target.restore_snapshot(&changed).unwrap();
    assert!(changed_commit.changed);
    assert_eq!(target.stored_marks(), None);

    let mut receiver = scoped_engine();
    apply(
        &mut receiver,
        1,
        vec![TypedOperation::RemoveMark {
            range: range(1, 1),
            mark_type: "bold".into(),
        }],
        SelectionIntent::Preserve,
    );
    assert!(receiver.stored_marks().is_some());
    receiver.restore_snapshot(&exact).unwrap();
    assert_eq!(receiver.stored_marks(), None);
}
