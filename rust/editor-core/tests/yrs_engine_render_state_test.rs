use editor_core::boundary::ResourceLimits;
use editor_core::model::{Document, Fragment, Mark, Node};
use editor_core::render::incremental::{
    render_blocks, safe_contiguous_render_blocks_patch, RenderBlocksPatch,
};
use editor_core::render::RenderElement;
use editor_core::schema::{NodeRole, Schema};
use editor_core::tiptap_schema;
use editor_core::yrs_engine::{
    Affinity, EditingLimits, EditorOffsetKind, HistoryPolicy, InitializationMode, RenderUpdate,
    RevisionedPosition, SelectionInput, SelectionIntent, TransactionOrigin, TypedOperation,
    TypedTransaction, YrsDocumentEngine, YrsEngineConfig,
};

fn engine(editing_limits: EditingLimits) -> YrsDocumentEngine {
    YrsDocumentEngine::new(YrsEngineConfig {
        schema: tiptap_schema(),
        fragment_name: "prosemirror".into(),
        initialization_mode: InitializationMode::LocalEmpty,
        resource_limits: ResourceLimits::default(),
        editing_limits,
        max_length: None,
        scope: None,
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

fn transaction(
    engine: &YrsDocumentEngine,
    request_id: u64,
    operations: Vec<TypedOperation>,
    selection_intent: SelectionIntent,
    history_policy: HistoryPolicy,
) -> TypedTransaction {
    TypedTransaction {
        request_id,
        base_document_revision: engine.revision(),
        origin: TransactionOrigin::LocalApi,
        operations,
        selection_intent,
        history_policy,
    }
}

fn apply_patch(
    mut old: Vec<Vec<editor_core::render::RenderElement>>,
    patch: &RenderBlocksPatch,
) -> Vec<Vec<editor_core::render::RenderElement>> {
    old.splice(
        patch.start_index..patch.start_index + patch.delete_count,
        patch.blocks.clone(),
    );
    old
}

fn rendered_text(document: &Document, schema: &Schema) -> String {
    let elements =
        editor_core::render::incremental::flatten_render_blocks(&render_blocks(document, schema));
    let mut text = String::new();
    let mut pending_prefix = String::new();
    let mut started_block = false;
    let begin_block = |text: &mut String, started: &mut bool| {
        if *started {
            text.push('\n');
        }
        *started = true;
    };
    for element in elements {
        match element {
            RenderElement::BlockStart {
                node_type,
                list_context,
                ..
            } => {
                if let Some(context) = list_context {
                    pending_prefix =
                        editor_core::render::list_marker_string(context.ordered, context.index);
                }
                if schema
                    .node(&node_type)
                    .is_some_and(|spec| matches!(spec.role, NodeRole::TextBlock))
                {
                    begin_block(&mut text, &mut started_block);
                    text.push_str(&pending_prefix);
                    pending_prefix.clear();
                }
            }
            RenderElement::TextRun { text: value, .. } => text.push_str(&value),
            RenderElement::VoidInline { .. } => text.push('\n'),
            RenderElement::VoidBlock { .. } => {
                begin_block(&mut text, &mut started_block);
                text.push('\u{fffc}');
            }
            RenderElement::OpaqueInlineAtom {
                node_type, label, ..
            } => text.push_str(&editor_core::render::opaque_atom_visible_string(
                &node_type, &label,
            )),
            RenderElement::OpaqueBlockAtom {
                node_type, label, ..
            } => {
                begin_block(&mut text, &mut started_block);
                text.push_str(&editor_core::render::opaque_atom_visible_string(
                    &node_type, &label,
                ));
            }
            RenderElement::BlockEnd => {}
        }
    }
    text
}

fn range(from: u32, to: u32) -> editor_core::yrs_engine::RevisionedRange {
    editor_core::yrs_engine::RevisionedRange {
        from: point(from),
        to: point(to),
    }
}

fn source_for_operation(kind: usize) -> serde_json::Value {
    match kind {
        4 => {
            serde_json::json!({"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","marks":[{"type":"bold"}],"text":"abcdef"}]}]})
        }
        5 => {
            serde_json::json!({"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","marks":[{"type":"link","attrs":{"href":"old"}}],"text":"abcdef"}]}]})
        }
        7 => {
            serde_json::json!({"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"ab"}]},{"type":"paragraph","content":[{"type":"text","text":"cd"}]}]})
        }
        8 => {
            serde_json::json!({"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"one"}]}]})
        }
        9 => {
            serde_json::json!({"type":"doc","content":[{"type":"bulletList","content":[{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"one"}]}]}]}]})
        }
        10 => {
            serde_json::json!({"type":"doc","content":[{"type":"bulletList","content":[{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"one"}]}]},{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"two"}]}]}]}]})
        }
        11 => {
            serde_json::json!({"type":"doc","content":[{"type":"bulletList","content":[{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"outer"}]},{"type":"bulletList","content":[{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"inner"}]}]}]}]}]}]})
        }
        13 => {
            serde_json::json!({"type":"doc","content":[{"type":"image","attrs":{"src":"old","alt":null,"title":null,"width":null,"height":null}}]})
        }
        _ => {
            serde_json::json!({"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abcdef"}]}]})
        }
    }
}

fn operation_for_kind(kind: usize, rendered: &str) -> TypedOperation {
    let scalar_index = |needle: &str| {
        u32::try_from(rendered[..rendered.find(needle).unwrap()].chars().count()).unwrap()
    };
    match kind {
        0 => TypedOperation::InsertText {
            at: point(3),
            text: "x".into(),
            marks: vec![],
        },
        1 => TypedOperation::DeleteRange { range: range(1, 4) },
        2 => TypedOperation::ReplaceRange {
            range: range(2, 4),
            content: Fragment::from(vec![Node::text("XY".into(), vec![])]),
        },
        3 => TypedOperation::AddMark {
            range: range(1, 5),
            mark: Mark::new("bold".into(), std::collections::HashMap::new()),
        },
        4 => TypedOperation::RemoveMark {
            range: range(1, 5),
            mark_type: "bold".into(),
        },
        5 => TypedOperation::ReplaceMark {
            range: range(1, 5),
            mark: Mark::new(
                "link".into(),
                std::collections::HashMap::from([("href".into(), serde_json::json!("new"))]),
            ),
        },
        6 => TypedOperation::SplitBlock {
            at: point(3),
            node_type: "paragraph".into(),
            attrs: std::collections::HashMap::new(),
        },
        7 => TypedOperation::JoinBlocks { at: point(2) },
        8 => TypedOperation::WrapInList {
            range: range(0, 3),
            list_type: "bulletList".into(),
            item_type: "listItem".into(),
            attrs: std::collections::HashMap::new(),
            item_attrs: std::collections::HashMap::new(),
        },
        9 => TypedOperation::UnwrapFromList {
            at: point(scalar_index("one") + 1),
        },
        10 => TypedOperation::IndentListItem {
            at: point(scalar_index("two") + 1),
        },
        11 => TypedOperation::OutdentListItem {
            at: point(scalar_index("inner") + 1),
        },
        12 => TypedOperation::InsertNode {
            at: point(3),
            node: Node::void("hardBreak".into(), std::collections::HashMap::new()),
        },
        13 => TypedOperation::UpdateNodeAttrs {
            at: point(0),
            attrs: std::collections::HashMap::from([
                ("src".into(), serde_json::json!("new")),
                ("alt".into(), serde_json::json!("trace")),
                ("title".into(), serde_json::Value::Null),
                ("width".into(), serde_json::Value::Null),
                ("height".into(), serde_json::Value::Null),
            ]),
        },
        _ => unreachable!(),
    }
}

#[test]
fn every_typed_operation_result_reconstructs_the_full_render() {
    let schema = tiptap_schema();

    for kind in 0..14 {
        let mut engine = engine(EditingLimits::default());
        engine
            .import_json(
                &source_for_operation(kind).to_string(),
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        let old_blocks = render_blocks(engine.document().unwrap(), &schema);
        let rendered = rendered_text(engine.document().unwrap(), &schema);

        let result = engine
            .apply_typed_transaction_with_result(transaction(
                &engine,
                100 + kind as u64,
                vec![operation_for_kind(kind, &rendered)],
                SelectionIntent::UseOperationResult,
                HistoryPolicy::Skip,
            ))
            .unwrap_or_else(|error| panic!("operation kind {kind} failed: {error:?}"));

        assert!(result.changed, "operation kind {kind} should change state");
        let expected = render_blocks(engine.document().unwrap(), &schema);
        match &result.render_update {
            RenderUpdate::Patch(patch) => assert_eq!(
                apply_patch(old_blocks, patch),
                expected,
                "operation kind {kind} produced an invalid patch"
            ),
            RenderUpdate::Full(blocks) => assert_eq!(
                blocks, &expected,
                "operation kind {kind} produced an invalid full render"
            ),
            RenderUpdate::None => {
                panic!("operation kind {kind} changed the document without a render update")
            }
        }
    }
}

#[test]
fn durable_result_reports_patch_and_reconstructs_the_full_render() {
    let mut engine = engine(EditingLimits::default());
    let schema = tiptap_schema();
    let before = engine.document().unwrap().clone();
    let old_blocks = render_blocks(&before, &schema);

    let result = engine
        .apply_typed_transaction_with_result(transaction(
            &engine,
            41,
            vec![TypedOperation::InsertText {
                at: point(1),
                text: "hello".into(),
                marks: vec![],
            }],
            SelectionIntent::UseOperationResult,
            HistoryPolicy::Boundary,
        ))
        .unwrap();

    assert_eq!(result.request_id, 41);
    assert_eq!(result.origin, TransactionOrigin::LocalApi);
    assert!(result.changed);
    assert_eq!(result.document_revision, 1);
    assert_eq!(result.state_revision, 1);
    assert!(matches!(
        result.selection,
        editor_core::yrs_engine::ResolvedSelection::Text { .. }
    ));
    assert_eq!(result.active_state.marks.get("bold"), Some(&false));
    assert!(result.history_state.can_undo);
    assert!(!result.history_state.can_redo);

    let RenderUpdate::Patch(patch) = &result.render_update else {
        panic!("durable local edit should produce a safe patch")
    };
    let expected = render_blocks(engine.document().unwrap(), &schema);
    assert_eq!(apply_patch(old_blocks, patch), expected);
}

#[test]
fn selection_only_and_complete_no_op_results_emit_no_render_update() {
    let mut engine = engine(EditingLimits::default());
    let selected = engine
        .apply_typed_transaction_with_result(transaction(
            &engine,
            42,
            vec![],
            SelectionIntent::Set(SelectionInput::All),
            HistoryPolicy::Skip,
        ))
        .unwrap();
    assert!(selected.changed);
    assert_eq!(selected.document_revision, 0);
    assert_eq!(selected.state_revision, 1);
    assert!(matches!(
        selected.selection,
        editor_core::yrs_engine::ResolvedSelection::All
    ));
    assert!(matches!(selected.render_update, RenderUpdate::None));
    assert!(selected.active_state.allowed_marks.is_empty());

    let no_op = engine
        .apply_typed_transaction_with_result(transaction(
            &engine,
            43,
            vec![],
            SelectionIntent::Preserve,
            HistoryPolicy::Skip,
        ))
        .unwrap();
    assert!(!no_op.changed);
    assert_eq!(no_op.document_revision, 0);
    assert_eq!(no_op.state_revision, 1);
    assert!(matches!(no_op.render_update, RenderUpdate::None));
}

#[test]
fn undo_result_has_undo_redo_state_and_reconstructable_patch() {
    let mut engine = engine(EditingLimits::default());
    engine
        .apply_typed_transaction_with_result(transaction(
            &engine,
            44,
            vec![TypedOperation::InsertText {
                at: point(1),
                text: "x".into(),
                marks: vec![],
            }],
            SelectionIntent::UseOperationResult,
            HistoryPolicy::Boundary,
        ))
        .unwrap();
    let schema = tiptap_schema();
    let old_blocks = render_blocks(engine.document().unwrap(), &schema);

    let result = engine.undo_with_result(45).unwrap().unwrap();
    assert_eq!(result.request_id, 45);
    assert_eq!(result.origin, TransactionOrigin::UndoRedo);
    assert!(result.changed);
    assert_eq!(result.document_revision, 2);
    assert_eq!(result.state_revision, 2);
    assert!(!result.history_state.can_undo);
    assert!(result.history_state.can_redo);
    let RenderUpdate::Patch(patch) = &result.render_update else {
        panic!("undo should produce a safe patch")
    };
    assert_eq!(
        apply_patch(old_blocks, patch),
        render_blocks(engine.document().unwrap(), &schema)
    );

    let old_blocks = render_blocks(engine.document().unwrap(), &schema);
    let redo = engine.redo_with_result(47).unwrap().unwrap();
    assert_eq!(redo.origin, TransactionOrigin::UndoRedo);
    assert!(redo.history_state.can_undo);
    assert!(!redo.history_state.can_redo);
    let RenderUpdate::Patch(patch) = &redo.render_update else {
        panic!("redo should produce a safe patch")
    };
    assert_eq!(
        apply_patch(old_blocks, patch),
        render_blocks(engine.document().unwrap(), &schema)
    );
}

#[test]
fn invalid_render_hint_falls_back_to_the_complete_new_render() {
    let schema = tiptap_schema();
    let paragraph = |text: &str| {
        Node::element(
            "paragraph".into(),
            std::collections::HashMap::new(),
            Fragment::from(vec![Node::text(text.into(), vec![])]),
        )
    };
    let document = |text: &str| {
        Document::new(Node::element(
            "doc".into(),
            std::collections::HashMap::new(),
            Fragment::from(vec![paragraph(text)]),
        ))
    };
    let old = document("old");
    let new = document("new");
    let full = safe_contiguous_render_blocks_patch(&old, &new, &schema, &[usize::MAX])
        .expect_err("unprovable compiler hint must force Full");
    assert_eq!(full, render_blocks(&new, &schema));
}

#[test]
fn stored_mark_only_result_changes_local_state_without_rendering() {
    let mut engine = engine(EditingLimits::default());
    let caret = point(1);
    let result = engine
        .apply_typed_transaction_with_result(transaction(
            &engine,
            48,
            vec![TypedOperation::AddMark {
                range: editor_core::yrs_engine::RevisionedRange {
                    from: caret,
                    to: caret,
                },
                mark: Mark::new("bold".into(), std::collections::HashMap::new()),
            }],
            SelectionIntent::Preserve,
            HistoryPolicy::Skip,
        ))
        .unwrap();
    assert!(result.changed);
    assert_eq!(result.document_revision, 0);
    assert_eq!(result.state_revision, 1);
    assert_eq!(result.active_state.marks.get("bold"), Some(&true));
    assert!(matches!(result.render_update, RenderUpdate::None));
}

#[test]
fn derived_result_budget_exact_boundary_passes_and_one_under_is_atomic() {
    let transaction_for = |engine: &YrsDocumentEngine| {
        transaction(
            engine,
            46,
            vec![TypedOperation::InsertText {
                at: point(1),
                text: "budget".into(),
                marks: vec![],
            }],
            SelectionIntent::UseOperationResult,
            HistoryPolicy::Skip,
        )
    };
    let mut measuring = engine(EditingLimits::default());
    let measured = measuring
        .apply_typed_transaction_with_result(transaction_for(&measuring))
        .unwrap()
        .derived_output_bytes();

    let exact_limits = EditingLimits {
        max_derived_output_bytes: measured,
        ..EditingLimits::default()
    };
    let mut exact = engine(exact_limits);
    exact
        .apply_typed_transaction_with_result(transaction_for(&exact))
        .unwrap();

    let one_under_limits = EditingLimits {
        max_derived_output_bytes: measured - 1,
        ..EditingLimits::default()
    };
    let mut one_under = engine(one_under_limits);
    let before_state = one_under.encoded_state().unwrap();
    let error = one_under
        .apply_typed_transaction_with_result(transaction_for(&one_under))
        .unwrap_err();
    assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
    assert_eq!(error.details.unwrap()["field"], "maxDerivedOutputBytes");
    assert_eq!(one_under.revision(), 0);
    assert_eq!(one_under.state_revision(), 0);
    assert_eq!(one_under.encoded_state().unwrap(), before_state);
}
