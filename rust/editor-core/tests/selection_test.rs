use std::collections::HashMap;

use editor_core::model::{Document, Fragment, Node};
use editor_core::position::PositionMap;
use editor_core::schema::presets::tiptap_schema;
use editor_core::selection::Selection;
use editor_core::transform::{Source, Step, StepMap, Transaction};

// ---------------------------------------------------------------------------
// Helper builders (matching conventions from other test files)
// ---------------------------------------------------------------------------

fn text(s: &str) -> Node {
    Node::text(s.to_string(), vec![])
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

fn horizontal_rule() -> Node {
    Node::void("horizontalRule".to_string(), HashMap::new())
}

// ===========================================================================
// TextSelection: creation and accessors
// ===========================================================================

#[test]
fn test_text_selection_creation() {
    let document = Document::new(doc(vec![paragraph(vec![text("Hello")])]));

    let sel = Selection::text(2, 5);
    assert_eq!(
        sel.anchor(&document),
        2,
        "text selection anchor should be 2"
    );
    assert_eq!(sel.head(&document), 5, "text selection head should be 5");
}

#[test]
fn test_text_selection_from_to_forward() {
    // anchor < head → from == anchor, to == head
    let document = Document::new(doc(vec![paragraph(vec![text("Hello")])]));

    let sel = Selection::text(1, 4);
    assert_eq!(
        sel.from(&document),
        1,
        "from() should be min(anchor=1, head=4) = 1"
    );
    assert_eq!(
        sel.to(&document),
        4,
        "to() should be max(anchor=1, head=4) = 4"
    );
}

#[test]
fn test_text_selection_from_to_backward() {
    // anchor > head (backward selection) → from == head, to == anchor
    let document = Document::new(doc(vec![paragraph(vec![text("Hello")])]));

    let sel = Selection::text(5, 2);
    assert_eq!(
        sel.from(&document),
        2,
        "from() should be min(anchor=5, head=2) = 2"
    );
    assert_eq!(
        sel.to(&document),
        5,
        "to() should be max(anchor=5, head=2) = 5"
    );
}

#[test]
fn test_text_selection_is_not_empty_when_range() {
    let document = Document::new(doc(vec![paragraph(vec![text("Hello")])]));

    let sel = Selection::text(1, 4);
    assert!(
        !sel.is_empty(&document),
        "text selection with anchor != head should not be empty"
    );
}

// ===========================================================================
// Cursor (collapsed text selection)
// ===========================================================================

#[test]
fn test_cursor_creation() {
    let document = Document::new(doc(vec![paragraph(vec![text("Hello")])]));

    let sel = Selection::cursor(3);
    assert_eq!(sel.anchor(&document), 3, "cursor anchor should be 3");
    assert_eq!(sel.head(&document), 3, "cursor head should be 3");
}

#[test]
fn test_cursor_is_empty() {
    let document = Document::new(doc(vec![paragraph(vec![text("Hello")])]));

    let sel = Selection::cursor(3);
    assert!(
        sel.is_empty(&document),
        "cursor (anchor == head) should be empty"
    );
}

#[test]
fn test_cursor_from_to_equal() {
    let document = Document::new(doc(vec![paragraph(vec![text("Hello")])]));

    let sel = Selection::cursor(3);
    assert_eq!(
        sel.from(&document),
        3,
        "cursor from() should equal the cursor position"
    );
    assert_eq!(
        sel.to(&document),
        3,
        "cursor to() should equal the cursor position"
    );
}

// ===========================================================================
// NodeSelection
// ===========================================================================

#[test]
fn test_node_selection_creation() {
    // <doc><p>Hi</p><hr/><p>Bye</p></doc>
    //   doc.open | p.open H i p.close | hr | p.open B y e p.close | doc.close
    //   pos:       0       1 2 3         4    5      6 7 8 9
    // The horizontalRule is at doc position 4.
    let document = Document::new(doc(vec![
        paragraph(vec![text("Hi")]),
        horizontal_rule(),
        paragraph(vec![text("Bye")]),
    ]));

    let sel = Selection::node(4);
    assert_eq!(
        sel.anchor(&document),
        4,
        "node selection anchor should be the void node position"
    );
    assert_eq!(
        sel.head(&document),
        4,
        "node selection head should be the void node position"
    );
}

#[test]
fn test_node_selection_is_empty() {
    // Node selection has anchor == head (both are the node pos), so is_empty
    // returns true in the current model. This is intentional: the "extent" of
    // the selection is the node itself, but anchor/head coincide.
    let document = Document::new(doc(vec![
        paragraph(vec![text("Hi")]),
        horizontal_rule(),
        paragraph(vec![text("Bye")]),
    ]));

    let sel = Selection::node(4);
    assert!(
        sel.is_empty(&document),
        "node selection anchor == head, so is_empty returns true"
    );
}

#[test]
fn test_node_selection_from_to() {
    let document = Document::new(doc(vec![
        paragraph(vec![text("Hi")]),
        horizontal_rule(),
        paragraph(vec![text("Bye")]),
    ]));

    let sel = Selection::node(4);
    assert_eq!(sel.from(&document), 4, "node selection from() == pos");
    assert_eq!(sel.to(&document), 4, "node selection to() == pos");
}

// ===========================================================================
// AllSelection
// ===========================================================================

#[test]
fn test_all_selection_resolves_to_full_content() {
    // <doc><p>Hello</p></doc>
    // content_size = 7 (p.open + H e l l o + p.close = 7)
    let document = Document::new(doc(vec![paragraph(vec![text("Hello")])]));

    let sel = Selection::all();
    assert_eq!(sel.anchor(&document), 0, "AllSelection anchor should be 0");
    assert_eq!(
        sel.head(&document),
        document.content_size(),
        "AllSelection head should be doc.content_size()"
    );
    assert_eq!(sel.from(&document), 0, "AllSelection from() should be 0");
    assert_eq!(
        sel.to(&document),
        document.content_size(),
        "AllSelection to() should be doc.content_size()"
    );
}

#[test]
fn test_all_selection_is_not_empty_on_nonempty_doc() {
    let document = Document::new(doc(vec![paragraph(vec![text("Hello")])]));

    let sel = Selection::all();
    assert!(
        !sel.is_empty(&document),
        "AllSelection on non-empty doc should not be empty"
    );
}

#[test]
fn test_all_selection_is_empty_on_empty_doc() {
    // <doc></doc> — content_size = 0
    let document = Document::new(doc(vec![]));

    let sel = Selection::all();
    assert!(
        sel.is_empty(&document),
        "AllSelection on empty doc (content_size=0) should be empty"
    );
}

#[test]
fn test_all_selection_multi_block() {
    // <doc><p>Hi</p><p>Bye</p></doc>
    // content_size = p.open + H i + p.close + p.open + B y e + p.close = 9
    let document = Document::new(doc(vec![
        paragraph(vec![text("Hi")]),
        paragraph(vec![text("Bye")]),
    ]));

    let sel = Selection::all();
    assert_eq!(sel.anchor(&document), 0, "AllSelection anchor is 0");
    assert_eq!(
        sel.head(&document),
        9,
        "AllSelection head should be 9 (content_size of two paragraphs)"
    );
}

// ===========================================================================
// Normalization: structural positions snap to cursorable
// ===========================================================================

#[test]
fn test_normalize_cursor_on_structural_position() {
    // <doc><p>Hello</p></doc>
    // Position 0 is before the paragraph open tag — structural, not cursorable.
    // It should snap to 1 (start of paragraph content).
    let document = Document::new(doc(vec![paragraph(vec![text("Hello")])]));
    let pos_map = PositionMap::build(&document);

    let sel = Selection::cursor(0).normalized(&document, &pos_map);
    match &sel {
        Selection::Text { anchor, head } => {
            assert_eq!(
                *anchor, 1,
                "cursor at structural pos 0 should normalize to 1 (start of paragraph content)"
            );
            assert_eq!(*head, 1, "cursor at structural pos 0 should normalize to 1");
        }
        other => panic!(
            "expected Text selection after normalization, got {:?}",
            other
        ),
    }
}

#[test]
fn test_normalize_text_selection_on_structural_positions() {
    // <doc><p>Hello</p><p>World</p></doc>
    // Positions:
    //   0: before first p.open (structural)
    //   1: start of first p content
    //   6: end of first p content
    //   7: between p.close and second p.open (structural)
    //   8: start of second p content
    //   13: end of second p content
    //
    // Selection from 0 to 7 should normalize anchor=1, head to nearest cursorable.
    let document = Document::new(doc(vec![
        paragraph(vec![text("Hello")]),
        paragraph(vec![text("World")]),
    ]));
    let pos_map = PositionMap::build(&document);

    let sel = Selection::text(0, 7).normalized(&document, &pos_map);
    match &sel {
        Selection::Text { anchor, head } => {
            assert_eq!(*anchor, 1, "anchor at structural pos 0 should snap to 1");
            // Position 7 is between two paragraphs. It's equidistant from
            // pos 6 (end of first p) and pos 8 (start of second p).
            // normalize_cursor_pos snaps it to the closer (or equal-distance) block.
            assert!(
                *head == 6 || *head == 8,
                "head at structural pos 7 should snap to either 6 or 8, got {}",
                head
            );
        }
        other => panic!(
            "expected Text selection after normalization, got {:?}",
            other
        ),
    }
}

#[test]
fn test_normalize_already_cursorable_position_unchanged() {
    // Position 3 is inside the first paragraph content — already cursorable.
    let document = Document::new(doc(vec![paragraph(vec![text("Hello")])]));
    let pos_map = PositionMap::build(&document);

    let sel = Selection::cursor(3).normalized(&document, &pos_map);
    match &sel {
        Selection::Text { anchor, head } => {
            assert_eq!(
                *anchor, 3,
                "cursorable position 3 should remain unchanged after normalization"
            );
            assert_eq!(*head, 3, "cursorable position 3 should remain 3");
        }
        other => panic!(
            "expected Text selection after normalization, got {:?}",
            other
        ),
    }
}

#[test]
fn test_normalize_all_selection_stays_all() {
    let document = Document::new(doc(vec![paragraph(vec![text("Hello")])]));
    let pos_map = PositionMap::build(&document);

    let sel = Selection::all().normalized(&document, &pos_map);
    assert_eq!(
        sel,
        Selection::All,
        "All selection should stay All after normalization"
    );
}

#[test]
fn test_normalize_position_past_end_snaps_to_last_content() {
    // <doc><p>Hi</p></doc> — content_size = 4
    // Position 99 is way past the end; should snap to the end of the last block.
    let document = Document::new(doc(vec![paragraph(vec![text("Hi")])]));
    let pos_map = PositionMap::build(&document);

    let sel = Selection::cursor(99).normalized(&document, &pos_map);
    match &sel {
        Selection::Text { anchor, head } => {
            assert_eq!(
                *anchor, 3,
                "position 99 should snap to end of last block content (3)"
            );
            assert_eq!(*head, 3, "position 99 should snap to 3");
        }
        other => panic!(
            "expected Text selection after normalization, got {:?}",
            other
        ),
    }
}

// ===========================================================================
// Map through StepMap: insertion
// ===========================================================================

#[test]
fn test_map_cursor_after_insert_text() {
    // Document: <doc><p>Hello</p></doc>
    // Insert "XX" at pos 3 (between "He" and "llo").
    // Cursor at pos 5 (before 'o') should shift to pos 7 (5 + 2 inserted).
    let document = Document::new(doc(vec![paragraph(vec![text("Hello")])]));
    let schema = tiptap_schema();

    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::InsertText {
        pos: 3,
        text: "XX".to_string(),
        marks: vec![],
    });

    let (_new_doc, step_map) = tx.apply(&document, &schema).expect("insert should succeed");

    let sel = Selection::cursor(5);
    let mapped = sel.map(&step_map);
    match &mapped {
        Selection::Text { anchor, head } => {
            assert_eq!(
                *anchor, 7,
                "cursor at 5, after inserting 2 chars at 3, should map to 7"
            );
            assert_eq!(*head, 7, "head should also be 7");
        }
        other => panic!("expected Text selection after map, got {:?}", other),
    }
}

#[test]
fn test_map_cursor_at_insertion_point() {
    // Cursor exactly at the insertion point (pos 3).
    // StepMap convention: pos == range_pos with insertion → push forward.
    let document = Document::new(doc(vec![paragraph(vec![text("Hello")])]));
    let schema = tiptap_schema();

    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::InsertText {
        pos: 3,
        text: "XX".to_string(),
        marks: vec![],
    });

    let (_new_doc, step_map) = tx.apply(&document, &schema).expect("insert should succeed");

    let sel = Selection::cursor(3);
    let mapped = sel.map(&step_map);
    match &mapped {
        Selection::Text { anchor, head } => {
            assert_eq!(
                *anchor, 5,
                "cursor at insertion point 3, after inserting 2 chars, should push to 5"
            );
            assert_eq!(*head, 5, "head should also be 5");
        }
        other => panic!("expected Text selection after map, got {:?}", other),
    }
}

#[test]
fn test_map_cursor_before_insertion_unaffected() {
    // Cursor at pos 1 (before the insertion point 3) should remain at 1.
    let document = Document::new(doc(vec![paragraph(vec![text("Hello")])]));
    let schema = tiptap_schema();

    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::InsertText {
        pos: 3,
        text: "XX".to_string(),
        marks: vec![],
    });

    let (_new_doc, step_map) = tx.apply(&document, &schema).expect("insert should succeed");

    let sel = Selection::cursor(1);
    let mapped = sel.map(&step_map);
    match &mapped {
        Selection::Text { anchor, head } => {
            assert_eq!(
                *anchor, 1,
                "cursor before insertion point should remain at 1"
            );
            assert_eq!(*head, 1, "head should remain at 1");
        }
        other => panic!("expected Text selection after map, got {:?}", other),
    }
}

#[test]
fn test_map_text_range_spanning_insertion() {
    // Selection from 2 to 5 (within "Hello").
    // Insert "XX" at pos 3.
    // anchor=2 is before insertion → stays 2.
    // head=5 is after insertion → shifts to 7.
    let document = Document::new(doc(vec![paragraph(vec![text("Hello")])]));
    let schema = tiptap_schema();

    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::InsertText {
        pos: 3,
        text: "XX".to_string(),
        marks: vec![],
    });

    let (_new_doc, step_map) = tx.apply(&document, &schema).expect("insert should succeed");

    let sel = Selection::text(2, 5);
    let mapped = sel.map(&step_map);
    match &mapped {
        Selection::Text { anchor, head } => {
            assert_eq!(*anchor, 2, "anchor before insertion should stay at 2");
            assert_eq!(*head, 7, "head after insertion (5 + 2) should shift to 7");
        }
        other => panic!("expected Text selection after map, got {:?}", other),
    }
}

// ===========================================================================
// Map through StepMap: deletion
// ===========================================================================

#[test]
fn test_map_cursor_after_delete() {
    // Document: <doc><p>Hello</p></doc>
    // Delete range [2, 4) — removes "ll" (pos 2..4 in doc = "el" wait let me
    // recalculate).
    //
    // Doc positions: 0=before p, 1=H, 2=e, 3=l, 4=l, 5=o, 6=after p
    // DeleteRange from=2, to=4 removes chars at doc pos 2 and 3 ("el").
    //
    // Cursor at pos 5 (on "o") should shift to 3 (5 - 2 deleted).
    let document = Document::new(doc(vec![paragraph(vec![text("Hello")])]));
    let schema = tiptap_schema();

    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::DeleteRange { from: 2, to: 4 });

    let (new_doc, step_map) = tx.apply(&document, &schema).expect("delete should succeed");
    assert_eq!(
        new_doc.root().text_content(),
        "Hlo",
        "deleting pos 2..4 from 'Hello' should produce 'Hlo'"
    );

    let sel = Selection::cursor(5);
    let mapped = sel.map(&step_map);
    match &mapped {
        Selection::Text { anchor, head } => {
            assert_eq!(
                *anchor, 3,
                "cursor at 5, after deleting 2 chars at [2,4), should map to 3"
            );
            assert_eq!(*head, 3, "head should also be 3");
        }
        other => panic!("expected Text selection after map, got {:?}", other),
    }
}

#[test]
fn test_map_cursor_inside_deleted_range_collapses() {
    // Cursor at pos 3 (inside the deleted range [2, 4)).
    // Should collapse to the deletion point + inserted (= 2 + 0 = 2).
    let document = Document::new(doc(vec![paragraph(vec![text("Hello")])]));
    let schema = tiptap_schema();

    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::DeleteRange { from: 2, to: 4 });

    let (_new_doc, step_map) = tx.apply(&document, &schema).expect("delete should succeed");

    let sel = Selection::cursor(3);
    let mapped = sel.map(&step_map);
    match &mapped {
        Selection::Text { anchor, head } => {
            assert_eq!(
                *anchor, 2,
                "cursor inside deleted range [2,4) should collapse to 2"
            );
            assert_eq!(*head, 2, "head should also collapse to 2");
        }
        other => panic!("expected Text selection after map, got {:?}", other),
    }
}

#[test]
fn test_map_selection_spanning_deleted_range() {
    // Selection anchor=1, head=5, delete [2,4).
    // anchor=1 is before deletion → stays 1.
    // head=5 is after deletion → 5 - 2 = 3.
    let document = Document::new(doc(vec![paragraph(vec![text("Hello")])]));
    let schema = tiptap_schema();

    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::DeleteRange { from: 2, to: 4 });

    let (_new_doc, step_map) = tx.apply(&document, &schema).expect("delete should succeed");

    let sel = Selection::text(1, 5);
    let mapped = sel.map(&step_map);
    match &mapped {
        Selection::Text { anchor, head } => {
            assert_eq!(*anchor, 1, "anchor before deletion should remain at 1");
            assert_eq!(*head, 3, "head after deletion (5 - 2) should shift to 3");
        }
        other => panic!("expected Text selection after map, got {:?}", other),
    }
}

// ===========================================================================
// Map through StepMap: AllSelection is unaffected
// ===========================================================================

#[test]
fn test_map_all_selection_stays_all() {
    let step_map = StepMap::from_insert(3, 5);

    let sel = Selection::all();
    let mapped = sel.map(&step_map);
    assert_eq!(
        mapped,
        Selection::All,
        "All selection should remain All after mapping through any StepMap"
    );
}

// ===========================================================================
// Map through StepMap: NodeSelection
// ===========================================================================

#[test]
fn test_map_node_selection_after_insert_before() {
    // <doc><p>Hi</p><hr/><p>Bye</p></doc>
    // horizontalRule is at pos 4.
    // Insert 3 chars at pos 1 (inside first paragraph).
    // Node selection pos 4 should shift to 7 (4 + 3).
    let step_map = StepMap::from_insert(1, 3);

    let sel = Selection::node(4);
    let mapped = sel.map(&step_map);
    match &mapped {
        Selection::Node { pos } => {
            assert_eq!(
                *pos, 7,
                "node selection at 4, after inserting 3 at 1, should map to 7"
            );
        }
        other => panic!("expected Node selection after map, got {:?}", other),
    }
}

// ===========================================================================
// Selection equality and clone
// ===========================================================================

#[test]
fn test_selection_equality() {
    assert_eq!(
        Selection::cursor(5),
        Selection::text(5, 5),
        "cursor(5) should equal text(5, 5)"
    );
    assert_ne!(
        Selection::text(1, 5),
        Selection::text(5, 1),
        "text(1,5) should not equal text(5,1) — direction matters"
    );
    assert_eq!(Selection::all(), Selection::all(), "All == All");
    assert_ne!(
        Selection::node(3),
        Selection::cursor(3),
        "Node(3) != Text{{3,3}}"
    );
}

#[test]
fn test_selection_clone() {
    let sel = Selection::text(2, 8);
    let cloned = sel.clone();
    assert_eq!(sel, cloned, "cloned selection should equal the original");
}
