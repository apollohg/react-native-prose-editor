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

include!("yrs_engine_stored_marks_test/operation_results.rs");
