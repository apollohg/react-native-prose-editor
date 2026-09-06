#[test]
fn test_delta_tree_empty() {
    let dt = crate::position::delta_tree::DeltaTree::empty();
    assert!(dt.is_empty());
    assert_eq!(dt.len(), 0);
    assert_eq!(dt.accumulated_delta(0), (0, 0));
    assert_eq!(dt.accumulated_delta(100), (0, 0));
}

#[test]
fn test_delta_tree_single_insert() {
    let mut dt = crate::position::delta_tree::DeltaTree::empty();
    dt.insert(2, 5, 3);

    assert_eq!(
        dt.accumulated_delta(0),
        (0, 0),
        "block 0 is before the delta"
    );
    assert_eq!(
        dt.accumulated_delta(1),
        (0, 0),
        "block 1 is before the delta"
    );
    assert_eq!(dt.accumulated_delta(2), (5, 3), "block 2 gets the delta");
    assert_eq!(
        dt.accumulated_delta(5),
        (5, 3),
        "block 5 also gets the delta (it's after)"
    );
}

#[test]
fn test_delta_tree_accumulation() {
    let mut dt = crate::position::delta_tree::DeltaTree::empty();
    dt.insert(1, 3, 2);
    dt.insert(3, 5, 4);

    assert_eq!(dt.accumulated_delta(0), (0, 0));
    assert_eq!(dt.accumulated_delta(1), (3, 2), "block 1 gets first delta");
    assert_eq!(
        dt.accumulated_delta(2),
        (3, 2),
        "block 2 inherits from block 1"
    );
    assert_eq!(
        dt.accumulated_delta(3),
        (8, 6),
        "block 3 accumulates both deltas: 3+5=8, 2+4=6"
    );
    assert_eq!(
        dt.accumulated_delta(10),
        (8, 6),
        "block 10 accumulates both"
    );
}

#[test]
fn test_delta_tree_same_index_merge() {
    let mut dt = crate::position::delta_tree::DeltaTree::empty();
    dt.insert(2, 3, 1);
    dt.insert(2, 4, 2);

    assert_eq!(dt.len(), 1, "should merge into one entry");
    assert_eq!(
        dt.accumulated_delta(2),
        (7, 3),
        "merged: 3+4=7 doc, 1+2=3 scalar"
    );
}

#[test]
fn test_delta_tree_clear() {
    let mut dt = crate::position::delta_tree::DeltaTree::empty();
    dt.insert(0, 1, 1);
    dt.insert(2, 3, 3);
    assert!(!dt.is_empty());

    dt.clear();
    assert!(dt.is_empty());
    assert_eq!(dt.accumulated_delta(5), (0, 0));
}

#[test]
fn test_incremental_update_insert_text_in_first_block() {
    // Start: <doc><p>Hello</p><p>World</p></doc>
    // Edit: insert "XX" at pos 2 (between H and ello) -> "HXXello"
    //
    // After: <doc><p>HXXello</p><p>World</p></doc>
    let doc_node = doc(vec![
        paragraph(vec![text("Hello")]),
        paragraph(vec![text("World")]),
    ]);
    let document = Document::new(doc_node);
    let schema = tiptap_schema();
    let mut map = PositionMap::build(&document, &tiptap_schema());

    // Verify initial state
    assert_eq!(map.block_count(), 2);
    assert_eq!(map.total_scalars(), 11);
    assert_eq!(map.block(0).unwrap().scalar_len, 5);
    assert_eq!(map.block(1).unwrap().scalar_len, 5);
    assert_eq!(map.block(1).unwrap().doc_start, 8);

    // Apply the transaction
    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::InsertText {
        pos: 2,
        text: "XX".to_string(),
        marks: vec![],
    });
    let (new_doc, step_map) = tx.apply(&document, &schema).expect("insert should succeed");

    // Update the position map
    map.update(
        &step_map,
        &document,
        &new_doc,
        UpdateMode::InlineTextOnly,
        &tiptap_schema(),
    );

    // Verify: first block should now have 7 scalars ("HXXello")
    assert_eq!(map.block_count(), 2, "block count should remain 2");

    // The first block was rebuilt:
    assert_eq!(
        map.block(0).unwrap().scalar_len,
        7,
        "first block should now be 'HXXello' = 7 scalars"
    );

    // Verify scalar_to_doc and doc_to_scalar still work correctly.
    // Second block's doc_start shifted by +2 (from 8 to 10).
    let second_block_start = map.scalar_to_doc(8, &new_doc);
    assert_eq!(
        second_block_start, 10,
        "start of second block content should be at doc 10 (was 8, shifted by +2)"
    );

    // Verify the total scalar count
    // "HXXello\nWorld" = 7 + 1 + 5 = 13
    // We need to compact to get accurate total_scalars
    map.compact();
    assert_eq!(
        map.total_scalars(),
        13,
        "'HXXello\\nWorld' = 7 + 1 + 5 = 13"
    );
}

#[test]
fn test_incremental_update_preserves_roundtrip() {
    // Start: <doc><p>AB</p><p>CD</p></doc>
    // Insert "X" at pos 2 (between A and B) -> "AXB"
    let doc_node = doc(vec![
        paragraph(vec![text("AB")]),
        paragraph(vec![text("CD")]),
    ]);
    let document = Document::new(doc_node);
    let schema = tiptap_schema();
    let mut map = PositionMap::build(&document, &tiptap_schema());

    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::InsertText {
        pos: 2,
        text: "X".to_string(),
        marks: vec![],
    });
    let (new_doc, step_map) = tx.apply(&document, &schema).expect("insert should succeed");
    map.update(
        &step_map,
        &document,
        &new_doc,
        UpdateMode::InlineTextOnly,
        &tiptap_schema(),
    );
    map.compact();

    // Verify round-trip for all scalar positions in the updated doc.
    // "AXB\nCD" = 3 + 1 + 2 = 6 scalars
    let total = map.total_scalars();
    assert_eq!(total, 6, "'AXB\\nCD' = 6 scalars");

    for scalar in 0..=total {
        let doc_pos = map.scalar_to_doc(scalar, &new_doc);
        let back = map.doc_to_scalar(doc_pos, &new_doc);
        assert_eq!(
            back, scalar,
            "post-update round-trip failed: scalar {} -> doc {} -> scalar {}",
            scalar, doc_pos, back
        );
    }
}

#[test]
fn test_compact_folds_deltas() {
    let doc_node = doc(vec![
        paragraph(vec![text("AB")]),
        paragraph(vec![text("CD")]),
    ]);
    let document = Document::new(doc_node);
    let schema = tiptap_schema();
    let mut map = PositionMap::build(&document, &tiptap_schema());

    // Insert "X" at pos 2 -> "AXB"
    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::InsertText {
        pos: 2,
        text: "X".to_string(),
        marks: vec![],
    });
    let (new_doc, step_map) = tx.apply(&document, &schema).unwrap();
    map.update(
        &step_map,
        &document,
        &new_doc,
        UpdateMode::InlineTextOnly,
        &tiptap_schema(),
    );

    // Before compact, the second block should have stale doc_start but correct
    // effective positions via delta tree.
    let b1_before = map.block(1).unwrap().clone();

    map.compact();

    let b1_after = map.block(1).unwrap();
    assert_eq!(
        b1_after.doc_start,
        b1_before.doc_start + 1,
        "after compact, second block doc_start should be shifted by +1"
    );
}

// Edge case: multiple text nodes in one paragraph

#[test]
fn test_multiple_text_nodes_single_paragraph() {
    // <doc><p>Hello World</p></doc> but split as two text nodes
    let document = Document::new(doc(vec![paragraph(vec![text("Hello "), text("World")])]));
    let map = PositionMap::build(&document, &tiptap_schema());

    assert_eq!(map.block_count(), 1);
    assert_eq!(map.total_scalars(), 11, "'Hello World' = 11 scalars");

    // Round-trip all positions
    for scalar in 0..=11u32 {
        let doc_pos = map.scalar_to_doc(scalar, &document);
        let back = map.doc_to_scalar(doc_pos, &document);
        assert_eq!(
            back, scalar,
            "round-trip failed for scalar {} (doc_pos {})",
            scalar, doc_pos
        );
    }
}

// Edge case: three paragraphs with breaks

#[test]
fn test_three_paragraphs() {
    // <doc><p>A</p><p>B</p><p>C</p></doc>
    // Rendered: "A\nB\nC" = 1 + 1 + 1 + 1 + 1 = 5 scalars
    let document = Document::new(doc(vec![
        paragraph(vec![text("A")]),
        paragraph(vec![text("B")]),
        paragraph(vec![text("C")]),
    ]));
    let map = PositionMap::build(&document, &tiptap_schema());

    assert_eq!(map.block_count(), 3);
    assert_eq!(map.total_scalars(), 5, "'A\\nB\\nC' = 5 scalars");

    assert_eq!(map.block(0).unwrap().rendered_break_after, 1);
    assert_eq!(map.block(1).unwrap().rendered_break_after, 1);
    assert_eq!(map.block(2).unwrap().rendered_break_after, 0);

    // scalar -> doc round-trip
    for scalar in 0..=5u32 {
        let doc_pos = map.scalar_to_doc(scalar, &document);
        let back = map.doc_to_scalar(doc_pos, &document);
        assert_eq!(
            back, scalar,
            "round-trip failed: scalar {} -> doc {} -> scalar {}",
            scalar, doc_pos, back
        );
    }
}

// Full rebuild via update with structural change

#[test]
fn test_update_fallback_to_rebuild() {
    // When the number of blocks changes, the update should fall back to a
    // full rebuild and still produce correct results.
    let doc_node = doc(vec![paragraph(vec![text("Hello")])]);
    let document = Document::new(doc_node);
    let mut map = PositionMap::build(&document, &tiptap_schema());

    // Simulate a structural change by providing a different document.
    let new_doc_node = doc(vec![
        paragraph(vec![text("Hello")]),
        paragraph(vec![text("World")]),
    ]);
    let new_document = Document::new(new_doc_node);

    // Use a StepMap that won't match the single-range optimization.
    let step_map = StepMap::empty();
    map.update(
        &step_map,
        &document,
        &new_document,
        UpdateMode::Rebuild,
        &tiptap_schema(),
    );

    assert_eq!(map.block_count(), 2, "should have rebuilt with 2 blocks");
    assert_eq!(
        map.total_scalars(),
        11,
        "'Hello\\nWorld' = 11 scalars after rebuild"
    );
}

// Renderer / position-map marker desync regression tests
//
// The renderer (`render::task_list_marker_metadata`) and the position map
// must agree on which list items render a task marker and how long that
// marker is. These tests ingest a document through the shared serializer
// (the same path the v2 session uses), take
// the render elements the native platform would actually receive, and derive
// the expected marker-prefixed scalar length purely from that render output
// (not from a hand re-derivation of the heuristic) — then assert the
// position map maps that scalar back to the correct document position.

/// A `checked` attr on an item in a NON-task list must not desync the
/// position map from the rendered text: whatever marker the renderer emits,
/// the position map must reserve the same number of scalars.
#[test]
fn checked_attr_in_ordered_list_keeps_positionmap_in_sync_with_render() {
    // The preset `tiptap_schema()` listItem declares no `checked` attr, so
    // ingestion (which strips attrs not declared by the schema) would drop
    // `checked` and this test would trivially pass without
    // exercising the render/position-map agreement it's meant to guard.
    // Use a custom schema — mirroring the fixture style of
    // `custom_named_task_list_positionmap_matches_render` below — whose
    // listItem explicitly declares `checked` so it survives ingestion.
    let schema = Schema::from_json(&serde_json::json!({
        "nodes": [
            { "name": "doc", "content": "block+", "role": "doc" },
            { "name": "paragraph", "content": "inline*", "group": "block", "role": "textBlock" },
            { "name": "orderedList", "content": "listItem+", "group": "block", "role": "list" },
            {
                "name": "listItem",
                "content": "paragraph+",
                "role": "listItem",
                "attrs": { "checked": { "default": false } }
            },
            { "name": "text", "content": "", "group": "inline", "role": "text" }
        ],
        "marks": []
    }))
    .expect("custom ordered-list schema with checked attr should parse");

    let document = crate::serialize::from_prosemirror_json(
        &serde_json::json!({
            "type": "doc",
            "content": [{
                "type": "orderedList",
                "content": [
                    {
                        "type": "listItem",
                        "attrs": { "checked": true },
                        "content": [{
                            "type": "paragraph",
                            "content": [{ "type": "text", "text": "Task A" }]
                        }]
                    },
                    {
                        "type": "listItem",
                        "content": [{
                            "type": "paragraph",
                            "content": [{ "type": "text", "text": "Task B" }]
                        }]
                    }
                ]
            }]
        }),
        &schema,
        crate::serialize::UnknownTypeMode::Preserve,
    )
    .expect("document ingestion should succeed");
    let elements = crate::render::incremental::flatten_render_blocks(
        &crate::render::incremental::render_blocks(&document, &schema),
    );
    let position_map = PositionMap::build(&document, &schema);

    // Walk the render elements (exactly what the native platform consumes)
    // and sum the scalar length of everything rendered before "Task B"'s
    // first character: item A's marker, item A's text, the inter-block
    // break, and item B's marker.
    let mut scalar_before_task_b: u32 = 0;
    for element in &elements {
        match element {
            RenderElement::BlockStart {
                node_type,
                list_context: Some(ctx),
                ..
            } if node_type == "listItem" => {
                let marker_len = if ctx.kind.as_deref() == Some("task") {
                    task_list_marker_string(ctx.checked.unwrap_or(false))
                        .chars()
                        .count() as u32
                } else {
                    list_marker_string(ctx.ordered, ctx.index).chars().count() as u32
                };
                scalar_before_task_b += marker_len;
            }
            RenderElement::TextRun { text, .. } if text == "Task B" => break,
            RenderElement::TextRun { text, .. } => {
                scalar_before_task_b += text.chars().count() as u32;
            }
            _ => {}
        }
    }
    // One inter-block break scalar between item A's paragraph and item B's
    // paragraph (they are two separate text blocks).
    scalar_before_task_b += BLOCK_BREAK_SCALARS;

    let task_b_doc_start = position_map
        .block(1)
        .expect("second paragraph should be block 1")
        .doc_start;

    // Convert doc -> scalar (not scalar -> doc): `scalar_to_doc` clamps any
    // offset that lands inside a block's marker-prefix window back to that
    // block's content start, which would mask a marker-length desync here.
    // `doc_to_scalar` of the exact content-start doc position does not.
    assert_eq!(
        position_map.doc_to_scalar(task_b_doc_start, &document),
        scalar_before_task_b,
        "position map must reserve exactly the marker length the renderer emitted \
         for item A (an ordinary ordered-list marker despite its default `checked` \
         attr) — doc position of \"Task B\"'s first char \
         should map back to scalar {}",
        scalar_before_task_b
    );
}

/// A custom schema list named `todoTaskList` (role: "list") must get the
/// same marker treatment in the position map as in the renderer.
#[test]
fn custom_named_task_list_positionmap_matches_render() {
    let schema = Schema::from_json(&serde_json::json!({
        "nodes": [
            { "name": "doc", "content": "block+", "role": "doc" },
            { "name": "paragraph", "content": "inline*", "group": "block", "role": "textBlock" },
            { "name": "todoTaskList", "content": "todoTaskItem+", "group": "block", "role": "list" },
            {
                "name": "todoTaskItem",
                "content": "paragraph+",
                "role": "listItem",
                "attrs": { "checked": { "default": false } }
            },
            { "name": "text", "content": "", "group": "inline", "role": "text" }
        ],
        "marks": []
    }))
    .expect("custom task-list schema should parse");

    let document = crate::serialize::from_prosemirror_json(
        &serde_json::json!({
            "type": "doc",
            "content": [{
                "type": "todoTaskList",
                "content": [{
                    "type": "todoTaskItem",
                    "content": [{
                        "type": "paragraph",
                        "content": [{ "type": "text", "text": "hello" }]
                    }]
                }]
            }]
        }),
        &schema,
        crate::serialize::UnknownTypeMode::Preserve,
    )
    .expect("document ingestion should succeed");
    let elements = crate::render::incremental::flatten_render_blocks(
        &crate::render::incremental::render_blocks(&document, &schema),
    );
    let position_map = PositionMap::build(&document, &schema);

    // The renderer decides "task" purely from the node type name containing
    // "task" (case-insensitively) — `todoTaskItem` qualifies — so it must
    // emit a checkbox marker even though this list isn't named `taskList`.
    let list_context = elements
        .iter()
        .find_map(|element| match element {
            RenderElement::BlockStart {
                node_type,
                list_context: Some(ctx),
                ..
            } if node_type == "todoTaskItem" => Some(ctx),
            _ => None,
        })
        .expect("todoTaskItem should emit a ListContext");
    assert_eq!(
        list_context.kind.as_deref(),
        Some("task"),
        "custom-named task item should be classified as a task marker by the renderer"
    );
    let marker_len = task_list_marker_string(list_context.checked.unwrap_or(false))
        .chars()
        .count() as u32;

    let hello_doc_start = position_map
        .block(0)
        .expect("paragraph should be block 0")
        .doc_start;

    assert_eq!(
        position_map.scalar_to_doc(marker_len, &document),
        hello_doc_start,
        "position map must reserve a task-marker prefix for a custom-named \
         task list item, matching what the renderer emitted"
    );
}
