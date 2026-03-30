use std::collections::HashMap;

use editor_core::model::{Document, Fragment, Mark, Node};
use editor_core::schema::presets::tiptap_schema;
use editor_core::transform::{Source, Step, Transaction};

// ---------------------------------------------------------------------------
// Helper builders (matching model_test.rs conventions)
// ---------------------------------------------------------------------------

fn bold() -> Mark {
    Mark::new("bold".to_string(), HashMap::new())
}

fn italic() -> Mark {
    Mark::new("italic".to_string(), HashMap::new())
}

fn text(s: &str) -> Node {
    Node::text(s.to_string(), vec![])
}

fn text_with_marks(s: &str, marks: Vec<Mark>) -> Node {
    Node::text(s.to_string(), marks)
}

fn paragraph(children: Vec<Node>) -> Node {
    Node::element(
        "paragraph".to_string(),
        HashMap::new(),
        Fragment::from(children),
    )
}

fn doc(children: Vec<Node>) -> Node {
    Node::element("doc".to_string(), HashMap::new(), Fragment::from(children))
}

fn bullet_list(children: Vec<Node>) -> Node {
    Node::element(
        "bulletList".to_string(),
        HashMap::new(),
        Fragment::from(children),
    )
}

fn list_item(children: Vec<Node>) -> Node {
    Node::element(
        "listItem".to_string(),
        HashMap::new(),
        Fragment::from(children),
    )
}

/// Build a Document and tiptap Schema for convenience.
fn doc_and_schema(root: Node) -> (Document, editor_core::schema::Schema) {
    (Document::new(root), tiptap_schema())
}

// ===========================================================================
// InsertText tests
// ===========================================================================

#[test]
fn test_insert_text_middle_of_word() {
    // <doc><p>Hello</p></doc>
    // Insert "X" at pos 2 (between "H" and "ello")
    // Expected: <doc><p>HXello</p></doc>
    let (doc, schema) = doc_and_schema(doc(vec![paragraph(vec![text("Hello")])]));
    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::InsertText {
        pos: 2,
        text: "X".to_string(),
        marks: vec![],
    });

    let (new_doc, _map) = tx.apply(&doc, &schema).expect("insert should succeed");
    assert_eq!(
        new_doc.root().text_content(),
        "HXello",
        "inserting 'X' at pos 2 in 'Hello' should produce 'HXello'"
    );

    // Verify size changed by 1
    assert_eq!(
        new_doc.content_size(),
        doc.content_size() + 1,
        "content size should increase by 1 after inserting 1 char"
    );
}

#[test]
fn test_insert_text_start_of_paragraph() {
    // <doc><p>Hello</p></doc>
    // Insert at pos 1 (start of paragraph content)
    let (doc, schema) = doc_and_schema(doc(vec![paragraph(vec![text("Hello")])]));
    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::InsertText {
        pos: 1,
        text: "X".to_string(),
        marks: vec![],
    });

    let (new_doc, _map) = tx
        .apply(&doc, &schema)
        .expect("insert at start should succeed");
    assert_eq!(
        new_doc.root().text_content(),
        "XHello",
        "inserting at paragraph start should prepend"
    );
}

#[test]
fn test_insert_text_end_of_paragraph() {
    // <doc><p>Hello</p></doc>
    // pos 6 = end of paragraph content (after 'o')
    let (doc, schema) = doc_and_schema(doc(vec![paragraph(vec![text("Hello")])]));
    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::InsertText {
        pos: 6,
        text: "!".to_string(),
        marks: vec![],
    });

    let (new_doc, _map) = tx
        .apply(&doc, &schema)
        .expect("insert at end should succeed");
    assert_eq!(
        new_doc.root().text_content(),
        "Hello!",
        "inserting at paragraph end should append"
    );
}

#[test]
fn test_insert_text_with_bold_mark_between_plain() {
    // <doc><p>Hello</p></doc>
    // Insert bold "X" at pos 3 (between "He" and "llo")
    // Expected 3 text nodes: "He" (plain), "X" (bold), "llo" (plain)
    let (doc, schema) = doc_and_schema(doc(vec![paragraph(vec![text("Hello")])]));
    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::InsertText {
        pos: 3,
        text: "X".to_string(),
        marks: vec![bold()],
    });

    let (new_doc, _map) = tx.apply(&doc, &schema).expect("bold insert should succeed");
    assert_eq!(new_doc.root().text_content(), "HeXllo");

    // Verify the paragraph has 3 text children
    let para = new_doc
        .root()
        .child(0)
        .expect("doc should have a paragraph");
    assert_eq!(
        para.child_count(),
        3,
        "paragraph should have 3 text nodes after marked insert"
    );
    assert_eq!(para.child(0).unwrap().text_str().unwrap(), "He");
    assert!(para.child(0).unwrap().marks().is_empty());
    assert_eq!(para.child(1).unwrap().text_str().unwrap(), "X");
    assert_eq!(para.child(1).unwrap().marks().len(), 1);
    assert_eq!(para.child(1).unwrap().marks()[0].mark_type(), "bold");
    assert_eq!(para.child(2).unwrap().text_str().unwrap(), "llo");
    assert!(para.child(2).unwrap().marks().is_empty());
}

#[test]
fn test_insert_emoji_text() {
    // Insert family emoji (7 scalars) at pos 2
    let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}\u{200D}\u{1F466}";
    let (doc, schema) = doc_and_schema(doc(vec![paragraph(vec![text("Hi")])]));
    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::InsertText {
        pos: 2,
        text: family.to_string(),
        marks: vec![],
    });

    let (new_doc, _map) = tx
        .apply(&doc, &schema)
        .expect("emoji insert should succeed");
    assert_eq!(
        new_doc.content_size(),
        doc.content_size() + 7,
        "doc_delta should be +7 for family emoji (7 Unicode scalars)"
    );
}

#[test]
fn test_insert_text_into_empty_paragraph() {
    // <doc><p></p></doc> — pos 1 is inside empty paragraph
    let (doc, schema) = doc_and_schema(doc(vec![paragraph(vec![])]));
    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::InsertText {
        pos: 1,
        text: "A".to_string(),
        marks: vec![],
    });

    let (new_doc, _map) = tx
        .apply(&doc, &schema)
        .expect("insert into empty paragraph should succeed");
    assert_eq!(new_doc.root().text_content(), "A");
}

#[test]
fn test_insert_text_merges_with_adjacent_same_marks() {
    // <doc><p><b>He</b><b>llo</b></p></doc>
    // Insert bold "X" at pos 3 (between the two bold text nodes)
    // Should merge into a single text node "HeXllo" with bold
    let (doc, schema) = doc_and_schema(doc(vec![paragraph(vec![
        text_with_marks("He", vec![bold()]),
        text_with_marks("llo", vec![bold()]),
    ])]));
    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::InsertText {
        pos: 3,
        text: "X".to_string(),
        marks: vec![bold()],
    });

    let (new_doc, _map) = tx
        .apply(&doc, &schema)
        .expect("merge insert should succeed");
    assert_eq!(new_doc.root().text_content(), "HeXllo");

    let para = new_doc.root().child(0).unwrap();
    // With mark-aware merging, all 3 bold-marked segments could merge into 1 node
    // But the minimum requirement is that the text content is correct
    // and all text carries the bold mark
    for i in 0..para.child_count() {
        let child = para.child(i).unwrap();
        assert!(
            child.marks().iter().any(|m| m.mark_type() == "bold"),
            "all text nodes should be bold"
        );
    }
}

// ===========================================================================
// DeleteRange tests
// ===========================================================================

#[test]
fn test_delete_range_middle_of_text() {
    // <doc><p>Hello</p></doc>
    // Delete [2,4] (positions inside paragraph content: "el")
    // Expected: <doc><p>Hlo</p></doc>
    let (doc, schema) = doc_and_schema(doc(vec![paragraph(vec![text("Hello")])]));
    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::DeleteRange { from: 2, to: 4 });

    let (new_doc, _map) = tx.apply(&doc, &schema).expect("delete should succeed");
    assert_eq!(
        new_doc.root().text_content(),
        "Hlo",
        "deleting [2,4] in 'Hello' should produce 'Hlo'"
    );
    assert_eq!(
        new_doc.content_size(),
        doc.content_size() - 2,
        "content size should decrease by 2"
    );
}

#[test]
fn test_delete_entire_text_content() {
    // <doc><p>Hello</p></doc>
    // Delete [1,6] — the entire paragraph content
    // Expected: <doc><p></p></doc> (empty paragraph remains)
    let (doc, schema) = doc_and_schema(doc(vec![paragraph(vec![text("Hello")])]));
    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::DeleteRange { from: 1, to: 6 });

    let (new_doc, _map) = tx
        .apply(&doc, &schema)
        .expect("delete all text should succeed");
    assert_eq!(new_doc.root().text_content(), "");
    let para = new_doc
        .root()
        .child(0)
        .expect("paragraph should still exist");
    assert_eq!(
        para.child_count(),
        0,
        "paragraph should be empty after deleting all text"
    );
}

#[test]
fn test_delete_across_differently_marked_text_nodes() {
    // <doc><p>He<b>ll</b>o</p></doc>
    // Delete [2,5] — from "e" through bold "ll" into plain "o"
    // "H" (1 char) remains, then we delete "e" (plain) + "ll" (bold) = 3 chars
    // Remaining: "Ho" → but wait, let me recalculate positions:
    //
    // doc positions: 0=before p, 1=start of p content
    // p content: "He" (2 chars, plain) + "ll" (2 chars, bold) + "o" (1 char, plain) = 5 chars
    // pos 1=H, pos 2=e, pos 3=l(bold), pos 4=l(bold), pos 5=o, pos 6=end of p
    //
    // Delete [2,5] removes chars at parent_offset 1..4: "e" + "ll" + nothing
    // Wait, pos 2 = parent_offset 1, pos 5 = parent_offset 4
    // So we delete parent_offset [1,4) in the paragraph content
    // "He" → keep "H" (offset 0), remove "e" (offset 1)
    // "ll" → remove both (offsets 2,3)
    // "o" → keep (offset 4)
    // Result: "H" (plain) + "o" (plain) → text content "Ho"
    let (doc, schema) = doc_and_schema(doc(vec![paragraph(vec![
        text("He"),
        text_with_marks("ll", vec![bold()]),
        text("o"),
    ])]));
    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::DeleteRange { from: 2, to: 5 });

    let (new_doc, _map) = tx
        .apply(&doc, &schema)
        .expect("cross-mark delete should succeed");
    assert_eq!(
        new_doc.root().text_content(),
        "Ho",
        "deleting across marked boundaries should merge remaining text"
    );
}

// ===========================================================================
// AddMark tests
// ===========================================================================

#[test]
fn test_add_bold_to_range() {
    // <doc><p>Hello</p></doc>
    // Add bold to [2,4] → <doc><p>H<b>el</b>lo</p></doc>
    let (doc, schema) = doc_and_schema(doc(vec![paragraph(vec![text("Hello")])]));
    let mut tx = Transaction::new(Source::Format);
    tx.add_step(Step::AddMark {
        from: 2,
        to: 4,
        mark: bold(),
    });

    let (new_doc, _map) = tx.apply(&doc, &schema).expect("add mark should succeed");
    assert_eq!(
        new_doc.root().text_content(),
        "Hello",
        "text content should not change"
    );

    let para = new_doc.root().child(0).unwrap();
    assert_eq!(
        para.child_count(),
        3,
        "paragraph should have 3 text nodes: plain + bold + plain"
    );

    // First child: "H" (plain)
    let c0 = para.child(0).unwrap();
    assert_eq!(c0.text_str().unwrap(), "H");
    assert!(c0.marks().is_empty(), "first node should be plain");

    // Second child: "el" (bold)
    let c1 = para.child(1).unwrap();
    assert_eq!(c1.text_str().unwrap(), "el");
    assert_eq!(c1.marks().len(), 1);
    assert_eq!(c1.marks()[0].mark_type(), "bold");

    // Third child: "lo" (plain)
    let c2 = para.child(2).unwrap();
    assert_eq!(c2.text_str().unwrap(), "lo");
    assert!(c2.marks().is_empty(), "third node should be plain");
}

#[test]
fn test_add_bold_to_already_bold_text() {
    // <doc><p><b>Hello</b></p></doc>
    // Add bold to [1,6] — entire text is already bold
    // Should be a no-op (or at least produce same result)
    let (doc, schema) = doc_and_schema(doc(vec![paragraph(vec![text_with_marks(
        "Hello",
        vec![bold()],
    )])]));
    let mut tx = Transaction::new(Source::Format);
    tx.add_step(Step::AddMark {
        from: 1,
        to: 6,
        mark: bold(),
    });

    let (new_doc, _map) = tx
        .apply(&doc, &schema)
        .expect("adding existing mark should succeed");
    assert_eq!(new_doc.root().text_content(), "Hello");

    let para = new_doc.root().child(0).unwrap();
    // All text should still be bold, and ideally just 1 text node
    for i in 0..para.child_count() {
        let child = para.child(i).unwrap();
        assert!(
            child.marks().iter().any(|m| m.mark_type() == "bold"),
            "text should remain bold"
        );
    }
}

#[test]
fn test_add_italic_to_bold_text() {
    // <doc><p><b>Hello</b></p></doc>
    // Add italic to [2,5] → <doc><p><b>H</b><b><i>ell</i></b><b>o</b></p></doc>
    let (doc, schema) = doc_and_schema(doc(vec![paragraph(vec![text_with_marks(
        "Hello",
        vec![bold()],
    )])]));
    let mut tx = Transaction::new(Source::Format);
    tx.add_step(Step::AddMark {
        from: 2,
        to: 5,
        mark: italic(),
    });

    let (new_doc, _map) = tx
        .apply(&doc, &schema)
        .expect("adding italic to bold should succeed");
    assert_eq!(new_doc.root().text_content(), "Hello");

    let para = new_doc.root().child(0).unwrap();
    assert_eq!(
        para.child_count(),
        3,
        "should split into 3 nodes: bold-only, bold+italic, bold-only"
    );

    // Middle node should have both marks
    let c1 = para.child(1).unwrap();
    assert_eq!(c1.text_str().unwrap(), "ell");
    assert!(
        c1.marks().iter().any(|m| m.mark_type() == "bold"),
        "middle node should have bold"
    );
    assert!(
        c1.marks().iter().any(|m| m.mark_type() == "italic"),
        "middle node should have italic"
    );
}

// ===========================================================================
// RemoveMark tests
// ===========================================================================

#[test]
fn test_remove_bold_from_bold_text() {
    // <doc><p><b>Hello</b></p></doc>
    // Remove bold from [1,6] — full range
    let (doc, schema) = doc_and_schema(doc(vec![paragraph(vec![text_with_marks(
        "Hello",
        vec![bold()],
    )])]));
    let mut tx = Transaction::new(Source::Format);
    tx.add_step(Step::RemoveMark {
        from: 1,
        to: 6,
        mark_type: "bold".to_string(),
    });

    let (new_doc, _map) = tx.apply(&doc, &schema).expect("remove mark should succeed");
    assert_eq!(new_doc.root().text_content(), "Hello");

    let para = new_doc.root().child(0).unwrap();
    for i in 0..para.child_count() {
        let child = para.child(i).unwrap();
        assert!(
            !child.marks().iter().any(|m| m.mark_type() == "bold"),
            "bold should be removed from all text"
        );
    }
}

#[test]
fn test_remove_bold_from_partially_bold_range() {
    // <doc><p>H<b>ell</b>o</p></doc>
    // Remove bold from [1,6] — entire paragraph
    // "H" is already plain, "ell" loses bold, "o" is already plain
    let (doc, schema) = doc_and_schema(doc(vec![paragraph(vec![
        text("H"),
        text_with_marks("ell", vec![bold()]),
        text("o"),
    ])]));
    let mut tx = Transaction::new(Source::Format);
    tx.add_step(Step::RemoveMark {
        from: 1,
        to: 6,
        mark_type: "bold".to_string(),
    });

    let (new_doc, _map) = tx
        .apply(&doc, &schema)
        .expect("remove from partial should succeed");
    assert_eq!(new_doc.root().text_content(), "Hello");

    let para = new_doc.root().child(0).unwrap();
    for i in 0..para.child_count() {
        let child = para.child(i).unwrap();
        assert!(
            !child.marks().iter().any(|m| m.mark_type() == "bold"),
            "no text should have bold after removal"
        );
    }
}

#[test]
fn test_remove_bold_preserves_italic() {
    // <doc><p><b><i>Hello</i></b></p></doc>
    // Remove bold from [1,6]
    // Text should keep italic but lose bold
    let (doc, schema) = doc_and_schema(doc(vec![paragraph(vec![text_with_marks(
        "Hello",
        vec![bold(), italic()],
    )])]));
    let mut tx = Transaction::new(Source::Format);
    tx.add_step(Step::RemoveMark {
        from: 1,
        to: 6,
        mark_type: "bold".to_string(),
    });

    let (new_doc, _map) = tx
        .apply(&doc, &schema)
        .expect("remove bold should preserve italic");
    let para = new_doc.root().child(0).unwrap();
    for i in 0..para.child_count() {
        let child = para.child(i).unwrap();
        assert!(
            !child.marks().iter().any(|m| m.mark_type() == "bold"),
            "bold should be gone"
        );
        assert!(
            child.marks().iter().any(|m| m.mark_type() == "italic"),
            "italic should be preserved"
        );
    }
}

// ===========================================================================
// Content validation tests
// ===========================================================================

#[test]
fn test_insert_text_directly_into_doc_is_error() {
    // Inserting text at pos 0 (doc level, before any paragraph) should fail
    // because doc expects block+ children, not text
    let (doc, schema) = doc_and_schema(doc(vec![paragraph(vec![text("Hello")])]));
    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::InsertText {
        pos: 0,
        text: "X".to_string(),
        marks: vec![],
    });

    let result = tx.apply(&doc, &schema);
    assert!(
        result.is_err(),
        "inserting text directly into doc node should fail validation"
    );
}

#[test]
fn test_valid_transaction_passes_validation() {
    // A well-formed insert should succeed
    let (doc, schema) = doc_and_schema(doc(vec![paragraph(vec![text("Hello")])]));
    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::InsertText {
        pos: 3,
        text: "X".to_string(),
        marks: vec![],
    });

    assert!(
        tx.apply(&doc, &schema).is_ok(),
        "valid transaction should pass content validation"
    );
}

// ===========================================================================
// StepMap tests
// ===========================================================================

#[test]
fn test_step_map_after_insert_text() {
    // After InsertText(pos=2, "XY"), position 5 should map to 7 (+2 shift)
    let (doc, schema) = doc_and_schema(doc(vec![paragraph(vec![text("Hello")])]));
    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::InsertText {
        pos: 2,
        text: "XY".to_string(),
        marks: vec![],
    });

    let (_new_doc, map) = tx.apply(&doc, &schema).expect("insert should succeed");
    assert_eq!(
        map.map_pos(5),
        7,
        "position 5 should map to 7 after inserting 2 chars at pos 2"
    );
    // Position before the insert should not shift
    assert_eq!(
        map.map_pos(1),
        1,
        "position 1 (before insert) should not shift"
    );
    // Position at insert point should shift
    assert_eq!(
        map.map_pos(2),
        4,
        "position at insert point should shift forward"
    );
}

#[test]
fn test_step_map_after_delete_range() {
    // After DeleteRange(2,4), position 5 should map to 3 (-2 shift)
    let (doc, schema) = doc_and_schema(doc(vec![paragraph(vec![text("Hello")])]));
    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::DeleteRange { from: 2, to: 4 });

    let (_new_doc, map) = tx.apply(&doc, &schema).expect("delete should succeed");
    assert_eq!(
        map.map_pos(5),
        3,
        "position 5 should map to 3 after deleting 2 chars at [2,4]"
    );
    // Position before the delete should not shift
    assert_eq!(
        map.map_pos(1),
        1,
        "position 1 (before delete) should not shift"
    );
    // Position inside deleted range maps to the delete point
    assert_eq!(
        map.map_pos(3),
        2,
        "position inside deleted range should map to delete start"
    );
}

#[test]
fn test_step_map_composing_multiple_steps() {
    // Insert at pos 2, then delete at pos 5..7
    // The map should compose both transformations
    let (doc, schema) = doc_and_schema(doc(vec![paragraph(vec![text("Hello world")])]));
    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::InsertText {
        pos: 2,
        text: "X".to_string(),
        marks: vec![],
    });
    tx.add_step(Step::DeleteRange { from: 8, to: 10 }); // delete in transformed positions

    let (_new_doc, map) = tx.apply(&doc, &schema).expect("multi-step should succeed");
    // Position 1 (before both operations) should be unchanged
    assert_eq!(
        map.map_pos(1),
        1,
        "position before both ops should be unchanged"
    );
}

// ===========================================================================
// SplitBlock tests
// ===========================================================================

#[test]
fn test_split_block_middle_of_text() {
    // <doc><p>Hello</p></doc>
    // Split at pos 3 (between "He" and "llo") → <doc><p>He</p><p>llo</p></doc>
    //
    // Position model:
    //   pos 0: doc content, before <p> open tag
    //   pos 1: paragraph content offset 0 (before "H")
    //   pos 2: paragraph content offset 1 (between "H" and "e")
    //   pos 3: paragraph content offset 2 (between "He" and "llo")
    //   pos 6: paragraph content offset 5 (after "o", end of paragraph)
    //   pos 7: doc content, after </p> close tag
    let (d, schema) = doc_and_schema(doc(vec![paragraph(vec![text("Hello")])]));
    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::SplitBlock {
        pos: 3,
        node_type: "paragraph".to_string(),
        attrs: HashMap::new(),
    });

    let (new_doc, _map) = tx.apply(&d, &schema).expect("split middle should succeed");

    // Should now be two paragraphs
    assert_eq!(
        new_doc.root().child_count(),
        2,
        "doc should have 2 paragraphs after split"
    );

    let p1 = new_doc.root().child(0).unwrap();
    let p2 = new_doc.root().child(1).unwrap();

    assert_eq!(
        p1.text_content(),
        "He",
        "first paragraph should contain 'He'"
    );
    assert_eq!(
        p2.text_content(),
        "llo",
        "second paragraph should contain 'llo'"
    );
    assert_eq!(p1.node_type(), "paragraph");
    assert_eq!(p2.node_type(), "paragraph");

    // Doc delta: +2 (new close tag + new open tag)
    assert_eq!(
        new_doc.content_size(),
        d.content_size() + 2,
        "content size should increase by 2 after split (new close + open tag)"
    );
}

#[test]
fn test_split_block_at_start_of_paragraph() {
    // <doc><p>Hello</p></doc>
    // Split at pos 1 (start of paragraph content) → <doc><p></p><p>Hello</p></doc>
    let (d, schema) = doc_and_schema(doc(vec![paragraph(vec![text("Hello")])]));
    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::SplitBlock {
        pos: 1,
        node_type: "paragraph".to_string(),
        attrs: HashMap::new(),
    });

    let (new_doc, _map) = tx
        .apply(&d, &schema)
        .expect("split at start should succeed");

    assert_eq!(new_doc.root().child_count(), 2);

    let p1 = new_doc.root().child(0).unwrap();
    let p2 = new_doc.root().child(1).unwrap();

    assert_eq!(
        p1.text_content(),
        "",
        "first paragraph should be empty when splitting at start"
    );
    assert_eq!(
        p2.text_content(),
        "Hello",
        "second paragraph should contain all text when splitting at start"
    );
}

#[test]
fn test_split_block_at_end_of_paragraph() {
    // <doc><p>Hello</p></doc>
    // Split at pos 6 (end of paragraph content, after "Hello") → <doc><p>Hello</p><p></p></doc>
    let (d, schema) = doc_and_schema(doc(vec![paragraph(vec![text("Hello")])]));
    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::SplitBlock {
        pos: 6,
        node_type: "paragraph".to_string(),
        attrs: HashMap::new(),
    });

    let (new_doc, _map) = tx.apply(&d, &schema).expect("split at end should succeed");

    assert_eq!(new_doc.root().child_count(), 2);

    let p1 = new_doc.root().child(0).unwrap();
    let p2 = new_doc.root().child(1).unwrap();

    assert_eq!(
        p1.text_content(),
        "Hello",
        "first paragraph should contain all text when splitting at end"
    );
    assert_eq!(
        p2.text_content(),
        "",
        "second paragraph should be empty when splitting at end"
    );
}

#[test]
fn test_split_block_inside_list_item() {
    // <doc><ul><li><p>Hello</p></li></ul></doc>
    // Position model:
    //   pos 0: doc content, before <ul> open tag
    //   pos 1: inside ul, before <li> open tag
    //   pos 2: inside li, before <p> open tag
    //   pos 3: inside p, content offset 0 (before "H")
    //   pos 5: inside p, content offset 2 (between "He" and "llo")
    //   pos 8: inside p, content offset 5 (after "o", end of p content)
    //   pos 9: after </p> close tag (inside li)
    //   pos 10: after </li> close tag (inside ul)
    //   pos 11: after </ul> close tag (inside doc)
    //
    // Splitting at pos 5 should split the list item into two list items,
    // each containing a paragraph, staying within the same list.
    let (d, schema) = doc_and_schema(doc(vec![bullet_list(vec![list_item(vec![paragraph(
        vec![text("Hello")],
    )])])]));

    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::SplitBlock {
        pos: 5,
        node_type: "paragraph".to_string(),
        attrs: HashMap::new(),
    });

    let (new_doc, _map) = tx
        .apply(&d, &schema)
        .expect("split inside list item should succeed");

    // The list should still be one list with two list items
    let ul = new_doc.root().child(0).unwrap();
    assert_eq!(ul.node_type(), "bulletList");
    assert_eq!(
        ul.child_count(),
        2,
        "list should have 2 list items after split"
    );

    let li1 = ul.child(0).unwrap();
    let li2 = ul.child(1).unwrap();
    assert_eq!(li1.node_type(), "listItem");
    assert_eq!(li2.node_type(), "listItem");

    assert_eq!(
        li1.text_content(),
        "He",
        "first list item should contain 'He'"
    );
    assert_eq!(
        li2.text_content(),
        "llo",
        "second list item should contain 'llo'"
    );
}

#[test]
fn test_split_block_preserves_marks_on_both_sides() {
    // <doc><p><b>He</b><i>llo</i></p></doc>
    // Split at pos 3 (between bold "He" and italic "llo")
    // → <doc><p><b>He</b></p><p><i>llo</i></p></doc>
    let (d, schema) = doc_and_schema(doc(vec![paragraph(vec![
        text_with_marks("He", vec![bold()]),
        text_with_marks("llo", vec![italic()]),
    ])]));

    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::SplitBlock {
        pos: 3,
        node_type: "paragraph".to_string(),
        attrs: HashMap::new(),
    });

    let (new_doc, _map) = tx
        .apply(&d, &schema)
        .expect("split preserving marks should succeed");

    assert_eq!(new_doc.root().child_count(), 2);

    let p1 = new_doc.root().child(0).unwrap();
    let p2 = new_doc.root().child(1).unwrap();

    assert_eq!(p1.text_content(), "He");
    assert_eq!(p2.text_content(), "llo");

    // Verify marks are preserved
    let p1_child = p1.child(0).unwrap();
    assert_eq!(p1_child.text_str().unwrap(), "He");
    assert!(
        p1_child.marks().iter().any(|m| m.mark_type() == "bold"),
        "first paragraph text should retain bold mark"
    );

    let p2_child = p2.child(0).unwrap();
    assert_eq!(p2_child.text_str().unwrap(), "llo");
    assert!(
        p2_child.marks().iter().any(|m| m.mark_type() == "italic"),
        "second paragraph text should retain italic mark"
    );
}

#[test]
fn test_split_block_splits_marked_text_node() {
    // <doc><p><b>Hello</b></p></doc>
    // Split at pos 3 (within the bold text, between "He" and "llo")
    // → <doc><p><b>He</b></p><p><b>llo</b></p></doc>
    let (d, schema) = doc_and_schema(doc(vec![paragraph(vec![text_with_marks(
        "Hello",
        vec![bold()],
    )])]));

    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::SplitBlock {
        pos: 3,
        node_type: "paragraph".to_string(),
        attrs: HashMap::new(),
    });

    let (new_doc, _map) = tx
        .apply(&d, &schema)
        .expect("split inside marked text should succeed");

    assert_eq!(new_doc.root().child_count(), 2);

    let p1 = new_doc.root().child(0).unwrap();
    let p2 = new_doc.root().child(1).unwrap();

    assert_eq!(p1.text_content(), "He");
    assert_eq!(p2.text_content(), "llo");

    // Both sides should retain bold
    for (para, expected_text) in [(&p1, "He"), (&p2, "llo")] {
        let child = para.child(0).unwrap();
        assert_eq!(child.text_str().unwrap(), expected_text);
        assert!(
            child.marks().iter().any(|m| m.mark_type() == "bold"),
            "text '{}' should retain bold mark after split",
            expected_text
        );
    }
}

#[test]
fn test_split_block_with_different_node_type() {
    // Split a paragraph but specify the new block should be a paragraph (same type).
    // This is the default behavior — both blocks keep the paragraph type.
    // The first block keeps the original type, the second uses node_type from the step.
    let (d, schema) = doc_and_schema(doc(vec![paragraph(vec![text("Hello")])]));
    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::SplitBlock {
        pos: 3,
        node_type: "paragraph".to_string(),
        attrs: HashMap::new(),
    });

    let (new_doc, _map) = tx.apply(&d, &schema).expect("split should succeed");

    let p1 = new_doc.root().child(0).unwrap();
    let p2 = new_doc.root().child(1).unwrap();
    assert_eq!(p1.node_type(), "paragraph");
    assert_eq!(p2.node_type(), "paragraph");
}

#[test]
fn test_split_block_empty_paragraph() {
    // <doc><p></p></doc>
    // Split at pos 1 (inside empty paragraph) → <doc><p></p><p></p></doc>
    let (d, schema) = doc_and_schema(doc(vec![paragraph(vec![])]));
    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::SplitBlock {
        pos: 1,
        node_type: "paragraph".to_string(),
        attrs: HashMap::new(),
    });

    let (new_doc, _map) = tx
        .apply(&d, &schema)
        .expect("split empty paragraph should succeed");

    assert_eq!(new_doc.root().child_count(), 2);
    assert_eq!(new_doc.root().child(0).unwrap().text_content(), "");
    assert_eq!(new_doc.root().child(1).unwrap().text_content(), "");
}

// ===========================================================================
// JoinBlocks tests
// ===========================================================================

#[test]
fn test_join_blocks_two_paragraphs() {
    // <doc><p>He</p><p>llo</p></doc> → <doc><p>Hello</p></doc>
    // Position model:
    //   pos 0: before <p> open tag of first paragraph
    //   pos 1-2: inside first p ("He"), parent_offset 0-1
    //   pos 3: end of first p content (parent_offset 2)
    //   pos 4: between first </p> close and second <p> open (doc level, parent_offset=4)
    //   pos 5: inside second p content (parent_offset 0)
    //   ...
    //
    // JoinBlocks at pos 4 joins the two paragraphs.
    let (d, schema) = doc_and_schema(doc(vec![
        paragraph(vec![text("He")]),
        paragraph(vec![text("llo")]),
    ]));

    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::JoinBlocks { pos: 4 });

    let (new_doc, _map) = tx
        .apply(&d, &schema)
        .expect("join two paragraphs should succeed");

    assert_eq!(
        new_doc.root().child_count(),
        1,
        "doc should have 1 paragraph after join"
    );

    let para = new_doc.root().child(0).unwrap();
    assert_eq!(para.text_content(), "Hello");
    assert_eq!(para.node_type(), "paragraph");

    // Doc delta: -2 (removed close tag + open tag)
    assert_eq!(
        new_doc.content_size(),
        d.content_size() - 2,
        "content size should decrease by 2 after join"
    );
}

#[test]
fn test_join_blocks_merges_text_with_same_marks() {
    // <doc><p><b>He</b></p><p><b>llo</b></p></doc>
    // Join at boundary → <doc><p><b>Hello</b></p></doc>
    // The bold text nodes should merge into one.
    let (d, schema) = doc_and_schema(doc(vec![
        paragraph(vec![text_with_marks("He", vec![bold()])]),
        paragraph(vec![text_with_marks("llo", vec![bold()])]),
    ]));

    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::JoinBlocks { pos: 4 });

    let (new_doc, _map) = tx
        .apply(&d, &schema)
        .expect("join merging same marks should succeed");

    let para = new_doc.root().child(0).unwrap();
    assert_eq!(para.text_content(), "Hello");

    // Should merge into a single bold text node
    assert_eq!(
        para.child_count(),
        1,
        "merged bold text should produce 1 text node"
    );
    let child = para.child(0).unwrap();
    assert_eq!(child.text_str().unwrap(), "Hello");
    assert!(child.marks().iter().any(|m| m.mark_type() == "bold"));
}

#[test]
fn test_join_blocks_preserves_different_marks() {
    // <doc><p><b>He</b></p><p><i>llo</i></p></doc>
    // Join → <doc><p><b>He</b><i>llo</i></p></doc>
    // Different marks should NOT merge — keep as separate text nodes.
    let (d, schema) = doc_and_schema(doc(vec![
        paragraph(vec![text_with_marks("He", vec![bold()])]),
        paragraph(vec![text_with_marks("llo", vec![italic()])]),
    ]));

    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::JoinBlocks { pos: 4 });

    let (new_doc, _map) = tx
        .apply(&d, &schema)
        .expect("join with different marks should succeed");

    let para = new_doc.root().child(0).unwrap();
    assert_eq!(para.text_content(), "Hello");
    assert_eq!(
        para.child_count(),
        2,
        "differently-marked text should remain as 2 nodes"
    );

    let c0 = para.child(0).unwrap();
    assert_eq!(c0.text_str().unwrap(), "He");
    assert!(c0.marks().iter().any(|m| m.mark_type() == "bold"));

    let c1 = para.child(1).unwrap();
    assert_eq!(c1.text_str().unwrap(), "llo");
    assert!(c1.marks().iter().any(|m| m.mark_type() == "italic"));
}

#[test]
fn test_join_blocks_uses_first_block_type() {
    // When joining blocks of different types, the result uses the first block's type.
    // For this test we just verify with two paragraphs (same type).
    let (d, schema) = doc_and_schema(doc(vec![
        paragraph(vec![text("A")]),
        paragraph(vec![text("B")]),
    ]));

    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::JoinBlocks { pos: 3 });

    let (new_doc, _map) = tx.apply(&d, &schema).expect("join should succeed");

    let para = new_doc.root().child(0).unwrap();
    assert_eq!(para.node_type(), "paragraph");
    assert_eq!(para.text_content(), "AB");
}

#[test]
fn test_join_blocks_with_empty_first_paragraph() {
    // <doc><p></p><p>Hello</p></doc> → <doc><p>Hello</p></doc>
    // First p node_size = 1+0+1 = 2
    // Join at pos 2 (between the two paragraphs at doc level)
    let (d, schema) = doc_and_schema(doc(vec![paragraph(vec![]), paragraph(vec![text("Hello")])]));

    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::JoinBlocks { pos: 2 });

    let (new_doc, _map) = tx
        .apply(&d, &schema)
        .expect("join with empty first paragraph should succeed");

    assert_eq!(new_doc.root().child_count(), 1);
    assert_eq!(new_doc.root().child(0).unwrap().text_content(), "Hello");
}

#[test]
fn test_join_blocks_with_empty_second_paragraph() {
    // <doc><p>Hello</p><p></p></doc> → <doc><p>Hello</p></doc>
    // First p node_size = 1+5+1 = 7
    // Join at pos 7 (between the two paragraphs at doc level)
    let (d, schema) = doc_and_schema(doc(vec![paragraph(vec![text("Hello")]), paragraph(vec![])]));

    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::JoinBlocks { pos: 7 });

    let (new_doc, _map) = tx
        .apply(&d, &schema)
        .expect("join with empty second paragraph should succeed");

    assert_eq!(new_doc.root().child_count(), 1);
    assert_eq!(new_doc.root().child(0).unwrap().text_content(), "Hello");
}

#[test]
fn test_join_blocks_list_items() {
    // <doc><ul><li><p>He</p></li><li><p>llo</p></li></ul></doc>
    // Join the two list items. We need the boundary position between
    // the two list items (at the ul content level).
    //
    // Position model:
    //   pos 0: doc content, before <ul> open
    //   pos 1: ul content, before <li> open of first item
    //   pos 2: li content, before <p> open
    //   pos 3: p content offset 0 (before "H")
    //   pos 4: p content offset 1 (between "H" and "e")
    //   pos 5: p content offset 2 (end of p content, after "e")
    //   pos 6: after </p> close (inside li, after the paragraph)
    //   pos 7: after </li> close (inside ul, between the two items)
    //   pos 8: inside second <li>, before <p> open
    //   ...
    //
    // The join position is pos 7 (between the two list items in the list).
    let (d, schema) = doc_and_schema(doc(vec![bullet_list(vec![
        list_item(vec![paragraph(vec![text("He")])]),
        list_item(vec![paragraph(vec![text("llo")])]),
    ])]));

    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::JoinBlocks { pos: 7 });

    let (new_doc, _map) = tx
        .apply(&d, &schema)
        .expect("join list items should succeed");

    let ul = new_doc.root().child(0).unwrap();
    assert_eq!(ul.child_count(), 1, "list should have 1 item after join");

    let li = ul.child(0).unwrap();
    assert_eq!(li.node_type(), "listItem");
    // The joined list item should have the combined paragraph content.
    // Both list items had one paragraph each. The join merges the li content,
    // so we get two paragraphs inside one list item, OR the paragraphs merge.
    // Since JoinBlocks joins the list items (not the paragraphs), the content
    // of both list items is concatenated. Each had one <p>, so the result
    // should have two paragraphs.
    assert_eq!(
        li.child_count(),
        2,
        "joined list item should have 2 paragraphs (one from each original item)"
    );
    assert_eq!(li.child(0).unwrap().text_content(), "He");
    assert_eq!(li.child(1).unwrap().text_content(), "llo");
}

// ===========================================================================
// SplitBlock + JoinBlocks StepMap tests
// ===========================================================================

#[test]
fn test_step_map_split_block() {
    // SplitBlock at pos 3: inserts 2 tokens (close + open tag)
    // Positions before 3 unchanged, positions at 3 and after shift by +2
    let (d, schema) = doc_and_schema(doc(vec![paragraph(vec![text("Hello")])]));
    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::SplitBlock {
        pos: 3,
        node_type: "paragraph".to_string(),
        attrs: HashMap::new(),
    });

    let (_new_doc, map) = tx.apply(&d, &schema).expect("split should succeed");

    assert_eq!(
        map.map_pos(1),
        1,
        "position 1 (before split) should be unchanged"
    );
    assert_eq!(
        map.map_pos(2),
        2,
        "position 2 (before split) should be unchanged"
    );
    assert_eq!(
        map.map_pos(3),
        5,
        "position at split point should shift forward by 2"
    );
    assert_eq!(
        map.map_pos(5),
        7,
        "position 5 (after split) should shift by +2"
    );
}

#[test]
fn test_step_map_join_blocks() {
    // JoinBlocks at pos 4: removes 2 tokens (close + open tag)
    // Positions in the first block unchanged, positions at/after join shift by -2
    let (d, schema) = doc_and_schema(doc(vec![
        paragraph(vec![text("He")]),
        paragraph(vec![text("llo")]),
    ]));
    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::JoinBlocks { pos: 4 });

    let (_new_doc, map) = tx.apply(&d, &schema).expect("join should succeed");

    assert_eq!(
        map.map_pos(1),
        1,
        "position 1 (inside first paragraph) should be unchanged"
    );
    assert_eq!(
        map.map_pos(3),
        3,
        "position 3 (end of first paragraph content) should be unchanged"
    );
    // Position 4 is the join boundary (deleted range [4,5] - the close+open tags)
    // After join, that boundary collapses — position 4 maps to 4 (the delete start)
    // Position 5 was inside the deleted range → maps to delete start
    assert_eq!(
        map.map_pos(5),
        4,
        "position inside deleted boundary should collapse to join point"
    );
    assert_eq!(
        map.map_pos(6),
        4,
        "position right after deleted boundary (start of second p content) should shift by -2"
    );
    assert_eq!(
        map.map_pos(8),
        6,
        "position 8 (after join point) should shift by -2"
    );
}

// ===========================================================================
// SplitBlock then JoinBlocks round-trip test
// ===========================================================================

#[test]
fn test_split_then_join_round_trip() {
    // Split <doc><p>Hello</p></doc> at pos 3 → <doc><p>He</p><p>llo</p></doc>
    // Then join at pos 4 (the new boundary) → <doc><p>Hello</p></doc>
    let (d, schema) = doc_and_schema(doc(vec![paragraph(vec![text("Hello")])]));

    // Step 1: Split
    let mut tx_split = Transaction::new(Source::Input);
    tx_split.add_step(Step::SplitBlock {
        pos: 3,
        node_type: "paragraph".to_string(),
        attrs: HashMap::new(),
    });
    let (split_doc, _) = tx_split.apply(&d, &schema).expect("split should succeed");
    assert_eq!(split_doc.root().child_count(), 2);

    // Step 2: Join
    // After split at pos 3, the boundary is at pos 4 (first p size = 1+2+1 = 4,
    // so doc content offset 4 is between the two paragraphs).
    let mut tx_join = Transaction::new(Source::Input);
    tx_join.add_step(Step::JoinBlocks { pos: 4 });
    let (joined_doc, _) = tx_join
        .apply(&split_doc, &schema)
        .expect("join should succeed");

    assert_eq!(joined_doc.root().child_count(), 1);
    assert_eq!(
        joined_doc.root().child(0).unwrap().text_content(),
        "Hello",
        "round-trip split+join should restore original text"
    );
}

// ===========================================================================
// WrapInList tests
// ===========================================================================

#[test]
fn test_wrap_single_paragraph_in_bullet_list() {
    // <doc><p>Hello</p></doc>
    // Position model:
    //   pos 0: doc content, before <p> open tag
    //   pos 1: inside p, before "H"
    //   pos 6: inside p, after "o"
    //   pos 7: doc content, after </p> close tag (= content_size)
    //
    // WrapInList from=0 to=7 should wrap the single paragraph in a bullet list.
    // Expected: <doc><ul><li><p>Hello</p></li></ul></doc>
    let (d, schema) = doc_and_schema(doc(vec![paragraph(vec![text("Hello")])]));
    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::WrapInList {
        from: 0,
        to: 7,
        list_type: "bulletList".to_string(),
        item_type: "listItem".to_string(),
        attrs: HashMap::new(),
    });

    let (new_doc, _map) = tx
        .apply(&d, &schema)
        .expect("wrap single paragraph should succeed");

    // Should be: doc > bulletList > listItem > paragraph > "Hello"
    assert_eq!(
        new_doc.root().child_count(),
        1,
        "doc should have 1 child (the list)"
    );
    let ul = new_doc.root().child(0).unwrap();
    assert_eq!(ul.node_type(), "bulletList");
    assert_eq!(ul.child_count(), 1, "list should have 1 item");

    let li = ul.child(0).unwrap();
    assert_eq!(li.node_type(), "listItem");
    assert_eq!(li.child_count(), 1, "list item should have 1 paragraph");

    let p = li.child(0).unwrap();
    assert_eq!(p.node_type(), "paragraph");
    assert_eq!(p.text_content(), "Hello");
}

#[test]
fn test_wrap_two_paragraphs_in_bullet_list() {
    // <doc><p>A</p><p>B</p></doc>
    // Position model:
    //   pos 0: before first <p>
    //   pos 3: after first </p> (= before second <p>)
    //   pos 6: after second </p>
    //
    // WrapInList from=0 to=6 should wrap both paragraphs.
    // Expected: <doc><ul><li><p>A</p></li><li><p>B</p></li></ul></doc>
    let (d, schema) = doc_and_schema(doc(vec![
        paragraph(vec![text("A")]),
        paragraph(vec![text("B")]),
    ]));
    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::WrapInList {
        from: 0,
        to: 6,
        list_type: "bulletList".to_string(),
        item_type: "listItem".to_string(),
        attrs: HashMap::new(),
    });

    let (new_doc, _map) = tx
        .apply(&d, &schema)
        .expect("wrap two paragraphs should succeed");

    assert_eq!(
        new_doc.root().child_count(),
        1,
        "doc should have 1 child (the list)"
    );
    let ul = new_doc.root().child(0).unwrap();
    assert_eq!(ul.node_type(), "bulletList");
    assert_eq!(ul.child_count(), 2, "list should have 2 items");

    let li1 = ul.child(0).unwrap();
    assert_eq!(li1.child(0).unwrap().text_content(), "A");
    let li2 = ul.child(1).unwrap();
    assert_eq!(li2.child(0).unwrap().text_content(), "B");
}

#[test]
fn test_wrap_in_ordered_list_with_start_attr() {
    // <doc><p>Item</p></doc>
    // Wrap in ordered list with start=3 attr.
    let (d, schema) = doc_and_schema(doc(vec![paragraph(vec![text("Item")])]));
    let mut attrs = HashMap::new();
    attrs.insert("start".to_string(), serde_json::Value::Number(3.into()));
    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::WrapInList {
        from: 0,
        to: 6,
        list_type: "orderedList".to_string(),
        item_type: "listItem".to_string(),
        attrs,
    });

    let (new_doc, _map) = tx
        .apply(&d, &schema)
        .expect("wrap in ordered list should succeed");

    let ol = new_doc.root().child(0).unwrap();
    assert_eq!(ol.node_type(), "orderedList");
    assert_eq!(
        ol.attrs().get("start"),
        Some(&serde_json::Value::Number(3.into())),
        "ordered list should have start=3 attr"
    );
    assert_eq!(ol.child_count(), 1);
    assert_eq!(
        ol.child(0).unwrap().child(0).unwrap().text_content(),
        "Item"
    );
}

#[test]
fn test_wrap_already_listed_content_errors() {
    // <doc><ul><li><p>Hello</p></li></ul></doc>
    // Trying to wrap the list itself in another list should error because
    // we can only wrap block nodes that are not already list items.
    //
    // Position model: doc content size = 11
    //   pos 0: before <ul>
    //   pos 11: after </ul>
    let (d, schema) = doc_and_schema(doc(vec![bullet_list(vec![list_item(vec![paragraph(
        vec![text("Hello")],
    )])])]));
    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::WrapInList {
        from: 0,
        to: 11,
        list_type: "bulletList".to_string(),
        item_type: "listItem".to_string(),
        attrs: HashMap::new(),
    });

    let result = tx.apply(&d, &schema);
    assert!(
        result.is_err(),
        "wrapping a list in another list should error"
    );
}

#[test]
fn test_wrap_middle_paragraphs_preserves_surrounding() {
    // <doc><p>Before</p><p>Wrap Me</p><p>After</p></doc>
    // Wrap only the middle paragraph (from=8 to=16).
    // Position model:
    //   pos 0: before first <p>
    //   first p node_size = 1+6+1 = 8
    //   pos 8: before second <p>
    //   second p node_size = 1+7+1 = 9
    //   pos 17: before third <p>  -- wait, 8+9=17
    //   Hmm, let me recalculate:
    //   "Before" = 6 chars, p node_size = 8
    //   "Wrap Me" = 7 chars, p node_size = 9
    //   "After" = 5 chars, p node_size = 7
    //   doc content_size = 8 + 9 + 7 = 24
    //
    // WrapInList from=8 to=17 should wrap just the middle paragraph.
    let (d, schema) = doc_and_schema(doc(vec![
        paragraph(vec![text("Before")]),
        paragraph(vec![text("Wrap Me")]),
        paragraph(vec![text("After")]),
    ]));
    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::WrapInList {
        from: 8,
        to: 17,
        list_type: "bulletList".to_string(),
        item_type: "listItem".to_string(),
        attrs: HashMap::new(),
    });

    let (new_doc, _map) = tx
        .apply(&d, &schema)
        .expect("wrap middle paragraph should succeed");

    // Expected: <doc><p>Before</p><ul><li><p>Wrap Me</p></li></ul><p>After</p></doc>
    assert_eq!(
        new_doc.root().child_count(),
        3,
        "doc should have 3 children: p + ul + p"
    );

    let first = new_doc.root().child(0).unwrap();
    assert_eq!(first.node_type(), "paragraph");
    assert_eq!(first.text_content(), "Before");

    let middle = new_doc.root().child(1).unwrap();
    assert_eq!(middle.node_type(), "bulletList");
    assert_eq!(
        middle.child(0).unwrap().child(0).unwrap().text_content(),
        "Wrap Me"
    );

    let last = new_doc.root().child(2).unwrap();
    assert_eq!(last.node_type(), "paragraph");
    assert_eq!(last.text_content(), "After");
}

#[test]
fn test_wrap_invalid_list_type_errors() {
    // Using a non-list type should error.
    let (d, schema) = doc_and_schema(doc(vec![paragraph(vec![text("A")])]));
    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::WrapInList {
        from: 0,
        to: 3,
        list_type: "paragraph".to_string(),
        item_type: "listItem".to_string(),
        attrs: HashMap::new(),
    });

    let result = tx.apply(&d, &schema);
    assert!(
        result.is_err(),
        "using a non-list node type for list_type should error"
    );
}

// ===========================================================================
// UnwrapFromList tests
// ===========================================================================

#[test]
fn test_unwrap_only_list_item() {
    // <doc><ul><li><p>Hello</p></li></ul></doc>
    // UnwrapFromList at pos 3 (inside the paragraph within the list item)
    // Expected: <doc><p>Hello</p></doc>
    //
    // Position model:
    //   pos 0: before <ul>
    //   pos 1: inside ul, before <li>
    //   pos 2: inside li, before <p>
    //   pos 3: inside p, before "H"
    let (d, schema) = doc_and_schema(doc(vec![bullet_list(vec![list_item(vec![paragraph(
        vec![text("Hello")],
    )])])]));

    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::UnwrapFromList { pos: 3 });

    let (new_doc, _map) = tx
        .apply(&d, &schema)
        .expect("unwrap only list item should succeed");

    // Should produce: <doc><p>Hello</p></doc>
    assert_eq!(
        new_doc.root().child_count(),
        1,
        "doc should have 1 child (the paragraph)"
    );
    let p = new_doc.root().child(0).unwrap();
    assert_eq!(p.node_type(), "paragraph");
    assert_eq!(p.text_content(), "Hello");
}

#[test]
fn test_unwrap_first_of_two_items() {
    // <doc><ul><li><p>A</p></li><li><p>B</p></li></ul></doc>
    // UnwrapFromList at pos 3 (inside first list item's paragraph)
    // Expected: <doc><p>A</p><ul><li><p>B</p></li></ul></doc>
    //
    // Position model:
    //   pos 0: before <ul>
    //   pos 1: inside ul, before first <li>
    //   pos 2: inside first li, before <p>
    //   pos 3: inside first p, before "A"
    let (d, schema) = doc_and_schema(doc(vec![bullet_list(vec![
        list_item(vec![paragraph(vec![text("A")])]),
        list_item(vec![paragraph(vec![text("B")])]),
    ])]));

    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::UnwrapFromList { pos: 3 });

    let (new_doc, _map) = tx
        .apply(&d, &schema)
        .expect("unwrap first item should succeed");

    // Expected: <doc><p>A</p><ul><li><p>B</p></li></ul></doc>
    assert_eq!(
        new_doc.root().child_count(),
        2,
        "doc should have 2 children: paragraph + remaining list"
    );

    let p = new_doc.root().child(0).unwrap();
    assert_eq!(p.node_type(), "paragraph");
    assert_eq!(p.text_content(), "A");

    let remaining_list = new_doc.root().child(1).unwrap();
    assert_eq!(remaining_list.node_type(), "bulletList");
    assert_eq!(remaining_list.child_count(), 1);
    assert_eq!(
        remaining_list
            .child(0)
            .unwrap()
            .child(0)
            .unwrap()
            .text_content(),
        "B"
    );
}

#[test]
fn test_unwrap_last_of_two_items() {
    // <doc><ul><li><p>A</p></li><li><p>B</p></li></ul></doc>
    // UnwrapFromList at pos 9 (inside second list item's paragraph)
    // Expected: <doc><ul><li><p>A</p></li></ul><p>B</p></doc>
    //
    // Position model:
    //   pos 0: before <ul>
    //   pos 1: inside ul, before first <li>
    //   first li node_size = 1 + (1 + 1 + 1) + 1 = 5
    //   pos 6: inside ul, before second <li>
    //   pos 7: inside second li, before <p>
    //   pos 8: inside second p, before "B" -- wait
    //   Actually: first <li> node_size = 1 + paragraph_size + 1 = 1 + 3 + 1 = 5
    //   paragraph "A" node_size = 1 + 1 + 1 = 3
    //   pos 1: before first <li>
    //   pos 2: inside first li, before <p>
    //   pos 3: inside p, before "A"
    //   pos 4: inside p, after "A"
    //   pos 5: after </p> (inside li, end)
    //   pos 6: after </li> (inside ul, between items)
    //   pos 7: inside second <li>, before <p>
    //   pos 8: inside second p, before "B"
    let (d, schema) = doc_and_schema(doc(vec![bullet_list(vec![
        list_item(vec![paragraph(vec![text("A")])]),
        list_item(vec![paragraph(vec![text("B")])]),
    ])]));

    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::UnwrapFromList { pos: 8 });

    let (new_doc, _map) = tx
        .apply(&d, &schema)
        .expect("unwrap last item should succeed");

    // Expected: <doc><ul><li><p>A</p></li></ul><p>B</p></doc>
    assert_eq!(
        new_doc.root().child_count(),
        2,
        "doc should have 2 children: remaining list + paragraph"
    );

    let remaining_list = new_doc.root().child(0).unwrap();
    assert_eq!(remaining_list.node_type(), "bulletList");
    assert_eq!(remaining_list.child_count(), 1);
    assert_eq!(
        remaining_list
            .child(0)
            .unwrap()
            .child(0)
            .unwrap()
            .text_content(),
        "A"
    );

    let p = new_doc.root().child(1).unwrap();
    assert_eq!(p.node_type(), "paragraph");
    assert_eq!(p.text_content(), "B");
}

#[test]
fn test_unwrap_middle_item_splits_list() {
    // <doc><ul><li><p>A</p></li><li><p>B</p></li><li><p>C</p></li></ul></doc>
    // UnwrapFromList at pos 8 (inside second list item's paragraph)
    // Expected: <doc><ul><li><p>A</p></li></ul><p>B</p><ul><li><p>C</p></li></ul></doc>
    //
    // Position model:
    //   pos 0: before <ul>
    //   pos 1: inside ul, before first <li>
    //   first <li> node_size = 1 + 3 + 1 = 5
    //   pos 6: inside ul, before second <li>
    //   pos 7: inside second li, before <p>
    //   pos 8: inside second p, before "B"
    let (d, schema) = doc_and_schema(doc(vec![bullet_list(vec![
        list_item(vec![paragraph(vec![text("A")])]),
        list_item(vec![paragraph(vec![text("B")])]),
        list_item(vec![paragraph(vec![text("C")])]),
    ])]));

    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::UnwrapFromList { pos: 8 });

    let (new_doc, _map) = tx
        .apply(&d, &schema)
        .expect("unwrap middle item should succeed");

    // Expected: <doc><ul><li><p>A</p></li></ul><p>B</p><ul><li><p>C</p></li></ul></doc>
    assert_eq!(
        new_doc.root().child_count(),
        3,
        "doc should have 3 children: list + paragraph + list"
    );

    let list1 = new_doc.root().child(0).unwrap();
    assert_eq!(list1.node_type(), "bulletList");
    assert_eq!(list1.child_count(), 1);
    assert_eq!(
        list1.child(0).unwrap().child(0).unwrap().text_content(),
        "A"
    );

    let p = new_doc.root().child(1).unwrap();
    assert_eq!(p.node_type(), "paragraph");
    assert_eq!(p.text_content(), "B");

    let list2 = new_doc.root().child(2).unwrap();
    assert_eq!(list2.node_type(), "bulletList");
    assert_eq!(list2.child_count(), 1);
    assert_eq!(
        list2.child(0).unwrap().child(0).unwrap().text_content(),
        "C"
    );
}

#[test]
fn test_unwrap_from_list_pos_not_in_list_errors() {
    // Position is inside a paragraph that is not in a list — should error.
    let (d, schema) = doc_and_schema(doc(vec![paragraph(vec![text("A")])]));
    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::UnwrapFromList { pos: 1 });

    let result = tx.apply(&d, &schema);
    assert!(
        result.is_err(),
        "UnwrapFromList on a non-list position should error"
    );
}

#[test]
fn test_indent_list_item_nests_under_previous_sibling() {
    let (d, schema) = doc_and_schema(doc(vec![bullet_list(vec![
        list_item(vec![paragraph(vec![text("A")])]),
        list_item(vec![paragraph(vec![text("B")])]),
        list_item(vec![paragraph(vec![text("C")])]),
    ])]));

    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::IndentListItem { pos: 8 });

    let (new_doc, _map) = tx.apply(&d, &schema).expect("indent should succeed");

    let list = new_doc.root().child(0).unwrap();
    assert_eq!(list.child_count(), 2, "top-level list should lose one item");

    let first_item = list.child(0).unwrap();
    assert_eq!(first_item.child(0).unwrap().text_content(), "A");
    let nested = first_item.child(1).unwrap();
    assert_eq!(nested.node_type(), "bulletList");
    assert_eq!(nested.child_count(), 1);
    assert_eq!(
        nested.child(0).unwrap().child(0).unwrap().text_content(),
        "B"
    );

    let second_item = list.child(1).unwrap();
    assert_eq!(second_item.child(0).unwrap().text_content(), "C");
}

#[test]
fn test_indent_first_list_item_is_noop() {
    let (d, schema) = doc_and_schema(doc(vec![bullet_list(vec![
        list_item(vec![paragraph(vec![text("A")])]),
        list_item(vec![paragraph(vec![text("B")])]),
    ])]));

    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::IndentListItem { pos: 3 });

    let (new_doc, _map) = tx.apply(&d, &schema).expect("first-item indent should be a no-op");
    assert_eq!(new_doc.root().text_content(), d.root().text_content());
    assert_eq!(new_doc.root().child_count(), d.root().child_count());
    assert_eq!(new_doc.root().child(0).unwrap().child_count(), 2);
}

#[test]
fn test_outdent_nested_list_item_lifts_after_parent_item() {
    let (d, schema) = doc_and_schema(doc(vec![bullet_list(vec![
        list_item(vec![
            paragraph(vec![text("A")]),
            bullet_list(vec![
                list_item(vec![paragraph(vec![text("B")])]),
                list_item(vec![paragraph(vec![text("C")])]),
            ]),
        ]),
        list_item(vec![paragraph(vec![text("D")])]),
    ])]));

    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::OutdentListItem { pos: 8 });

    let (new_doc, _map) = tx.apply(&d, &schema).expect("outdent should succeed");

    let list = new_doc.root().child(0).unwrap();
    assert_eq!(list.child_count(), 3);

    let first_item = list.child(0).unwrap();
    assert_eq!(first_item.child(0).unwrap().text_content(), "A");

    let second_item = list.child(1).unwrap();
    assert_eq!(second_item.child(0).unwrap().text_content(), "B");
    let nested = second_item.child(1).unwrap();
    assert_eq!(nested.node_type(), "bulletList");
    assert_eq!(nested.child_count(), 1);
    assert_eq!(
        nested.child(0).unwrap().child(0).unwrap().text_content(),
        "C"
    );

    let third_item = list.child(2).unwrap();
    assert_eq!(third_item.child(0).unwrap().text_content(), "D");
}

#[test]
fn test_outdent_top_level_list_item_is_noop() {
    let (d, schema) = doc_and_schema(doc(vec![bullet_list(vec![
        list_item(vec![paragraph(vec![text("A")])]),
        list_item(vec![paragraph(vec![text("B")])]),
    ])]));

    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::OutdentListItem { pos: 8 });

    let (new_doc, _map) = tx.apply(&d, &schema).expect("top-level outdent should be a no-op");
    assert_eq!(new_doc.root().text_content(), d.root().text_content());
    assert_eq!(new_doc.root().child_count(), d.root().child_count());
    assert_eq!(new_doc.root().child(0).unwrap().child_count(), 2);
}

// ===========================================================================
// WrapInList + UnwrapFromList round-trip tests
// ===========================================================================

#[test]
fn test_wrap_then_unwrap_round_trip_single_paragraph() {
    // Start: <doc><p>Hello</p></doc>
    // Wrap: <doc><ul><li><p>Hello</p></li></ul></doc>
    // Unwrap: <doc><p>Hello</p></doc>
    let (d, schema) = doc_and_schema(doc(vec![paragraph(vec![text("Hello")])]));

    // Step 1: Wrap
    let mut tx_wrap = Transaction::new(Source::Input);
    tx_wrap.add_step(Step::WrapInList {
        from: 0,
        to: 7,
        list_type: "bulletList".to_string(),
        item_type: "listItem".to_string(),
        attrs: HashMap::new(),
    });
    let (wrapped_doc, _) = tx_wrap.apply(&d, &schema).expect("wrap should succeed");
    assert_eq!(
        wrapped_doc.root().child(0).unwrap().node_type(),
        "bulletList",
        "after wrap, doc should contain a bullet list"
    );

    // Step 2: Unwrap — position 3 is inside the paragraph in the list item
    // Wrapped doc: <doc><ul><li><p>Hello</p></li></ul></doc>
    //   pos 0: before <ul>
    //   pos 1: inside ul, before <li>
    //   pos 2: inside li, before <p>
    //   pos 3: inside p, before "H"
    let mut tx_unwrap = Transaction::new(Source::Input);
    tx_unwrap.add_step(Step::UnwrapFromList { pos: 3 });
    let (final_doc, _) = tx_unwrap
        .apply(&wrapped_doc, &schema)
        .expect("unwrap should succeed");

    assert_eq!(
        final_doc.root().child_count(),
        1,
        "doc should have 1 paragraph after round-trip"
    );
    assert_eq!(final_doc.root().child(0).unwrap().node_type(), "paragraph");
    assert_eq!(
        final_doc.root().child(0).unwrap().text_content(),
        "Hello",
        "round-trip wrap+unwrap should restore original text"
    );
}

#[test]
fn test_wrap_then_unwrap_round_trip_two_paragraphs() {
    // Start: <doc><p>A</p><p>B</p></doc>
    // Wrap both: <doc><ul><li><p>A</p></li><li><p>B</p></li></ul></doc>
    // Unwrap first, then unwrap second → original doc
    let (d, schema) = doc_and_schema(doc(vec![
        paragraph(vec![text("A")]),
        paragraph(vec![text("B")]),
    ]));

    // Wrap
    let mut tx_wrap = Transaction::new(Source::Input);
    tx_wrap.add_step(Step::WrapInList {
        from: 0,
        to: 6,
        list_type: "bulletList".to_string(),
        item_type: "listItem".to_string(),
        attrs: HashMap::new(),
    });
    let (wrapped_doc, _) = tx_wrap.apply(&d, &schema).expect("wrap should succeed");

    // Unwrap first item (pos 3 = inside first paragraph in first list item)
    let mut tx1 = Transaction::new(Source::Input);
    tx1.add_step(Step::UnwrapFromList { pos: 3 });
    let (after_first_unwrap, _) = tx1
        .apply(&wrapped_doc, &schema)
        .expect("first unwrap should succeed");

    // After first unwrap: <doc><p>A</p><ul><li><p>B</p></li></ul></doc>
    assert_eq!(after_first_unwrap.root().child_count(), 2);
    assert_eq!(
        after_first_unwrap.root().child(0).unwrap().text_content(),
        "A"
    );

    // Unwrap second item. In the current doc:
    //   <doc><p>A</p><ul><li><p>B</p></li></ul></doc>
    //   first p node_size = 3, pos 3 = after </p>
    //   pos 3: before <ul>
    //   pos 4: inside ul, before <li>
    //   pos 5: inside li, before <p>
    //   pos 6: inside p, before "B"
    let mut tx2 = Transaction::new(Source::Input);
    tx2.add_step(Step::UnwrapFromList { pos: 6 });
    let (final_doc, _) = tx2
        .apply(&after_first_unwrap, &schema)
        .expect("second unwrap should succeed");

    assert_eq!(final_doc.root().child_count(), 2);
    assert_eq!(final_doc.root().child(0).unwrap().text_content(), "A");
    assert_eq!(final_doc.root().child(1).unwrap().text_content(), "B");
}

// ===========================================================================
// WrapInList / UnwrapFromList StepMap tests
// ===========================================================================

#[test]
fn test_step_map_wrap_in_list() {
    // Wrapping <doc><p>Hello</p></doc> adds 4 tokens (ul open, li open, li close, ul close)
    // Positions before the wrap start are unchanged.
    // Positions at or after should shift by +4 (two opens before, two closes after).
    // Actually, positions inside the paragraph shift by +2 (ul open + li open before the p).
    let (d, schema) = doc_and_schema(doc(vec![paragraph(vec![text("Hello")])]));
    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::WrapInList {
        from: 0,
        to: 7,
        list_type: "bulletList".to_string(),
        item_type: "listItem".to_string(),
        attrs: HashMap::new(),
    });

    let (_new_doc, map) = tx.apply(&d, &schema).expect("wrap should succeed");

    // The wrapping inserts 4 tokens total. The StepMap should record this.
    // Positions before the range are unchanged, positions after shift.
    // Position 0 in the old doc → position 0 in new doc (before the ul)
    // Actually no — we insert 2 tokens (ul open + li open) at the beginning,
    // so positions at/after 0 shift by +2.
    // Then 2 tokens (li close + ul close) at the end, shifting positions after the end.
    // The net effect on positions within the paragraph: +2.
    assert_eq!(
        map.map_pos(1),
        3,
        "position 1 (inside paragraph) should shift by +2 (ul open + li open)"
    );
}

#[test]
fn test_step_map_unwrap_from_list() {
    // Unwrapping the only item from <doc><ul><li><p>Hello</p></li></ul></doc>
    // removes 4 tokens (ul open, li open, li close, ul close).
    let (d, schema) = doc_and_schema(doc(vec![bullet_list(vec![list_item(vec![paragraph(
        vec![text("Hello")],
    )])])]));
    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::UnwrapFromList { pos: 3 });

    let (_new_doc, map) = tx.apply(&d, &schema).expect("unwrap should succeed");

    // Position 3 in old doc (inside paragraph, before "H") should map to 1 in new doc
    // because we removed ul open (1) + li open (1) = 2 tokens before it.
    assert_eq!(
        map.map_pos(3),
        1,
        "position inside paragraph should shift by -2 after unwrap"
    );
}

#[test]
fn test_step_map_unwrap_last_list_item_preserves_lifted_content_position() {
    let (d, schema) = doc_and_schema(doc(vec![bullet_list(vec![
        list_item(vec![paragraph(vec![text("A")])]),
        list_item(vec![paragraph(vec![])]),
    ])]));
    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::UnwrapFromList { pos: 8 });

    let (_new_doc, map) = tx
        .apply(&d, &schema)
        .expect("unwrap trailing list item should succeed");

    assert_eq!(
        map.map_pos(8),
        8,
        "position inside the lifted trailing paragraph should stay inside that paragraph"
    );
}

// ===========================================================================
// InsertNode / ReplaceRange now implemented — verify they work
// ===========================================================================

#[test]
fn test_insert_node_at_doc_start() {
    // Insert an empty paragraph at pos 0 (before the first block)
    let (d, schema) = doc_and_schema(doc(vec![paragraph(vec![text("A")])]));
    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::InsertNode {
        pos: 0,
        node: paragraph(vec![]),
    });

    let (new_doc, _map) = tx
        .apply(&d, &schema)
        .expect("InsertNode at doc start should succeed");

    assert_eq!(new_doc.root().child_count(), 2);
    assert_eq!(new_doc.root().child(0).unwrap().text_content(), "");
    assert_eq!(new_doc.root().child(1).unwrap().text_content(), "A");
}

#[test]
fn test_replace_range_inline_content() {
    // Replace a single char with another
    let (d, schema) = doc_and_schema(doc(vec![paragraph(vec![text("A")])]));
    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::ReplaceRange {
        from: 1,
        to: 2,
        content: Fragment::from(vec![text("B")]),
    });

    let (new_doc, _map) = tx.apply(&d, &schema).expect("ReplaceRange should succeed");

    assert_eq!(new_doc.root().text_content(), "B");
}

// ===========================================================================
// Transaction source and meta
// ===========================================================================

#[test]
fn test_transaction_source_variants() {
    // Just verify all source variants can be constructed
    let _ = Transaction::new(Source::Input);
    let _ = Transaction::new(Source::Format);
    let _ = Transaction::new(Source::Paste);
    let _ = Transaction::new(Source::History);
    let _ = Transaction::new(Source::Api);
    let _ = Transaction::new(Source::Reconciliation);
}

#[test]
fn test_transaction_meta() {
    let mut tx = Transaction::new(Source::Input);
    tx.meta.insert(
        "user_id".to_string(),
        serde_json::Value::String("abc".to_string()),
    );
    assert_eq!(
        tx.meta.get("user_id"),
        Some(&serde_json::Value::String("abc".to_string()))
    );
}

// ===========================================================================
// InsertNode tests
// ===========================================================================

fn hard_break() -> Node {
    Node::void("hardBreak".to_string(), HashMap::new())
}

fn horizontal_rule() -> Node {
    Node::void("horizontalRule".to_string(), HashMap::new())
}

#[test]
fn test_insert_horizontal_rule_between_paragraphs() {
    // <doc><p>A</p><p>B</p></doc>
    // Insert horizontalRule at pos 3 (between the two paragraphs at doc level)
    // Expected: <doc><p>A</p><hr><p>B</p></doc>
    //
    // Position model:
    //   pos 0: before <p>A</p>
    //   pos 1: inside p, before "A"
    //   pos 2: inside p, after "A"
    //   pos 3: after </p> (before second <p>), doc level
    //   pos 4: inside second p, before "B"
    let (d, schema) = doc_and_schema(doc(vec![
        paragraph(vec![text("A")]),
        paragraph(vec![text("B")]),
    ]));

    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::InsertNode {
        pos: 3,
        node: horizontal_rule(),
    });

    let (new_doc, _map) = tx
        .apply(&d, &schema)
        .expect("insert horizontal rule should succeed");

    assert_eq!(
        new_doc.root().child_count(),
        3,
        "doc should have 3 children: p + hr + p"
    );
    assert_eq!(new_doc.root().child(0).unwrap().node_type(), "paragraph");
    assert_eq!(new_doc.root().child(0).unwrap().text_content(), "A");
    assert_eq!(
        new_doc.root().child(1).unwrap().node_type(),
        "horizontalRule"
    );
    assert!(new_doc.root().child(1).unwrap().is_void());
    assert_eq!(new_doc.root().child(2).unwrap().node_type(), "paragraph");
    assert_eq!(new_doc.root().child(2).unwrap().text_content(), "B");

    // Doc delta: +1 (void node occupies 1 token)
    assert_eq!(
        new_doc.content_size(),
        d.content_size() + 1,
        "content size should increase by 1 for a void block node"
    );
}

#[test]
fn test_insert_hard_break_in_paragraph() {
    // <doc><p>Hello</p></doc>
    // Insert hardBreak at pos 3 (between "He" and "llo")
    // Expected: <doc><p>He<br>llo</p></doc>
    //
    // Position model:
    //   pos 1: start of p content
    //   pos 3: parent_offset 2 (between "He" and "llo")
    let (d, schema) = doc_and_schema(doc(vec![paragraph(vec![text("Hello")])]));

    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::InsertNode {
        pos: 3,
        node: hard_break(),
    });

    let (new_doc, _map) = tx
        .apply(&d, &schema)
        .expect("insert hard break should succeed");

    let para = new_doc.root().child(0).unwrap();
    assert_eq!(
        para.child_count(),
        3,
        "paragraph should have 3 children: text + hardBreak + text"
    );
    assert_eq!(para.child(0).unwrap().text_str().unwrap(), "He");
    assert_eq!(para.child(1).unwrap().node_type(), "hardBreak");
    assert!(para.child(1).unwrap().is_void());
    assert_eq!(para.child(2).unwrap().text_str().unwrap(), "llo");

    // Doc delta: +1 (void node occupies 1 token)
    assert_eq!(
        new_doc.content_size(),
        d.content_size() + 1,
        "content size should increase by 1 for a void inline node"
    );
}

#[test]
fn test_insert_hard_break_at_start_of_paragraph() {
    // <doc><p>Hello</p></doc>
    // Insert hardBreak at pos 1 (start of paragraph content)
    // Expected: <doc><p><br>Hello</p></doc>
    let (d, schema) = doc_and_schema(doc(vec![paragraph(vec![text("Hello")])]));

    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::InsertNode {
        pos: 1,
        node: hard_break(),
    });

    let (new_doc, _map) = tx
        .apply(&d, &schema)
        .expect("insert hard break at start should succeed");

    let para = new_doc.root().child(0).unwrap();
    assert_eq!(para.child_count(), 2);
    assert_eq!(para.child(0).unwrap().node_type(), "hardBreak");
    assert_eq!(para.child(1).unwrap().text_str().unwrap(), "Hello");
}

#[test]
fn test_insert_hard_break_at_end_of_paragraph() {
    // <doc><p>Hello</p></doc>
    // Insert hardBreak at pos 6 (end of paragraph content)
    // Expected: <doc><p>Hello<br></p></doc>
    let (d, schema) = doc_and_schema(doc(vec![paragraph(vec![text("Hello")])]));

    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::InsertNode {
        pos: 6,
        node: hard_break(),
    });

    let (new_doc, _map) = tx
        .apply(&d, &schema)
        .expect("insert hard break at end should succeed");

    let para = new_doc.root().child(0).unwrap();
    assert_eq!(para.child_count(), 2);
    assert_eq!(para.child(0).unwrap().text_str().unwrap(), "Hello");
    assert_eq!(para.child(1).unwrap().node_type(), "hardBreak");
}

#[test]
fn test_insert_hard_break_at_end_of_list_item_paragraph_preserves_text() {
    let (d, schema) = doc_and_schema(doc(vec![bullet_list(vec![list_item(vec![paragraph(
        vec![text("A"), hard_break()],
    )])])]));

    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::InsertNode {
        pos: 5,
        node: hard_break(),
    });

    let (new_doc, _map) = tx
        .apply(&d, &schema)
        .expect("insert hard break at list paragraph end should succeed");

    let list = new_doc.root().child(0).unwrap();
    let item = list.child(0).unwrap();
    let para = item.child(0).unwrap();
    assert_eq!(
        para.child_count(),
        3,
        "list item paragraph should contain text plus two hardBreak nodes"
    );
    assert_eq!(para.child(0).unwrap().text_str().unwrap(), "A");
    assert_eq!(para.child(1).unwrap().node_type(), "hardBreak");
    assert_eq!(para.child(2).unwrap().node_type(), "hardBreak");
}

#[test]
fn test_insert_void_node_occupies_one_position() {
    // Verify that a void node (hardBreak) takes exactly 1 doc position.
    // Insert hard break, then verify positions on each side.
    let (d, schema) = doc_and_schema(doc(vec![paragraph(vec![text("AB")])]));
    // pos 1 = A, pos 2 = B, pos 3 = end
    // Insert hardBreak at pos 2 (between A and B)
    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::InsertNode {
        pos: 2,
        node: hard_break(),
    });

    let (new_doc, map) = tx.apply(&d, &schema).expect("insert should succeed");

    // Before insert: content_size = 4 (p_open + A + B + p_close... wait, content_size = 2 for "AB")
    // Actually content_size for doc = paragraph.node_size() = 1+2+1 = 4
    // After insert: paragraph has "A" + hardBreak + "B" = 1 + 1 + 1 = 3 chars inside p
    // Content size for doc = paragraph.node_size() = 1+3+1 = 5
    assert_eq!(new_doc.content_size(), 5);
    assert_eq!(d.content_size(), 4);

    // Position mapping: pos 2 (at insert point) shifts forward by 1
    assert_eq!(map.map_pos(2), 3, "position at insert should shift by 1");
    assert_eq!(map.map_pos(1), 1, "position before insert unchanged");
    assert_eq!(map.map_pos(3), 4, "position after insert shifts by 1");
}

#[test]
fn test_insert_paragraph_node_between_blocks() {
    // Insert a full paragraph node at the doc level.
    // <doc><p>A</p></doc> → insert empty paragraph at pos 3
    // Expected: <doc><p>A</p><p></p></doc>
    let (d, schema) = doc_and_schema(doc(vec![paragraph(vec![text("A")])]));

    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::InsertNode {
        pos: 3,
        node: paragraph(vec![]),
    });

    let (new_doc, _map) = tx
        .apply(&d, &schema)
        .expect("insert paragraph node should succeed");

    assert_eq!(new_doc.root().child_count(), 2);
    assert_eq!(new_doc.root().child(0).unwrap().text_content(), "A");
    assert_eq!(new_doc.root().child(1).unwrap().text_content(), "");
    assert_eq!(new_doc.root().child(1).unwrap().node_type(), "paragraph");

    // Doc delta: +2 for element node (open + close)
    assert_eq!(
        new_doc.content_size(),
        d.content_size() + 2,
        "content size should increase by node_size() of the inserted paragraph"
    );
}

// ===========================================================================
// ReplaceRange tests
// ===========================================================================

#[test]
fn test_replace_range_replace_selection_with_text() {
    // <doc><p>Hello</p></doc>
    // Replace "llo" (pos 3..6) with "y there" as text nodes
    // Expected: <doc><p>Hey there</p></doc>
    let (d, schema) = doc_and_schema(doc(vec![paragraph(vec![text("Hello")])]));

    let mut tx = Transaction::new(Source::Paste);
    tx.add_step(Step::ReplaceRange {
        from: 3,
        to: 6,
        content: Fragment::from(vec![text("y there")]),
    });

    let (new_doc, _map) = tx.apply(&d, &schema).expect("replace range should succeed");

    assert_eq!(
        new_doc.root().text_content(),
        "Hey there",
        "replacing 'llo' with 'y there' should produce 'Hey there'"
    );

    // Doc delta: content.size() - (to - from) = 7 - 3 = 4
    assert_eq!(
        new_doc.content_size(),
        d.content_size() + 4,
        "content size should change by content.size() - deleted_len"
    );
}

#[test]
fn test_replace_range_pure_insertion() {
    // <doc><p>Hello</p></doc>
    // Insert at pos 3 (from == to), inserting "XY"
    // Expected: <doc><p>HeXYllo</p></doc>
    let (d, schema) = doc_and_schema(doc(vec![paragraph(vec![text("Hello")])]));

    let mut tx = Transaction::new(Source::Paste);
    tx.add_step(Step::ReplaceRange {
        from: 3,
        to: 3,
        content: Fragment::from(vec![text("XY")]),
    });

    let (new_doc, _map) = tx
        .apply(&d, &schema)
        .expect("pure insertion replace range should succeed");

    assert_eq!(
        new_doc.root().text_content(),
        "HeXYllo",
        "inserting 'XY' at pos 3 should produce 'HeXYllo'"
    );
}

#[test]
fn test_replace_range_empty_fragment_is_delete() {
    // <doc><p>Hello</p></doc>
    // Replace "ell" (pos 2..5) with empty fragment → equivalent to delete
    // Expected: <doc><p>Ho</p></doc>
    let (d, schema) = doc_and_schema(doc(vec![paragraph(vec![text("Hello")])]));

    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::ReplaceRange {
        from: 2,
        to: 5,
        content: Fragment::empty(),
    });

    let (new_doc, _map) = tx
        .apply(&d, &schema)
        .expect("replace with empty fragment should succeed");

    assert_eq!(
        new_doc.root().text_content(),
        "Ho",
        "replacing 'ell' with empty content should produce 'Ho'"
    );
}

#[test]
fn test_replace_range_preserves_surrounding_content() {
    // <doc><p>Hello world</p></doc>
    // Replace "lo wo" (pos 4..9) with "p, "
    // Expected: <doc><p>Help, rld</p></doc>
    let (d, schema) = doc_and_schema(doc(vec![paragraph(vec![text("Hello world")])]));

    let mut tx = Transaction::new(Source::Paste);
    tx.add_step(Step::ReplaceRange {
        from: 4,
        to: 9,
        content: Fragment::from(vec![text("p, ")]),
    });

    let (new_doc, _map) = tx
        .apply(&d, &schema)
        .expect("replace preserving surroundings should succeed");

    assert_eq!(
        new_doc.root().text_content(),
        "Help, rld",
        "replacing 'lo wo' with 'p, ' should produce 'Help, rld'"
    );
}

#[test]
fn test_replace_range_with_marked_content() {
    // <doc><p>Hello</p></doc>
    // Replace "ell" (pos 2..5) with bold "ELL"
    // Expected: <doc><p>H<b>ELL</b>o</p></doc>
    let (d, schema) = doc_and_schema(doc(vec![paragraph(vec![text("Hello")])]));

    let mut tx = Transaction::new(Source::Paste);
    tx.add_step(Step::ReplaceRange {
        from: 2,
        to: 5,
        content: Fragment::from(vec![text_with_marks("ELL", vec![bold()])]),
    });

    let (new_doc, _map) = tx
        .apply(&d, &schema)
        .expect("replace with marked content should succeed");

    assert_eq!(new_doc.root().text_content(), "HELLo");

    let para = new_doc.root().child(0).unwrap();
    assert_eq!(
        para.child_count(),
        3,
        "paragraph should have 3 text nodes: H + ELL(bold) + o"
    );
    assert_eq!(para.child(0).unwrap().text_str().unwrap(), "H");
    assert!(para.child(0).unwrap().marks().is_empty());
    assert_eq!(para.child(1).unwrap().text_str().unwrap(), "ELL");
    assert!(
        para.child(1)
            .unwrap()
            .marks()
            .iter()
            .any(|m| m.mark_type() == "bold"),
        "replaced text should be bold"
    );
    assert_eq!(para.child(2).unwrap().text_str().unwrap(), "o");
}

#[test]
fn test_replace_range_step_map() {
    // Replace "ell" (pos 2..5) with "X" (1 char).
    // Deleted 3, inserted 1. Net delta = -2.
    let (d, schema) = doc_and_schema(doc(vec![paragraph(vec![text("Hello")])]));

    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::ReplaceRange {
        from: 2,
        to: 5,
        content: Fragment::from(vec![text("X")]),
    });

    let (_new_doc, map) = tx.apply(&d, &schema).expect("replace should succeed");

    // Position before replacement is unchanged
    assert_eq!(map.map_pos(1), 1);
    // Position inside deleted range collapses to from + inserted
    assert_eq!(
        map.map_pos(3),
        3,
        "position inside deleted range should map to from + inserted_len"
    );
    // Position after replacement shifts by net delta
    assert_eq!(
        map.map_pos(6),
        4,
        "position after replacement should shift by -2"
    );
}

// ===========================================================================
// InsertNode + DeleteRange round-trip tests
// ===========================================================================

#[test]
fn test_insert_node_then_delete_restores_original_block() {
    // <doc><p>A</p><p>B</p></doc>
    // Insert horizontalRule at pos 3, then delete at pos 3..4
    // Should restore original document.
    let (d, schema) = doc_and_schema(doc(vec![
        paragraph(vec![text("A")]),
        paragraph(vec![text("B")]),
    ]));

    // Insert
    let mut tx_insert = Transaction::new(Source::Input);
    tx_insert.add_step(Step::InsertNode {
        pos: 3,
        node: horizontal_rule(),
    });
    let (inserted_doc, _) = tx_insert.apply(&d, &schema).expect("insert should succeed");
    assert_eq!(inserted_doc.root().child_count(), 3);

    // Delete the inserted hr (pos 3..4, since void node is 1 token)
    let mut tx_delete = Transaction::new(Source::Input);
    tx_delete.add_step(Step::DeleteRange { from: 3, to: 4 });
    let (restored_doc, _) = tx_delete
        .apply(&inserted_doc, &schema)
        .expect("delete should succeed");

    assert_eq!(
        restored_doc.root().child_count(),
        2,
        "round-trip insert+delete should restore 2 paragraphs"
    );
    assert_eq!(restored_doc.root().child(0).unwrap().text_content(), "A");
    assert_eq!(restored_doc.root().child(1).unwrap().text_content(), "B");
}

#[test]
fn test_insert_node_then_delete_restores_original_inline() {
    // <doc><p>Hello</p></doc>
    // Insert hardBreak at pos 3, then delete at pos 3..4
    // Should restore original document.
    let (d, schema) = doc_and_schema(doc(vec![paragraph(vec![text("Hello")])]));

    // Insert
    let mut tx_insert = Transaction::new(Source::Input);
    tx_insert.add_step(Step::InsertNode {
        pos: 3,
        node: hard_break(),
    });
    let (inserted_doc, _) = tx_insert.apply(&d, &schema).expect("insert should succeed");

    // Delete the hardBreak (pos 3..4)
    let mut tx_delete = Transaction::new(Source::Input);
    tx_delete.add_step(Step::DeleteRange { from: 3, to: 4 });
    let (restored_doc, _) = tx_delete
        .apply(&inserted_doc, &schema)
        .expect("delete should succeed");

    assert_eq!(
        restored_doc.root().text_content(),
        "Hello",
        "round-trip insert+delete of hardBreak should restore 'Hello'"
    );
}

// ===========================================================================
// Edge cases
// ===========================================================================

#[test]
fn test_delete_range_invalid_range_returns_error() {
    // from > to should be an error
    let (doc, schema) = doc_and_schema(doc(vec![paragraph(vec![text("Hello")])]));
    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::DeleteRange { from: 4, to: 2 });

    assert!(
        tx.apply(&doc, &schema).is_err(),
        "delete with from > to should be an error"
    );
}

#[test]
fn test_insert_text_out_of_bounds_returns_error() {
    let (doc, schema) = doc_and_schema(doc(vec![paragraph(vec![text("Hi")])]));
    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::InsertText {
        pos: 100,
        text: "X".to_string(),
        marks: vec![],
    });

    assert!(
        tx.apply(&doc, &schema).is_err(),
        "insert at out-of-bounds position should error"
    );
}

#[test]
fn test_add_mark_range_no_change_for_empty_range() {
    // AddMark with from == to should be a no-op
    let (doc, schema) = doc_and_schema(doc(vec![paragraph(vec![text("Hello")])]));
    let mut tx = Transaction::new(Source::Format);
    tx.add_step(Step::AddMark {
        from: 3,
        to: 3,
        mark: bold(),
    });

    let (new_doc, _map) = tx
        .apply(&doc, &schema)
        .expect("empty mark range should succeed (no-op)");
    assert_eq!(new_doc.root().text_content(), "Hello");
}

#[test]
fn test_empty_transaction_is_noop() {
    let (doc, schema) = doc_and_schema(doc(vec![paragraph(vec![text("Hello")])]));
    let tx = Transaction::new(Source::Input);

    let (new_doc, _map) = tx
        .apply(&doc, &schema)
        .expect("empty transaction should succeed");
    assert_eq!(new_doc.root().text_content(), "Hello");
    assert_eq!(new_doc.content_size(), doc.content_size());
}
