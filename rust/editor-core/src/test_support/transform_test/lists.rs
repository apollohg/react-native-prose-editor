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
        item_attrs: HashMap::new(),
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
        item_attrs: HashMap::new(),
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
        item_attrs: HashMap::new(),
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
fn test_wrap_in_list_applies_item_attrs_to_created_items() {
    // <doc><p>Todo</p></doc>
    // Wrap with item_attrs carrying checked=true. Every created list item
    // must receive those attrs — this is what lets the inverse of
    // UnwrapFromList restore a taskItem's `checked` state on undo.
    let (d, base_schema) = doc_and_schema(doc(vec![paragraph(vec![text("Todo")])]));
    let mut nodes = base_schema.all_nodes().cloned().collect::<Vec<_>>();
    nodes
        .iter_mut()
        .find(|node| node.name == "listItem")
        .unwrap()
        .attrs
        .insert(
            "checked".to_string(),
            AttrSpec {
                default: Some(serde_json::Value::Bool(false)),
                has_default: true,
                ..AttrSpec::default()
            },
        );
    let schema = Schema::new(nodes, base_schema.all_marks().cloned().collect());
    let mut item_attrs = HashMap::new();
    item_attrs.insert("checked".to_string(), serde_json::Value::Bool(true));
    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::WrapInList {
        from: 0,
        to: 6,
        list_type: "bulletList".to_string(),
        item_type: "listItem".to_string(),
        attrs: HashMap::new(),
        item_attrs,
    });

    let (new_doc, _map) = tx
        .apply(&d, &schema)
        .expect("wrap with item attrs should succeed");

    let list = new_doc.root().child(0).unwrap();
    assert_eq!(list.node_type(), "bulletList");
    let li = list.child(0).unwrap();
    assert_eq!(li.node_type(), "listItem");
    assert_eq!(
        li.attrs().get("checked"),
        Some(&serde_json::Value::Bool(true)),
        "created list item should carry the step's item_attrs, got: {:?}",
        li.attrs()
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
        item_attrs: HashMap::new(),
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
        item_attrs: HashMap::new(),
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
        item_attrs: HashMap::new(),
    });

    let result = tx.apply(&d, &schema);
    assert!(
        result.is_err(),
        "using a non-list node type for list_type should error"
    );
}

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

    let (new_doc, _map) = tx
        .apply(&d, &schema)
        .expect("first-item indent should be a no-op");
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
fn test_outdent_nested_prosemirror_list_item_lifts_after_parent_item() {
    let root = doc(vec![Node::element(
        "bullet_list".to_string(),
        HashMap::new(),
        Fragment::from(vec![
            Node::element(
                "list_item".to_string(),
                HashMap::new(),
                Fragment::from(vec![
                    paragraph(vec![text("A")]),
                    Node::element(
                        "bullet_list".to_string(),
                        HashMap::new(),
                        Fragment::from(vec![Node::element(
                            "list_item".to_string(),
                            HashMap::new(),
                            Fragment::from(vec![paragraph(vec![text("B")])]),
                        )]),
                    ),
                ]),
            ),
            Node::element(
                "list_item".to_string(),
                HashMap::new(),
                Fragment::from(vec![paragraph(vec![text("C")])]),
            ),
        ]),
    )]);
    let document = Document::new(root);
    let schema = crate::schema::presets::prosemirror_schema();
    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::OutdentListItem { pos: 8 });

    let (new_doc, _) = tx
        .apply(&document, &schema)
        .expect("outdent should succeed");
    let list = new_doc.root().child(0).unwrap();

    assert_eq!(list.child_count(), 3);
    assert_eq!(list.child(1).unwrap().text_content(), "B");
}

#[test]
fn test_outdent_top_level_list_item_is_noop() {
    let (d, schema) = doc_and_schema(doc(vec![bullet_list(vec![
        list_item(vec![paragraph(vec![text("A")])]),
        list_item(vec![paragraph(vec![text("B")])]),
    ])]));

    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::OutdentListItem { pos: 8 });

    let (new_doc, _map) = tx
        .apply(&d, &schema)
        .expect("top-level outdent should be a no-op");
    assert_eq!(new_doc.root().text_content(), d.root().text_content());
    assert_eq!(new_doc.root().child_count(), d.root().child_count());
    assert_eq!(new_doc.root().child(0).unwrap().child_count(), 2);
}

// WrapInList + UnwrapFromList round-trip tests

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
        item_attrs: HashMap::new(),
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
        item_attrs: HashMap::new(),
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

// WrapInList / UnwrapFromList StepMap tests

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
        item_attrs: HashMap::new(),
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
