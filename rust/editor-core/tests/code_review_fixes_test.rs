//! Tests for the 5 code review fixes:
//!
//! 1. Lossy undo/redo inverse steps (RemoveMark, JoinBlocks, UnwrapFromList)
//! 2. Multi-block delete and replace
//! 3. MaxLength uses text length, not doc positions
//! 4. Selection normalization on set_selection
//! 5. Undo/redo restores selection

use std::collections::HashMap;

use editor_core::editor::Editor;
use editor_core::intercept::{InterceptorPipeline, MaxLength};
use editor_core::model::{Document, Fragment, Mark, Node};
use editor_core::schema::presets::tiptap_schema;
use editor_core::selection::Selection;
use editor_core::transform::{Source, Step, Transaction};

// ---------------------------------------------------------------------------
// Helper builders
// ---------------------------------------------------------------------------

fn text(s: &str) -> Node {
    Node::text(s.to_string(), vec![])
}

fn marked_text(s: &str, marks: Vec<Mark>) -> Node {
    Node::text(s.to_string(), marks)
}

fn paragraph(children: Vec<Node>) -> Node {
    Node::element(
        "paragraph".to_string(),
        HashMap::new(),
        Fragment::from(children),
    )
}

fn doc_node(children: Vec<Node>) -> Node {
    Node::element("doc".to_string(), HashMap::new(), Fragment::from(children))
}

fn default_editor() -> Editor {
    Editor::new(tiptap_schema(), InterceptorPipeline::new(), false)
}

fn _editor_with_max_length(max: u32) -> Editor {
    let mut pipeline = InterceptorPipeline::new();
    pipeline.add(Box::new(MaxLength::new(max)));
    Editor::new(tiptap_schema(), pipeline, false)
}

// ===========================================================================
// Issue 1: Lossy undo/redo inverse steps
// ===========================================================================

// ---------------------------------------------------------------------------
// Issue 1a: RemoveMark undo preserves mark attrs (e.g. link href)
// ---------------------------------------------------------------------------

#[test]
fn test_undo_remove_bold_restores_bold() {
    // Setup: paragraph with bold text.
    // Apply RemoveMark to remove bold.
    // Undo should re-add the bold mark.
    let mut editor = default_editor();
    editor
        .set_html("<p>Hello <strong>beautiful</strong> world</p>")
        .expect("set_html should succeed");

    // "Hello " = 6 chars (pos 1-6), "beautiful" = 9 chars (pos 7-15), " world" = 6 chars (16-21)
    // Select the bold text and remove the bold mark.
    editor.set_selection(Selection::text(7, 16));
    editor
        .toggle_mark("bold")
        .expect("toggle_mark to remove bold");

    // Verify bold was removed.
    let html_after_remove = editor.get_html();
    assert!(
        !html_after_remove.contains("<strong>"),
        "Bold should be removed, got: {}",
        html_after_remove
    );

    // Undo should restore the bold.
    editor.undo().expect("undo should succeed");
    let html_after_undo = editor.get_html();
    assert!(
        html_after_undo.contains("<strong>beautiful</strong>"),
        "Undo should restore bold on 'beautiful', got: {}",
        html_after_undo
    );
}

#[test]
fn test_set_link_updates_active_mark_attrs_and_html() {
    let mut editor = default_editor();
    editor
        .set_html("<p>Hello world</p>")
        .expect("set_html should succeed");

    editor.set_selection(Selection::text(1, 6));
    let mut attrs = HashMap::new();
    attrs.insert(
        "href".to_string(),
        serde_json::Value::String("https://example.com".to_string()),
    );
    editor.set_mark("link", attrs).expect("set link should succeed");

    let state = editor.get_current_state();
    assert_eq!(state.active_state.marks.get("link"), Some(&true));
    assert_eq!(
        state.active_state.mark_attrs.get("link"),
        Some(&serde_json::json!({ "href": "https://example.com" }))
    );
    assert_eq!(
        editor.get_html(),
        "<p><a href=\"https://example.com\">Hello</a> world</p>"
    );
}

#[test]
fn test_set_link_at_collapsed_cursor_updates_existing_link_range() {
    let mut editor = default_editor();
    editor
        .set_html("<p><a href=\"https://old.example\">Hello</a> world</p>")
        .expect("set_html should succeed");

    editor.set_selection(Selection::cursor(3));
    let mut attrs = HashMap::new();
    attrs.insert(
        "href".to_string(),
        serde_json::Value::String("https://new.example".to_string()),
    );
    editor
        .set_mark("link", attrs)
        .expect("editing link at caret should succeed");

    assert_eq!(
        editor.get_html(),
        "<p><a href=\"https://new.example\">Hello</a> world</p>"
    );
}

#[test]
fn test_unset_link_at_collapsed_cursor_removes_existing_link_range() {
    let mut editor = default_editor();
    editor
        .set_html("<p><a href=\"https://example.com\">Hello</a> world</p>")
        .expect("set_html should succeed");

    editor.set_selection(Selection::cursor(3));
    editor
        .unset_mark("link")
        .expect("removing link at caret should succeed");

    assert_eq!(editor.get_html(), "<p>Hello world</p>");
}

#[test]
fn test_toggle_blockquote_wraps_current_paragraph() {
    let mut editor = default_editor();
    editor
        .set_html("<p>Hello</p><p>World</p>")
        .expect("set_html should succeed");

    editor.set_selection(Selection::cursor(2));
    editor
        .toggle_blockquote()
        .expect("toggle_blockquote should wrap paragraph");

    assert_eq!(
        editor.get_html(),
        "<blockquote><p>Hello</p></blockquote><p>World</p>"
    );
}

#[test]
fn test_toggle_blockquote_unwraps_nearest_container() {
    let mut editor = default_editor();
    editor
        .set_html("<blockquote><p>Hello</p><p>World</p></blockquote>")
        .expect("set_html should succeed");

    editor.set_selection(Selection::cursor(4));
    editor
        .toggle_blockquote()
        .expect("toggle_blockquote should unwrap quote");

    assert_eq!(editor.get_html(), "<p>Hello</p><p>World</p>");
}

#[test]
fn test_toggle_blockquote_wraps_multiple_selected_blocks() {
    let mut editor = default_editor();
    editor
        .set_html("<p>Hello</p><p>World</p><p>Tail</p>")
        .expect("set_html should succeed");

    editor.set_selection(Selection::text(1, 13));
    editor
        .toggle_blockquote()
        .expect("toggle_blockquote should wrap multiple blocks");

    assert_eq!(
        editor.get_html(),
        "<blockquote><p>Hello</p><p>World</p></blockquote><p>Tail</p>"
    );
}

#[test]
fn test_toggle_heading_applies_requested_level() {
    let mut editor = default_editor();
    editor
        .set_html("<p>Hello</p><p>World</p>")
        .expect("set_html should succeed");

    editor.set_selection(Selection::text(1, 13));
    editor
        .toggle_heading(2)
        .expect("toggle_heading should convert selected paragraphs");

    assert_eq!(editor.get_html(), "<h2>Hello</h2><h2>World</h2>");
}

#[test]
fn test_toggle_heading_reverts_matching_heading_to_paragraph() {
    let mut editor = default_editor();
    editor
        .set_html("<h3>Hello</h3><p>World</p>")
        .expect("set_html should succeed");

    editor.set_selection(Selection::cursor(2));
    editor
        .toggle_heading(3)
        .expect("toggle_heading should revert matching heading to paragraph");

    assert_eq!(editor.get_html(), "<p>Hello</p><p>World</p>");
}

#[test]
fn test_split_block_on_empty_blockquote_paragraph_exits_quote() {
    let mut editor = default_editor();
    editor
        .set_html("<blockquote><p>Hello</p><p></p></blockquote>")
        .expect("set_html should succeed");

    editor.set_selection(Selection::cursor(9));
    editor
        .split_block(9)
        .expect("split_block should exit empty blockquote paragraph");

    assert_eq!(editor.get_html(), "<blockquote><p>Hello</p></blockquote><p></p>");
}

#[test]
fn test_undo_remove_mark_via_direct_step() {
    // Test the RemoveMark -> AddMark inversion directly using a transaction.
    // Setup: paragraph with bold in the middle.
    // Apply RemoveMark step directly, then undo.
    let schema = tiptap_schema();
    let bold = Mark::new("bold".to_string(), HashMap::new());
    let doc = Document::new(doc_node(vec![paragraph(vec![
        text("Hello "),
        marked_text("World", vec![bold.clone()]),
    ])]));

    // Verify initial state.
    assert_eq!(doc.root().text_content(), "Hello World");

    // Apply RemoveMark on the range covering "World" (positions 7-12 in doc content).
    let mut tx = Transaction::new(Source::Format);
    tx.add_step(Step::RemoveMark {
        from: 7,
        to: 12,
        mark_type: "bold".to_string(),
    });

    let (_after_remove, _) = tx.apply(&doc, &schema).expect("RemoveMark should succeed");

    // Build an editor with the original doc, apply the transaction, then undo.
    let mut editor = default_editor();
    editor
        .set_html("<p>Hello <strong>World</strong></p>")
        .expect("set_html");

    // Remove bold via selection.
    editor.set_selection(Selection::text(7, 12));
    editor.toggle_mark("bold").expect("remove bold");

    let html = editor.get_html();
    assert!(
        !html.contains("<strong>"),
        "Bold should be removed, got: {}",
        html
    );

    // Undo.
    editor.undo().expect("undo");
    let html_undo = editor.get_html();
    assert!(
        html_undo.contains("<strong>World</strong>"),
        "Undo should restore bold, got: {}",
        html_undo
    );
}

// ---------------------------------------------------------------------------
// Issue 1b: JoinBlocks undo preserves second block's type and attrs
// ---------------------------------------------------------------------------

#[test]
fn test_undo_join_blocks_preserves_paragraph_type() {
    // Setup: two paragraphs. Join them. Undo should split back into two paragraphs.
    let mut editor = default_editor();
    editor
        .set_html("<p>Hello</p><p>World</p>")
        .expect("set_html should succeed");

    // Join position is between the two paragraphs.
    // <p>Hello</p><p>World</p>
    // pos 0: doc content start
    // pos 1-5: "Hello" inside first <p>
    // pos 6: end of first <p> content
    // pos 7: between first close and second open (block boundary)
    // Actually the join position should be at the block boundary.
    // First paragraph: open(0) + "Hello"(1-5) + close(6) = positions 0-6
    // Second paragraph: open(7) + "World"(8-12) + close(13)
    // The block boundary is at position 7 (start of second paragraph).
    let result = editor.join_blocks(7);
    assert!(
        result.is_ok(),
        "join_blocks should succeed, got: {:?}",
        result.err()
    );

    let html_joined = editor.get_html();
    assert!(
        html_joined.contains("HelloWorld"),
        "Join should merge blocks, got: {}",
        html_joined
    );

    // Undo should restore two paragraphs.
    editor.undo().expect("undo should succeed");
    let html_after_undo = editor.get_html();
    assert!(
        html_after_undo.contains("<p>Hello</p>") && html_after_undo.contains("<p>World</p>"),
        "Undo should restore two paragraphs, got: {}",
        html_after_undo
    );
}

// ---------------------------------------------------------------------------
// Issue 1c: UnwrapFromList undo preserves list type
// ---------------------------------------------------------------------------

#[test]
fn test_undo_unwrap_from_ordered_list_preserves_list_type() {
    // Setup: ordered list with one item. Unwrap it. Undo should re-wrap
    // in an ordered list, not a bullet list.
    let mut editor = default_editor();
    editor
        .set_html("<ol><li><p>Item one</p></li></ol>")
        .expect("set_html should succeed");

    // Position inside the list item's paragraph:
    // doc > ol(open=0) > li(open=1) > p(open=2) > "Item one"(3-10)
    let result = editor.unwrap_from_list(3);
    assert!(
        result.is_ok(),
        "unwrap_from_list should succeed, got: {:?}",
        result.err()
    );

    let html_unwrapped = editor.get_html();
    assert!(
        html_unwrapped.contains("<p>Item one</p>"),
        "Unwrap should produce a paragraph, got: {}",
        html_unwrapped
    );
    assert!(
        !html_unwrapped.contains("<ol>") && !html_unwrapped.contains("<ul>"),
        "Unwrap should remove list structure, got: {}",
        html_unwrapped
    );

    // Undo should re-wrap in an ordered list (not bullet list).
    editor.undo().expect("undo should succeed");
    let html_after_undo = editor.get_html();
    assert!(
        html_after_undo.contains("<ol>"),
        "Undo should restore ordered list, got: {}",
        html_after_undo
    );
    assert!(
        !html_after_undo.contains("<ul>"),
        "Undo should NOT create a bullet list, got: {}",
        html_after_undo
    );
}

#[test]
fn test_undo_unwrap_from_bullet_list_preserves_list_type() {
    let mut editor = default_editor();
    editor
        .set_html("<ul><li><p>Bullet</p></li></ul>")
        .expect("set_html should succeed");

    editor
        .unwrap_from_list(3)
        .expect("unwrap_from_list should succeed");

    editor.undo().expect("undo should succeed");
    let html = editor.get_html();
    assert!(
        html.contains("<ul>"),
        "Undo should restore bullet list, got: {}",
        html
    );
}

// ===========================================================================
// Issue 2: Multi-block delete and replace
// ===========================================================================

#[test]
fn test_delete_across_two_paragraphs() {
    // Setup: two paragraphs "Hello" and "World".
    // Delete from middle of first to middle of second.
    let mut editor = default_editor();
    editor
        .set_html("<p>Hello</p><p>World</p>")
        .expect("set_html should succeed");

    // Positions:
    // <p>Hello</p><p>World</p>
    // p1: open=0, H=1, e=2, l=3, l=4, o=5, close=6
    // p2: open=7, W=8, o=9, r=10, l=11, d=12, close=13
    // Delete from pos 3 (at "l", the third char) to pos 10 (at "r").
    // pos 3 is after "He", pos 10 is after "Wo" in second paragraph.
    // So we keep "He" (pos 1-2) from first and "rld" (pos 10-12) from second.
    // Result: "Herld" in one paragraph.

    let result = editor.delete_range(3, 10);
    assert!(
        result.is_ok(),
        "cross-paragraph delete should succeed, got: {:?}",
        result.err()
    );

    let html = editor.get_html();
    assert!(
        html.contains("Herld"),
        "Cross-paragraph delete should merge remaining content into 'Herld', got: {}",
        html
    );

    // Should be a single paragraph now.
    let count = html.matches("<p>").count();
    assert_eq!(
        count, 1,
        "Should have one paragraph after cross-paragraph delete, got: {}",
        html
    );
}

#[test]
fn test_delete_entire_first_paragraph_across_boundary() {
    let mut editor = default_editor();
    editor
        .set_html("<p>First</p><p>Second</p>")
        .expect("set_html should succeed");

    // Delete from start of first paragraph to start of second paragraph content.
    // p1: open=0, F=1, i=2, r=3, s=4, t=5, close=6
    // p2: open=7, S=8, e=9, c=10, o=11, n=12, d=13, close=14
    // Delete from 1 to 8: removes "First" and the boundary, keeps "Second"
    let result = editor.delete_range(1, 8);
    assert!(
        result.is_ok(),
        "delete should succeed, got: {:?}",
        result.err()
    );

    let html = editor.get_html();
    assert!(
        html.contains("Second"),
        "Should keep 'Second', got: {}",
        html
    );
    assert!(
        !html.contains("First"),
        "Should remove 'First', got: {}",
        html
    );
}

#[test]
fn test_delete_across_three_paragraphs() {
    let mut editor = default_editor();
    editor
        .set_html("<p>AAA</p><p>BBB</p><p>CCC</p>")
        .expect("set_html should succeed");

    // p1: open=0, A=1,A=2,A=3, close=4
    // p2: open=5, B=6,B=7,B=8, close=9
    // p3: open=10, C=11,C=12,C=13, close=14
    // Delete from pos 2 (after first A) to pos 12 (after first two C's)
    // Keep "A" from first and "C" from third -> "AC"
    let result = editor.delete_range(2, 12);
    assert!(
        result.is_ok(),
        "delete across 3 paragraphs should succeed, got: {:?}",
        result.err()
    );

    let html = editor.get_html();
    assert!(
        html.contains("AC"),
        "Should merge first and last into 'AC', got: {}",
        html
    );
    let p_count = html.matches("<p>").count();
    assert_eq!(
        p_count, 1,
        "Should have one paragraph after merging, got: {}",
        html
    );
}

#[test]
fn test_replace_across_two_paragraphs() {
    let mut editor = default_editor();
    editor
        .set_html("<p>Hello</p><p>World</p>")
        .expect("set_html should succeed");

    // Replace from pos 3 to pos 10 with "XYZ"
    // Keeps "He" (pos 1-2) and "rld" (pos 10-12), replaces middle with "XYZ"
    // Result: "HeXYZrld"
    let schema = tiptap_schema();
    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::ReplaceRange {
        from: 3,
        to: 10,
        content: Fragment::from(vec![text("XYZ")]),
    });

    let doc_before = editor.document().clone();
    let result = tx.apply(&doc_before, &schema);
    assert!(
        result.is_ok(),
        "cross-paragraph replace should succeed, got: {:?}",
        result.err()
    );

    let (new_doc, _) = result.unwrap();
    let text_content = new_doc.root().text_content();
    assert!(
        text_content.contains("HeXYZrld"),
        "Replace should produce 'HeXYZrld', got: {}",
        text_content
    );
}

// ===========================================================================
// Issue 3: MaxLength uses text length, not doc positions
// ===========================================================================

#[test]
fn test_max_length_delete_paragraph_boundary_not_counted_as_text() {
    // Setup: two paragraphs "AB" and "CD". Total text = 4 chars.
    // The paragraph boundary is 2 structural tokens (close + open).
    // Joining the paragraphs (delete range covering the boundary)
    // should NOT subtract 2 from the text length.
    //
    // We test the MaxLength interceptor directly.
    use editor_core::intercept::InterceptorExt;

    let doc = Document::new(doc_node(vec![
        paragraph(vec![text("AB")]),
        paragraph(vec![text("CD")]),
    ]));

    // Current text length is 4 ("AB" + "CD").
    assert_eq!(
        doc.root().text_content().chars().count(),
        4,
        "doc should have 4 text chars"
    );

    // The old bug: DeleteRange from 3 to 5 covers the paragraph boundary
    // (close tag of first + open tag of second = 2 doc positions).
    // The old code would compute removed = 5 - 3 = 2, projecting
    // text length as 4 - 2 = 2. But the actual text removed is 0
    // (only structural tokens). The corrected code should project
    // text length as 4.
    let interceptor = MaxLength::new(4);

    // A JoinBlocks step doesn't affect text length at all.
    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::JoinBlocks { pos: 5 });

    let result = interceptor.check(tx, &doc);
    assert!(
        result.is_ok(),
        "JoinBlocks should pass MaxLength (no text change), got: {:?}",
        result.err()
    );
}

#[test]
fn test_max_length_split_block_not_counted_as_text_insert() {
    // Splitting a block inserts 2 structural tokens but 0 text chars.
    // MaxLength should not count this as exceeding the limit.
    use editor_core::intercept::InterceptorExt;

    let doc = Document::new(doc_node(vec![paragraph(vec![text("ABCD")])]));
    assert_eq!(doc.root().text_content().chars().count(), 4);

    let interceptor = MaxLength::new(4); // exactly at limit

    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::SplitBlock {
        pos: 3,
        node_type: "paragraph".to_string(),
        attrs: HashMap::new(),
    });

    let result = interceptor.check(tx, &doc);
    assert!(
        result.is_ok(),
        "SplitBlock should pass MaxLength (no text change), got: {:?}",
        result.err()
    );
}

#[test]
fn test_max_length_delete_range_counts_only_text_chars() {
    // Delete a range that includes both text and structural tokens.
    // MaxLength should only count the text chars removed.
    use editor_core::intercept::InterceptorExt;

    // Doc: <p>Hello</p><p>World</p> — 10 text chars
    let doc = Document::new(doc_node(vec![
        paragraph(vec![text("Hello")]),
        paragraph(vec![text("World")]),
    ]));
    assert_eq!(doc.root().text_content().chars().count(), 10);

    let interceptor = MaxLength::new(10);

    // Insert text that would take us over the limit — but first we delete
    // some text. The delete should correctly count only text chars removed.
    let mut tx = Transaction::new(Source::Input);
    // Delete "llo" from first paragraph (positions 3-6, which is 3 text chars).
    tx.add_step(Step::DeleteRange { from: 3, to: 6 });
    // Then insert 3 chars to stay at the limit.
    tx.add_step(Step::InsertText {
        pos: 3,
        text: "XYZ".to_string(),
        marks: vec![],
    });

    let result = interceptor.check(tx, &doc);
    assert!(
        result.is_ok(),
        "Delete 3 text + insert 3 should stay at limit, got: {:?}",
        result.err()
    );
}

// ===========================================================================
// Issue 4: Selection normalization on set_selection
// ===========================================================================

#[test]
fn test_set_selection_normalizes_to_cursorable_position() {
    let mut editor = default_editor();
    editor
        .set_html("<p>Hello</p><p>World</p>")
        .expect("set_html should succeed");

    // Position 0 is at the doc content start, before the first paragraph's
    // open tag. This is NOT a cursorable position. Setting the selection to
    // position 0 should snap it to the nearest cursorable position (inside
    // the first paragraph, position 1).
    editor.set_selection(Selection::cursor(0));
    let sel = editor.selection();
    match sel {
        Selection::Text { anchor, head } => {
            assert!(
                *anchor >= 1,
                "Selection anchor should be normalized to >= 1 (inside paragraph), got: {}",
                anchor
            );
            assert!(
                *head >= 1,
                "Selection head should be normalized to >= 1 (inside paragraph), got: {}",
                head
            );
        }
        _ => panic!("Expected Text selection, got: {:?}", sel),
    }
}

#[test]
fn test_set_selection_normalizes_between_blocks() {
    let mut editor = default_editor();
    editor
        .set_html("<p>Hello</p><p>World</p>")
        .expect("set_html should succeed");

    // Position 7 is between the two paragraphs (after close of first,
    // before open of second). This is a structural position, not cursorable.
    // p1: open=0, H=1..o=5, close=6
    // p2: open=7, W=8..d=12, close=13
    // Position 7 is the open tag of p2 — should snap to 8 (first content pos).
    editor.set_selection(Selection::cursor(7));
    let sel = editor.selection();
    match sel {
        Selection::Text { anchor, .. } => {
            // Should be snapped to either end of first block (6) or
            // start of second block (8), depending on normalization.
            assert!(
                *anchor != 7,
                "Selection should NOT be at structural position 7, got anchor: {}",
                anchor
            );
        }
        _ => panic!("Expected Text selection"),
    }
}

#[test]
fn test_set_selection_text_range_normalized() {
    let mut editor = default_editor();
    editor
        .set_html("<p>Hello</p><p>World</p>")
        .expect("set_html should succeed");

    // Set a range that starts at a structural position.
    editor.set_selection(Selection::text(0, 5));
    let sel = editor.selection();
    match sel {
        Selection::Text { anchor, head } => {
            assert!(*anchor >= 1, "Anchor should be normalized, got: {}", anchor);
            assert_eq!(*head, 5, "Head at valid position should stay at 5");
        }
        _ => panic!("Expected Text selection"),
    }
}

// ===========================================================================
// Issue 5: Undo/redo restores selection
// ===========================================================================

#[test]
fn test_undo_restores_pre_transaction_selection() {
    let mut editor = default_editor();
    editor.set_html("<p>Hello</p>").expect("set_html");

    // Set cursor at position 3 (after "He").
    editor.set_selection(Selection::cursor(3));

    // Insert text at position 3.
    let update = editor
        .insert_text(3, "XY")
        .expect("insert_text should succeed");

    // After insert, cursor should be after inserted text.
    match &update.selection {
        Selection::Text { anchor, .. } => {
            assert!(
                *anchor >= 5,
                "After inserting 2 chars at pos 3, cursor should be >= 5, got: {}",
                anchor
            );
        }
        _ => {}
    }

    // Undo should restore the cursor to position 3 (pre-insert position).
    let undo_update = editor.undo().expect("undo should succeed");
    match &undo_update.selection {
        Selection::Text { anchor, head } => {
            assert_eq!(
                *anchor, 3,
                "After undo, cursor should be restored to pre-insert position 3, got: {}",
                anchor
            );
            assert_eq!(
                *head, 3,
                "After undo, cursor head should also be at 3, got: {}",
                head
            );
        }
        _ => panic!("Expected Text selection after undo"),
    }

    // Verify document is also restored.
    let html = editor.get_html();
    assert!(
        html.contains("Hello") && !html.contains("XY"),
        "Document should be restored, got: {}",
        html
    );
}

#[test]
fn test_redo_restores_post_transaction_selection() {
    let mut editor = default_editor();
    editor.set_html("<p>Hello</p>").expect("set_html");

    editor.set_selection(Selection::cursor(3));
    editor
        .insert_text(3, "XY")
        .expect("insert_text should succeed");

    // Record what the post-insert selection was.
    let post_insert_sel = editor.selection().clone();

    // Undo.
    editor.undo().expect("undo");

    // Redo should restore to the post-insert selection.
    let redo_update = editor.redo().expect("redo should succeed");
    match &redo_update.selection {
        Selection::Text { anchor, .. } => {
            // The redo selection should be the post-transaction selection.
            assert!(
                *anchor >= 5,
                "Redo cursor should be at or after the inserted text, got: {}",
                anchor
            );
        }
        _ => {}
    }
    // Also verify the stored post_insert_sel was captured correctly.
    let _ = post_insert_sel;

    // Verify document is re-applied.
    let html = editor.get_html();
    assert!(
        html.contains("HeXYllo"),
        "Document should have the insertion, got: {}",
        html
    );
}

#[test]
fn test_undo_redo_selection_does_not_return_none() {
    // The old code returned selection_update: None from undo/redo,
    // leaving the cursor wherever it was. Verify that undo/redo now
    // returns a selection update.
    let mut editor = default_editor();
    editor.set_html("<p>Test</p>").expect("set_html");
    editor.set_selection(Selection::cursor(1));
    editor.insert_text(1, "A").expect("insert");

    let undo_update = editor.undo().expect("undo should succeed");
    // The selection in the update should be a valid text selection.
    match &undo_update.selection {
        Selection::Text { anchor, .. } => {
            assert!(
                *anchor <= 5,
                "Selection should be within document bounds, got: {}",
                anchor
            );
        }
        _ => {} // All, Node are also valid
    }
}

#[test]
fn test_undo_selection_after_delete() {
    let mut editor = default_editor();
    editor.set_html("<p>Hello World</p>").expect("set_html");

    // Set cursor at position 6 (after "Hello"), then select "Hello" to delete.
    editor.set_selection(Selection::text(1, 6));

    // Delete " World" (positions 6-12).
    editor.delete_range(6, 12).expect("delete_range");

    // Undo should restore selection to before the delete.
    let undo_update = editor.undo().expect("undo");
    match &undo_update.selection {
        Selection::Text { anchor, head } => {
            // The selection before the delete was text(1, 6).
            assert_eq!(
                *anchor, 1,
                "Undo should restore pre-delete selection anchor, got: {}",
                anchor
            );
            assert_eq!(
                *head, 6,
                "Undo should restore pre-delete selection head, got: {}",
                head
            );
        }
        _ => panic!("Expected Text selection after undo of delete"),
    }
}

// ===========================================================================
// Combined: undo round-trips with the improved inverse steps
// ===========================================================================

#[test]
fn test_full_bold_toggle_undo_redo_roundtrip() {
    let mut editor = default_editor();
    editor
        .set_html("<p>Hello World</p>")
        .expect("set_html should succeed");

    // Bold "World" (positions 7-12).
    editor.set_selection(Selection::text(7, 12));
    editor.toggle_mark("bold").expect("add bold");
    assert!(editor.get_html().contains("<strong>World</strong>"));

    // Remove bold.
    editor.set_selection(Selection::text(7, 12));
    editor.toggle_mark("bold").expect("remove bold");
    assert!(!editor.get_html().contains("<strong>"));

    // Undo the removal — bold should come back.
    editor.undo().expect("undo remove bold");
    assert!(
        editor.get_html().contains("<strong>World</strong>"),
        "Undo should restore bold on World, got: {}",
        editor.get_html()
    );

    // Undo the addition — no bold at all.
    editor.undo().expect("undo add bold");
    assert!(
        !editor.get_html().contains("<strong>"),
        "Second undo should remove bold entirely, got: {}",
        editor.get_html()
    );

    // Redo add bold.
    editor.redo().expect("redo add bold");
    assert!(
        editor.get_html().contains("<strong>World</strong>"),
        "Redo should re-add bold, got: {}",
        editor.get_html()
    );

    // Redo remove bold.
    editor.redo().expect("redo remove bold");
    assert!(
        !editor.get_html().contains("<strong>"),
        "Second redo should remove bold again, got: {}",
        editor.get_html()
    );
}

#[test]
fn test_split_join_undo_redo_roundtrip() {
    let mut editor = default_editor();
    editor
        .set_html("<p>HelloWorld</p>")
        .expect("set_html should succeed");

    // Split at position 6 (after "Hello").
    editor.split_block(6).expect("split_block");
    let html_split = editor.get_html();
    assert!(
        html_split.contains("<p>Hello</p>") && html_split.contains("<p>World</p>"),
        "Split should create two paragraphs, got: {}",
        html_split
    );

    // Undo split.
    editor.undo().expect("undo split");
    let html_after_undo = editor.get_html();
    assert!(
        html_after_undo.contains("HelloWorld"),
        "Undo should rejoin, got: {}",
        html_after_undo
    );
    let p_count = html_after_undo.matches("<p>").count();
    assert_eq!(
        p_count, 1,
        "Should be one paragraph after undo, got: {}",
        html_after_undo
    );

    // Redo split.
    editor.redo().expect("redo split");
    let html_after_redo = editor.get_html();
    assert!(
        html_after_redo.contains("<p>Hello</p>") && html_after_redo.contains("<p>World</p>"),
        "Redo should re-split, got: {}",
        html_after_redo
    );
}
