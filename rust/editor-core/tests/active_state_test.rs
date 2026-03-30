//! Tests for schema-aware active state: allowed_marks, insertable_nodes, commands.

use editor_core::editor::Editor;
use editor_core::intercept::InterceptorPipeline;
use editor_core::schema::presets::tiptap_schema;
use editor_core::schema::{MarkSpec, NodeRole, Schema};
use editor_core::selection::Selection;
use std::collections::HashMap;

fn default_editor() -> Editor {
    Editor::new(tiptap_schema(), InterceptorPipeline::new())
}

#[test]
fn test_allowed_marks_in_text_block_all_returned() {
    let schema = tiptap_schema();
    let para = schema.node("paragraph").unwrap();
    let active_marks: Vec<&str> = vec![];
    let result = schema.allowed_marks_at(para, &active_marks);
    assert_eq!(result.len(), 4);
    assert!(result.contains(&"bold".to_string()));
    assert!(result.contains(&"italic".to_string()));
    assert!(result.contains(&"underline".to_string()));
    assert!(result.contains(&"strike".to_string()));
}

#[test]
fn test_allowed_marks_on_void_block_empty() {
    let schema = tiptap_schema();
    let hr = schema.node("horizontalRule").unwrap();
    let active_marks: Vec<&str> = vec![];
    let result = schema.allowed_marks_at(hr, &active_marks);
    assert!(
        result.is_empty(),
        "void block should not allow marks, got: {:?}",
        result
    );
}

#[test]
fn test_allowed_marks_active_mark_always_toggleable() {
    let marks = vec![
        MarkSpec {
            name: "bold".to_string(),
            attrs: HashMap::new(),
            excludes: None,
        },
        MarkSpec {
            name: "code".to_string(),
            attrs: HashMap::new(),
            excludes: Some("_".to_string()),
        },
    ];
    let nodes = tiptap_schema().all_nodes().cloned().collect::<Vec<_>>();
    let schema = Schema::new(nodes, marks);
    let para = schema.node("paragraph").unwrap();
    let active_marks = vec!["code"];
    let result = schema.allowed_marks_at(para, &active_marks);
    assert!(
        result.contains(&"code".to_string()),
        "active mark should always be toggleable"
    );
    assert!(
        !result.contains(&"bold".to_string()),
        "bold should be excluded by code's excludes: _"
    );
}

#[test]
fn test_allowed_marks_bidirectional_excludes() {
    let marks = vec![
        MarkSpec {
            name: "bold".to_string(),
            attrs: HashMap::new(),
            excludes: Some("code".to_string()),
        },
        MarkSpec {
            name: "code".to_string(),
            attrs: HashMap::new(),
            excludes: None,
        },
    ];
    let nodes = tiptap_schema().all_nodes().cloned().collect::<Vec<_>>();
    let schema = Schema::new(nodes, marks);
    let para = schema.node("paragraph").unwrap();
    let active_marks = vec!["bold"];
    let result = schema.allowed_marks_at(para, &active_marks);
    assert!(
        result.contains(&"bold".to_string()),
        "active mark always toggleable"
    );
    assert!(
        !result.contains(&"code".to_string()),
        "code excluded by bold's excludes field"
    );
}

#[test]
fn test_insertable_nodes_doc_level() {
    let schema = tiptap_schema();
    let doc_spec = schema.node("doc").unwrap();
    let result = schema.insertable_nodes_at(doc_spec, 1);
    assert!(
        result.contains(&"horizontalRule".to_string()),
        "horizontalRule should be insertable at doc level, got: {:?}",
        result
    );
    assert!(!result.contains(&"doc".to_string()));
    assert!(!result.contains(&"text".to_string()));
    assert!(!result.contains(&"listItem".to_string()));
}

#[test]
fn test_insertable_nodes_list_item_first_child() {
    let schema = tiptap_schema();
    let li_spec = schema.node("listItem").unwrap();
    // listItem content: "paragraph block*"
    // With 0 children, the mandatory paragraph slot is unfilled.
    let result_empty = schema.insertable_nodes_at(li_spec, 0);
    assert!(
        !result_empty.contains(&"horizontalRule".to_string()),
        "horizontalRule should NOT be insertable when mandatory paragraph is unfilled, got: {:?}",
        result_empty
    );

    let result_with_para = schema.insertable_nodes_at(li_spec, 1);
    assert!(
        result_with_para.contains(&"horizontalRule".to_string()),
        "horizontalRule should be insertable after first paragraph in listItem, got: {:?}",
        result_with_para
    );
}

#[test]
fn test_insertable_nodes_filters_by_role() {
    let schema = tiptap_schema();
    let doc_spec = schema.node("doc").unwrap();
    let result = schema.insertable_nodes_at(doc_spec, 1);
    assert!(
        !result.iter().any(|n| {
            schema
                .node(n)
                .map(|s| {
                    matches!(
                        s.role,
                        NodeRole::Doc | NodeRole::Text | NodeRole::ListItem | NodeRole::TextBlock
                    )
                })
                .unwrap_or(false)
        }),
        "should not contain doc, text, listItem, or textBlock roles, got: {:?}",
        result
    );
    assert!(result.contains(&"horizontalRule".to_string()));
}

// ===========================================================================
// ActiveState integration tests (via Editor)
// ===========================================================================

#[test]
fn test_active_state_allowed_marks_in_paragraph() {
    // Cursor at pos 1 in <p>Hello</p> — a paragraph allows all 4 tiptap marks.
    let mut editor = default_editor();
    editor
        .set_html("<p>Hello</p>")
        .expect("set_html should succeed");
    editor.set_selection(Selection::cursor(1));
    let state = editor.get_current_state();

    let allowed = &state.active_state.allowed_marks;
    assert_eq!(
        allowed.len(),
        4,
        "paragraph should allow all 4 marks, got: {:?}",
        allowed
    );
    assert!(
        allowed.contains(&"bold".to_string()),
        "allowed_marks should contain bold, got: {:?}",
        allowed
    );
    assert!(
        allowed.contains(&"italic".to_string()),
        "allowed_marks should contain italic, got: {:?}",
        allowed
    );
    assert!(
        allowed.contains(&"underline".to_string()),
        "allowed_marks should contain underline, got: {:?}",
        allowed
    );
    assert!(
        allowed.contains(&"strike".to_string()),
        "allowed_marks should contain strike, got: {:?}",
        allowed
    );
}

#[test]
fn test_active_state_allowed_marks_on_void_node_empty() {
    // NodeSelection on HR — void nodes have no text cursor, so allowed_marks is empty.
    // Document: <p>Hello</p><hr><p>World</p>
    // Positions: 0[p 1..6]7 8[hr]9 9[p 10..15]16
    // HR is at position 7 (before the HR void node in doc-level content).
    let mut editor = default_editor();
    editor
        .set_html("<p>Hello</p><hr><p>World</p>")
        .expect("set_html should succeed");
    editor.set_selection(Selection::node(7));
    let state = editor.get_current_state();

    assert!(
        state.active_state.allowed_marks.is_empty(),
        "NodeSelection on HR should have empty allowed_marks, got: {:?}",
        state.active_state.allowed_marks
    );
}

#[test]
fn test_active_state_insertable_nodes_in_doc_paragraph() {
    // Cursor in a paragraph that is a direct child of doc.
    // Block-level parent is doc, which accepts (block | list)+.
    // horizontalRule (group: block) should be insertable.
    let mut editor = default_editor();
    editor
        .set_html("<p>Hello</p>")
        .expect("set_html should succeed");
    editor.set_selection(Selection::cursor(1));
    let state = editor.get_current_state();

    let insertable = &state.active_state.insertable_nodes;
    assert!(
        insertable.contains(&"horizontalRule".to_string()),
        "horizontalRule should be insertable at doc level, got: {:?}",
        insertable
    );
}

#[test]
fn test_active_state_insertable_nodes_in_list_item() {
    // Cursor inside <ul><li><p>Item</p></li></ul>.
    // Horizontal rules are intentionally disabled inside lists even though
    // the raw schema would allow them at the listItem block slot.
    let mut editor = default_editor();
    editor
        .set_html("<ul><li><p>Item</p></li></ul>")
        .expect("set_html should succeed");
    // Position layout: 0[ul 1[li 2[p 3..6]7]8]9
    // Cursor inside the paragraph text at position 3.
    editor.set_selection(Selection::cursor(3));
    let state = editor.get_current_state();

    let insertable = &state.active_state.insertable_nodes;
    assert!(
        !insertable.contains(&"horizontalRule".to_string()),
        "horizontalRule should NOT be insertable in listItem, got: {:?}",
        insertable
    );
}

#[test]
fn test_active_state_wrap_list_commands_in_paragraph() {
    // Plain paragraph — not in any list. Should be wrappable in either list type.
    let mut editor = default_editor();
    editor
        .set_html("<p>Hello</p>")
        .expect("set_html should succeed");
    editor.set_selection(Selection::cursor(1));
    let state = editor.get_current_state();

    let commands = &state.active_state.commands;
    assert_eq!(
        commands.get("wrapBulletList"),
        Some(&true),
        "wrapBulletList should be true for plain paragraph, commands: {:?}",
        commands
    );
    assert_eq!(
        commands.get("wrapOrderedList"),
        Some(&true),
        "wrapOrderedList should be true for plain paragraph, commands: {:?}",
        commands
    );
}

#[test]
fn test_active_state_wrap_list_commands_already_in_list() {
    // Already inside a bullet list — both wrap commands should still be true
    // (toggle off or switch to other list type).
    let mut editor = default_editor();
    editor
        .set_html("<ul><li><p>Item</p></li></ul>")
        .expect("set_html should succeed");
    editor.set_selection(Selection::cursor(3));
    let state = editor.get_current_state();

    let commands = &state.active_state.commands;
    assert_eq!(
        commands.get("wrapBulletList"),
        Some(&true),
        "wrapBulletList should be true when already in list (toggle/switch), commands: {:?}",
        commands
    );
    assert_eq!(
        commands.get("wrapOrderedList"),
        Some(&true),
        "wrapOrderedList should be true when already in list (toggle/switch), commands: {:?}",
        commands
    );
}

#[test]
fn test_active_state_only_marks_nearest_mixed_nested_list_type_active() {
    let mut editor = default_editor();
    editor
        .set_html("<ul><li><p>A</p><ol><li><p>B</p></li></ol></li></ul>")
        .expect("set_html should succeed");
    editor.set_selection(Selection::cursor(8));

    let state = editor.get_current_state();
    let nodes = &state.active_state.nodes;

    assert_eq!(
        nodes.get("orderedList"),
        Some(&true),
        "nearest nested ordered list should be active, nodes: {:?}",
        nodes
    );
    assert_ne!(
        nodes.get("bulletList"),
        Some(&true),
        "ancestor bullet list should not also be marked active, nodes: {:?}",
        nodes
    );
}

#[test]
fn test_apply_list_type_on_nested_list_keeps_only_converted_type_active() {
    let mut editor = default_editor();
    editor
        .set_html("<ol><li><p>A</p><ol><li><p>B</p></li></ol></li></ol>")
        .expect("set_html should succeed");
    editor.set_selection(Selection::cursor(8));

    let update = editor
        .apply_list_type("bulletList")
        .expect("convert nested ordered list to bullet");

    assert_eq!(
        editor.get_html(),
        "<ol><li><p>A</p><ul><li><p>B</p></li></ul></li></ol>",
        "only the nearest nested list should convert"
    );
    assert_eq!(
        update.active_state.nodes.get("bulletList"),
        Some(&true),
        "converted nested bullet list should be active, nodes: {:?}",
        update.active_state.nodes
    );
    assert_ne!(
        update.active_state.nodes.get("orderedList"),
        Some(&true),
        "ancestor ordered list should not also be marked active after conversion, nodes: {:?}",
        update.active_state.nodes
    );
}

#[test]
fn test_active_state_all_selection_disables_everything() {
    // AllSelection should return empty allowed_marks and empty insertable_nodes.
    let mut editor = default_editor();
    editor
        .set_html("<p>Hello</p>")
        .expect("set_html should succeed");
    editor.set_selection(Selection::all());
    let state = editor.get_current_state();

    assert!(
        state.active_state.allowed_marks.is_empty(),
        "AllSelection should have empty allowed_marks, got: {:?}",
        state.active_state.allowed_marks
    );
    assert!(
        state.active_state.insertable_nodes.is_empty(),
        "AllSelection should have empty insertable_nodes, got: {:?}",
        state.active_state.insertable_nodes
    );
}

// ===========================================================================
// Silent no-op tests: operations that would fail schema validation should
// return Ok with current state unchanged instead of Err(Transform(...)).
// ===========================================================================

#[test]
fn test_silent_noop_toggle_mark_across_void_node() {
    let mut editor = default_editor();
    editor.set_html("<p>Hello</p><hr><p>World</p>").unwrap();
    editor.set_selection(Selection::text(1, 14));
    let result = editor.toggle_mark("bold");
    assert!(
        result.is_ok(),
        "toggle_mark across void node should not error, got: {:?}",
        result.err()
    );
}

#[test]
fn test_silent_noop_insert_node_invalid_position() {
    let mut editor = default_editor();
    editor.set_html("<ul><li><p>Item</p></li></ul>").unwrap();
    let result = editor.insert_node(3, "horizontalRule");
    assert!(
        result.is_ok(),
        "insert_node should silently no-op, got: {:?}",
        result.err()
    );
}

#[test]
fn test_insertable_nodes_list_item_multiple_children() {
    let schema = tiptap_schema();
    let li_spec = schema.node("listItem").unwrap();
    // listItem content: "paragraph block*" with 2 existing children (paragraph + one block)
    // The block* part is unbounded, so more blocks should still be insertable
    let result = schema.insertable_nodes_at(li_spec, 2);
    assert!(result.contains(&"horizontalRule".to_string()),
        "horizontalRule should still be insertable with 2 children in listItem (unbounded block*), got: {:?}", result);
}

#[test]
fn test_silent_noop_wrap_in_list_invalid() {
    let mut editor = default_editor();
    editor.set_html("<p>Hello</p>").unwrap();
    // Try wrapping at an invalid range — this should silently no-op
    let result = editor.wrap_in_list(0, 0, "bulletList");
    assert!(
        result.is_ok(),
        "wrap_in_list should silently no-op on invalid input, got: {:?}",
        result.err()
    );
}

#[test]
fn test_silent_noop_unwrap_from_list_not_in_list() {
    let mut editor = default_editor();
    editor.set_html("<p>Hello</p>").unwrap();
    // Try unwrapping when not in a list — should silently no-op
    let result = editor.unwrap_from_list(1);
    assert!(
        result.is_ok(),
        "unwrap_from_list should silently no-op when not in list, got: {:?}",
        result.err()
    );
}

// ===========================================================================
// insert_node position resolution: inserting a block node while the cursor
// is inside a TextBlock should resolve to the block level.
// ===========================================================================

#[test]
fn test_insert_node_resolves_to_block_level_in_paragraph() {
    // Cursor at pos 3 inside <p>Hello</p>. Inserting HR should place it
    // after the paragraph at the doc level, not inside the paragraph.
    let mut editor = default_editor();
    editor.set_html("<p>Hello</p>").unwrap();
    editor.set_selection(Selection::cursor(3));
    let result = editor
        .insert_node(3, "horizontalRule")
        .expect("insert_node in paragraph should succeed");
    let html = editor.get_html();
    assert!(
        html.contains("<hr") && html.contains("<p></p>"),
        "HTML should contain an <hr> followed by an empty paragraph, got: {}",
        html
    );
    let expected_cursor = result.selection.from(editor.document());
    assert_eq!(
        result.selection,
        Selection::cursor(expected_cursor),
        "cursor should move below the inserted horizontalRule"
    );
    assert!(
        editor
            .get_current_state()
            .active_state
            .allowed_marks
            .contains(&"bold".to_string()),
        "cursor should land in the trailing paragraph after horizontalRule insertion"
    );
}

#[test]
fn test_insert_node_in_list_item_paragraph_is_noop() {
    // Cursor inside a list item's paragraph. HR insertion should be disabled.
    let mut editor = default_editor();
    editor.set_html("<ul><li><p>Item</p></li></ul>").unwrap();
    editor.set_selection(Selection::cursor(4)); // inside "Item"
    let result = editor.insert_node(4, "horizontalRule");
    assert!(
        result.is_ok(),
        "insert_node in list item should no-op, got: {:?}",
        result.err()
    );
    let html = editor.get_html();
    assert!(
        !html.contains("<hr"),
        "HTML should not contain an <hr> after inserting in list item, got: {}",
        html
    );
    assert_eq!(
        editor.get_current_state().selection,
        Selection::cursor(4),
        "selection should remain at the original cursor position after no-op"
    );
}

#[test]
fn test_insert_node_hard_break_stays_inline_and_moves_cursor_after_break() {
    let mut editor = default_editor();
    editor.set_html("<p>Hello</p>").unwrap();
    editor.set_selection(Selection::cursor(3));

    let result = editor
        .insert_node(3, "hardBreak")
        .expect("hardBreak insertion should succeed");

    assert_eq!(
        editor.get_html(),
        "<p>He<br>llo</p>",
        "hardBreak insertion should stay inside the current paragraph"
    );
    assert_eq!(
        result.selection,
        Selection::cursor(4),
        "cursor should move to immediately after the inserted hardBreak"
    );
}

#[test]
fn test_active_state_exposes_hard_break_in_paragraph_and_list_item() {
    let mut editor = default_editor();
    editor.set_html("<p>Hello</p>").unwrap();
    editor.set_selection(Selection::cursor(3));
    assert!(
        editor
            .get_current_state()
            .active_state
            .insertable_nodes
            .contains(&"hardBreak".to_string()),
        "hardBreak should be insertable in a paragraph"
    );

    editor.set_html("<ul><li><p>Item</p></li></ul>").unwrap();
    editor.set_selection(Selection::cursor(4));
    let insertable = editor.get_current_state().active_state.insertable_nodes;
    assert!(
        insertable.contains(&"hardBreak".to_string()),
        "hardBreak should remain insertable inside list item paragraphs, got: {:?}",
        insertable
    );
    assert!(
        !insertable.contains(&"horizontalRule".to_string()),
        "horizontalRule should still be excluded inside list item paragraphs, got: {:?}",
        insertable
    );
}
