use editor_core::boundary::ResourceLimits;
use editor_core::editor::Editor;
use editor_core::intercept::{InterceptorPipeline, MaxLength};
use editor_core::schema::Schema;
use editor_core::selection::Selection;
use editor_core::tiptap_schema;
use editor_core::yrs_engine::{
    Affinity, CommandPlan, EditingLimits, EditorOffsetKind, HistoryPolicy, InitializationMode,
    RevisionedPosition, RevisionedRange, SelectionInput, SelectionIntent, TransactionOrigin,
    TypedCommand, TypedOperation, TypedTransaction, YrsDocumentEngine, YrsEngineConfig,
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

fn assert_strict_selection_normalization(
    legacy_editor: &Editor,
    legacy: &editor_core::selection::Selection,
    yrs: &editor_core::yrs_engine::ResolvedSelection,
    yrs_engine: &YrsDocumentEngine,
    legacy_has_one_past_end_artifact: bool,
) {
    let editor_core::selection::Selection::Text { anchor, head } = legacy else {
        panic!("expected legacy text selection")
    };
    let editor_core::yrs_engine::ResolvedSelection::Text {
        anchor: yrs_anchor,
        head: yrs_head,
    } = yrs
    else {
        panic!("expected Yrs text selection")
    };
    assert_eq!(*anchor, *head);
    assert_eq!(yrs_anchor.scalar, yrs_head.scalar);
    // Legacy block insertion retains its historical synthetic one-past-end
    // scalar. The strict Yrs selection is the normalized rendered-text end.
    if legacy_has_one_past_end_artifact {
        assert_eq!(*anchor, yrs_anchor.scalar.saturating_add(1));
    } else {
        assert_eq!(*anchor, yrs_anchor.scalar);
    }
    assert_eq!(
        legacy_editor.normalize_pos(legacy_editor.selection().from(legacy_editor.document())),
        legacy_editor.selection().from(legacy_editor.document())
    );
    assert!(yrs_anchor.scalar <= yrs_engine.position_map().unwrap().total_scalars());
}

fn point(offset: u32) -> RevisionedPosition {
    RevisionedPosition {
        offset,
        kind: EditorOffsetKind::Scalar,
        affinity: Affinity::After,
    }
}

fn select_yrs(engine: &mut YrsDocumentEngine, request_id: u64, anchor: u32, head: u32) {
    engine
        .apply_typed_transaction(TypedTransaction {
            request_id,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: vec![],
            selection_intent: SelectionIntent::Set(SelectionInput::Text {
                anchor: RevisionedPosition {
                    affinity: Affinity::Before,
                    ..point(anchor)
                },
                head: RevisionedPosition {
                    affinity: Affinity::Before,
                    ..point(head)
                },
            }),
            history_policy: HistoryPolicy::Skip,
        })
        .unwrap();
}

fn structural_parity(
    label: &str,
    document: serde_json::Value,
    scalar_anchor: u32,
    scalar_head: u32,
    command: TypedCommand,
) {
    structural_parity_with_schema(
        label,
        tiptap_schema(),
        document,
        scalar_anchor,
        scalar_head,
        command,
    );
}

fn structural_parity_with_schema(
    label: &str,
    schema: Schema,
    document: serde_json::Value,
    scalar_anchor: u32,
    scalar_head: u32,
    command: TypedCommand,
) {
    let mut legacy = Editor::new(schema.clone(), InterceptorPipeline::new(), false);
    legacy.set_json(&document).unwrap();
    legacy.set_selection_scalar(scalar_anchor, scalar_head);
    let mut yrs = YrsDocumentEngine::new(YrsEngineConfig {
        schema,
        fragment_name: "prosemirror".into(),
        initialization_mode: InitializationMode::LocalEmpty,
        resource_limits: ResourceLimits::default(),
        editing_limits: EditingLimits::default(),
        max_length: None,
        scope: None,
    })
    .unwrap();
    yrs.import_json(&document.to_string(), TransactionOrigin::DocumentImport)
        .unwrap();
    select_yrs(&mut yrs, 800, scalar_anchor, scalar_head);
    let legacy_update = match &command {
        TypedCommand::DeleteBackward => legacy
            .delete_backward_at_selection_scalar(scalar_anchor, scalar_head)
            .unwrap(),
        TypedCommand::DeleteRange { .. } => legacy
            .delete_scalar_range(
                scalar_anchor.min(scalar_head),
                scalar_anchor.max(scalar_head),
            )
            .unwrap(),
        TypedCommand::SplitBlock => legacy
            .split_block_scalar(scalar_head)
            .unwrap_or_else(|error| panic!("legacy split failed in {label}: {error}")),
        TypedCommand::DeleteAndSplit => legacy
            .delete_and_split_scalar(
                scalar_anchor.min(scalar_head),
                scalar_anchor.max(scalar_head),
            )
            .unwrap_or_else(|error| panic!("legacy delete-and-split failed in {label}: {error}")),
        _ => unreachable!(),
    };
    let result = yrs
        .apply_command(801, command)
        .unwrap_or_else(|error| panic!("Yrs command failed in {label}: {error:?}"))
        .unwrap();
    assert_eq!(
        result.changed,
        legacy.get_json() != document,
        "scenario: {label}"
    );
    assert_eq!(yrs.document_json().unwrap(), legacy.get_json());
    assert_eq!(yrs.document_html().unwrap(), legacy.get_html());
    let Selection::Text { anchor, head } = legacy_update.selection_scalar else {
        panic!("expected legacy scalar text selection")
    };
    let editor_core::yrs_engine::ResolvedSelection::Text {
        anchor: yrs_anchor,
        head: yrs_head,
    } = result.selection
    else {
        panic!("expected Yrs text selection")
    };
    assert_eq!(anchor, yrs_anchor.scalar, "anchor mismatch in {label}");
    assert_eq!(head, yrs_head.scalar, "head mismatch in {label}");
    assert_eq!(legacy.can_undo(), yrs.can_undo());
    let legacy_undo = legacy.undo();
    let yrs_undo = yrs.undo_with_result(802).unwrap();
    assert_eq!(
        legacy_undo.is_some(),
        yrs_undo.is_some(),
        "scenario: {label}"
    );
    if legacy_undo.is_none() {
        return;
    }
    assert_eq!(yrs.document_json().unwrap(), legacy.get_json());
    assert_eq!(yrs.document_html().unwrap(), legacy.get_html());
    assert!(!legacy.can_undo());
    assert!(!yrs.can_undo());
    assert_eq!(legacy.can_redo(), yrs.can_redo());

    let legacy_redo = legacy.redo();
    let yrs_redo = yrs.redo_with_result(803).unwrap();
    assert_eq!(
        legacy_redo.is_some(),
        yrs_redo.is_some(),
        "scenario: {label}"
    );
    assert_eq!(yrs.document_json().unwrap(), legacy.get_json());
    assert_eq!(yrs.document_html().unwrap(), legacy.get_html());
    assert!(!legacy.can_redo());
    assert!(!yrs.can_redo());
}

fn structural_parity_at_doc_cursor(
    label: &str,
    document: serde_json::Value,
    document_position: u32,
    command: TypedCommand,
) {
    let mut position_editor = Editor::new(tiptap_schema(), InterceptorPipeline::new(), false);
    position_editor.set_json(&document).unwrap();
    let scalar = position_editor
        .position_map()
        .doc_to_scalar(document_position, position_editor.document());
    structural_parity(label, document, scalar, scalar, command);
}

#[test]
fn complete_command_contract_routes_structural_work_explicitly() {
    let engine = engine();
    let structural = [
        TypedCommand::ApplyListType {
            list_type: "bulletList".into(),
        },
        TypedCommand::WrapInList {
            list_type: "bulletList".into(),
            item_type: "listItem".into(),
        },
        TypedCommand::UnwrapFromList,
        TypedCommand::IndentListItem,
        TypedCommand::OutdentListItem,
        TypedCommand::ToggleTaskItemChecked,
        TypedCommand::InsertNode {
            node_type: "image".into(),
        },
        TypedCommand::ResizeImage {
            at: point(0),
            width: 10,
            height: 20,
        },
    ];
    for command in structural {
        assert_eq!(
            engine.plan_command(1, command).unwrap(),
            CommandPlan::NotApplicable
        );
    }
}

#[test]
fn structural_command_plan_is_one_concrete_sealed_replacement() {
    let mut engine = engine();
    engine
        .import_json(
            &serde_json::json!({"type":"doc","content":[{"type":"bulletList","content":[
                {"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"one"}]}]},
                {"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"two"}]}]}
            ]}]})
            .to_string(),
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    select_yrs(&mut engine, 900, 8, 8);

    let CommandPlan::Transaction(transaction) = engine
        .plan_command(901, TypedCommand::DeleteBackward)
        .unwrap()
    else {
        panic!("list marker delete must produce a transaction")
    };
    assert!(matches!(
        transaction.operations.as_slice(),
        [TypedOperation::ReplaceStructure(_)]
    ));
}

#[test]
fn exact_text_delete_and_split_keep_concrete_operation_classification() {
    let mut engine = engine();
    engine
        .import_json(
            &serde_json::json!({"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"a😀b"}]}]})
                .to_string(),
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    select_yrs(&mut engine, 910, 2, 2);
    let CommandPlan::Transaction(delete) = engine
        .plan_command(911, TypedCommand::DeleteBackward)
        .unwrap()
    else {
        panic!("unicode delete must plan")
    };
    assert!(matches!(
        delete.operations.as_slice(),
        [TypedOperation::DeleteRange { .. }]
    ));

    let CommandPlan::Transaction(reverse) = engine
        .plan_command(
            914,
            TypedCommand::DeleteRange {
                range: RevisionedRange {
                    from: point(3),
                    to: point(1),
                },
            },
        )
        .unwrap()
    else {
        panic!("reverse public delete range must normalize during planning")
    };
    let [TypedOperation::DeleteRange { range }] = reverse.operations.as_slice() else {
        panic!("normalized reverse delete must stay a concrete DeleteRange")
    };
    assert_eq!((range.from.offset, range.to.offset), (1, 3));

    select_yrs(&mut engine, 912, 1, 1);
    let CommandPlan::Transaction(split) =
        engine.plan_command(913, TypedCommand::SplitBlock).unwrap()
    else {
        panic!("paragraph split must plan")
    };
    assert!(matches!(
        split.operations.as_slice(),
        [TypedOperation::SplitBlock { .. }]
    ));
}

#[test]
fn compatible_command_delete_preserves_collapsed_stored_marks() {
    let mut engine = engine();
    engine
        .import_json(
            &serde_json::json!({"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"ab"}]}]})
                .to_string(),
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    select_yrs(&mut engine, 920, 2, 2);
    engine
        .apply_command(
            921,
            TypedCommand::SetMark {
                mark_type: "italic".into(),
                attrs: Default::default(),
            },
        )
        .unwrap();
    let before = engine.stored_marks().unwrap().to_vec();

    engine
        .apply_command(922, TypedCommand::DeleteBackward)
        .unwrap();
    assert_eq!(engine.stored_marks(), Some(before.as_slice()));
}

#[test]
fn structural_delete_history_charge_uses_the_conservative_live_clock_bound() {
    let editing_limits = EditingLimits {
        max_undo_retained_units: 256,
        ..EditingLimits::default()
    };
    let mut limited = YrsDocumentEngine::new(YrsEngineConfig {
        schema: tiptap_schema(),
        fragment_name: "prosemirror".into(),
        initialization_mode: InitializationMode::LocalEmpty,
        resource_limits: ResourceLimits::default(),
        editing_limits,
        max_length: None,
        scope: None,
    })
    .unwrap();
    let prefix = "x".repeat(8_192);
    limited
        .import_json(
            &serde_json::json!({"type":"doc","content":[
                {"type":"paragraph","content":[{"type":"text","text":prefix}]},
                {"type":"bulletList","content":[
                    {"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"one"}]}]},
                    {"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"two"}]}]}
                ]}
            ]})
            .to_string(),
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    select_yrs(&mut limited, 930, 8_192 + 1 + 8, 8_192 + 1 + 8);

    let before = limited.encoded_state().unwrap();
    let error = limited
        .apply_command(931, TypedCommand::DeleteBackward)
        .unwrap_err();
    assert_eq!(error.code, "OPERATION_LIMIT_EXCEEDED");
    assert_eq!(error.limit, Some(256));
    assert_eq!(error.actual, Some(8_229));
    assert_eq!(limited.encoded_state().unwrap(), before);
    assert!(!limited.can_undo());

    let mut accepted = engine();
    accepted
        .import_json(
            &serde_json::json!({"type":"doc","content":[
                {"type":"paragraph","content":[{"type":"text","text":"x".repeat(8_192)}]},
                {"type":"bulletList","content":[
                    {"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"one"}]}]},
                    {"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"two"}]}]}
                ]}
            ]})
            .to_string(),
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    select_yrs(&mut accepted, 932, 8_192 + 1 + 8, 8_192 + 1 + 8);
    let result = accepted
        .apply_command(933, TypedCommand::DeleteBackward)
        .unwrap()
        .unwrap();
    assert!(result.changed);
    assert!(accepted.can_undo());
}

#[test]
fn insert_text_is_one_command_transaction_and_one_history_group() {
    let mut engine = engine();
    let result = engine
        .apply_command(
            1,
            TypedCommand::InsertText {
                text: "a😀".into()
            },
        )
        .unwrap()
        .unwrap();

    assert!(result.changed);
    assert_eq!(engine.document().unwrap().root().text_content(), "a😀");
    match result.selection {
        editor_core::yrs_engine::ResolvedSelection::Text { anchor, head } => {
            assert_eq!((anchor.scalar, head.scalar), (2, 2));
        }
        selection => panic!("expected text selection, got {selection:?}"),
    }
    assert!(engine.can_undo());
    assert!(engine.undo(2).unwrap().is_some());
    assert_eq!(engine.document().unwrap().root().text_content(), "");
    assert!(!engine.can_undo());
}

#[test]
fn composition_range_replacement_and_stored_mark_insertion_are_one_command_groups() {
    let mut composition = engine();
    composition
        .apply_command(
            1,
            TypedCommand::InsertText {
                text: "a😀b".into(),
            },
        )
        .unwrap();
    select_yrs(&mut composition, 2, 3, 1);
    let result = composition
        .apply_command(3, TypedCommand::ReplaceSelectionText { text: "é".into() })
        .unwrap()
        .unwrap();
    assert_eq!(composition.document().unwrap().root().text_content(), "aé");
    let editor_core::yrs_engine::ResolvedSelection::Text { anchor, head } = result.selection else {
        panic!("expected text selection");
    };
    assert_eq!((anchor.scalar, head.scalar), (2, 2));
    composition.undo(4).unwrap().unwrap();
    assert_eq!(
        composition.document().unwrap().root().text_content(),
        "a😀b"
    );

    let mut marked = engine();
    marked
        .apply_command(
            5,
            TypedCommand::ToggleMark {
                mark_type: "bold".into(),
            },
        )
        .unwrap();
    marked
        .apply_command(6, TypedCommand::InsertText { text: "x".into() })
        .unwrap();
    assert_eq!(
        marked
            .document()
            .unwrap()
            .root()
            .child(0)
            .unwrap()
            .child(0)
            .unwrap()
            .marks()[0]
            .mark_type(),
        "bold"
    );
}

#[test]
fn json_and_html_content_match_legacy_empty_block_and_selection_semantics() {
    let schema = tiptap_schema();
    let cases = [
        (
            TypedCommand::InsertContentJson {
                json: serde_json::json!({"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"json"}]}]}),
            },
            false,
            false,
        ),
        (
            TypedCommand::InsertContentHtml {
                html: "<p><strong>html</strong><br></p>".into(),
            },
            true,
            true,
        ),
    ];
    for (index, (command, html, artifact)) in cases.into_iter().enumerate() {
        let mut legacy = Editor::new(schema.clone(), InterceptorPipeline::new(), false);
        let mut yrs = engine();
        let legacy_update = match &command {
            TypedCommand::InsertContentJson { json } => legacy.insert_content_json(json).unwrap(),
            TypedCommand::InsertContentHtml { html } => legacy.insert_content_html(html).unwrap(),
            _ => unreachable!(),
        };
        let result = yrs
            .apply_command(10 + index as u64, command)
            .unwrap()
            .unwrap();
        assert_eq!(yrs.document_json().unwrap(), legacy.get_json());
        assert_strict_selection_normalization(
            &legacy,
            &legacy_update.selection_scalar,
            &result.selection,
            &yrs,
            artifact,
        );
        assert_eq!(
            result.history_state.can_undo,
            legacy_update.history_state.can_undo
        );
        if html {
            assert!(yrs
                .document_html()
                .unwrap()
                .contains("<strong>html</strong>"));
        }
    }
}

#[test]
fn max_length_rejects_before_authoritative_command_mutation() {
    let schema = tiptap_schema();
    let mut pipeline = InterceptorPipeline::new();
    pipeline.add(Box::new(MaxLength::new(1)));
    let mut legacy = Editor::new(schema.clone(), pipeline, false);
    let mut yrs = YrsDocumentEngine::new(YrsEngineConfig {
        schema,
        fragment_name: "prosemirror".into(),
        initialization_mode: InitializationMode::LocalEmpty,
        resource_limits: ResourceLimits::default(),
        editing_limits: EditingLimits::default(),
        max_length: Some(1),
        scope: None,
    })
    .unwrap();
    assert!(legacy.insert_text_scalar(0, "ab").is_err());
    let before = yrs.encoded_state().unwrap();
    assert!(yrs
        .apply_command(1, TypedCommand::InsertText { text: "ab".into() })
        .is_err());
    assert_eq!(yrs.encoded_state().unwrap(), before);
    assert_eq!(yrs.revision(), 0);
}

#[test]
fn content_admission_rejects_invalid_and_oversized_inputs_before_mutation() {
    let limits = ResourceLimits {
        max_input_bytes: 64,
        ..ResourceLimits::default()
    };
    let mut yrs = YrsDocumentEngine::new(YrsEngineConfig {
        schema: tiptap_schema(),
        fragment_name: "prosemirror".into(),
        initialization_mode: InitializationMode::LocalEmpty,
        resource_limits: limits,
        editing_limits: EditingLimits::default(),
        max_length: None,
        scope: None,
    })
    .unwrap();
    let before = (
        yrs.encoded_state().unwrap(),
        yrs.revision(),
        yrs.state_revision(),
    );
    let invalid = yrs
        .apply_command(
            1,
            TypedCommand::InsertContentJson {
                json: serde_json::json!({"type":"doc","content":"invalid"}),
            },
        )
        .unwrap_err();
    assert_eq!(invalid.code, "DOCUMENT_INVALID");
    let oversized = yrs
        .apply_command(
            2,
            TypedCommand::InsertContentHtml {
                html: format!("<p>{}</p>", "x".repeat(128)),
            },
        )
        .unwrap_err();
    assert_eq!(oversized.code, "DOCUMENT_LIMIT_EXCEEDED");
    assert_eq!(
        before,
        (
            yrs.encoded_state().unwrap(),
            yrs.revision(),
            yrs.state_revision()
        )
    );
}

#[test]
fn delete_command_structural_matrix_matches_legacy() {
    let scenarios = [
        (
            serde_json::json!({"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"a😀b"}]}]}),
            2,
            2,
            TypedCommand::DeleteBackward,
        ),
        (
            serde_json::json!({"type":"doc","content":[{"type":"bulletList","content":[{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"one"}]}]},{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"two"}]}]}]}]}),
            8,
            8,
            TypedCommand::DeleteBackward,
        ),
        (
            serde_json::json!({"type":"doc","content":[{"type":"bulletList","content":[{"type":"listItem","content":[{"type":"paragraph"}]}]}]}),
            2,
            2,
            TypedCommand::DeleteBackward,
        ),
        (
            serde_json::json!({"type":"doc","content":[{"type":"blockquote","content":[{"type":"paragraph"}]}]}),
            0,
            0,
            TypedCommand::DeleteBackward,
        ),
        (
            serde_json::json!({"type":"doc","content":[{"type":"h2"}]}),
            0,
            0,
            TypedCommand::DeleteBackward,
        ),
        (
            serde_json::json!({"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"ab"}]},{"type":"paragraph","content":[{"type":"text","text":"cd"}]}]}),
            1,
            4,
            TypedCommand::DeleteRange {
                range: RevisionedRange {
                    from: point(1),
                    to: point(4),
                },
            },
        ),
    ];
    for (index, (document, anchor, head, command)) in scenarios.into_iter().enumerate() {
        structural_parity(&format!("delete-{index}"), document, anchor, head, command);
    }

    for (label, document, document_position) in [
        (
            "delete-empty-list-middle",
            serde_json::json!({"type":"doc","content":[{"type":"bulletList","content":[
                {"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"one"}]}]},
                {"type":"listItem","content":[{"type":"paragraph"}]},
                {"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"three"}]}]}
            ]}]}),
            10,
        ),
        (
            "delete-empty-list-last",
            serde_json::json!({"type":"doc","content":[{"type":"bulletList","content":[
                {"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"one"}]}]},
                {"type":"listItem","content":[{"type":"paragraph"}]}
            ]}]}),
            10,
        ),
        (
            "delete-trailing-empty-list-block",
            serde_json::json!({"type":"doc","content":[{"type":"bulletList","content":[
                {"type":"listItem","content":[
                    {"type":"paragraph","content":[{"type":"text","text":"one"}]},
                    {"type":"paragraph"}
                ]}
            ]}]}),
            8,
        ),
        (
            "delete-nested-empty-list-item",
            serde_json::json!({"type":"doc","content":[{"type":"bulletList","content":[
                {"type":"listItem","content":[
                    {"type":"paragraph","content":[{"type":"text","text":"parent"}]},
                    {"type":"bulletList","content":[{"type":"listItem","content":[{"type":"paragraph"}]}]}
                ]}
            ]}]}),
            13,
        ),
        (
            "delete-void-and-empty-block",
            serde_json::json!({"type":"doc","content":[
                {"type":"image","attrs":{"src":"x","alt":null,"title":null,"width":null,"height":null}},
                {"type":"paragraph"}
            ]}),
            2,
        ),
        (
            "delete-empty-block-after-text",
            serde_json::json!({"type":"doc","content":[
                {"type":"paragraph","content":[{"type":"text","text":"a"}]},
                {"type":"paragraph"}
            ]}),
            4,
        ),
    ] {
        structural_parity_at_doc_cursor(
            label,
            document,
            document_position,
            TypedCommand::DeleteBackward,
        );
    }

    structural_parity(
        "delete-reverse-cross-block-selection",
        serde_json::json!({"type":"doc","content":[
            {"type":"paragraph","content":[{"type":"text","text":"ab"}]},
            {"type":"paragraph","content":[{"type":"text","text":"cd"}]}
        ]}),
        4,
        1,
        TypedCommand::DeleteRange {
            range: RevisionedRange {
                from: point(4),
                to: point(1),
            },
        },
    );
}

#[test]
fn split_command_structural_matrix_matches_legacy() {
    let scenarios = [
        (
            serde_json::json!({"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"ab"}]}]}),
            1,
            1,
            TypedCommand::SplitBlock,
        ),
        (
            serde_json::json!({"type":"doc","content":[{"type":"blockquote","content":[{"type":"paragraph"}]}]}),
            0,
            0,
            TypedCommand::SplitBlock,
        ),
        (
            serde_json::json!({"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abcd"}]}]}),
            1,
            3,
            TypedCommand::DeleteAndSplit,
        ),
    ];
    for (index, (document, anchor, head, command)) in scenarios.into_iter().enumerate() {
        structural_parity(&format!("split-{index}"), document, anchor, head, command);
    }

    for (label, document, document_position) in [
        (
            "split-empty-list-middle",
            serde_json::json!({"type":"doc","content":[{"type":"bulletList","content":[
                {"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"one"}]}]},
                {"type":"listItem","content":[{"type":"paragraph"}]},
                {"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"three"}]}]}
            ]}]}),
            10,
        ),
        (
            "split-empty-list-last",
            serde_json::json!({"type":"doc","content":[{"type":"bulletList","content":[
                {"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"one"}]}]},
                {"type":"listItem","content":[{"type":"paragraph"}]}
            ]}]}),
            10,
        ),
        (
            "split-nested-empty-list-item",
            serde_json::json!({"type":"doc","content":[{"type":"bulletList","content":[
                {"type":"listItem","content":[
                    {"type":"paragraph","content":[{"type":"text","text":"parent"}]},
                    {"type":"bulletList","content":[{"type":"listItem","content":[{"type":"paragraph"}]}]}
                ]}
            ]}]}),
            13,
        ),
    ] {
        structural_parity_at_doc_cursor(
            label,
            document,
            document_position,
            TypedCommand::SplitBlock,
        );
    }

    let code_schema = Schema::from_json(&serde_json::json!({
        "nodes": [
            { "name": "doc", "content": "block+", "role": "doc" },
            { "name": "paragraph", "content": "text*", "group": "block", "role": "textBlock", "htmlTag": "p" },
            { "name": "codeBlock", "content": "text*", "group": "block", "role": "textBlock", "htmlTag": "pre" },
            { "name": "text", "content": "", "role": "text" }
        ],
        "marks": []
    }))
    .unwrap();
    for (label, document, scalar) in [
        (
            "split-code-newline",
            serde_json::json!({"type":"doc","content":[{"type":"codeBlock","content":[{"type":"text","text":"ab"}]}]}),
            2,
        ),
        (
            "split-code-exit",
            serde_json::json!({"type":"doc","content":[{"type":"codeBlock","content":[{"type":"text","text":"ab\n"}]}]}),
            3,
        ),
    ] {
        structural_parity_with_schema(
            label,
            code_schema.clone(),
            document,
            scalar,
            scalar,
            TypedCommand::SplitBlock,
        );
    }
}
