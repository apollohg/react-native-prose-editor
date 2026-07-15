use editor_core::boundary::ResourceLimits;
use editor_core::editor::Editor;
use editor_core::intercept::InterceptorPipeline;
use editor_core::render::incremental::{flatten_render_blocks, render_blocks};
use editor_core::render::RenderElement;
use editor_core::schema::{NodeRole, Schema};
use editor_core::selection::Selection;
use editor_core::tiptap_schema;
use editor_core::yrs_engine::{
    Affinity, EditingLimits, EditorOffsetKind, HistoryPolicy, InitializationMode,
    RevisionedPosition, SelectionInput, SelectionIntent, TransactionOrigin, TypedTransaction,
    YrsDocumentEngine, YrsEngineConfig,
};

#[derive(Clone, Copy)]
enum TestSelection {
    Text(&'static str),
    Node(u32),
    All,
}

fn point(offset: u32) -> RevisionedPosition {
    RevisionedPosition {
        offset,
        kind: EditorOffsetKind::Scalar,
        affinity: Affinity::After,
    }
}

fn scalar_for(document: &editor_core::model::Document, schema: &Schema, needle: &str) -> u32 {
    let mut rendered = String::new();
    let mut started_block = false;
    let mut pending_prefix = String::new();
    for element in flatten_render_blocks(&render_blocks(document, schema)) {
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
                    if started_block {
                        rendered.push('\n');
                    }
                    started_block = true;
                    rendered.push_str(&pending_prefix);
                    pending_prefix.clear();
                }
            }
            RenderElement::TextRun { text, .. } => rendered.push_str(&text),
            RenderElement::VoidInline { .. } => rendered.push('\n'),
            RenderElement::VoidBlock { .. } => rendered.push('\u{fffc}'),
            RenderElement::OpaqueInlineAtom {
                node_type, label, ..
            }
            | RenderElement::OpaqueBlockAtom {
                node_type, label, ..
            } => rendered.push_str(&editor_core::render::opaque_atom_visible_string(
                &node_type, &label,
            )),
            RenderElement::BlockEnd => {}
        }
    }
    let index = rendered
        .find(needle)
        .unwrap_or_else(|| panic!("rendered text {rendered:?} did not contain {needle:?}"));
    u32::try_from(rendered[..index].chars().count()).unwrap() + 1
}

fn assert_command_parity(
    label: &str,
    schema: Schema,
    document: serde_json::Value,
    selection: TestSelection,
) {
    let mut editor = Editor::new(schema.clone(), InterceptorPipeline::new(), false);
    editor.set_json(&document).unwrap();

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

    let selection_input = match selection {
        TestSelection::Text(needle) => {
            let scalar = scalar_for(yrs.document().unwrap(), &schema, needle);
            editor.set_selection_scalar(scalar, scalar);
            SelectionInput::Text {
                anchor: point(scalar),
                head: point(scalar),
            }
        }
        TestSelection::Node(scalar) => {
            editor.set_selection(Selection::node(editor.scalar_to_doc(scalar)));
            SelectionInput::Node { at: point(scalar) }
        }
        TestSelection::All => {
            editor.set_selection(Selection::All);
            SelectionInput::All
        }
    };

    let legacy = editor.get_selection_state().active_state.commands;
    let result = yrs
        .apply_typed_transaction_with_result(TypedTransaction {
            request_id: 701,
            base_document_revision: yrs.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: vec![],
            selection_intent: SelectionIntent::Set(selection_input),
            history_policy: HistoryPolicy::Skip,
        })
        .unwrap();

    assert_eq!(result.active_state.commands, legacy, "scenario: {label}");
}

#[test]
fn yrs_commands_match_legacy_editor_across_selection_contexts() {
    let scenarios = [
        (
            "paragraph",
            serde_json::json!({"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"plain"}]}]}),
            TestSelection::Text("plain"),
        ),
        (
            "list item",
            serde_json::json!({"type":"doc","content":[{"type":"bulletList","content":[{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"item"}]}]}]}]}),
            TestSelection::Text("item"),
        ),
        (
            "nested list",
            serde_json::json!({"type":"doc","content":[{"type":"bulletList","content":[{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"outer"}]},{"type":"orderedList","attrs":{"start":1},"content":[{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"inner"}]}]}]}]}]}]}),
            TestSelection::Text("inner"),
        ),
        (
            "blockquote",
            serde_json::json!({"type":"doc","content":[{"type":"blockquote","content":[{"type":"paragraph","content":[{"type":"text","text":"quote"}]}]}]}),
            TestSelection::Text("quote"),
        ),
        (
            "node selection",
            serde_json::json!({"type":"doc","content":[{"type":"image","attrs":{"src":"x","alt":null,"title":null,"width":null,"height":null}}]}),
            TestSelection::Node(0),
        ),
        (
            "all selection",
            serde_json::json!({"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"all"}]}]}),
            TestSelection::All,
        ),
    ];

    for (label, document, selection) in scenarios {
        assert_command_parity(label, tiptap_schema(), document, selection);
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
    assert_command_parity(
        "code block",
        code_schema,
        serde_json::json!({"type":"doc","content":[{"type":"codeBlock","content":[{"type":"text","text":"code"}]}]}),
        TestSelection::Text("code"),
    );

    let task_schema = Schema::from_json(&serde_json::json!({
        "nodes": [
            { "name": "doc", "content": "taskList", "role": "doc" },
            { "name": "taskList", "content": "taskItem+", "role": "list", "htmlTag": "ul" },
            { "name": "taskItem", "content": "paragraph block*", "role": "listItem", "htmlTag": "li", "attrs": { "checked": { "default": false } } },
            { "name": "paragraph", "content": "text*", "group": "block", "role": "textBlock", "htmlTag": "p" },
            { "name": "text", "content": "", "role": "text" }
        ],
        "marks": []
    }))
    .unwrap();
    assert_command_parity(
        "task item",
        task_schema,
        serde_json::json!({"type":"doc","content":[{"type":"taskList","content":[{"type":"taskItem","attrs":{"checked":false},"content":[{"type":"paragraph","content":[{"type":"text","text":"task"}]}]}]}]}),
        TestSelection::Text("task"),
    );
}

#[test]
fn yrs_heading_in_list_is_explicitly_false_like_legacy_editor() {
    let schema = tiptap_schema();
    let document = serde_json::json!({"type":"doc","content":[{"type":"bulletList","content":[{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"item"}]}]}]}]});
    let mut editor = Editor::new(schema.clone(), InterceptorPipeline::new(), false);
    editor.set_json(&document).unwrap();
    let scalar = scalar_for(editor.document(), &schema, "item");
    editor.set_selection_scalar(scalar, scalar);
    assert_eq!(
        editor
            .get_selection_state()
            .active_state
            .commands
            .get("toggleHeading1"),
        Some(&false)
    );
    assert_command_parity(
        "heading in list is false",
        schema,
        document,
        TestSelection::Text("item"),
    );
}

#[test]
fn yrs_commands_match_when_schema_commands_are_absent_or_forbidden() {
    let absent = Schema::from_json(&serde_json::json!({
        "nodes": [
            { "name": "doc", "content": "paragraph+", "role": "doc" },
            { "name": "paragraph", "content": "text*", "role": "textBlock", "htmlTag": "p" },
            { "name": "text", "content": "", "role": "text" }
        ],
        "marks": []
    }))
    .unwrap();
    assert_command_parity(
        "commands absent",
        absent,
        serde_json::json!({"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"plain"}]}]}),
        TestSelection::Text("plain"),
    );

    let forbidden = Schema::from_json(&serde_json::json!({
        "nodes": [
            { "name": "doc", "content": "bulletList", "role": "doc" },
            { "name": "bulletList", "content": "listItem+", "role": "list", "htmlTag": "ul" },
            { "name": "listItem", "content": "paragraph", "role": "listItem", "htmlTag": "li" },
            { "name": "paragraph", "content": "text*", "role": "textBlock", "htmlTag": "p" },
            { "name": "heading", "content": "text*", "role": "textBlock", "htmlTag": "h1" },
            { "name": "text", "content": "", "role": "text" }
        ],
        "marks": []
    }))
    .unwrap();
    assert_command_parity(
        "heading forbidden by list item",
        forbidden,
        serde_json::json!({"type":"doc","content":[{"type":"bulletList","content":[{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"item"}]}]}]}]}),
        TestSelection::Text("item"),
    );
}
