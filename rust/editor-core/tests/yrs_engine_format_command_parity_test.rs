use std::collections::HashMap;

use editor_core::boundary::ResourceLimits;
use editor_core::editor::Editor;
use editor_core::intercept::InterceptorPipeline;
use editor_core::schema::Schema;
use editor_core::tiptap_schema;
use editor_core::yrs_engine::{
    Affinity, CommandPlan, EditingLimits, EditorOffsetKind, HistoryPolicy, InitializationMode,
    RevisionedPosition, SelectionInput, SelectionIntent, TransactionOrigin, TypedCommand,
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

#[test]
fn collapsed_mark_command_is_a_history_boundary_without_a_document_write() {
    let mut engine = engine();
    engine
        .apply_typed_transaction(TypedTransaction {
            request_id: 1,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalInput,
            operations: vec![TypedOperation::InsertText {
                at: point(0),
                text: "a".into(),
                marks: vec![],
            }],
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Auto,
        })
        .unwrap();
    engine
        .apply_command(
            2,
            TypedCommand::ToggleMark {
                mark_type: "bold".into(),
            },
        )
        .unwrap()
        .unwrap();
    engine
        .apply_typed_transaction(TypedTransaction {
            request_id: 3,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalInput,
            operations: vec![TypedOperation::InsertText {
                at: point(1),
                text: "b".into(),
                marks: vec![],
            }],
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Auto,
        })
        .unwrap();

    engine.undo(4).unwrap().unwrap();
    assert_eq!(engine.document().unwrap().root().text_content(), "a");
    engine.undo(5).unwrap().unwrap();
    assert_eq!(engine.document().unwrap().root().text_content(), "");
    engine.redo(6).unwrap().unwrap();
    engine.redo(7).unwrap().unwrap();
    assert_eq!(engine.document().unwrap().root().text_content(), "ab");
    engine.undo(8).unwrap().unwrap();
    assert_eq!(engine.document().unwrap().root().text_content(), "a");
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

fn assert_mark_parity(
    document: serde_json::Value,
    anchor: u32,
    head: u32,
    command: TypedCommand,
    legacy_apply: impl FnOnce(&mut Editor),
) {
    let schema = tiptap_schema();
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
    select(&mut yrs, 50, anchor, head);
    legacy_apply(&mut legacy);
    let result = yrs.apply_command(51, command).unwrap().unwrap();
    assert_eq!(yrs.document_json().unwrap(), legacy.get_json());
    assert_eq!(
        result.active_state,
        legacy.get_selection_state().active_state
    );
    assert_eq!(result.history_state.can_undo, legacy.can_undo());
}

fn assert_range_mark_history_parity(
    label: &str,
    document: serde_json::Value,
    anchor: u32,
    head: u32,
    command: TypedCommand,
    legacy_apply: impl FnOnce(&mut Editor),
) {
    let schema = tiptap_schema();
    let mut legacy = Editor::new(schema.clone(), InterceptorPipeline::new(), false);
    legacy.set_json(&document).unwrap();
    let mut yrs = engine();
    yrs.import_json(&document.to_string(), TransactionOrigin::DocumentImport)
        .unwrap();
    select(&mut yrs, 70, anchor, head);
    let Some(editor_core::yrs_engine::ResolvedSelection::Text {
        anchor: selected_anchor,
        head: selected_head,
    }) = yrs.resolved_selection()
    else {
        panic!("expected resolved text selection in {label}");
    };
    let selected = (selected_anchor.scalar, selected_head.scalar);
    legacy.set_selection_scalar(selected.0, selected.1);

    legacy_apply(&mut legacy);
    let result = yrs.apply_command(71, command).unwrap().unwrap();
    assert_eq!(yrs.document_json().unwrap(), legacy.get_json(), "{label}");
    let editor_core::selection::Selection::Text {
        anchor: legacy_anchor,
        head: legacy_head,
    } = legacy.get_selection_state().selection
    else {
        panic!("expected legacy text selection in {label}");
    };
    assert_eq!(legacy.doc_to_scalar(legacy_anchor), selected.0, "{label}");
    assert_eq!(legacy.doc_to_scalar(legacy_head), selected.1, "{label}");
    let editor_core::yrs_engine::ResolvedSelection::Text {
        anchor: yrs_anchor,
        head: yrs_head,
    } = result.selection
    else {
        panic!("expected Yrs text selection in {label}");
    };
    assert_eq!((yrs_anchor.scalar, yrs_head.scalar), selected, "{label}");
    assert_eq!(yrs.stored_marks(), None, "{label}");
    assert!(legacy.can_undo(), "{label}");
    assert!(yrs.can_undo(), "{label}");

    assert!(legacy.undo().is_some(), "{label}");
    assert!(yrs.undo(72).unwrap().is_some(), "{label}");
    assert_eq!(yrs.document_json().unwrap(), legacy.get_json(), "{label}");
    assert!(!legacy.can_undo(), "{label}");
    assert!(!yrs.can_undo(), "{label}");
    assert!(legacy.undo().is_none(), "{label}");
    assert!(yrs.undo(73).unwrap().is_none(), "{label}");

    assert!(legacy.redo().is_some(), "{label}");
    assert!(yrs.redo(74).unwrap().is_some(), "{label}");
    assert_eq!(yrs.document_json().unwrap(), legacy.get_json(), "{label}");
    assert!(!legacy.can_redo(), "{label}");
    assert!(!yrs.can_redo(), "{label}");
}

#[test]
fn collapsed_set_mark_updates_stored_marks_without_document_mutation() {
    let mut engine = engine();
    let result = engine
        .apply_command(
            1,
            TypedCommand::SetMark {
                mark_type: "link".into(),
                attrs: HashMap::from([("href".into(), serde_json::json!("https://example.com"))]),
            },
        )
        .unwrap()
        .unwrap();

    assert!(result.changed);
    assert_eq!(engine.revision(), 0);
    assert_eq!(engine.state_revision(), 1);
    assert_eq!(engine.stored_marks().unwrap()[0].mark_type(), "link");
}

#[test]
fn heading_toggle_uses_schema_role_and_round_trips() {
    let mut engine = engine();
    engine
        .apply_command(
            1,
            TypedCommand::InsertText {
                text: "title".into(),
            },
        )
        .unwrap();
    engine
        .apply_command(2, TypedCommand::ToggleHeading { level: 2 })
        .unwrap();
    assert_eq!(
        engine
            .document()
            .unwrap()
            .root()
            .child(0)
            .unwrap()
            .node_type(),
        "h2"
    );

    engine
        .apply_command(3, TypedCommand::ToggleHeading { level: 2 })
        .unwrap();
    assert_eq!(
        engine
            .document()
            .unwrap()
            .root()
            .child(0)
            .unwrap()
            .node_type(),
        "paragraph"
    );
}

#[test]
fn mark_command_matrix_matches_legacy_ranges_attrs_and_stored_marks() {
    let bold = serde_json::json!({"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc","marks":[{"type":"bold"}]}]}]});
    assert_mark_parity(
        bold,
        0,
        3,
        TypedCommand::ToggleMark {
            mark_type: "bold".into(),
        },
        |editor| {
            editor.toggle_mark("bold").unwrap();
        },
    );

    let link = serde_json::json!({"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc","marks":[{"type":"link","attrs":{"href":"old"}}]}]}]});
    let attrs = HashMap::from([("href".into(), serde_json::json!("new"))]);
    assert_mark_parity(
        link.clone(),
        1,
        1,
        TypedCommand::SetMark {
            mark_type: "link".into(),
            attrs: attrs.clone(),
        },
        |editor| {
            editor.set_mark("link", attrs).unwrap();
        },
    );
    assert_mark_parity(
        link,
        1,
        1,
        TypedCommand::UnsetMark {
            mark_type: "link".into(),
        },
        |editor| {
            editor.unset_mark("link").unwrap();
        },
    );

    let plain = serde_json::json!({"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]});
    assert_mark_parity(
        plain,
        1,
        1,
        TypedCommand::ToggleMark {
            mark_type: "italic".into(),
        },
        |editor| {
            editor.toggle_mark("italic").unwrap();
        },
    );
}

#[test]
fn range_set_and_unset_mark_matrix_preserves_direction_attrs_and_one_undo_group() {
    let mixed = serde_json::json!({"type":"doc","content":[{"type":"paragraph","content":[
        {"type":"text","text":"a","marks":[{"type":"link","attrs":{"href":"old"}}]},
        {"type":"text","text":"bc"}
    ]}]});
    for (label, anchor, head) in [("forward", 0, 2), ("reverse", 2, 0)] {
        let attrs = HashMap::from([("href".into(), serde_json::json!("new"))]);
        assert_range_mark_history_parity(
            &format!("set-{label}"),
            mixed.clone(),
            anchor,
            head,
            TypedCommand::SetMark {
                mark_type: "link".into(),
                attrs: attrs.clone(),
            },
            |editor| {
                editor.set_mark("link", attrs).unwrap();
            },
        );
        assert_range_mark_history_parity(
            &format!("unset-{label}"),
            mixed.clone(),
            anchor,
            head,
            TypedCommand::UnsetMark {
                mark_type: "link".into(),
            },
            |editor| {
                editor.unset_mark("link").unwrap();
            },
        );
    }
}

#[test]
fn range_set_mark_lowers_to_marks_only_operations_and_preserves_reverse_selection() {
    let document = serde_json::json!({"type":"doc","content":[{"type":"paragraph","content":[
        {"type":"text","text":"a","marks":[{"type":"link","attrs":{"href":"old"}}]},
        {"type":"text","text":"bc"}
    ]}]});
    let mut engine = engine();
    engine
        .import_json(&document.to_string(), TransactionOrigin::DocumentImport)
        .unwrap();
    select(&mut engine, 75, 2, 0);

    let CommandPlan::Transaction(transaction) = engine
        .plan_command(
            76,
            TypedCommand::SetMark {
                mark_type: "link".into(),
                attrs: HashMap::from([("href".into(), serde_json::json!("new"))]),
            },
        )
        .unwrap()
    else {
        panic!("range set-mark must plan a transaction")
    };
    assert!(matches!(
        transaction.operations.as_slice(),
        [
            TypedOperation::RemoveMark { .. },
            TypedOperation::AddMark { .. }
        ]
    ));
    assert_eq!(transaction.selection_intent, SelectionIntent::Preserve);
    assert_eq!(transaction.origin, TransactionOrigin::LocalCommand);
    assert_eq!(transaction.history_policy, HistoryPolicy::Boundary);
}

#[test]
fn mark_request_validation_shares_required_default_undeclared_and_unset_type_rules() {
    let schema = Schema::from_json(&serde_json::json!({
        "nodes": [
            { "name": "doc", "content": "block+", "role": "doc" },
            { "name": "paragraph", "content": "text*", "group": "block", "role": "textBlock", "htmlTag": "p" },
            { "name": "text", "content": "", "role": "text" }
        ],
        "marks": [{
            "name": "custom",
            "htmlTag": "span",
            "attrs": { "required": {}, "withDefault": { "default": "defaulted" } }
        }]
    }))
    .unwrap();
    let new_engine = || {
        YrsDocumentEngine::new(YrsEngineConfig {
            schema: schema.clone(),
            fragment_name: "prosemirror".into(),
            initialization_mode: InitializationMode::LocalEmpty,
            resource_limits: ResourceLimits::default(),
            editing_limits: EditingLimits::default(),
            max_length: None,
            scope: None,
        })
        .unwrap()
    };

    for attrs in [
        HashMap::new(),
        HashMap::from([
            ("required".into(), serde_json::json!("ok")),
            ("undeclared".into(), serde_json::json!(true)),
        ]),
    ] {
        let mut yrs = new_engine();
        let before = (
            yrs.encoded_state().unwrap(),
            yrs.revision(),
            yrs.state_revision(),
            yrs.last_committed_origin(),
        );
        assert!(yrs
            .apply_command(
                80,
                TypedCommand::SetMark {
                    mark_type: "custom".into(),
                    attrs: attrs.clone(),
                },
            )
            .is_err());
        assert_eq!(
            before,
            (
                yrs.encoded_state().unwrap(),
                yrs.revision(),
                yrs.state_revision(),
                yrs.last_committed_origin(),
            )
        );
        let mut legacy = Editor::new(schema.clone(), InterceptorPipeline::new(), false);
        assert!(legacy.set_mark("custom", attrs).is_err());
    }

    let required_only = HashMap::from([("required".into(), serde_json::json!("ok"))]);
    let mut yrs = new_engine();
    yrs.apply_command(
        81,
        TypedCommand::SetMark {
            mark_type: "custom".into(),
            attrs: required_only.clone(),
        },
    )
    .unwrap()
    .unwrap();
    assert_eq!(yrs.stored_marks().unwrap()[0].attrs(), &required_only);
    assert!(yrs
        .apply_command(
            82,
            TypedCommand::UnsetMark {
                mark_type: "custom".into(),
            },
        )
        .unwrap()
        .is_some());
    assert_eq!(yrs.stored_marks(), Some(&[][..]));

    let mut unknown_yrs = new_engine();
    let before = unknown_yrs.encoded_state().unwrap();
    assert!(unknown_yrs
        .apply_command(
            83,
            TypedCommand::UnsetMark {
                mark_type: "missing".into(),
            },
        )
        .is_err());
    assert_eq!(unknown_yrs.encoded_state().unwrap(), before);
    let mut unknown_legacy = Editor::new(schema, InterceptorPipeline::new(), false);
    assert!(unknown_legacy.unset_mark("missing").is_err());
}

#[test]
fn custom_pre_code_block_and_reverse_selection_are_schema_driven() {
    let schema = Schema::from_json(&serde_json::json!({
        "nodes": [
            { "name": "root", "content": "block+", "role": "doc" },
            { "name": "body", "content": "text*", "group": "block", "role": "textBlock", "htmlTag": "p" },
            { "name": "source", "content": "text*", "group": "block", "role": "textBlock", "htmlTag": "pre" },
            { "name": "text", "content": "", "role": "text" }
        ],
        "marks": []
    }))
    .unwrap();
    let new_engine = || {
        YrsDocumentEngine::new(YrsEngineConfig {
            schema: schema.clone(),
            fragment_name: "prosemirror".into(),
            initialization_mode: InitializationMode::LocalEmpty,
            resource_limits: ResourceLimits::default(),
            editing_limits: EditingLimits::default(),
            max_length: None,
            scope: None,
        })
        .unwrap()
    };
    let mut engine = new_engine();
    engine
        .import_json(
            &serde_json::json!({"type":"root","content":[{"type":"body","content":[{"type":"text","text":"abc"}]}]}).to_string(),
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    select(&mut engine, 1, 3, 0);
    let toggled = engine
        .apply_command(2, TypedCommand::ToggleCodeBlock)
        .unwrap()
        .unwrap();
    assert_eq!(
        engine
            .document()
            .unwrap()
            .root()
            .child(0)
            .unwrap()
            .node_type(),
        "source"
    );
    let editor_core::yrs_engine::ResolvedSelection::Text { anchor, head } = toggled.selection
    else {
        panic!("expected text selection");
    };
    assert_eq!((anchor.scalar, head.scalar), (3, 0));
    engine
        .apply_command(3, TypedCommand::ToggleCodeBlock)
        .unwrap()
        .unwrap();
    assert_eq!(
        engine
            .document()
            .unwrap()
            .root()
            .child(0)
            .unwrap()
            .node_type(),
        "body"
    );

    let mut split = new_engine();
    split
        .import_json(
            &serde_json::json!({"type":"root","content":[{"type":"source","content":[{"type":"text","text":"abc"}]}]}).to_string(),
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    select(&mut split, 4, 1, 1);
    split
        .apply_command(5, TypedCommand::SplitBlock)
        .unwrap()
        .unwrap();
    assert_eq!(split.document().unwrap().root().text_content(), "a\nbc");
}

#[test]
fn blockquote_wrap_unwrap_multiblock_preserves_reverse_direction() {
    let document = serde_json::json!({"type":"doc","content":[
        {"type":"paragraph","content":[{"type":"text","text":"a"}]},
        {"type":"paragraph","content":[{"type":"text","text":"b"}]}
    ]});
    let mut engine = engine();
    engine
        .import_json(&document.to_string(), TransactionOrigin::DocumentImport)
        .unwrap();
    select(&mut engine, 1, 3, 0);
    let wrapped = engine
        .apply_command(2, TypedCommand::ToggleBlockquote)
        .unwrap()
        .unwrap();
    assert_eq!(
        engine
            .document()
            .unwrap()
            .root()
            .child(0)
            .unwrap()
            .node_type(),
        "blockquote"
    );
    let editor_core::yrs_engine::ResolvedSelection::Text { anchor, head } = wrapped.selection
    else {
        panic!("expected text selection");
    };
    assert_eq!((anchor.scalar, head.scalar), (3, 0));
    engine
        .apply_command(3, TypedCommand::ToggleBlockquote)
        .unwrap()
        .unwrap();
    assert_eq!(engine.document_json().unwrap(), document);
    engine.undo(4).unwrap().unwrap();
    engine.redo(5).unwrap().unwrap();
    assert_eq!(engine.document_json().unwrap(), document);
}

#[test]
fn unknown_mark_command_is_atomic_and_code_without_schema_is_not_applicable() {
    let mut engine = engine();
    let before = engine.encoded_state().unwrap();
    let revision = engine.revision();
    assert!(engine
        .apply_command(
            1,
            TypedCommand::SetMark {
                mark_type: "notInSchema".into(),
                attrs: HashMap::new(),
            },
        )
        .is_err());
    assert_eq!(engine.encoded_state().unwrap(), before);
    assert_eq!(engine.revision(), revision);
    assert!(engine
        .apply_command(2, TypedCommand::ToggleCodeBlock)
        .unwrap()
        .is_none());
}

#[test]
fn heading_mixed_list_custom_name_and_forbidden_node_matrix() {
    let mixed = serde_json::json!({"type":"doc","content":[
        {"type":"paragraph","content":[{"type":"text","text":"a"}]},
        {"type":"h2","content":[{"type":"text","text":"b"}]}
    ]});
    let mut mixed_engine = engine();
    mixed_engine
        .import_json(&mixed.to_string(), TransactionOrigin::DocumentImport)
        .unwrap();
    select(&mut mixed_engine, 1, 3, 0);
    let result = mixed_engine
        .apply_command(2, TypedCommand::ToggleHeading { level: 2 })
        .unwrap()
        .unwrap();
    assert!(mixed_engine
        .document()
        .unwrap()
        .root()
        .content()
        .unwrap()
        .iter()
        .all(|node| node.node_type() == "h2"));
    let editor_core::yrs_engine::ResolvedSelection::Text { anchor, head } = result.selection else {
        panic!("expected text selection");
    };
    assert_eq!((anchor.scalar, head.scalar), (3, 0));

    let list = serde_json::json!({"type":"doc","content":[{"type":"bulletList","content":[
        {"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"item"}]}]}
    ]}]});
    let mut list_engine = engine();
    list_engine
        .import_json(&list.to_string(), TransactionOrigin::DocumentImport)
        .unwrap();
    select(&mut list_engine, 3, 1, 1);
    assert!(list_engine
        .apply_command(4, TypedCommand::ToggleHeading { level: 2 })
        .unwrap()
        .is_none());
    assert_eq!(list_engine.document_json().unwrap(), list);

    let custom = Schema::from_json(&serde_json::json!({
        "nodes": [
            { "name": "root", "content": "block+", "role": "doc" },
            { "name": "body", "content": "text*", "group": "block", "role": "textBlock", "htmlTag": "p" },
            { "name": "titleTwo", "content": "text*", "group": "block", "role": "textBlock", "htmlTag": "h2" },
            { "name": "text", "content": "", "role": "text" }
        ],
        "marks": []
    }))
    .unwrap();
    let mut custom_engine = YrsDocumentEngine::new(YrsEngineConfig {
        schema: custom,
        fragment_name: "prosemirror".into(),
        initialization_mode: InitializationMode::LocalEmpty,
        resource_limits: ResourceLimits::default(),
        editing_limits: EditingLimits::default(),
        max_length: None,
        scope: None,
    })
    .unwrap();
    custom_engine
        .apply_command(5, TypedCommand::InsertText { text: "x".into() })
        .unwrap();
    custom_engine
        .apply_command(6, TypedCommand::ToggleHeading { level: 2 })
        .unwrap()
        .unwrap();
    assert_eq!(
        custom_engine
            .document()
            .unwrap()
            .root()
            .child(0)
            .unwrap()
            .node_type(),
        "titleTwo"
    );

    let mut forbidden = engine();
    forbidden
        .import_json(
            &serde_json::json!({"type":"doc","content":[{"type":"image","attrs":{"src":"x","alt":null,"title":null,"width":null,"height":null}}]}).to_string(),
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    forbidden
        .apply_typed_transaction(TypedTransaction {
            request_id: 7,
            base_document_revision: forbidden.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: vec![],
            selection_intent: SelectionIntent::Set(SelectionInput::Node { at: point(0) }),
            history_policy: HistoryPolicy::Skip,
        })
        .unwrap();
    assert!(forbidden
        .apply_command(8, TypedCommand::ToggleHeading { level: 2 })
        .unwrap()
        .is_none());
    assert!(forbidden
        .apply_command(
            9,
            TypedCommand::ToggleMark {
                mark_type: "bold".into(),
            },
        )
        .unwrap()
        .is_none());
}
