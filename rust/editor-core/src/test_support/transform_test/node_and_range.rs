// InsertNode / ReplaceRange now implemented — verify they work

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
fn replace_range_preserves_preceding_split_text_siblings() {
    let (document, schema) = doc_and_schema(doc(vec![paragraph(vec![
        text("a"),
        text_with_marks("bcd", vec![bold()]),
        text("ef"),
    ])]));
    let mut transaction = Transaction::new(Source::Input);
    transaction.add_step(Step::ReplaceRange {
        from: 3,
        to: 4,
        content: Fragment::from(vec![text("a")]),
    });

    let (actual, _) = transaction.apply(&document, &schema).unwrap();

    assert_eq!(actual.root().text_content(), "abadef");
    assert_eq!(
        actual.root().child(0).unwrap().child(0).unwrap().text_str(),
        Some("a")
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

// InsertNode + DeleteRange round-trip tests

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
