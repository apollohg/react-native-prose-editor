use editor_core::boundary::ResourceLimits;
use editor_core::editor::Editor;
use editor_core::intercept::{InterceptError, Interceptor, InterceptorPipeline};
use editor_core::schema::Schema;
use editor_core::selection::Selection;
use editor_core::tiptap_schema;
use editor_core::transform::{Source, Transaction};
use editor_core::yrs_engine::{
    Affinity, EditingLimits, EditorOffsetKind, HistoryPolicy, InitializationMode, RenderUpdate,
    ResolvedSelection, RevisionedPosition, SelectionInput, SelectionIntent, TransactionOrigin,
    TypedCommand, TypedTransaction, TypedTransactionResult, YrsDocumentEngine, YrsEngineConfig,
};
use std::sync::{Arc, Mutex};

struct SourceRecorder {
    observed: Arc<Mutex<Vec<Source>>>,
}

impl Interceptor for SourceRecorder {
    fn intercept(
        &self,
        transaction: Transaction,
        _document: &editor_core::model::Document,
    ) -> Result<Transaction, InterceptError> {
        self.observed
            .lock()
            .unwrap()
            .push(transaction.source.clone());
        Ok(transaction)
    }

    fn sources(&self) -> &[Source] {
        const SOURCES: &[Source] = &[
            Source::Input,
            Source::Paste,
            Source::Format,
            Source::Api,
            Source::History,
            Source::Reconciliation,
        ];
        SOURCES
    }
}

fn point(offset: u32) -> RevisionedPosition {
    RevisionedPosition {
        offset,
        kind: EditorOffsetKind::Scalar,
        affinity: Affinity::Before,
    }
}

fn select(engine: &mut YrsDocumentEngine, request_id: u64, anchor: u32, head: u32) {
    engine
        .apply_typed_transaction(TypedTransaction {
            request_id,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: vec![],
            selection_intent: SelectionIntent::Set(SelectionInput::Text {
                anchor: point(anchor),
                head: point(head),
            }),
            history_policy: HistoryPolicy::Skip,
        })
        .unwrap();
}

fn apply_legacy(editor: &mut Editor, command: &TypedCommand) {
    match command {
        TypedCommand::ApplyListType { list_type } => {
            editor.apply_list_type(list_type).unwrap();
        }
        TypedCommand::WrapInList {
            list_type,
            item_type: _,
        } => {
            let from = editor.selection().from(editor.document());
            let to = editor.selection().to(editor.document());
            editor.wrap_in_list(from, to, list_type).unwrap();
        }
        TypedCommand::UnwrapFromList => {
            let pos = editor.selection().from(editor.document());
            editor.unwrap_from_list(pos).unwrap();
        }
        TypedCommand::IndentListItem => {
            editor.indent_list_item().unwrap();
        }
        TypedCommand::OutdentListItem => {
            editor.outdent_list_item().unwrap();
        }
        TypedCommand::ToggleTaskItemChecked => {
            editor.toggle_task_item_checked().unwrap();
        }
        TypedCommand::InsertNode { node_type } => {
            editor.insert_node_at_selection(node_type).unwrap();
        }
        TypedCommand::ResizeImage { at, width, height } => {
            let doc_pos = editor.scalar_to_doc(at.offset);
            editor
                .resize_image_at_doc_pos(doc_pos, *width, *height)
                .unwrap();
        }
        _ => panic!("non-structural command in structural parity fixture"),
    }
}

fn rendered_text(document: &editor_core::model::Document, schema: &Schema) -> String {
    use editor_core::render::RenderElement;
    use editor_core::schema::NodeRole;

    let blocks = editor_core::render::incremental::render_blocks(document, schema);
    let mut text = String::new();
    let mut pending_prefix = String::new();
    let mut started_block = false;
    for element in editor_core::render::incremental::flatten_render_blocks(&blocks) {
        match element {
            RenderElement::BlockStart {
                node_type,
                list_context,
                ..
            } => {
                if let Some(context) = list_context {
                    pending_prefix = if context.kind.as_deref() == Some("task") {
                        editor_core::render::task_list_marker_string(
                            context.checked.unwrap_or(false),
                        )
                    } else {
                        editor_core::render::list_marker_string(context.ordered, context.index)
                    };
                }
                if schema
                    .node(&node_type)
                    .is_some_and(|spec| matches!(spec.role, NodeRole::TextBlock))
                {
                    if started_block {
                        text.push('\n');
                    }
                    started_block = true;
                    text.push_str(&pending_prefix);
                    pending_prefix.clear();
                }
            }
            RenderElement::TextRun { text: value, .. } => text.push_str(&value),
            RenderElement::VoidInline { .. } => text.push('\n'),
            RenderElement::VoidBlock { .. } => {
                if started_block {
                    text.push('\n');
                }
                started_block = true;
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
                if started_block {
                    text.push('\n');
                }
                started_block = true;
                text.push_str(&editor_core::render::opaque_atom_visible_string(
                    &node_type, &label,
                ));
            }
            RenderElement::BlockEnd => {}
        }
    }
    text
}

fn assert_selection_parity(label: &str, legacy: &Editor, yrs: &ResolvedSelection, schema: &Schema) {
    let rendered = rendered_text(legacy.document(), schema);
    let assert_point =
        |side: &str, document: u32, point: &editor_core::yrs_engine::ResolvedPoint| {
            let scalar = legacy.doc_to_scalar(document);
            assert_eq!(point.document, document, "{label}: {side} document");
            assert_eq!(point.scalar, scalar, "{label}: {side} scalar");
            assert_eq!(
                point.utf16,
                editor_core::yrs_engine::scalar_offset_to_utf16(&rendered, scalar).unwrap(),
                "{label}: {side} utf16"
            );
        };
    match (legacy.selection(), yrs) {
        (
            Selection::Text { anchor, head },
            ResolvedSelection::Text {
                anchor: yrs_anchor,
                head: yrs_head,
            },
        ) => {
            assert_point("anchor", *anchor, yrs_anchor);
            assert_point("head", *head, yrs_head);
        }
        (Selection::Node { pos }, ResolvedSelection::Node { at }) => {
            assert_point("node", *pos, at);
        }
        (Selection::All, ResolvedSelection::All) => {}
        pair => panic!("{label}: selection variant/direction mismatch: {pair:?}"),
    }
}

fn assert_render_reconstructs(
    label: &str,
    before: &[Vec<editor_core::render::RenderElement>],
    result: &TypedTransactionResult,
    after: &[Vec<editor_core::render::RenderElement>],
) {
    match &result.render_update {
        RenderUpdate::Patch(patch) => {
            let mut reconstructed = before.to_vec();
            reconstructed.splice(
                patch.start_index..patch.start_index + patch.delete_count,
                patch.blocks.clone(),
            );
            assert_eq!(reconstructed, after, "{label}: render patch reconstruction");
        }
        RenderUpdate::None => assert_eq!(before, after, "{label}: missing render patch"),
        RenderUpdate::Full(_) => panic!("{label}: localized command degraded to full render"),
    }
}

fn assert_document_state(label: &str, legacy: &Editor, yrs: &YrsDocumentEngine, schema: &Schema) {
    assert_eq!(
        yrs.document_json().unwrap(),
        legacy.get_json(),
        "{label}: json"
    );
    assert_eq!(
        yrs.document_html().unwrap(),
        legacy.get_html(),
        "{label}: html"
    );
    assert_eq!(yrs.can_undo(), legacy.can_undo(), "{label}: canUndo");
    assert_eq!(yrs.can_redo(), legacy.can_redo(), "{label}: canRedo");
    assert_selection_parity(label, legacy, yrs.resolved_selection().unwrap(), schema);
}

fn assert_parity(
    label: &str,
    document: serde_json::Value,
    anchor: u32,
    head: u32,
    command: TypedCommand,
) {
    assert_parity_with_schema(label, tiptap_schema(), document, anchor, head, command);
}

fn assert_parity_with_schema(
    label: &str,
    schema: Schema,
    document: serde_json::Value,
    anchor: u32,
    head: u32,
    command: TypedCommand,
) {
    let mut legacy = Editor::new(schema.clone(), InterceptorPipeline::new(), false);
    legacy.set_json(&document).unwrap();
    legacy.set_selection_scalar(anchor, head);

    let mut yrs = YrsDocumentEngine::new(YrsEngineConfig {
        schema: schema.clone(),
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
    select(&mut yrs, 1, anchor, head);

    if let editor_core::yrs_engine::ResolvedSelection::Text { anchor, head } =
        yrs.resolved_selection().unwrap()
    {
        legacy.set_selection_scalar(anchor.scalar, head.scalar);
    }

    apply_legacy(&mut legacy, &command);
    let result = yrs
        .apply_command(2, command)
        .unwrap_or_else(|error| panic!("{label}: {error:?}"));
    assert!(
        result.is_some(),
        "{label}: command unexpectedly not applicable"
    );
    let result = result.unwrap();
    assert_eq!(yrs.document_json().unwrap(), legacy.get_json(), "{label}");
    assert_eq!(yrs.document_html().unwrap(), legacy.get_html(), "{label}");
    assert_eq!(
        result.active_state,
        legacy.get_selection_state().active_state,
        "{label}"
    );
    assert_eq!(result.history_state.can_undo, legacy.can_undo(), "{label}");
    assert_eq!(result.history_state.can_redo, legacy.can_redo(), "{label}");
    let rendered =
        editor_core::render::incremental::render_blocks(yrs.document().unwrap(), &schema);
    match &result.render_update {
        editor_core::yrs_engine::RenderUpdate::Patch(patch) => {
            let mut before = editor_core::render::incremental::render_blocks(
                &editor_core::serialize::from_prosemirror_json(
                    &document,
                    &schema,
                    editor_core::serialize::UnknownTypeMode::Preserve,
                )
                .unwrap(),
                &schema,
            );
            before.splice(
                patch.start_index..patch.start_index + patch.delete_count,
                patch.blocks.clone(),
            );
            assert_eq!(before, rendered, "{label}: render patch reconstruction");
        }
        editor_core::yrs_engine::RenderUpdate::None => {
            assert!(
                !result.changed,
                "{label}: changed command omitted render update"
            );
        }
        editor_core::yrs_engine::RenderUpdate::Full(_) => {
            panic!("{label}: localized structural command degraded to full render")
        }
    }

    assert_selection_parity(label, &legacy, &result.selection, &schema);
    let before_undo = rendered.clone();
    let legacy_undo = legacy.undo();
    let yrs_undo = yrs.undo_with_result(3).unwrap();
    assert_eq!(
        legacy_undo.is_some(),
        yrs_undo.is_some(),
        "{label}: undo outcome"
    );
    let after_undo =
        editor_core::render::incremental::render_blocks(yrs.document().unwrap(), &schema);
    if let Some(result) = yrs_undo.as_ref() {
        assert_render_reconstructs(&format!("{label} undo"), &before_undo, result, &after_undo);
        assert_eq!(
            result.active_state,
            legacy.get_selection_state().active_state,
            "{label}: undo active"
        );
    }
    assert_document_state(&format!("{label} undo"), &legacy, &yrs, &schema);

    let before_redo = after_undo;
    let legacy_redo = legacy.redo();
    let yrs_redo = yrs.redo_with_result(4).unwrap();
    assert_eq!(
        legacy_redo.is_some(),
        yrs_redo.is_some(),
        "{label}: redo outcome"
    );
    let after_redo =
        editor_core::render::incremental::render_blocks(yrs.document().unwrap(), &schema);
    if let Some(result) = yrs_redo.as_ref() {
        assert_render_reconstructs(&format!("{label} redo"), &before_redo, result, &after_redo);
        assert_eq!(
            result.active_state,
            legacy.get_selection_state().active_state,
            "{label}: redo active"
        );
    }
    assert_document_state(&format!("{label} redo"), &legacy, &yrs, &schema);
}

fn custom_role_schema() -> Schema {
    Schema::from_json(&serde_json::json!({
        "nodes": [
            {"name":"root","content":"block+","role":"doc"},
            {"name":"body","content":"inline*","group":"block","role":"textBlock","htmlTag":"p"},
            {"name":"numbered","content":"entry+","group":"block","role":"list","ordered":true},
            {"name":"entry","content":"body block*","role":"listItem"},
            {"name":"softAtom","content":"","group":"inline","role":"inline","isVoid":true,"attrs":{"label":{"default":"@x"}}},
            {"name":"panelAtom","content":"","group":"block","role":"block","isVoid":true,"attrs":{"label":{"default":"panel"}}},
            {"name":"text","content":"","group":"inline","role":"text"}
        ],
        "marks": []
    }))
    .unwrap()
}

fn task_list_schema() -> Schema {
    Schema::from_json(&serde_json::json!({
        "nodes": [
            {"name":"doc","content":"block+","role":"doc"},
            {"name":"paragraph","content":"inline*","group":"block","role":"textBlock","htmlTag":"p"},
            {"name":"bulletList","content":"listItem+","group":"block","role":"list","htmlTag":"ul"},
            {"name":"orderedList","content":"listItem+","group":"block","role":"list","htmlTag":"ol","attrs":{"start":{"default":1}}},
            {"name":"taskList","content":"taskItem+","group":"block","role":"list","htmlTag":"ul"},
            {"name":"listItem","content":"paragraph block*","role":"listItem","htmlTag":"li"},
            {"name":"taskItem","content":"paragraph block*","role":"listItem","htmlTag":"li","attrs":{"checked":{"default":false}}},
            {"name":"hardBreak","content":"","group":"inline","role":"hardBreak","isVoid":true},
            {"name":"text","content":"","group":"inline","role":"text"}
        ],
        "marks": []
    }))
    .unwrap()
}

fn scalar_at_path(schema: Schema, document: &serde_json::Value, path: &[u32]) -> u32 {
    let mut editor = Editor::new(schema, InterceptorPipeline::new(), false);
    editor.set_json(document).unwrap();
    let block = (0..editor.position_map().block_count())
        .filter_map(|index| editor.position_map().block(index))
        .find(|block| block.node_path.as_slice() == path)
        .unwrap();
    editor.doc_to_scalar(block.doc_start)
}

#[test]
fn structural_plan_shapes_require_live_selection_proof_and_keep_exact_granular_ops() {
    let document = serde_json::json!({"type":"doc","content":[
        {"type":"paragraph","content":[{"type":"text","text":"one"}]},
        {"type":"paragraph","content":[{"type":"text","text":"two"}]},
        {"type":"image","attrs":{"src":"https://example.com/a.png","alt":null,"title":null,"width":10,"height":20}}
    ]});
    let mut engine = YrsDocumentEngine::new(YrsEngineConfig {
        schema: tiptap_schema(),
        fragment_name: "prosemirror".into(),
        initialization_mode: InitializationMode::LocalEmpty,
        resource_limits: ResourceLimits::default(),
        editing_limits: EditingLimits::default(),
        max_length: None,
        scope: None,
    })
    .unwrap();
    engine
        .import_json(&document.to_string(), TransactionOrigin::DocumentImport)
        .unwrap();
    select(&mut engine, 1, 1, 5);

    let editor_core::yrs_engine::CommandPlan::Transaction(wrap) = engine
        .plan_command(
            2,
            TypedCommand::ApplyListType {
                list_type: "bulletList".into(),
            },
        )
        .unwrap()
    else {
        panic!("list wrap must plan")
    };
    assert!(matches!(
        wrap.operations.as_slice(),
        [editor_core::yrs_engine::TypedOperation::ReplaceStructure(_)]
    ));
    assert!(matches!(
        wrap.selection_intent,
        SelectionIntent::UseOperationResult
    ));

    let editor_core::yrs_engine::CommandPlan::Transaction(resize) = engine
        .plan_command(
            3,
            TypedCommand::ResizeImage {
                at: point(8),
                width: 30,
                height: 40,
            },
        )
        .unwrap()
    else {
        panic!("image resize must plan")
    };
    assert!(matches!(
        resize.operations.as_slice(),
        [editor_core::yrs_engine::TypedOperation::UpdateNodeAttrs { .. }]
    ));
}

#[test]
fn successful_transform_no_op_is_not_applicable_and_creates_no_history() {
    let document = serde_json::json!({"type":"doc","content":[{"type":"bulletList","content":[
        {"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"one"}]}]},
        {"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"two"}]}]}
    ]}]});
    let mut engine = YrsDocumentEngine::new(YrsEngineConfig {
        schema: tiptap_schema(),
        fragment_name: "prosemirror".into(),
        initialization_mode: InitializationMode::LocalEmpty,
        resource_limits: ResourceLimits::default(),
        editing_limits: EditingLimits::default(),
        max_length: None,
        scope: None,
    })
    .unwrap();
    engine
        .import_json(&document.to_string(), TransactionOrigin::DocumentImport)
        .unwrap();
    select(&mut engine, 1, 1, 1);
    let before = engine.encoded_state().unwrap();
    assert_eq!(
        engine
            .plan_command(2, TypedCommand::IndentListItem)
            .unwrap(),
        editor_core::yrs_engine::CommandPlan::NotApplicable
    );
    assert!(engine
        .apply_command(3, TypedCommand::IndentListItem)
        .unwrap()
        .is_none());
    assert_eq!(engine.encoded_state().unwrap(), before);
    assert!(!engine.can_undo());
}

#[test]
fn every_structural_command_matches_legacy_basics() {
    let paragraph = serde_json::json!({"type":"doc","content":[
        {"type":"paragraph","content":[{"type":"text","text":"one"}]},
        {"type":"paragraph","content":[{"type":"text","text":"two"}]}
    ]});
    let list = serde_json::json!({"type":"doc","content":[{"type":"bulletList","content":[
        {"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"one"}]}]},
        {"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"two"}]}]}
    ]}]});
    let nested = serde_json::json!({"type":"doc","content":[{"type":"bulletList","content":[
        {"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"one"}]},{"type":"bulletList","content":[
            {"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"two"}]}]}
        ]}]}
    ]}]});
    let image = serde_json::json!({"type":"doc","content":[
        {"type":"image","attrs":{"src":"https://example.com/a.png","width":10,"height":20}},
        {"type":"paragraph"}
    ]});

    let cases = vec![
        (
            "apply list",
            paragraph.clone(),
            1,
            5,
            TypedCommand::ApplyListType {
                list_type: "orderedList".into(),
            },
        ),
        (
            "wrap list",
            paragraph.clone(),
            1,
            5,
            TypedCommand::WrapInList {
                list_type: "bulletList".into(),
                item_type: "listItem".into(),
            },
        ),
        ("unwrap", list.clone(), 1, 1, TypedCommand::UnwrapFromList),
        ("indent", list, 8, 8, TypedCommand::IndentListItem),
        ("outdent", nested, 9, 9, TypedCommand::OutdentListItem),
        (
            "insert node",
            paragraph,
            1,
            1,
            TypedCommand::InsertNode {
                node_type: "hardBreak".into(),
            },
        ),
        (
            "resize image",
            image,
            0,
            0,
            TypedCommand::ResizeImage {
                at: point(0),
                width: 120,
                height: 80,
            },
        ),
    ];
    for (label, document, anchor, head, command) in cases {
        assert_parity(label, document, anchor, head, command);
    }
}

#[test]
fn structural_list_matrix_covers_reverse_multi_block_empty_and_nested_items() {
    let multi = serde_json::json!({"type":"doc","content":[
        {"type":"paragraph","content":[{"type":"text","text":"alpha"}]},
        {"type":"paragraph"},
        {"type":"paragraph","content":[{"type":"text","text":"omega"}]}
    ]});
    assert_parity_with_schema(
        "reverse multi-block task wrap",
        task_list_schema(),
        multi,
        11,
        1,
        TypedCommand::ApplyListType {
            list_type: "taskList".into(),
        },
    );

    let tasks = serde_json::json!({"type":"doc","content":[{"type":"taskList","content":[
        {"type":"taskItem","attrs":{"checked":false},"content":[{"type":"paragraph","content":[{"type":"text","text":"todo"}]}]}
    ]}]});
    assert_parity_with_schema(
        "task checked attrs",
        task_list_schema(),
        tasks,
        1,
        1,
        TypedCommand::ToggleTaskItemChecked,
    );

    let nested = serde_json::json!({"type":"doc","content":[{"type":"orderedList","attrs":{"start":3},"content":[
        {"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"outer"}]},{"type":"bulletList","content":[
            {"type":"listItem","content":[{"type":"paragraph"}]},
            {"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"inner"}]}]}
        ]}]}
    ]}]});
    let nested_scalar = scalar_at_path(task_list_schema(), &nested, &[0, 0, 1, 0, 0]);
    assert_parity_with_schema(
        "nested bullet to task conversion",
        task_list_schema(),
        nested.clone(),
        nested_scalar,
        nested_scalar,
        TypedCommand::ApplyListType {
            list_type: "taskList".into(),
        },
    );
    assert_parity_with_schema(
        "nested empty item outdent",
        task_list_schema(),
        nested,
        nested_scalar,
        nested_scalar,
        TypedCommand::OutdentListItem,
    );
}

#[test]
fn structural_insert_matrix_covers_block_inline_mention_image_and_custom_roles() {
    let paragraph = serde_json::json!({"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]});
    assert_parity(
        "horizontal rule block insertion",
        paragraph.clone(),
        1,
        1,
        TypedCommand::InsertNode {
            node_type: "horizontalRule".into(),
        },
    );

    let custom = custom_role_schema();
    let document = serde_json::json!({"type":"root","content":[{"type":"body","content":[{"type":"text","text":"abc"}]}]});
    assert_parity_with_schema(
        "custom inline mention-like atom",
        custom.clone(),
        document.clone(),
        1,
        2,
        TypedCommand::InsertNode {
            node_type: "softAtom".into(),
        },
    );
    assert_parity_with_schema(
        "custom opaque block atom",
        custom.clone(),
        document.clone(),
        1,
        1,
        TypedCommand::InsertNode {
            node_type: "panelAtom".into(),
        },
    );
    assert_parity_with_schema(
        "custom list roles and root",
        custom,
        document,
        2,
        1,
        TypedCommand::WrapInList {
            list_type: "numbered".into(),
            item_type: "entry".into(),
        },
    );

    let image = serde_json::json!({"type":"doc","content":[
        {"type":"image","attrs":{"src":"https://example.com/a.png","alt":null,"title":null,"width":10,"height":20}},
        {"type":"paragraph"}
    ]});
    assert_parity(
        "image attrs resize",
        image,
        0,
        0,
        TypedCommand::ResizeImage {
            at: point(0),
            width: 200,
            height: 100,
        },
    );
}

#[test]
fn invalid_structural_contexts_are_not_applicable_and_atomic() {
    let document = serde_json::json!({"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]});
    for command in [
        TypedCommand::UnwrapFromList,
        TypedCommand::IndentListItem,
        TypedCommand::OutdentListItem,
        TypedCommand::ToggleTaskItemChecked,
        TypedCommand::InsertNode {
            node_type: "missing".into(),
        },
        TypedCommand::InsertNode {
            node_type: "image".into(),
        },
        TypedCommand::ResizeImage {
            at: point(1),
            width: 20,
            height: 20,
        },
        TypedCommand::ResizeImage {
            at: point(0),
            width: 0,
            height: 20,
        },
    ] {
        let mut engine = YrsDocumentEngine::new(YrsEngineConfig {
            schema: tiptap_schema(),
            fragment_name: "prosemirror".into(),
            initialization_mode: InitializationMode::LocalEmpty,
            resource_limits: ResourceLimits::default(),
            editing_limits: EditingLimits::default(),
            max_length: None,
            scope: None,
        })
        .unwrap();
        engine
            .import_json(&document.to_string(), TransactionOrigin::DocumentImport)
            .unwrap();
        let before_revision = engine.revision();
        let before_state = engine.state_revision();
        assert!(engine.apply_command(99, command).unwrap().is_none());
        assert_eq!(engine.document_json().unwrap(), document);
        assert_eq!(engine.revision(), before_revision);
        assert_eq!(engine.state_revision(), before_state);
        assert!(!engine.can_undo());
        assert!(!engine.can_redo());
    }
}

#[test]
fn existing_list_type_conversion_preserves_format_source_semantics() {
    let observed = Arc::new(Mutex::new(Vec::new()));
    let mut interceptors = InterceptorPipeline::new();
    interceptors.add(Box::new(SourceRecorder {
        observed: observed.clone(),
    }));
    let mut editor = Editor::new(tiptap_schema(), interceptors, false);
    editor
        .set_json(&serde_json::json!({"type":"doc","content":[{"type":"bulletList","content":[
            {"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"one"}]}]}
        ]}]}))
        .unwrap();
    editor.set_selection(Selection::cursor(1));
    observed.lock().unwrap().clear();

    editor.apply_list_type("orderedList").unwrap();

    assert_eq!(*observed.lock().unwrap(), vec![Source::Format]);
}

#[test]
fn custom_ordered_list_conversion_preserves_declared_attrs_and_round_trips_history() {
    let schema = Schema::from_json(&serde_json::json!({
        "nodes": [
            {"name":"doc","content":"block+","role":"doc"},
            {"name":"paragraph","content":"inline*","group":"block","role":"textBlock","htmlTag":"p"},
            {"name":"alphaOrdered","content":"item+","group":"block","role":"list","htmlTag":"ol","attrs":{"start":{}}},
            {"name":"betaOrdered","content":"item+","group":"block","role":"list","htmlTag":"ol","attrs":{"start":{}}},
            {"name":"item","content":"paragraph block*","role":"listItem","htmlTag":"li"},
            {"name":"text","content":"","group":"inline","role":"text"}
        ],
        "marks": []
    }))
    .unwrap();
    let document = serde_json::json!({"type":"doc","content":[
        {"type":"alphaOrdered","attrs":{"start":3},"content":[
            {"type":"item","content":[
                {"type":"paragraph","content":[{"type":"text","text":"outer"}]},
                {"type":"alphaOrdered","attrs":{"start":4},"content":[
                    {"type":"item","content":[{"type":"paragraph","content":[{"type":"text","text":"inner"}]}]}
                ]}
            ]}
        ]}
    ]});
    let mut engine = YrsDocumentEngine::new(YrsEngineConfig {
        schema,
        fragment_name: "prosemirror".into(),
        initialization_mode: InitializationMode::LocalEmpty,
        resource_limits: ResourceLimits::default(),
        editing_limits: EditingLimits::default(),
        max_length: None,
        scope: None,
    })
    .unwrap();
    engine
        .import_json(&document.to_string(), TransactionOrigin::DocumentImport)
        .unwrap();
    select(&mut engine, 300, 1, 1);

    let result = engine
        .apply_command(
            301,
            TypedCommand::ApplyListType {
                list_type: "betaOrdered".into(),
            },
        )
        .unwrap()
        .expect("compatible custom list conversion must apply");
    assert!(result.changed);
    let converted = engine.document_json().unwrap();
    assert_eq!(converted["content"][0]["type"], "betaOrdered");
    assert_eq!(converted["content"][0]["attrs"]["start"], 3);
    assert_eq!(
        converted["content"][0]["content"][0]["content"][1]["type"],
        "alphaOrdered"
    );
    assert_eq!(
        converted["content"][0]["content"][0]["content"][1]["attrs"]["start"],
        4
    );

    engine.undo_with_result(302).unwrap().expect("undo");
    assert_eq!(engine.document_json().unwrap(), document);
    engine.redo_with_result(303).unwrap().expect("redo");
    assert_eq!(engine.document_json().unwrap(), converted);
}

#[test]
fn list_conversion_preserves_opted_in_metadata_and_requires_target_declared_attrs() {
    let schema = Schema::from_json(&serde_json::json!({
        "nodes": [
            {"name":"doc","content":"block+","role":"doc"},
            {"name":"paragraph","content":"inline*","group":"block","role":"textBlock","htmlTag":"p"},
            {"name":"sourceList","content":"sourceItem+","group":"block","role":"list","htmlTag":"ul","allowUndeclaredAttrs":true,"attrs":{"shared":{}}},
            {"name":"targetList","content":"targetItem*","group":"block","role":"list","htmlTag":"ul","allowUndeclaredAttrs":true,"attrs":{"shared":{},"listDefault":{"default":"target-default"}}},
            {"name":"missingWrapperList","content":"targetItem*","group":"block","role":"list","htmlTag":"ul","allowUndeclaredAttrs":true,"attrs":{"requiredTarget":{}}},
            {"name":"missingItemList","content":"missingRequiredItem*","group":"block","role":"list","htmlTag":"ul","allowUndeclaredAttrs":true,"attrs":{"shared":{}}},
            {"name":"sourceItem","content":"paragraph block*","role":"listItem","htmlTag":"li","allowUndeclaredAttrs":true},
            {"name":"targetItem","content":"paragraph block*","role":"listItem","htmlTag":"li","allowUndeclaredAttrs":true,"attrs":{"itemShared":{},"itemDefault":{"default":7}}},
            {"name":"missingRequiredItem","content":"paragraph block*","role":"listItem","htmlTag":"li","allowUndeclaredAttrs":true,"attrs":{"missingItemRequired":{}}},
            {"name":"text","content":"","group":"inline","role":"text"}
        ],
        "marks": []
    }))
    .unwrap();
    let document = serde_json::json!({"type":"doc","content":[
        {"type":"sourceList","attrs":{"shared":"wrapper-shared","wrapperMeta":{"owner":"app"}},"content":[
            {"type":"sourceItem","attrs":{"itemShared":"item-shared","itemMeta":["keep",2]},"content":[
                {"type":"paragraph","content":[{"type":"text","text":"entry"}]}
            ]}
        ]}
    ]});
    let mut engine = YrsDocumentEngine::new(YrsEngineConfig {
        schema,
        fragment_name: "prosemirror".into(),
        initialization_mode: InitializationMode::LocalEmpty,
        resource_limits: ResourceLimits::default(),
        editing_limits: EditingLimits::default(),
        max_length: None,
        scope: None,
    })
    .unwrap();
    engine
        .import_json(&document.to_string(), TransactionOrigin::DocumentImport)
        .unwrap();
    select(&mut engine, 400, 1, 1);

    engine
        .apply_command(
            401,
            TypedCommand::ApplyListType {
                list_type: "targetList".into(),
            },
        )
        .unwrap()
        .expect("compatible pass-through conversion must apply");
    let converted = engine.document_json().unwrap();
    let list = &converted["content"][0];
    assert_eq!(list["type"], "targetList");
    let item = &list["content"][0];
    assert_eq!(item["type"], "targetItem");
    let converted_document = engine.document().unwrap();
    let list_node = converted_document.root().child(0).unwrap();
    assert_eq!(
        list_node.attrs().get("shared"),
        Some(&serde_json::json!("wrapper-shared"))
    );
    assert_eq!(
        list_node.attrs().get("listDefault"),
        Some(&serde_json::json!("target-default"))
    );
    assert_eq!(
        list_node.attrs().get("wrapperMeta"),
        Some(&serde_json::json!({"owner":"app"}))
    );
    let item_node = list_node.child(0).unwrap();
    assert_eq!(
        item_node.attrs().get("itemShared"),
        Some(&serde_json::json!("item-shared"))
    );
    assert_eq!(
        item_node.attrs().get("itemDefault"),
        Some(&serde_json::json!(7))
    );
    assert_eq!(
        item_node.attrs().get("itemMeta"),
        Some(&serde_json::json!(["keep", 2]))
    );

    engine.undo_with_result(402).unwrap().expect("undo");
    assert_eq!(engine.document_json().unwrap(), document);

    let revision = engine.revision();
    assert!(engine
        .apply_command(
            403,
            TypedCommand::ApplyListType {
                list_type: "missingWrapperList".into(),
            },
        )
        .unwrap()
        .is_none());
    assert_eq!(engine.revision(), revision);
    assert_eq!(engine.document_json().unwrap(), document);

    assert!(engine
        .apply_command(
            404,
            TypedCommand::ApplyListType {
                list_type: "missingItemList".into(),
            },
        )
        .unwrap()
        .is_none());
    assert_eq!(engine.revision(), revision);
    assert_eq!(engine.document_json().unwrap(), document);
}
