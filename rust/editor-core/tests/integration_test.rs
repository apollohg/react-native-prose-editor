//! Integration tests for the Editor, Backend, and Registry.
//!
//! These tests exercise the full lifecycle: create editor, set content,
//! edit, verify state, undo/redo, interceptors, position mapping, and
//! multi-editor isolation.

use editor_core::editor::Editor;
use editor_core::intercept::{InterceptorPipeline, MaxLength};
use editor_core::registry::{EditorRegistry, INVALID_EDITOR_ID};
use editor_core::schema::presets::tiptap_schema;
use editor_core::schema::{AttrSpec, NodeRole, NodeSpec, Schema};
use editor_core::selection::Selection;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Create an editor with the default tiptap schema and no interceptors.
fn default_editor() -> Editor {
    Editor::new(tiptap_schema(), InterceptorPipeline::new(), false)
}

fn mention_schema() -> Schema {
    let base = tiptap_schema();
    let mut nodes: Vec<NodeSpec> = base.all_nodes().cloned().collect();
    if !nodes.iter().any(|node| node.name == "mention") {
        let mut attrs = std::collections::HashMap::new();
        attrs.insert(
            "label".to_string(),
            AttrSpec {
                default: Some(serde_json::Value::Null),
            },
        );
        nodes.push(NodeSpec {
            name: "mention".to_string(),
            content: editor_core::schema::content_rule::ContentRule::parse("")
                .expect("mention content rule should parse"),
            group: Some("inline".to_string()),
            attrs,
            role: NodeRole::Inline,
            html_tag: None,
            is_void: true,
        });
    }
    let marks = base.all_marks().cloned().collect();
    Schema::new(nodes, marks)
}

fn mention_editor() -> Editor {
    Editor::new(mention_schema(), InterceptorPipeline::new(), false)
}

fn title_first_schema() -> Schema {
    let base = tiptap_schema();
    let mut nodes: Vec<NodeSpec> = base.all_nodes().cloned().collect();
    if let Some(doc) = nodes.iter_mut().find(|node| node.name == "doc") {
        doc.content = editor_core::schema::content_rule::ContentRule::parse("title block*")
            .expect("title-first doc content rule should parse");
    }
    if !nodes.iter().any(|node| node.name == "title") {
        nodes.push(NodeSpec {
            name: "title".to_string(),
            content: editor_core::schema::content_rule::ContentRule::parse("inline*")
                .expect("title content rule should parse"),
            group: Some("block".to_string()),
            attrs: std::collections::HashMap::new(),
            role: NodeRole::TextBlock,
            html_tag: None,
            is_void: false,
        });
    }
    let marks = base.all_marks().cloned().collect();
    Schema::new(nodes, marks)
}

fn title_first_editor() -> Editor {
    Editor::new(title_first_schema(), InterceptorPipeline::new(), false)
}

/// Create an editor with a max-length interceptor.
fn editor_with_max_length(max: u32) -> Editor {
    let mut pipeline = InterceptorPipeline::new();
    pipeline.add(Box::new(MaxLength::new(max)));
    Editor::new(tiptap_schema(), pipeline, false)
}

// ===========================================================================
// Full lifecycle test
// ===========================================================================

#[test]
fn test_editor_initializes_custom_empty_doc_with_required_title_block() {
    let editor = title_first_editor();

    assert_eq!(
        editor.get_json(),
        serde_json::json!({
            "type": "doc",
            "content": [{
                "type": "title"
            }]
        })
    );
}

#[test]
fn test_full_lifecycle_set_html_insert_toggle_undo_redo() {
    let mut editor = default_editor();

    // 1. Set initial HTML.
    let elements = editor
        .set_html("<p>Hello world</p>")
        .expect("set_html should succeed");
    assert!(
        !elements.is_empty(),
        "set_html should return render elements"
    );

    // 2. Verify get_html returns equivalent content.
    let html = editor.get_html();
    assert!(
        html.contains("Hello world"),
        "get_html should contain 'Hello world', got: {}",
        html
    );

    // 3. Insert text at position 6 (after "Hello").
    //    Document: <p>Hello world</p>
    //    Position 1 = start of "Hello", position 6 = after "Hello".
    let update = editor
        .insert_text(6, ", beautiful")
        .expect("insert_text should succeed");
    assert!(
        !update.render_elements.is_empty(),
        "insert_text should return render elements"
    );

    // 4. Verify updated HTML.
    let html_after_insert = editor.get_html();
    assert!(
        html_after_insert.contains("Hello, beautiful world"),
        "HTML should contain 'Hello, beautiful world', got: {}",
        html_after_insert
    );

    // 5. Toggle bold on a range.
    //    First, set a text selection covering "beautiful" (positions 8..17).
    //    "Hello, " = 7 chars at pos 1-7, "beautiful" starts at pos 8.
    editor.set_selection(Selection::text(8, 17));
    let update = editor
        .toggle_mark("bold")
        .expect("toggle_mark should succeed");
    assert!(
        !update.render_elements.is_empty(),
        "toggle_mark should return render elements"
    );

    // 6. Verify bold is in the HTML output.
    let html_after_bold = editor.get_html();
    assert!(
        html_after_bold.contains("<strong>beautiful</strong>"),
        "HTML should contain <strong>beautiful</strong>, got: {}",
        html_after_bold
    );

    // 7. Undo the bold toggle.
    let undo_result = editor.undo();
    assert!(
        undo_result.is_some(),
        "undo should return Some after toggle_mark"
    );
    let html_after_undo_bold = editor.get_html();
    assert!(
        !html_after_undo_bold.contains("<strong>"),
        "HTML should not contain <strong> after undoing bold, got: {}",
        html_after_undo_bold
    );
    assert!(
        html_after_undo_bold.contains("Hello, beautiful world"),
        "text should still contain 'Hello, beautiful world' after undoing bold, got: {}",
        html_after_undo_bold
    );

    // 8. Undo the text insertion.
    let undo_result2 = editor.undo();
    assert!(
        undo_result2.is_some(),
        "undo should return Some for the text insertion"
    );
    let html_after_undo_insert = editor.get_html();
    assert!(
        html_after_undo_insert.contains("Hello world"),
        "HTML should be back to 'Hello world' after undoing insert, got: {}",
        html_after_undo_insert
    );
    assert!(
        !html_after_undo_insert.contains("beautiful"),
        "HTML should not contain 'beautiful' after undoing insert, got: {}",
        html_after_undo_insert
    );

    // 9. Redo the text insertion.
    let redo_result = editor.redo();
    assert!(
        redo_result.is_some(),
        "redo should return Some for the text insertion"
    );
    let html_after_redo = editor.get_html();
    assert!(
        html_after_redo.contains("Hello, beautiful world"),
        "HTML should contain 'Hello, beautiful world' after redo, got: {}",
        html_after_redo
    );

    // 10. Redo the bold toggle.
    let redo_result2 = editor.redo();
    assert!(
        redo_result2.is_some(),
        "redo should return Some for the bold toggle"
    );
    let html_after_redo_bold = editor.get_html();
    assert!(
        html_after_redo_bold.contains("<strong>beautiful</strong>"),
        "HTML should contain <strong>beautiful</strong> after redo, got: {}",
        html_after_redo_bold
    );
}

// ===========================================================================
// Set/Get HTML roundtrip
// ===========================================================================

#[test]
fn test_set_html_get_html_roundtrip() {
    let mut editor = default_editor();

    let input = "<p>Simple text</p>";
    editor.set_html(input).expect("set_html should succeed");
    let output = editor.get_html();
    assert_eq!(
        output, "<p>Simple text</p>",
        "HTML should roundtrip faithfully"
    );
}

#[test]
fn test_set_html_with_formatting() {
    let mut editor = default_editor();
    editor
        .set_html("<p><strong>Bold</strong> and <em>italic</em></p>")
        .expect("set_html should succeed");
    let output = editor.get_html();
    assert!(
        output.contains("<strong>Bold</strong>"),
        "Output should contain bold, got: {}",
        output
    );
    assert!(
        output.contains("<em>italic</em>"),
        "Output should contain italic, got: {}",
        output
    );
}

#[test]
fn test_insert_horizontal_rule_then_return_and_backspace_does_not_panic() {
    let mut editor = default_editor();
    editor
        .set_html("<p>Hello</p>")
        .expect("set_html should succeed");
    editor.set_selection(Selection::cursor(3));

    let insert_update = editor
        .insert_node(3, "horizontalRule")
        .expect("horizontal rule insertion should succeed");
    assert!(
        editor.get_html().contains("<hr"),
        "inserting a horizontal rule should produce an <hr>"
    );

    let cursor_after_insert = insert_update.selection.from(editor.document());
    let split_update = editor
        .split_block(cursor_after_insert)
        .expect("pressing return after a horizontal rule should not panic");
    let cursor_after_split = split_update.selection.from(editor.document());
    let scalar_after_split = editor.doc_to_scalar(cursor_after_split);
    assert!(
        scalar_after_split > 0,
        "cursor after splitting below a horizontal rule should be backspace-able"
    );

    editor
        .delete_scalar_range(scalar_after_split - 1, scalar_after_split)
        .expect("backspacing after splitting below a horizontal rule should not panic");
}

#[test]
fn test_backspace_below_horizontal_rule_replaces_rule_with_paragraph() {
    let mut editor = default_editor();
    editor
        .set_html("<p>Hello</p>")
        .expect("set_html should succeed");
    editor.set_selection(Selection::cursor(3));

    let insert_update = editor
        .insert_node(3, "horizontalRule")
        .expect("horizontal rule insertion should succeed");
    let first_cursor = insert_update.selection.from(editor.document());
    let first_scalar = editor.doc_to_scalar(first_cursor);
    assert!(
        first_scalar > 0,
        "cursor after hr insert should support backspace"
    );

    let update = editor
        .delete_scalar_range(first_scalar - 1, first_scalar)
        .expect("first backspace after hr insert should not panic");
    assert_eq!(
        editor.get_html(),
        "<p>Hello</p><p></p>",
        "backspace from the empty paragraph immediately below an hr should replace the rule with an empty paragraph"
    );

    let insert_pos = update.selection.from(editor.document());
    editor
        .insert_text(insert_pos, "B")
        .expect("typing after horizontal rule removal should stay on the replacement line");
    assert_eq!(
        editor.get_html(),
        "<p>Hello</p><p>B</p>",
        "typing after horizontal rule removal should target the replacement paragraph, not the preceding block"
    );
}

#[test]
fn test_backspace_below_image_replaces_image_with_paragraph() {
    let mut editor = default_editor();
    editor
        .set_html("<p>Hello</p>")
        .expect("set_html should succeed");
    editor.set_selection(Selection::cursor(3));

    let image_doc = serde_json::json!({
        "type": "doc",
        "content": [
            {
                "type": "image",
                "attrs": {
                    "src": "https://example.com/cat.png",
                    "alt": "Cat"
                }
            }
        ]
    });

    let insert_update = editor
        .insert_content_json(&image_doc)
        .expect("image insertion should succeed");
    let first_cursor = insert_update.selection.from(editor.document());
    let first_scalar = editor.doc_to_scalar(first_cursor);
    assert!(
        first_scalar > 0,
        "cursor after image insert should support backspace"
    );

    let update = editor
        .delete_scalar_range(first_scalar - 1, first_scalar)
        .expect("first backspace after image insert should not panic");
    assert_eq!(
        editor.get_html(),
        "<p>Hello</p><p></p>",
        "backspace from the empty paragraph immediately below an image should replace it with an empty paragraph"
    );

    let insert_pos = update.selection.from(editor.document());
    editor
        .insert_text(insert_pos, "B")
        .expect("typing after image removal should stay on the replacement line");
    assert_eq!(
        editor.get_html(),
        "<p>Hello</p><p>B</p>",
        "typing after image removal should target the replacement paragraph, not the preceding block"
    );
}

#[test]
fn test_insert_horizontal_rule_in_empty_paragraph_replaces_blank_line() {
    let mut editor = default_editor();
    editor
        .set_html("<p>Hello</p><p></p>")
        .expect("set_html should succeed");
    editor.set_selection(Selection::cursor(8));

    let update = editor
        .insert_node(8, "horizontalRule")
        .expect("horizontal rule insertion should succeed");

    assert_eq!(
        editor.get_html(),
        "<p>Hello</p><hr><p></p>",
        "inserting a horizontal rule from a blank line should replace the empty paragraph"
    );
    assert!(
        update.selection.from(editor.document()) > 7,
        "selection should land below the inserted horizontal rule"
    );
}

#[test]
fn test_boundary_backspace_before_horizontal_rule_returns_error_instead_of_panicking() {
    let mut editor = default_editor();
    editor
        .set_html("<p>Hello</p><hr>")
        .expect("set_html should succeed");
    editor.set_selection(Selection::cursor(7));

    let cursor_scalar = editor.doc_to_scalar(7);
    assert!(
        cursor_scalar > 0,
        "cursor before a horizontal rule should map to a backspace-able scalar"
    );

    let invalid_delete_result = editor.delete_scalar_range(cursor_scalar - 1, cursor_scalar);
    assert!(
        invalid_delete_result.is_err(),
        "the old boundary delete should be rejected cleanly instead of panicking"
    );
    assert_eq!(
        editor.get_html(),
        "<p>Hello</p><hr>",
        "rejecting the invalid boundary delete should leave the document unchanged"
    );
}

// ===========================================================================
// Set/Get JSON roundtrip
// ===========================================================================

#[test]
fn test_set_json_get_json_roundtrip() {
    let mut editor = default_editor();

    let json = serde_json::json!({
        "type": "doc",
        "content": [
            {
                "type": "paragraph",
                "content": [
                    {"type": "text", "text": "JSON content"}
                ]
            }
        ]
    });

    editor.set_json(&json).expect("set_json should succeed");
    let output = editor.get_json();
    assert_eq!(output["type"], "doc", "Output JSON should have type 'doc'");
    let content = &output["content"][0]["content"][0]["text"];
    assert_eq!(
        content, "JSON content",
        "Output JSON should contain original text"
    );
}

// ===========================================================================
// Active state
// ===========================================================================

#[test]
fn test_active_marks_inside_bold_text() {
    let mut editor = default_editor();
    editor
        .set_html("<p><strong>Bold text</strong></p>")
        .expect("set_html should succeed");

    // Cursor inside the bold text (position 2 = after 'B').
    editor.set_selection(Selection::cursor(2));
    let active = editor.active_marks();
    assert!(
        active.contains(&"bold".to_string()),
        "active_marks should include 'bold' when cursor is inside bold text, got: {:?}",
        active
    );
}

#[test]
fn test_active_marks_outside_bold_text() {
    let mut editor = default_editor();
    editor
        .set_html("<p>plain <strong>bold</strong> plain</p>")
        .expect("set_html should succeed");

    // Cursor at position 1 (inside "plain", before any bold text).
    editor.set_selection(Selection::cursor(1));
    let active = editor.active_marks();
    assert!(
        !active.contains(&"bold".to_string()),
        "active_marks should not include 'bold' when cursor is in plain text, got: {:?}",
        active
    );
}

#[test]
fn test_toggle_mark_at_cursor_preserves_existing_text_on_insert() {
    let mut editor = default_editor();
    editor
        .set_html("<p>abc</p>")
        .expect("set_html should succeed");

    // Cursor at the end of the paragraph content.
    editor.set_selection(Selection::cursor(4));

    let toggle_update = editor
        .toggle_mark("bold")
        .expect("collapsed toggle_mark should succeed");
    assert_eq!(toggle_update.selection, Selection::cursor(4));
    assert_eq!(editor.get_html(), "<p>abc</p>");

    let insert_update = editor
        .insert_text(4, "d")
        .expect("insert_text should succeed after collapsed toggle");
    assert_eq!(insert_update.selection, Selection::cursor(5));
    assert_eq!(editor.get_html(), "<p>abc<strong>d</strong></p>");
}

#[test]
fn test_toggle_mark_at_cursor_preserves_existing_text_across_multiple_inserts() {
    let mut editor = default_editor();
    editor
        .set_html("<p>abc</p>")
        .expect("set_html should succeed");

    editor.set_selection(Selection::cursor(4));

    editor
        .toggle_mark("bold")
        .expect("collapsed toggle_mark should succeed");

    let first_insert = editor
        .insert_text(4, " ")
        .expect("first insert should succeed");
    assert_eq!(first_insert.selection, Selection::cursor(5));
    assert_eq!(editor.get_html(), "<p>abc<strong> </strong></p>");

    let second_insert = editor
        .insert_text(5, "d")
        .expect("second insert should succeed");
    assert_eq!(second_insert.selection, Selection::cursor(6));
    assert_eq!(editor.get_html(), "<p>abc<strong> d</strong></p>");
}

#[test]
fn test_active_nodes_in_paragraph() {
    let mut editor = default_editor();
    editor
        .set_html("<p>Hello</p>")
        .expect("set_html should succeed");

    editor.set_selection(Selection::cursor(1));
    let active = editor.active_nodes();
    assert!(
        active.contains(&"paragraph".to_string()),
        "active_nodes should include 'paragraph', got: {:?}",
        active
    );
    assert!(
        active.contains(&"doc".to_string()),
        "active_nodes should include 'doc', got: {:?}",
        active
    );
}

#[test]
fn test_active_nodes_in_list() {
    let mut editor = default_editor();
    editor
        .set_html("<ul><li><p>Item</p></li></ul>")
        .expect("set_html should succeed");

    // Position inside the list item's paragraph:
    // doc(1) > bulletList(1) > listItem(1) > paragraph(1) > "Item"
    // Positions: 0=doc content start, doc contains bulletList.
    // bulletList open = pos 0, listItem open = pos 1, paragraph open = pos 2, "I" = pos 3.
    editor.set_selection(Selection::cursor(3));
    let active = editor.active_nodes();
    assert!(
        active.contains(&"bulletList".to_string()),
        "active_nodes should include 'bulletList', got: {:?}",
        active
    );
    assert!(
        active.contains(&"listItem".to_string()),
        "active_nodes should include 'listItem', got: {:?}",
        active
    );
}

// ===========================================================================
// Multiple editors (registry)
// ===========================================================================

#[test]
fn test_multiple_editors_independent_state() {
    let id1 = EditorRegistry::create(tiptap_schema(), InterceptorPipeline::new(), false);
    let id2 = EditorRegistry::create(tiptap_schema(), InterceptorPipeline::new(), false);

    assert_ne!(id1, id2, "Two editors should have different IDs");
    assert_ne!(id1, INVALID_EDITOR_ID, "ID should not be INVALID_EDITOR_ID");
    assert_ne!(id2, INVALID_EDITOR_ID, "ID should not be INVALID_EDITOR_ID");

    // Set different content in each editor.
    {
        let arc1 = EditorRegistry::get(id1).expect("editor 1 should exist");
        let mut ed1 = arc1.lock().expect("lock should not be poisoned");
        ed1.set_html("<p>Editor one</p>").expect("set_html 1");

        let arc2 = EditorRegistry::get(id2).expect("editor 2 should exist");
        let mut ed2 = arc2.lock().expect("lock should not be poisoned");
        ed2.set_html("<p>Editor two</p>").expect("set_html 2");
    }

    // Verify they have independent state.
    {
        let arc1 = EditorRegistry::get(id1).expect("editor 1 should exist");
        let ed1 = arc1.lock().unwrap();
        assert!(
            ed1.get_html().contains("Editor one"),
            "Editor 1 should contain 'Editor one'"
        );

        let arc2 = EditorRegistry::get(id2).expect("editor 2 should exist");
        let ed2 = arc2.lock().unwrap();
        assert!(
            ed2.get_html().contains("Editor two"),
            "Editor 2 should contain 'Editor two'"
        );
    }

    // Destroy one, other still works.
    EditorRegistry::destroy(id1);
    assert!(
        EditorRegistry::get(id1).is_none(),
        "Destroyed editor should not be retrievable"
    );

    let arc2 =
        EditorRegistry::get(id2).expect("editor 2 should still exist after destroying editor 1");
    let ed2 = arc2.lock().unwrap();
    assert!(
        ed2.get_html().contains("Editor two"),
        "Editor 2 should still work after destroying editor 1"
    );

    // Cleanup.
    drop(ed2);
    drop(arc2);
    EditorRegistry::destroy(id2);
}

#[test]
fn test_registry_get_nonexistent_returns_none() {
    assert!(
        EditorRegistry::get(999_999).is_none(),
        "Getting a non-existent editor should return None"
    );
}

// ===========================================================================
// Interceptors
// ===========================================================================

#[test]
fn test_max_length_interceptor_allows_under_limit() {
    let mut editor = editor_with_max_length(20);
    editor
        .set_html("<p>Hello</p>")
        .expect("set_html should succeed");

    // "Hello" = 5 chars. Insert 5 more = 10 total, under limit of 20.
    let result = editor.insert_text(6, " World");
    assert!(
        result.is_ok(),
        "insert_text should succeed under max length limit, got: {:?}",
        result.err()
    );
    assert!(
        editor.get_html().contains("Hello World"),
        "HTML should contain 'Hello World'"
    );
}

#[test]
fn test_max_length_interceptor_rejects_over_limit() {
    let mut editor = editor_with_max_length(10);
    editor
        .set_html("<p>Hello</p>")
        .expect("set_html should succeed");

    // "Hello" = 5 chars. Try to insert 10 more chars = 15 total > limit of 10.
    let result = editor.insert_text(6, " wonderful world!");
    assert!(
        result.is_err(),
        "insert_text should fail when exceeding max length limit"
    );

    // Document should be unchanged.
    let html = editor.get_html();
    assert_eq!(
        html, "<p>Hello</p>",
        "Document should be unchanged after rejected insertion, got: {}",
        html
    );
}

#[test]
fn test_max_length_interceptor_at_exact_limit() {
    let mut editor = editor_with_max_length(11);
    editor
        .set_html("<p>Hello</p>")
        .expect("set_html should succeed");

    // "Hello" = 5 chars. Insert " World" = 6 more = 11 total = exact limit.
    let result = editor.insert_text(6, " World");
    assert!(
        result.is_ok(),
        "insert_text should succeed at exact max length limit, got: {:?}",
        result.err()
    );
    assert!(
        editor.get_html().contains("Hello World"),
        "HTML should contain 'Hello World' at exact limit"
    );
}

// ===========================================================================
// Position mapping
// ===========================================================================

#[test]
fn test_scalar_to_doc_and_back_roundtrip() {
    let mut editor = default_editor();
    editor
        .set_html("<p>Hello world</p>")
        .expect("set_html should succeed");

    // For a simple paragraph, scalar offsets should map 1:1 with doc positions
    // (offset by the paragraph's open tag).
    // scalar 0 -> doc pos 1 (inside the paragraph, at start of "Hello")
    let doc_pos = editor.scalar_to_doc(0);
    let scalar_back = editor.doc_to_scalar(doc_pos);
    assert_eq!(
        scalar_back, 0,
        "scalar_to_doc -> doc_to_scalar should roundtrip, got scalar_back={}",
        scalar_back
    );

    // Middle of text.
    let doc_pos_mid = editor.scalar_to_doc(5);
    let scalar_mid_back = editor.doc_to_scalar(doc_pos_mid);
    assert_eq!(
        scalar_mid_back, 5,
        "Mid-text roundtrip: expected 5, got {}",
        scalar_mid_back
    );
}

#[test]
fn test_normalize_pos_snaps_to_content() {
    let mut editor = default_editor();
    editor
        .set_html("<p>Hello</p><p>World</p>")
        .expect("set_html should succeed");

    // Position 0 is at the doc content start, before the first paragraph's
    // open tag. Normalizing should snap to position 1 (inside first paragraph).
    let normalized = editor.normalize_pos(0);
    assert!(
        normalized >= 1,
        "normalize_pos(0) should snap to inside first paragraph (>=1), got {}",
        normalized
    );
}

// ===========================================================================
// Delete and join operations
// ===========================================================================

#[test]
fn test_delete_range_removes_text() {
    let mut editor = default_editor();
    editor
        .set_html("<p>Hello world</p>")
        .expect("set_html should succeed");

    // Delete "world" (positions 7-12).
    //   H=1, e=2, l=3, l=4, o=5, ' '=6, w=7, o=8, r=9, l=10, d=11
    let result = editor.delete_range(7, 12);
    assert!(
        result.is_ok(),
        "delete_range should succeed, got: {:?}",
        result.err()
    );
    let html = editor.get_html();
    assert!(
        html.contains("Hello "),
        "HTML should contain 'Hello ' after deleting 'world', got: {}",
        html
    );
    assert!(
        !html.contains("world"),
        "HTML should not contain 'world' after deletion, got: {}",
        html
    );
}

#[test]
fn test_split_block_creates_new_paragraph() {
    let mut editor = default_editor();
    editor
        .set_html("<p>HelloWorld</p>")
        .expect("set_html should succeed");

    // Split at position 6 (after "Hello").
    let result = editor.split_block(6);
    assert!(
        result.is_ok(),
        "split_block should succeed, got: {:?}",
        result.err()
    );
    let html = editor.get_html();
    assert!(
        html.contains("<p>Hello</p>") && html.contains("<p>World</p>"),
        "HTML should contain two paragraphs after split, got: {}",
        html
    );
}

// ===========================================================================
// History state tracking
// ===========================================================================

#[test]
fn test_can_undo_can_redo_tracking() {
    let mut editor = default_editor();
    editor.set_html("<p>Hello</p>").expect("set_html");

    assert!(
        !editor.can_undo(),
        "can_undo should be false before any edits"
    );
    assert!(
        !editor.can_redo(),
        "can_redo should be false before any edits"
    );

    // Make an edit.
    editor.insert_text(6, "!").expect("insert_text");
    assert!(editor.can_undo(), "can_undo should be true after an edit");
    assert!(
        !editor.can_redo(),
        "can_redo should still be false (no undo performed)"
    );

    // Undo.
    editor.undo();
    assert!(editor.can_redo(), "can_redo should be true after undo");

    // Redo.
    editor.redo();
    assert!(!editor.can_redo(), "can_redo should be false after redo");
    assert!(
        editor.can_undo(),
        "can_undo should still be true after redo"
    );
}

#[test]
fn test_undo_returns_none_when_nothing_to_undo() {
    let mut editor = default_editor();
    assert!(
        editor.undo().is_none(),
        "undo should return None when there's nothing to undo"
    );
}

#[test]
fn test_redo_returns_none_when_nothing_to_redo() {
    let mut editor = default_editor();
    assert!(
        editor.redo().is_none(),
        "redo should return None when there's nothing to redo"
    );
}

// ===========================================================================
// Editor update structure
// ===========================================================================

#[test]
fn test_editor_update_contains_all_fields() {
    let mut editor = default_editor();
    editor.set_html("<p>Hello</p>").expect("set_html");

    let update = editor.insert_text(6, "!").expect("insert_text");
    assert!(
        !update.render_elements.is_empty(),
        "EditorUpdate should contain render elements"
    );
    // Selection should be valid.
    match &update.selection {
        Selection::Text { anchor, head } => {
            assert!(
                *anchor > 0 && *head > 0,
                "Selection positions should be positive after insert"
            );
        }
        _ => {} // other selection types are also valid
    }
    // History state should reflect that we can undo.
    assert!(
        update.history_state.can_undo,
        "History state should indicate can_undo after edit"
    );
}

// ===========================================================================
// UniFFI-exported functions
// ===========================================================================

#[test]
fn test_uniffi_editor_create_destroy_lifecycle() {
    let id = editor_core::editor_create("{}".to_string());
    assert_ne!(id, 0, "Editor ID should not be 0");

    // Set HTML through the UniFFI interface.
    let result = editor_core::editor_set_html(id, "<p>Test</p>".to_string());
    assert!(
        !result.contains("error"),
        "editor_set_html should succeed, got: {}",
        result
    );

    // Get HTML back.
    let html = editor_core::editor_get_html(id);
    assert!(
        html.contains("Test"),
        "editor_get_html should return content, got: {}",
        html
    );

    // Insert text.
    let insert_result = editor_core::editor_insert_text(id, 1, "Hello ".to_string());
    assert!(
        !insert_result.contains("error"),
        "editor_insert_text should succeed, got: {}",
        insert_result
    );

    // Verify updated content.
    let html_after = editor_core::editor_get_html(id);
    assert!(
        html_after.contains("Hello Test"),
        "HTML should contain 'Hello Test', got: {}",
        html_after
    );

    // Undo.
    assert!(editor_core::editor_can_undo(id), "should be able to undo");
    let undo_result = editor_core::editor_undo(id);
    assert!(
        !undo_result.is_empty(),
        "undo should return a non-empty result"
    );

    // Verify undo worked.
    let html_after_undo = editor_core::editor_get_html(id);
    assert!(
        !html_after_undo.contains("Hello "),
        "HTML should not contain 'Hello ' after undo, got: {}",
        html_after_undo
    );

    // Redo.
    assert!(editor_core::editor_can_redo(id), "should be able to redo");
    let redo_result = editor_core::editor_redo(id);
    assert!(
        !redo_result.is_empty(),
        "redo should return a non-empty result"
    );

    // Destroy.
    editor_core::editor_destroy(id);
    let html_after_destroy = editor_core::editor_get_html(id);
    assert!(
        html_after_destroy.is_empty(),
        "editor_get_html on destroyed editor should return empty, got: {}",
        html_after_destroy
    );
}

#[test]
fn test_uniffi_editor_toggle_mark() {
    let id = editor_core::editor_create("{}".to_string());
    editor_core::editor_set_html(id, "<p>Hello World</p>".to_string());

    // Select "Hello" (positions 1-6).
    editor_core::editor_set_selection(id, 1, 6);

    // Toggle bold.
    let result = editor_core::editor_toggle_mark(id, "bold".to_string());
    assert!(
        !result.contains("error"),
        "toggle_mark should succeed, got: {}",
        result
    );

    let html = editor_core::editor_get_html(id);
    assert!(
        html.contains("<strong>Hello</strong>"),
        "HTML should contain bold 'Hello', got: {}",
        html
    );

    // Cleanup.
    editor_core::editor_destroy(id);
}

#[test]
fn test_uniffi_editor_toggle_mark_at_selection_scalar() {
    let id = editor_core::editor_create("{}".to_string());
    editor_core::editor_set_html(id, "<p>Hello World</p>".to_string());

    let scalar_anchor = editor_core::editor_doc_to_scalar(id, 1);
    let scalar_head = editor_core::editor_doc_to_scalar(id, 6);
    let result = editor_core::editor_toggle_mark_at_selection_scalar(
        id,
        scalar_anchor,
        scalar_head,
        "bold".to_string(),
    );
    assert!(
        !result.contains("error"),
        "toggle_mark_at_selection_scalar should succeed, got: {}",
        result
    );

    let html = editor_core::editor_get_html(id);
    assert!(
        html.contains("<strong>Hello</strong>"),
        "HTML should contain bold 'Hello', got: {}",
        html
    );

    editor_core::editor_destroy(id);
}

#[test]
fn test_uniffi_editor_toggle_blockquote_at_selection_scalar() {
    let id = editor_core::editor_create("{}".to_string());
    editor_core::editor_set_html(id, "<p>Hello</p><p>World</p>".to_string());

    let scalar_anchor = editor_core::editor_doc_to_scalar(id, 1);
    let scalar_head = editor_core::editor_doc_to_scalar(id, 1);
    let result =
        editor_core::editor_toggle_blockquote_at_selection_scalar(id, scalar_anchor, scalar_head);
    assert!(
        !result.contains("error"),
        "toggle_blockquote_at_selection_scalar should succeed, got: {}",
        result
    );

    let html = editor_core::editor_get_html(id);
    assert_eq!(html, "<blockquote><p>Hello</p></blockquote><p>World</p>");

    editor_core::editor_destroy(id);
}

#[test]
fn test_uniffi_editor_toggle_heading_at_selection_scalar() {
    let id = editor_core::editor_create("{}".to_string());
    editor_core::editor_set_html(id, "<p>Hello</p><p>World</p>".to_string());

    let scalar_anchor = editor_core::editor_doc_to_scalar(id, 1);
    let scalar_head = editor_core::editor_doc_to_scalar(id, 13);
    let result =
        editor_core::editor_toggle_heading_at_selection_scalar(id, scalar_anchor, scalar_head, 4);
    assert!(
        !result.contains("error"),
        "toggle_heading_at_selection_scalar should succeed, got: {}",
        result
    );

    let html = editor_core::editor_get_html(id);
    assert_eq!(html, "<h4>Hello</h4><h4>World</h4>");

    editor_core::editor_destroy(id);
}

#[test]
fn test_uniffi_editor_insert_node_at_selection_scalar() {
    let id = editor_core::editor_create("{}".to_string());
    editor_core::editor_set_html(id, "<p>Hello</p>".to_string());

    let scalar_anchor = editor_core::editor_doc_to_scalar(id, 3);
    let scalar_head = editor_core::editor_doc_to_scalar(id, 5);
    let result = editor_core::editor_insert_node_at_selection_scalar(
        id,
        scalar_anchor,
        scalar_head,
        "hardBreak".to_string(),
    );
    assert!(
        !result.contains("error"),
        "insert_node_at_selection_scalar should succeed, got: {}",
        result
    );

    let html = editor_core::editor_get_html(id);
    assert_eq!(
        html, "<p>He<br>o</p>",
        "hardBreak insertion at an explicit scalar selection should replace the selected text"
    );

    editor_core::editor_destroy(id);
}

#[test]
fn test_uniffi_editor_json_roundtrip() {
    let id = editor_core::editor_create("{}".to_string());

    let json_str = r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"JSON test"}]}]}"#;
    let result = editor_core::editor_set_json(id, json_str.to_string());
    assert!(
        !result.contains("error"),
        "editor_set_json should succeed, got: {}",
        result
    );

    let output = editor_core::editor_get_json(id);
    assert!(
        output.contains("JSON test"),
        "editor_get_json should contain original text, got: {}",
        output
    );

    // Cleanup.
    editor_core::editor_destroy(id);
}

#[test]
fn test_uniffi_editor_with_max_length() {
    let id = editor_core::editor_create("{\"maxLength\":10}".to_string());
    editor_core::editor_set_html(id, "<p>Hello</p>".to_string());

    // Try to insert text that would exceed the limit.
    let result = editor_core::editor_insert_text(id, 6, " this is way too long".to_string());
    assert!(
        result.contains("error"),
        "insert_text exceeding max length should return error, got: {}",
        result
    );

    // Verify document unchanged.
    let html = editor_core::editor_get_html(id);
    assert_eq!(
        html, "<p>Hello</p>",
        "Document should be unchanged after rejected insertion"
    );

    // Cleanup.
    editor_core::editor_destroy(id);
}

// ===========================================================================
// Edge cases
// ===========================================================================

#[test]
fn test_empty_editor_has_empty_paragraph() {
    let editor = default_editor();
    let html = editor.get_html();
    assert_eq!(
        html, "<p></p>",
        "New editor should have an empty paragraph, got: {}",
        html
    );
}

#[test]
fn test_set_html_replaces_previous_content() {
    let mut editor = default_editor();
    editor.set_html("<p>First</p>").expect("set_html 1");
    editor.set_html("<p>Second</p>").expect("set_html 2");
    let html = editor.get_html();
    assert!(
        html.contains("Second") && !html.contains("First"),
        "set_html should replace previous content, got: {}",
        html
    );
}

#[test]
fn test_multiple_inserts_and_undo_all() {
    let mut editor = default_editor();
    editor.set_html("<p></p>").expect("set_html");

    // Insert 'A', 'B', 'C' one at a time.
    editor.insert_text(1, "A").expect("insert A");
    editor.insert_text(2, "B").expect("insert B");
    editor.insert_text(3, "C").expect("insert C");

    let html = editor.get_html();
    assert!(
        html.contains("ABC"),
        "HTML should contain 'ABC', got: {}",
        html
    );

    // Undo all three inserts. Note: they may be grouped by the history
    // module (sequential Input-source inserts within 500ms merge).
    // So we may need only one or two undos.
    let mut undo_count = 0;
    while editor.can_undo() {
        editor.undo();
        undo_count += 1;
        if undo_count > 10 {
            panic!("Too many undos — something is wrong");
        }
    }

    let html_final = editor.get_html();
    assert_eq!(
        html_final, "<p></p>",
        "After undoing all inserts, should have empty paragraph, got: {}",
        html_final
    );
    assert!(undo_count >= 1, "Should have performed at least one undo");
}

#[test]
fn test_delete_range_then_undo_restores() {
    let mut editor = default_editor();
    editor.set_html("<p>Hello World</p>").expect("set_html");

    // Delete " World" (positions 6-12).
    editor.delete_range(6, 12).expect("delete_range");
    let html_after_delete = editor.get_html();
    assert_eq!(
        html_after_delete, "<p>Hello</p>",
        "After delete, should have 'Hello', got: {}",
        html_after_delete
    );

    // Undo should restore.
    editor.undo().expect("undo should work");
    let html_after_undo = editor.get_html();
    assert!(
        html_after_undo.contains("Hello World"),
        "After undo, should have 'Hello World', got: {}",
        html_after_undo
    );
}

// ===========================================================================
// Selection state in updates
// ===========================================================================

#[test]
fn test_selection_updates_after_insert() {
    let mut editor = default_editor();
    editor.set_html("<p>AB</p>").expect("set_html");
    editor.set_selection(Selection::cursor(2)); // between A and B

    let update = editor.insert_text(2, "X").expect("insert X");
    // After inserting 'X' at position 2, the cursor should have advanced.
    match &update.selection {
        Selection::Text { anchor, head } => {
            assert!(
                *anchor >= 3 || *head >= 3,
                "Cursor should advance past inserted text, got anchor={}, head={}",
                anchor,
                head
            );
        }
        _ => {} // other selection types acceptable
    }
}

#[test]
fn test_split_list_item_moves_selection_to_new_item() {
    let mut editor = default_editor();
    editor
        .set_html("<ul><li><p>Hello</p></li></ul>")
        .expect("set_html");

    // End of the paragraph content inside the list item.
    editor.set_selection(Selection::cursor(8));

    let update = editor.split_block(8).expect("split_block in list item");
    assert_eq!(
        editor.get_html(),
        "<ul><li><p>Hello</p></li><li><p></p></li></ul>"
    );

    let split_selection_pos = update.selection.from(editor.document());
    editor
        .insert_text(split_selection_pos, "X")
        .expect("insert into split list item");

    assert_eq!(
        editor.get_html(),
        "<ul><li><p>Hello</p></li><li><p>X</p></li></ul>",
        "typing after split should target the new list item"
    );
}

#[test]
fn test_backspace_from_empty_list_item_prefix_unwraps_list_item() {
    let mut editor = default_editor();
    editor
        .set_html("<ul><li><p>A</p></li></ul>")
        .expect("set_html");

    let text_end = editor.doc_to_scalar(4);
    editor
        .delete_scalar_range(text_end - 1, text_end)
        .expect("delete list item text");
    assert_eq!(editor.get_html(), "<ul><li><p></p></li></ul>");

    let cursor_scalar = editor.doc_to_scalar(3);
    assert!(
        cursor_scalar > 0,
        "empty list item cursor should still sit after the visible marker"
    );

    let update = editor
        .delete_scalar_range(cursor_scalar - 1, cursor_scalar)
        .expect("backspace over list marker prefix should succeed");

    assert_eq!(
        editor.get_html(),
        "<p></p>",
        "backspacing at the start of an empty list item should unwrap it"
    );

    let insert_pos = update.selection.from(editor.document());
    editor
        .insert_text(insert_pos, "B")
        .expect("typing after unwrap should succeed");
    assert_eq!(editor.get_html(), "<p>B</p>");
}

#[test]
fn test_backspace_from_last_empty_list_item_breaks_out_of_list() {
    let mut editor = default_editor();
    editor
        .set_html("<ul><li><p>A</p></li><li><p></p></li></ul>")
        .expect("set_html");
    editor.set_selection(Selection::cursor(8));

    let cursor_scalar = editor.doc_to_scalar(8);
    assert!(
        cursor_scalar > 0,
        "empty trailing list item cursor should sit after the visible marker"
    );

    let update = editor
        .delete_scalar_range(cursor_scalar - 1, cursor_scalar)
        .expect("backspace over trailing list marker should succeed");

    assert_eq!(
        editor.get_html(),
        "<ul><li><p>A</p></li></ul><p></p>",
        "backspacing the trailing empty list item marker should break out of the list"
    );

    let insert_pos = update.selection.from(editor.document());
    editor
        .insert_text(insert_pos, "B")
        .expect("typing after trailing list unwrap should succeed");
    assert_eq!(editor.get_html(), "<ul><li><p>A</p></li></ul><p>B</p>");
}

#[test]
fn test_backspace_again_after_breaking_out_of_list_removes_empty_paragraph() {
    let mut editor = default_editor();
    editor
        .set_html("<ul><li><p>A</p></li><li><p></p></li></ul>")
        .expect("set_html");
    editor.set_selection(Selection::cursor(8));

    let update = editor
        .delete_scalar_range(editor.doc_to_scalar(8) - 1, editor.doc_to_scalar(8))
        .expect("first backspace should unwrap trailing list item");

    assert_eq!(editor.get_html(), "<ul><li><p>A</p></li></ul><p></p>");

    let escaped_cursor = update.selection.from(editor.document());
    assert_eq!(
        escaped_cursor, 8,
        "selection should stay in the lifted paragraph"
    );

    let escaped_scalar = editor.doc_to_scalar(escaped_cursor);
    let second_update = editor
        .delete_scalar_range(escaped_scalar - 1, escaped_scalar)
        .expect("second backspace should remove the empty paragraph");

    assert_eq!(
        editor.get_html(),
        "<ul><li><p>A</p></li></ul>",
        "second backspace should delete the lifted empty paragraph"
    );

    match second_update.selection {
        Selection::Text { anchor, head } => {
            assert_eq!(anchor, 4);
            assert_eq!(head, 4);
        }
        other => panic!(
            "expected collapsed text selection after deleting empty paragraph, got {other:?}"
        ),
    }
}

#[test]
fn test_backspace_from_empty_blockquote_paragraph_breaks_out_of_quote() {
    let mut editor = default_editor();
    editor
        .set_html("<blockquote><p>Hello</p><p></p></blockquote>")
        .expect("set_html");
    editor.set_selection(Selection::cursor(9));

    let cursor_scalar = editor.doc_to_scalar(9);
    assert!(
        cursor_scalar > 0,
        "empty quoted paragraph cursor should sit after a backspace-able rendered boundary"
    );

    let update = editor
        .delete_scalar_range(cursor_scalar - 1, cursor_scalar)
        .expect("backspace from empty blockquote paragraph should succeed");

    assert_eq!(
        editor.get_html(),
        "<blockquote><p>Hello</p></blockquote><p></p>",
        "backspacing at the start of an empty quoted paragraph should break out of the quote"
    );

    let insert_pos = update.selection.from(editor.document());
    editor
        .insert_text(insert_pos, "B")
        .expect("typing after blockquote exit should succeed");
    assert_eq!(
        editor.get_html(),
        "<blockquote><p>Hello</p></blockquote><p>B</p>"
    );
}

#[test]
fn test_backspace_again_after_breaking_out_of_blockquote_removes_empty_paragraph() {
    let mut editor = default_editor();
    editor
        .set_html("<blockquote><p>Hello</p><p></p></blockquote>")
        .expect("set_html");
    editor.set_selection(Selection::cursor(9));

    let update = editor
        .delete_scalar_range(editor.doc_to_scalar(9) - 1, editor.doc_to_scalar(9))
        .expect("first backspace should exit the quote");

    assert_eq!(
        editor.get_html(),
        "<blockquote><p>Hello</p></blockquote><p></p>"
    );

    let escaped_cursor = update.selection.from(editor.document());
    assert_eq!(
        escaped_cursor, 10,
        "selection should stay in the lifted paragraph"
    );

    let escaped_scalar = editor.doc_to_scalar(escaped_cursor);
    let second_update = editor
        .delete_scalar_range(escaped_scalar - 1, escaped_scalar)
        .expect("second backspace should remove the lifted paragraph");

    assert_eq!(
        editor.get_html(),
        "<blockquote><p>Hello</p></blockquote>",
        "second backspace should delete the lifted empty paragraph"
    );

    match second_update.selection {
        Selection::Text { anchor, head } => {
            assert_eq!(anchor, 7);
            assert_eq!(head, 7);
        }
        other => panic!(
            "expected collapsed text selection after deleting empty paragraph, got {other:?}"
        ),
    }
}

#[test]
fn test_backspace_from_blank_document_preserves_paragraph() {
    let mut editor = default_editor();

    let update = editor
        .delete_backward_at_selection_scalar(0, 0)
        .expect("backspace from blank document should succeed");

    assert_eq!(
        editor.get_html(),
        "<p></p>",
        "backspacing a blank default document should keep the empty paragraph"
    );

    match update.selection {
        Selection::Text { anchor, head } => {
            assert_eq!(anchor, 1);
            assert_eq!(head, 1);
        }
        other => panic!(
            "expected collapsed text selection after backspacing blank document, got {other:?}"
        ),
    }
}

#[test]
fn test_backspace_from_empty_first_heading_reverts_to_paragraph() {
    let mut editor = default_editor();
    editor.set_html("<h2>A</h2>").expect("set_html");
    editor.set_selection(Selection::cursor(2));

    let text_end = editor.doc_to_scalar(2);
    editor
        .delete_scalar_range(text_end - 1, text_end)
        .expect("delete heading text");

    assert_eq!(editor.get_html(), "<h2></h2>");

    let empty_heading_cursor = editor.selection().from(editor.document());
    let empty_heading_scalar = editor.doc_to_scalar(empty_heading_cursor);
    assert_eq!(
        empty_heading_scalar, 1,
        "editor-core should still track the empty heading placeholder at scalar 1"
    );

    let update = editor
        .delete_backward_at_selection_scalar(0, 0)
        .expect("backspace from empty first heading should succeed");

    assert_eq!(
        editor.get_html(),
        "<p></p>",
        "backspacing an empty first heading should revert it to a paragraph"
    );

    match update.selection {
        Selection::Text { anchor, head } => {
            assert_eq!(anchor, 1);
            assert_eq!(head, 1);
        }
        other => {
            panic!("expected collapsed text selection after reverting empty heading, got {other:?}")
        }
    }
}

#[test]
fn test_backspace_from_empty_first_required_title_stays_schema_valid() {
    let mut editor = title_first_editor();
    editor
        .set_json(&serde_json::json!({
            "type": "doc",
            "content": [{
                "type": "title",
                "content": [{ "type": "text", "text": "A" }]
            }]
        }))
        .expect("set_json");
    editor.set_selection(Selection::cursor(2));

    let text_end = editor.doc_to_scalar(2);
    editor
        .delete_scalar_range(text_end - 1, text_end)
        .expect("delete title text");

    assert_eq!(
        editor.get_json(),
        serde_json::json!({
            "type": "doc",
            "content": [{
                "type": "title"
            }]
        })
    );

    let update = editor
        .delete_backward_at_selection_scalar(0, 0)
        .expect("backspace from empty first title should not error");

    assert_eq!(
        editor.get_json(),
        serde_json::json!({
            "type": "doc",
            "content": [{
                "type": "title"
            }]
        }),
        "backspacing an empty required title should remain schema-valid"
    );

    match update.selection {
        Selection::Text { anchor, head } => {
            assert_eq!(anchor, 1);
            assert_eq!(head, 1);
        }
        other => {
            panic!("expected collapsed text selection after preserving empty title, got {other:?}")
        }
    }
}

#[test]
fn test_backspace_from_empty_trailing_heading_reverts_to_paragraph() {
    let mut editor = default_editor();
    editor.set_html("<p>Intro</p><h2>A</h2>").expect("set_html");
    editor.set_selection(Selection::cursor(9));

    let text_end = editor.doc_to_scalar(9);
    editor
        .delete_scalar_range(text_end - 1, text_end)
        .expect("delete trailing heading text");

    assert_eq!(editor.get_html(), "<p>Intro</p><h2></h2>");

    let empty_heading_cursor = editor.selection().from(editor.document());
    let empty_heading_scalar = editor.doc_to_scalar(empty_heading_cursor);
    let update = editor
        .delete_backward_at_selection_scalar(empty_heading_scalar, empty_heading_scalar)
        .expect("backspace from empty trailing heading should succeed");

    assert_eq!(
        editor.get_html(),
        "<p>Intro</p><p></p>",
        "backspacing an empty trailing heading should revert it to a paragraph"
    );

    match update.selection {
        Selection::Text { anchor, head } => {
            assert_eq!(anchor, 8);
            assert_eq!(head, 8);
        }
        other => panic!(
            "expected collapsed text selection after reverting empty trailing heading, got {other:?}"
        ),
    }
}

#[test]
fn test_backspace_twice_from_nested_empty_list_item_outdents_then_breaks_out() {
    let mut editor = default_editor();
    editor
        .set_html("<ul><li><p>A</p><ul><li><p></p></li></ul></li></ul>")
        .expect("set_html");
    editor.set_selection(Selection::cursor(8));

    let first_scalar = editor.doc_to_scalar(8);
    let first_update = editor
        .delete_scalar_range(first_scalar - 1, first_scalar)
        .expect("first backspace should outdent nested empty item");

    assert_eq!(
        editor.get_html(),
        "<ul><li><p>A</p><p></p></li></ul>",
        "first backspace should unwrap the empty nested item into the parent list item"
    );

    let second_cursor = first_update.selection.from(editor.document());
    let second_scalar = editor.doc_to_scalar(second_cursor);
    let second_update = editor
        .delete_scalar_range(second_scalar - 1, second_scalar)
        .expect("second backspace should break out of the list");

    assert_eq!(
        editor.get_html(),
        "<ul><li><p>A</p></li></ul><p></p>",
        "second backspace should break the lifted empty paragraph out of the list"
    );

    let final_cursor = second_update.selection.from(editor.document());
    assert_eq!(final_cursor, 8);
}

#[test]
fn test_backspace_twice_after_indenting_empty_list_item_breaks_out_of_list() {
    let mut editor = default_editor();
    editor
        .set_html("<ul><li><p>A</p></li><li><p></p></li></ul>")
        .expect("set_html");
    editor.set_selection(Selection::cursor(8));

    editor.indent_list_item().expect("indent empty list item");

    let first_cursor = editor.get_current_state().selection.from(editor.document());
    let first_scalar = editor.doc_to_scalar(first_cursor);
    let first_update = editor
        .delete_scalar_range(first_scalar - 1, first_scalar)
        .expect("first backspace should outdent nested empty item");

    let second_cursor = first_update.selection.from(editor.document());
    let second_scalar = editor.doc_to_scalar(second_cursor);
    let second_update = editor
        .delete_scalar_range(second_scalar - 1, second_scalar)
        .expect("second backspace should break out of the list");

    assert_eq!(
        editor.get_html(),
        "<ul><li><p>A</p></li></ul><p></p>",
        "second backspace should break the lifted empty paragraph out of the list"
    );
    assert_eq!(second_update.selection.from(editor.document()), 8);
}

#[test]
fn test_apply_list_type_converts_entire_containing_list() {
    let mut editor = default_editor();
    editor
        .set_html("<ul><li><p>One</p></li><li><p>Two</p></li></ul>")
        .expect("set_html");
    editor.set_selection(Selection::cursor(4));

    editor
        .apply_list_type("orderedList")
        .expect("convert containing list to ordered");

    assert_eq!(
        editor.get_html(),
        "<ol><li><p>One</p></li><li><p>Two</p></li></ol>",
        "switching list types should convert the whole containing list"
    );
}

#[test]
fn test_apply_list_type_converts_nearest_nested_list_only() {
    let mut editor = default_editor();
    editor
        .set_html("<ul><li><p>A</p><ul><li><p>B</p></li></ul></li><li><p>C</p></li></ul>")
        .expect("set_html");
    editor.set_selection(Selection::cursor(8));

    editor
        .apply_list_type("orderedList")
        .expect("convert nested containing list to ordered");

    assert_eq!(
        editor.get_html(),
        "<ul><li><p>A</p><ol><li><p>B</p></li></ol></li><li><p>C</p></li></ul>",
        "switching list types inside a nested item should only convert the nearest list"
    );
}

#[test]
fn test_apply_list_type_inside_blockquote_wraps_paragraph() {
    let mut editor = default_editor();
    editor
        .set_html("<blockquote><p>Hello</p></blockquote>")
        .expect("set_html");
    editor.set_selection(Selection::cursor(2));

    editor
        .apply_list_type("orderedList")
        .expect("applying list type inside blockquote should wrap the current paragraph");

    assert_eq!(
        editor.get_html(),
        "<blockquote><ol><li><p>Hello</p></li></ol></blockquote>",
        "lists should be allowed as block children inside blockquotes"
    );
}

#[test]
fn test_insert_node_at_selection_replaces_selected_text_with_hard_break() {
    let mut editor = default_editor();
    editor.set_html("<p>Hello</p>").expect("set_html");
    editor.set_selection(Selection::text(3, 5));

    let update = editor
        .insert_node_at_selection("hardBreak")
        .expect("replace selection with hardBreak");

    assert_eq!(
        editor.get_html(),
        "<p>He<br>o</p>",
        "hardBreak insertion at the current selection should replace the selected text"
    );
    assert_eq!(
        update.selection,
        Selection::cursor(4),
        "cursor should land immediately after the inserted hardBreak"
    );
}

#[test]
fn test_list_item_hard_break_scalar_mapping_after_insert() {
    let mut editor = default_editor();
    editor
        .set_html("<ul><li><p>A</p></li></ul>")
        .expect("set_html");

    editor
        .insert_node_at_selection_scalar(3, 3, "hardBreak")
        .expect("insert first hardBreak");

    assert_eq!(
        editor.get_html(),
        "<ul><li><p>A<br></p></li></ul>",
        "first hardBreak should preserve the list item text"
    );
    assert_eq!(
        editor.scalar_to_doc(4),
        5,
        "scalar offset after the first hardBreak should map to the paragraph end"
    );
    assert_eq!(
        editor.doc_to_scalar(5),
        4,
        "paragraph end should map back to the scalar after the first hardBreak"
    );
}

#[test]
fn test_insert_node_at_doc_end_after_list_item_hard_break_preserves_text() {
    let mut editor = default_editor();
    editor
        .set_html("<ul><li><p>A<br></p></li></ul>")
        .expect("set_html");

    editor
        .insert_node(5, "hardBreak")
        .expect("insert second hardBreak at paragraph end");

    assert_eq!(
        editor.get_html(),
        "<ul><li><p>A<br><br></p></li></ul>",
        "inserting another hardBreak at the list paragraph end should append after the existing break"
    );
}

#[test]
fn test_insert_node_at_selection_scalar_after_list_item_hard_break_preserves_text() {
    let mut editor = default_editor();
    editor
        .set_html("<ul><li><p>A</p></li></ul>")
        .expect("set_html");

    editor
        .insert_node_at_selection_scalar(3, 3, "hardBreak")
        .expect("insert first hardBreak");
    editor
        .insert_node_at_selection_scalar(4, 4, "hardBreak")
        .expect("insert second hardBreak");

    assert_eq!(
        editor.get_html(),
        "<ul><li><p>A<br><br></p></li></ul>",
        "scalar insertion after the first hardBreak should append without replacing the existing text"
    );
}

#[test]
fn test_insert_content_json_at_selection_scalar_inserts_mention_and_preserves_attrs() {
    let mut editor = mention_editor();
    editor.set_html("<p>Hello @al</p>").expect("set_html");

    let mention_doc = serde_json::json!({
        "type": "doc",
        "content": [
            {
                "type": "mention",
                "attrs": {
                    "id": "u1",
                    "kind": "user",
                    "label": "@Alice"
                }
            }
        ]
    });

    let update = editor
        .insert_content_json_at_selection_scalar(6, 9, &mention_doc)
        .expect("replace @mention query with mention node");

    let json = editor.get_json();
    let paragraph = &json["content"][0];
    assert_eq!(
        paragraph["content"][0]["text"], "Hello ",
        "mention insertion should keep the prefix text"
    );
    assert_eq!(paragraph["content"][1]["type"], "mention");
    assert_eq!(paragraph["content"][1]["attrs"]["id"], "u1");
    assert_eq!(paragraph["content"][1]["attrs"]["kind"], "user");
    assert_eq!(paragraph["content"][1]["attrs"]["label"], "@Alice");

    let html = editor.get_html();
    assert!(
        html.contains("data-native-editor-mention=\"true\""),
        "mention insertion should serialize to the native mention span, got: {html}"
    );
    assert!(
        html.contains("@Alice"),
        "mention insertion should preserve the visible label, got: {html}"
    );
    assert_eq!(
        update.selection,
        Selection::cursor(8),
        "caret should land immediately after the inserted inline mention"
    );
}

#[test]
fn test_insert_content_json_block_image_resolves_to_block_level() {
    let mut editor = default_editor();
    editor.set_html("<p>Hello</p>").expect("set_html");
    editor.set_selection(Selection::cursor(3));

    let image_doc = serde_json::json!({
        "type": "doc",
        "content": [
            {
                "type": "image",
                "attrs": {
                    "src": "https://example.com/cat.png",
                    "alt": "Cat"
                }
            }
        ]
    });

    let update = editor
        .insert_content_json(&image_doc)
        .expect("insert image JSON at cursor");

    let html = editor.get_html();
    assert!(
        html.starts_with("<p>Hello</p><img "),
        "image JSON insertion should resolve to the block level after the paragraph, got: {html}"
    );
    assert!(
        html.contains("src=\"https://example.com/cat.png\""),
        "image JSON insertion should preserve attrs, got: {html}"
    );
    assert!(
        html.ends_with("<p></p>"),
        "image JSON insertion should leave a trailing paragraph for continued typing, got: {html}"
    );

    let json = editor.get_json();
    assert_eq!(json["content"][1]["type"], "image");
    assert_eq!(json["content"][1]["attrs"]["alt"], "Cat");
    assert_eq!(json["content"][2]["type"], "paragraph");
    assert!(
        update.selection.from(editor.document()) > 7,
        "selection should land below the inserted image"
    );
}

#[test]
fn test_insert_content_html_block_image_resolves_to_block_level() {
    let mut editor = default_editor();
    editor.set_html("<p>Hello</p>").expect("set_html");
    editor.set_selection(Selection::cursor(3));

    let update = editor
        .insert_content_html("<img src=\"https://example.com/cat.png\" alt=\"Cat\">")
        .expect("insert image HTML at cursor");

    let html = editor.get_html();
    assert!(
        html.starts_with("<p>Hello</p><img "),
        "image HTML insertion should resolve to the block level after the paragraph, got: {html}"
    );
    assert!(
        html.contains("src=\"https://example.com/cat.png\""),
        "image HTML insertion should preserve attrs, got: {html}"
    );
    assert!(
        html.ends_with("<p></p>"),
        "image HTML insertion should leave a trailing paragraph for continued typing, got: {html}"
    );
    assert!(
        update.selection.from(editor.document()) > 7,
        "selection should land below the inserted image"
    );
}

#[test]
fn test_resize_image_at_doc_pos_updates_attrs_and_selects_image_node() {
    let mut editor = default_editor();
    editor
        .set_html("<p>Hello</p><img src=\"https://example.com/cat.png\" alt=\"Cat\"><p></p>")
        .expect("set_html");
    editor.set_selection(Selection::cursor(9));

    let update = editor
        .resize_image_at_doc_pos(7, 240, 135)
        .expect("resize image");

    let html = editor.get_html();
    assert!(html.contains("src=\"https://example.com/cat.png\""));
    assert!(html.contains("width=\"240\""));
    assert!(html.contains("height=\"135\""));

    let json = editor.get_json();
    assert_eq!(json["content"][1]["type"], "image");
    assert_eq!(json["content"][1]["attrs"]["width"], 240);
    assert_eq!(json["content"][1]["attrs"]["height"], 135);
    assert_eq!(
        update.selection,
        Selection::node(7),
        "resize should keep the image node selected for continued dragging"
    );
}

#[test]
fn test_indent_list_item_nests_current_item() {
    let mut editor = default_editor();
    editor
        .set_html("<ul><li><p>A</p></li><li><p>B</p></li><li><p>C</p></li></ul>")
        .expect("set_html");
    editor.set_selection(Selection::cursor(8));

    editor.indent_list_item().expect("indent list item");

    assert_eq!(
        editor.get_html(),
        "<ul><li><p>A</p><ul><li><p>B</p></li></ul></li><li><p>C</p></li></ul>"
    );
}

#[test]
fn test_outdent_list_item_keeps_selection_in_lifted_item() {
    let mut editor = default_editor();
    editor
        .set_html("<ul><li><p>A</p><ul><li><p>B</p></li><li><p>C</p></li></ul></li><li><p>D</p></li></ul>")
        .expect("set_html");
    editor.set_selection(Selection::cursor(8));

    let update = editor.outdent_list_item().expect("outdent list item");

    assert_eq!(
        editor.get_html(),
        "<ul><li><p>A</p></li><li><p>B</p><ul><li><p>C</p></li></ul></li><li><p>D</p></li></ul>"
    );

    let insert_pos = update.selection.from(editor.document());
    assert!(
        insert_pos > 4,
        "selection should stay in the lifted item rather than moving to the previous line"
    );
    editor
        .insert_text(insert_pos, "X")
        .expect("typing after outdent should stay in lifted item");

    assert_eq!(
        editor.get_html(),
        "<ul><li><p>A</p></li><li><p>XB</p><ul><li><p>C</p></li></ul></li><li><p>D</p></li></ul>"
    );
}

// ===========================================================================
// Empty list item unwrap on Enter (split_block)
// ===========================================================================

#[test]
fn test_split_block_empty_last_list_item_unwraps() {
    let mut editor = default_editor();
    editor
        .set_html("<ul><li><p>A</p></li><li><p></p></li></ul>")
        .expect("set_html");
    editor.set_selection(Selection::cursor(8));
    editor
        .split_block(8)
        .expect("split_block on empty list item");
    assert_eq!(
        editor.get_html(),
        "<ul><li><p>A</p></li></ul><p></p>",
        "Enter on empty last list item should unwrap to paragraph after list"
    );
}

#[test]
fn test_split_block_empty_mid_list_item_unwraps() {
    let mut editor = default_editor();
    editor
        .set_html("<ul><li><p>A</p></li><li><p></p></li><li><p>B</p></li></ul>")
        .expect("set_html");
    editor.set_selection(Selection::cursor(8));
    editor
        .split_block(8)
        .expect("split_block on empty mid-list item");
    assert_eq!(
        editor.get_html(),
        "<ul><li><p>A</p></li></ul><p></p><ul><li><p>B</p></li></ul>",
        "Enter on empty mid-list item should split list with paragraph between"
    );
}

#[test]
fn test_split_block_empty_only_list_item_unwraps() {
    let mut editor = default_editor();
    editor
        .set_html("<ul><li><p></p></li></ul>")
        .expect("set_html");
    editor.set_selection(Selection::cursor(3));
    editor
        .split_block(3)
        .expect("split_block on empty only list item");
    assert_eq!(
        editor.get_html(),
        "<p></p>",
        "Enter on empty only list item should remove list entirely"
    );
}

#[test]
fn test_split_block_nonempty_list_item_splits_normally() {
    let mut editor = default_editor();
    editor
        .set_html("<ul><li><p>Hello</p></li></ul>")
        .expect("set_html");
    editor.set_selection(Selection::cursor(8));
    editor
        .split_block(8)
        .expect("split_block on non-empty list item");
    assert_eq!(
        editor.get_html(),
        "<ul><li><p>Hello</p></li><li><p></p></li></ul>",
        "Enter on non-empty list item should split normally"
    );
}

#[test]
fn test_split_block_empty_nested_list_item_outdents() {
    let mut editor = default_editor();
    editor
        .set_html("<ul><li><p>A</p><ul><li><p></p></li></ul></li></ul>")
        .expect("set_html");

    // Position inside the empty <p> of the nested <li>.
    // Structure: <ul(0)> <li(1)> <p(2)> A </p(4)> <ul(5)> <li(6)> <p(7)> </p(8)> </li(9)> </ul(10)> </li(11)> </ul(12)>
    // pos 8 is inside the empty <p> of the nested <li>.
    editor.set_selection(Selection::cursor(8));
    editor
        .split_block(8)
        .expect("split_block on empty nested list item");

    assert_eq!(
        editor.get_html(),
        "<ul><li><p>A</p></li><li><p></p></li></ul>",
        "Enter on empty nested list item should outdent to parent list"
    );
}

#[test]
fn test_split_block_empty_doubly_nested_list_item_outdents_one_level() {
    let mut editor = default_editor();
    editor
        .set_html("<ul><li><p>A</p><ul><li><p>B</p><ul><li><p></p></li></ul></li></ul></li></ul>")
        .expect("set_html");

    // The empty <p> in the innermost list item.
    // pos 13 is inside the empty <p>.
    editor.set_selection(Selection::cursor(13));
    editor
        .split_block(13)
        .expect("split_block on doubly nested empty");

    assert_eq!(
        editor.get_html(),
        "<ul><li><p>A</p><ul><li><p>B</p></li><li><p></p></li></ul></li></ul>",
        "Enter on doubly-nested empty list item should outdent one level only"
    );
}

#[test]
fn test_split_block_empty_paragraph_with_sublist_splits_normally() {
    let mut editor = default_editor();
    // List item has empty <p> followed by a nested <ul> — child_count == 2.
    editor
        .set_html("<ul><li><p></p><ul><li><p>Sub</p></li></ul></li></ul>")
        .expect("set_html");

    // pos 3 is inside the empty <p> of the outer list item.
    editor.set_selection(Selection::cursor(3));
    editor
        .split_block(3)
        .expect("split_block on list item with sublist");

    // Should NOT unwrap — the list item has a sublist beneath the empty paragraph.
    // Normal split creates a new list item.
    let html = editor.get_html();
    assert!(
        html.contains("<li><p></p></li><li>"),
        "Enter on empty paragraph with sublist should split normally, not unwrap. Got: {}",
        html
    );
}

// ===========================================================================
// delete_and_split_scalar — empty list item detection after delete
// ===========================================================================

#[test]
fn test_delete_and_split_scalar_empties_list_item_then_unwraps() {
    let mut editor = default_editor();
    editor
        .set_html("<ul><li><p>X</p></li></ul>")
        .expect("set_html");

    // "X" is at scalar position range. Find scalar positions for the text.
    let scalar_start = editor.doc_to_scalar(3); // start of "X"
    let scalar_end = editor.doc_to_scalar(4); // end of "X"

    // Simulate: user selects "X" and presses Enter.
    // This should delete "X" (emptying the list item) then unwrap.
    editor
        .delete_and_split_scalar(scalar_start, scalar_end)
        .expect("delete_and_split_scalar");

    assert_eq!(
        editor.get_html(),
        "<p></p>",
        "delete_and_split_scalar that empties a list item should unwrap, not split"
    );
}

// ===========================================================================
// split_block_scalar — scalar API inherits unwrap/outdent behavior
// ===========================================================================

#[test]
fn test_split_block_scalar_empty_list_item_unwraps() {
    let mut editor = default_editor();
    editor
        .set_html("<ul><li><p></p></li></ul>")
        .expect("set_html");

    // Get scalar position for the cursor inside the empty <p>.
    let scalar_pos = editor.doc_to_scalar(3);
    editor.set_selection(Selection::cursor(3));
    editor
        .split_block_scalar(scalar_pos)
        .expect("split_block_scalar on empty list item");

    assert_eq!(
        editor.get_html(),
        "<p></p>",
        "split_block_scalar on empty only list item should unwrap"
    );
}

#[test]
fn test_split_block_scalar_empty_nested_list_item_outdents() {
    let mut editor = default_editor();
    editor
        .set_html("<ul><li><p>A</p><ul><li><p></p></li></ul></li></ul>")
        .expect("set_html");

    let scalar_pos = editor.doc_to_scalar(8);
    editor.set_selection(Selection::cursor(8));
    editor
        .split_block_scalar(scalar_pos)
        .expect("split_block_scalar on nested empty");

    assert_eq!(
        editor.get_html(),
        "<ul><li><p>A</p></li><li><p></p></li></ul>",
        "split_block_scalar on nested empty should outdent"
    );
}

#[test]
fn test_split_block_scalar_empty_blockquote_paragraph_exits_quote() {
    let mut editor = default_editor();
    editor
        .set_html("<blockquote><p>Hello</p><p></p></blockquote>")
        .expect("set_html");

    let scalar_pos = editor.doc_to_scalar(9);
    editor.set_selection(Selection::cursor(9));
    editor
        .split_block_scalar(scalar_pos)
        .expect("split_block_scalar on empty blockquote paragraph");

    assert_eq!(
        editor.get_html(),
        "<blockquote><p>Hello</p></blockquote><p></p>",
        "split_block_scalar on empty quoted paragraph should exit the quote"
    );
}
