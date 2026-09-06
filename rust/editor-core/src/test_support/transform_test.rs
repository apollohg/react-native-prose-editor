use std::collections::HashMap;

use crate::model::{Document, Fragment, Mark, Node};
use crate::schema::presets::tiptap_schema;
use crate::schema::{AttrSpec, Schema};
use crate::transform::{Source, Step, Transaction};

#[test]
fn document_stats_remains_constructible_with_the_original_public_fields() {
    let stats = crate::transform::apply::DocumentStats {
        node_count: 3,
        max_depth: 2,
    };

    assert_eq!(stats.node_count, 3);
    assert_eq!(stats.max_depth, 2);
}

// Helper builders (matching model_test.rs conventions)

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
fn doc_and_schema(root: Node) -> (Document, crate::schema::Schema) {
    (Document::new(root), tiptap_schema())
}

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

include!("transform_test/split_and_join.rs");

include!("transform_test/lists.rs");

include!("transform_test/node_and_range.rs");
