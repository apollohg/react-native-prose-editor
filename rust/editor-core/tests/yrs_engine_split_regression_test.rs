use editor_core::boundary::ResourceLimits;
use editor_core::tiptap_schema;
use editor_core::yrs_engine::{
    Affinity, EditingLimits, EditorOffsetKind, HistoryPolicy, InitializationMode,
    RevisionedPosition, RevisionedRange, SelectionIntent, TransactionOrigin, TypedOperation,
    TypedTransaction, YrsDocumentEngine, YrsEngineConfig,
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
