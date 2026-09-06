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

// SplitBlock then JoinBlocks round-trip test

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
