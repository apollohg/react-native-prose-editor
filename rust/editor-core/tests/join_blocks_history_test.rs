use editor_core::editor::Editor;
use editor_core::intercept::InterceptorPipeline;
use editor_core::selection::Selection;
use editor_core::tiptap_schema;

fn editor_with(document: serde_json::Value, selection: Selection) -> Editor {
    let mut editor = Editor::new(tiptap_schema(), InterceptorPipeline::new(), false);
    editor.set_json(&document).unwrap();
    editor.set_selection(selection);
    editor
}

#[test]
fn list_item_join_undo_restores_exact_document_and_selection() {
    let document = serde_json::json!({
        "type": "doc",
        "content": [{
            "type": "bulletList",
            "content": [
                {"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"one"}]}]},
                {"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"two"}]}]}
            ]
        }]
    });
    let selection = Selection::cursor(8);
    let mut editor = editor_with(document.clone(), selection.clone());
    let normalized_selection = editor.selection().clone();

    editor.join_blocks(8).unwrap();
    assert!(editor.can_undo());
    editor.undo().expect("list item join must be undoable");

    assert_eq!(editor.get_json(), document);
    assert_eq!(editor.selection(), &normalized_selection);
    assert!(!editor.can_undo());
}

#[test]
fn paragraph_join_keeps_its_existing_split_inverse() {
    let document = serde_json::json!({
        "type": "doc",
        "content": [
            {"type":"paragraph","content":[{"type":"text","text":"a"}]},
            {"type":"paragraph","content":[{"type":"text","text":"b"}]}
        ]
    });
    let selection = Selection::cursor(3);
    let mut editor = editor_with(document.clone(), selection.clone());
    let normalized_selection = editor.selection().clone();

    editor.join_blocks(3).unwrap();
    assert!(editor.can_undo());
    editor.undo().expect("paragraph join must remain undoable");

    assert_eq!(editor.get_json(), document);
    assert_eq!(editor.selection(), &normalized_selection);
    assert!(!editor.can_undo());
}

#[test]
fn empty_blockquote_exit_undo_restores_exact_sibling_slice_and_selection() {
    let document = serde_json::json!({
        "type": "doc",
        "content": [
            {"type":"paragraph","content":[{"type":"text","text":"before"}]},
            {"type":"blockquote","content":[{"type":"paragraph"}]},
            {"type":"paragraph","content":[{"type":"text","text":"after"}]}
        ]
    });
    // The first paragraph has node-size 8; quote-open + paragraph-open puts
    // the cursor in the empty quote paragraph at document position 10.
    let mut editor = editor_with(document.clone(), Selection::cursor(10));
    let normalized_selection = editor.selection().clone();

    editor.split_block(10).unwrap();
    assert!(editor.can_undo());
    editor
        .undo()
        .expect("empty blockquote exit must be undoable");

    assert_eq!(editor.get_json(), document);
    assert_eq!(editor.selection(), &normalized_selection);
    assert!(!editor.can_undo());
}

#[test]
fn inline_replace_range_undo_keeps_the_existing_text_inverse() {
    let document = serde_json::json!({
        "type": "doc",
        "content": [{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]
    });
    let mut editor = editor_with(document.clone(), Selection::cursor(1));
    let normalized_selection = editor.selection().clone();

    editor
        .insert_content_json(
            &serde_json::json!({"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"X"}]}]}),
        )
        .unwrap();
    assert!(editor.can_undo());
    editor
        .undo()
        .expect("inline content replacement must remain undoable");

    assert_eq!(editor.get_json(), document);
    assert_eq!(editor.selection(), &normalized_selection);
    assert!(!editor.can_undo());
}
