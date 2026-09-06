// Block 0: doc_start=1, doc_end=7, scalar_len=6 (H, e, \n, l, l, o)
// Rendered: "He\nllo" = 6 scalars

#[test]
fn test_void_inline_build() {
    let document = Document::new(doc(vec![paragraph(vec![
        text("He"),
        hard_break(),
        text("llo"),
    ])]));
    let map = PositionMap::build(&document, &tiptap_schema());

    assert_eq!(
        map.block_count(),
        1,
        "one paragraph with hardBreak = 1 block"
    );
    assert_eq!(
        map.total_scalars(),
        6,
        "'He\\nllo' = 2 + 1(hardBreak) + 3 = 6 scalars"
    );

    let b = map.block(0).unwrap();
    assert_eq!(b.doc_start, 1);
    assert_eq!(b.doc_end, 7);
    assert_eq!(b.scalar_len, 6);
}

#[test]
fn test_void_inline_scalar_to_doc() {
    let document = Document::new(doc(vec![paragraph(vec![
        text("He"),
        hard_break(),
        text("llo"),
    ])]));
    let map = PositionMap::build(&document, &tiptap_schema());

    assert_eq!(map.scalar_to_doc(0, &document), 1, "scalar 0 -> doc 1 (H)");
    assert_eq!(map.scalar_to_doc(1, &document), 2, "scalar 1 -> doc 2 (e)");
    assert_eq!(
        map.scalar_to_doc(2, &document),
        3,
        "scalar 2 -> doc 3 (hardBreak)"
    );
    assert_eq!(
        map.scalar_to_doc(3, &document),
        4,
        "scalar 3 -> doc 4 (first l)"
    );
    assert_eq!(
        map.scalar_to_doc(4, &document),
        5,
        "scalar 4 -> doc 5 (second l)"
    );
    assert_eq!(map.scalar_to_doc(5, &document), 6, "scalar 5 -> doc 6 (o)");
    assert_eq!(
        map.scalar_to_doc(6, &document),
        7,
        "scalar 6 -> doc 7 (end of p)"
    );
}

#[test]
fn test_void_inline_doc_to_scalar() {
    let document = Document::new(doc(vec![paragraph(vec![
        text("He"),
        hard_break(),
        text("llo"),
    ])]));
    let map = PositionMap::build(&document, &tiptap_schema());

    assert_eq!(map.doc_to_scalar(1, &document), 0, "doc 1 -> scalar 0");
    assert_eq!(map.doc_to_scalar(2, &document), 1, "doc 2 -> scalar 1");
    assert_eq!(
        map.doc_to_scalar(3, &document),
        2,
        "doc 3 (hardBreak) -> scalar 2"
    );
    assert_eq!(map.doc_to_scalar(4, &document), 3, "doc 4 -> scalar 3");
    assert_eq!(map.doc_to_scalar(5, &document), 4, "doc 5 -> scalar 4");
    assert_eq!(map.doc_to_scalar(6, &document), 5, "doc 6 -> scalar 5");
    assert_eq!(map.doc_to_scalar(7, &document), 6, "doc 7 -> scalar 6");
}

#[test]
fn test_void_inline_roundtrip() {
    let document = Document::new(doc(vec![paragraph(vec![
        text("He"),
        hard_break(),
        text("llo"),
    ])]));
    let map = PositionMap::build(&document, &tiptap_schema());

    for scalar in 0..=6u32 {
        let doc_pos = map.scalar_to_doc(scalar, &document);
        let back = map.doc_to_scalar(doc_pos, &document);
        assert_eq!(
            back, scalar,
            "round-trip failed: scalar {} -> doc {} -> scalar {}",
            scalar, doc_pos, back
        );
    }
}

// Test 5: Horizontal rule — <doc><p>A</p><hr><p>B</p></doc>
//
// Doc layout:
//   p1.open | A | p1.close | hr | p2.open | B | p2.close
//   paragraph1.node_size = 3, hr.node_size = 1, paragraph2.node_size = 3
//   doc content_size = 7
//
// Positions:
//   0: before p1
//   1: inside p1 (A)
//   2: end of p1 content
//   3: after p1 / at hr
//   4: after hr / before p2
//   5: inside p2 (B)
//   6: end of p2 content
//   7: after p2
//
// Blocks:
//   Block 0: paragraph1, doc_start=1, doc_end=2, scalar_len=1
//   Block 1: hr (void block), doc_start=3, doc_end=3, scalar_len=1
//   Block 2: paragraph2, doc_start=5, doc_end=6, scalar_len=1
//
// With breaks: block 0 break=1, block 1 break=1, block 2 break=0
// scalar_start: block 0=0, block 1=0+1+1=2, block 2=2+1+1=4
// Total scalars: 4 + 1 = 5 ("A\n\uFFFC\nB")

#[test]
fn test_horizontal_rule_build() {
    let document = Document::new(doc(vec![
        paragraph(vec![text("A")]),
        horizontal_rule(),
        paragraph(vec![text("B")]),
    ]));
    let map = PositionMap::build(&document, &tiptap_schema());

    assert_eq!(map.block_count(), 3, "p + hr + p = 3 blocks");
    assert_eq!(
        map.total_scalars(),
        5,
        "'A\\n\\uFFFC\\nB' = 1 + 1 break + 1 + 1 break + 1 = 5"
    );

    let b0 = map.block(0).unwrap();
    assert_eq!(b0.doc_start, 1);
    assert_eq!(b0.doc_end, 2);
    assert_eq!(b0.scalar_start, 0);
    assert_eq!(b0.scalar_len, 1);
    assert_eq!(b0.rendered_break_after, 1);

    let b1 = map.block(1).unwrap();
    assert_eq!(b1.doc_start, 3, "hr is at doc pos 3");
    assert_eq!(b1.doc_end, 3, "void block: doc_start == doc_end");
    assert_eq!(b1.scalar_start, 2);
    assert_eq!(b1.scalar_len, 1, "hr renders as 1 scalar placeholder");
    assert_eq!(b1.rendered_break_after, 1);

    let b2 = map.block(2).unwrap();
    assert_eq!(b2.doc_start, 5);
    assert_eq!(b2.doc_end, 6);
    assert_eq!(b2.scalar_start, 4);
    assert_eq!(b2.scalar_len, 1);
    assert_eq!(b2.rendered_break_after, 0);
}

#[test]
fn test_horizontal_rule_scalar_to_doc() {
    let document = Document::new(doc(vec![
        paragraph(vec![text("A")]),
        horizontal_rule(),
        paragraph(vec![text("B")]),
    ]));
    let map = PositionMap::build(&document, &tiptap_schema());

    assert_eq!(map.scalar_to_doc(0, &document), 1, "scalar 0 -> doc 1 (A)");
    assert_eq!(
        map.scalar_to_doc(1, &document),
        2,
        "scalar 1 -> doc 2 (end of p1)"
    );
    assert_eq!(map.scalar_to_doc(2, &document), 3, "scalar 2 -> doc 3 (hr)");
    // scalar 3 is after hr content (the break between hr and p2)
    // falls in hr block at intra-offset 1, which is end of block
    assert_eq!(
        map.scalar_to_doc(3, &document),
        4,
        "scalar 3 -> doc 4 (after hr, before p2)"
    );
    assert_eq!(map.scalar_to_doc(4, &document), 5, "scalar 4 -> doc 5 (B)");
    assert_eq!(
        map.scalar_to_doc(5, &document),
        6,
        "scalar 5 -> doc 6 (end of p2)"
    );
}

#[test]
fn test_horizontal_rule_doc_to_scalar() {
    let document = Document::new(doc(vec![
        paragraph(vec![text("A")]),
        horizontal_rule(),
        paragraph(vec![text("B")]),
    ]));
    let map = PositionMap::build(&document, &tiptap_schema());

    assert_eq!(map.doc_to_scalar(1, &document), 0, "doc 1 -> scalar 0");
    assert_eq!(map.doc_to_scalar(2, &document), 1, "doc 2 -> scalar 1");
    assert_eq!(map.doc_to_scalar(3, &document), 2, "doc 3 (hr) -> scalar 2");
    assert_eq!(map.doc_to_scalar(4, &document), 3, "doc 4 -> scalar 3");
    assert_eq!(map.doc_to_scalar(5, &document), 4, "doc 5 -> scalar 4");
    assert_eq!(map.doc_to_scalar(6, &document), 5, "doc 6 -> scalar 5");
}

// Test 6: Emoji — <doc><p>Hi 👨‍👩‍👧‍👦!</p></doc>
//
// Text: "Hi 👨‍👩‍👧‍👦!" where the family emoji is 7 Unicode scalars
// Total text scalars: 3 + 7 + 1 = 11
// (H=1, i=1, space=1, family=7, !=1)
//
// Block 0: doc_start=1, doc_end=12, scalar_len=11
// Total scalars: 11

#[test]
fn test_emoji_build() {
    let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}\u{200D}\u{1F466}";
    let content = format!("Hi {}!", family);
    let document = Document::new(doc(vec![paragraph(vec![text(&content)])]));
    let map = PositionMap::build(&document, &tiptap_schema());

    assert_eq!(map.block_count(), 1);
    assert_eq!(
        map.total_scalars(),
        11,
        "'Hi ' (3) + family emoji (7) + '!' (1) = 11 scalars"
    );

    let b = map.block(0).unwrap();
    assert_eq!(b.doc_start, 1);
    assert_eq!(b.doc_end, 12, "1 + 11 = 12");
    assert_eq!(b.scalar_len, 11);
}

#[test]
fn test_emoji_scalar_to_doc_roundtrip() {
    let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}\u{200D}\u{1F466}";
    let content = format!("Hi {}!", family);
    let document = Document::new(doc(vec![paragraph(vec![text(&content)])]));
    let map = PositionMap::build(&document, &tiptap_schema());

    // For text-only blocks, scalar offset = doc intra-block offset.
    // So scalar i -> doc (1 + i), and doc (1 + i) -> scalar i.
    for scalar in 0..=11u32 {
        let doc_pos = map.scalar_to_doc(scalar, &document);
        assert_eq!(
            doc_pos,
            1 + scalar,
            "scalar {} should map to doc {}",
            scalar,
            1 + scalar
        );
        let back = map.doc_to_scalar(doc_pos, &document);
        assert_eq!(
            back, scalar,
            "round-trip failed: scalar {} -> doc {} -> scalar {}",
            scalar, doc_pos, back
        );
    }
}

// Test 7: Empty paragraph — <doc><p></p></doc>
//
// Doc layout:
//   p.open | (empty) | p.close
//   paragraph.node_size = 2 (1+0+1)
//   doc content_size = 2
//
// Block 0: doc_start=1, doc_end=1, scalar_len=1
//   Empty text blocks render an invisible placeholder scalar so native text
//   views have a concrete paragraph anchor for caret placement and styling.
// Total scalars: 1

#[test]
fn test_empty_paragraph_build() {
    let document = Document::new(doc(vec![paragraph(vec![])]));
    let map = PositionMap::build(&document, &tiptap_schema());

    assert_eq!(map.block_count(), 1, "empty paragraph = 1 block");
    assert_eq!(
        map.total_scalars(),
        1,
        "empty paragraph = 1 placeholder scalar"
    );

    let b = map.block(0).unwrap();
    assert_eq!(b.doc_start, 1);
    assert_eq!(
        b.doc_end, 1,
        "empty paragraph: content start == content end"
    );
    assert_eq!(b.scalar_len, 1);
    assert_eq!(b.rendered_break_after, 0, "terminal block");
}

#[test]
fn test_empty_paragraph_doc_to_scalar() {
    let document = Document::new(doc(vec![paragraph(vec![])]));
    let map = PositionMap::build(&document, &tiptap_schema());

    // doc 0 (before p) snaps to the only cursorable point in the empty block.
    assert_eq!(map.doc_to_scalar(0, &document), 1);
    // doc 1 (inside empty p) -> caret sits after the placeholder scalar
    assert_eq!(map.doc_to_scalar(1, &document), 1);
    // doc 2 (after p) -> scalar 1
    assert_eq!(map.doc_to_scalar(2, &document), 1);
}

#[test]
fn test_empty_paragraph_scalar_to_doc() {
    let document = Document::new(doc(vec![paragraph(vec![])]));
    let map = PositionMap::build(&document, &tiptap_schema());

    assert_eq!(map.scalar_to_doc(0, &document), 1);
    assert_eq!(map.scalar_to_doc(1, &document), 1);
}

#[test]
fn test_normalize_cursor_pos_inside_text() {
    let document = Document::new(doc(vec![paragraph(vec![text("Hello")])]));
    let map = PositionMap::build(&document, &tiptap_schema());

    // Positions inside text content are already cursorable.
    for pos in 1..=6u32 {
        assert_eq!(
            map.normalize_cursor_pos(pos, &document),
            pos,
            "position {} inside text should be returned as-is",
            pos
        );
    }
}

#[test]
fn test_normalize_cursor_pos_structural_tokens() {
    let document = Document::new(doc(vec![
        paragraph(vec![text("Hello")]),
        paragraph(vec![text("World")]),
    ]));
    let map = PositionMap::build(&document, &tiptap_schema());

    // doc 0: before first paragraph (structural) -> snap to start of first block = 1
    assert_eq!(
        map.normalize_cursor_pos(0, &document),
        1,
        "pos 0 (before p1) -> snap to doc_start of block 0 = 1"
    );

    // doc 7: between paragraphs (after p1.close, before p2.open) -> snap to nearest
    let norm = map.normalize_cursor_pos(7, &document);
    assert!(
        norm == 6 || norm == 8,
        "pos 7 (between blocks) should snap to 6 (end of p1) or 8 (start of p2), got {}",
        norm
    );

    // doc 14: after second paragraph (structural) -> snap to end of last block = 13
    assert_eq!(
        map.normalize_cursor_pos(14, &document),
        13,
        "pos 14 (after p2) -> snap to doc_end of last block = 13"
    );
}

#[test]
fn test_normalize_cursor_pos_nested_list() {
    let document = Document::new(doc(vec![bullet_list(vec![list_item(vec![paragraph(
        vec![text("X")],
    )])])]));
    let map = PositionMap::build(&document, &tiptap_schema());

    // doc 0: before bulletList -> snap to first block content start
    assert_eq!(
        map.normalize_cursor_pos(0, &document),
        3,
        "pos 0 (before bulletList) -> snap to doc_start of first block = 3"
    );

    // doc 3: inside paragraph content -> cursorable
    assert_eq!(map.normalize_cursor_pos(3, &document), 3);

    // doc 4: end of paragraph content -> cursorable
    assert_eq!(map.normalize_cursor_pos(4, &document), 4);

    // doc 7: after bulletList -> snap to end of last block
    assert_eq!(
        map.normalize_cursor_pos(7, &document),
        4,
        "pos 7 (after bulletList) -> snap to doc_end of last block = 4"
    );
}

#[test]
fn terminal_boundary_after_nested_void_block_normalizes_to_the_void() {
    let document = Document::new(doc(vec![blockquote(vec![
        paragraph(vec![text("caption")]),
        image(),
    ])]));
    let map = PositionMap::build(&document, &tiptap_schema());
    let void_block = map.block(map.block_count() - 1).unwrap();
    let nested_gap = void_block.doc_start + 1;

    assert!(void_block.is_void_block);
    assert_eq!(void_block.node_path.as_slice(), &[0, 1]);
    assert_eq!(
        map.normalize_cursor_pos(nested_gap, &document),
        void_block.doc_start
    );
}
