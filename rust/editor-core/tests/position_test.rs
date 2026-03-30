use std::collections::HashMap;

use editor_core::model::{Document, Fragment, Node};
use editor_core::position::PositionMap;
use editor_core::schema::presets::tiptap_schema;
use editor_core::transform::{Source, Step, StepMap, Transaction};

// ---------------------------------------------------------------------------
// Helper builders (matching model_test.rs conventions)
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

fn hard_break() -> Node {
    Node::void("hardBreak".to_string(), HashMap::new())
}

fn horizontal_rule() -> Node {
    Node::void("horizontalRule".to_string(), HashMap::new())
}

// ===========================================================================
// Test 1: Single paragraph — <doc><p>Hello</p></doc>
// ===========================================================================
//
// Doc layout:
//   doc.open | p.open | H e l l o | p.close | doc.close
//   (pos 0=before p, 1=start of p content, 2..5=in text, 6=end of p content, 7=after p)
//
// Block 0: doc_start=1, doc_end=6, scalar_len=5
// Rendered: "Hello" = 5 scalars
// Terminal block → rendered_break_after=0
// Total scalars: 5

#[test]
fn test_single_paragraph_build() {
    let document = Document::new(doc(vec![paragraph(vec![text("Hello")])]));
    let map = PositionMap::build(&document);

    assert_eq!(map.block_count(), 1, "single paragraph = 1 block");
    assert_eq!(map.total_scalars(), 5, "rendered text 'Hello' = 5 scalars");

    let b = map.block(0).unwrap();
    assert_eq!(b.doc_start, 1, "paragraph content starts at doc pos 1");
    assert_eq!(b.doc_end, 6, "paragraph content ends at doc pos 6");
    assert_eq!(b.scalar_start, 0, "first block starts at scalar 0");
    assert_eq!(b.scalar_len, 5, "'Hello' = 5 scalars");
    assert_eq!(
        b.rendered_break_after, 0,
        "terminal block has no trailing break"
    );
}

#[test]
fn test_single_paragraph_scalar_to_doc() {
    let document = Document::new(doc(vec![paragraph(vec![text("Hello")])]));
    let map = PositionMap::build(&document);

    // scalar 0 -> doc 1 (H)
    assert_eq!(map.scalar_to_doc(0, &document), 1, "scalar 0 -> doc 1 (H)");
    // scalar 1 -> doc 2 (e)
    assert_eq!(map.scalar_to_doc(1, &document), 2, "scalar 1 -> doc 2 (e)");
    // scalar 2 -> doc 3 (l)
    assert_eq!(
        map.scalar_to_doc(2, &document),
        3,
        "scalar 2 -> doc 3 (first l)"
    );
    // scalar 3 -> doc 4 (l)
    assert_eq!(
        map.scalar_to_doc(3, &document),
        4,
        "scalar 3 -> doc 4 (second l)"
    );
    // scalar 4 -> doc 5 (o)
    assert_eq!(map.scalar_to_doc(4, &document), 5, "scalar 4 -> doc 5 (o)");
    // scalar 5 -> doc 6 (end of paragraph content)
    assert_eq!(
        map.scalar_to_doc(5, &document),
        6,
        "scalar 5 -> doc 6 (end of p content)"
    );
}

#[test]
fn test_single_paragraph_doc_to_scalar() {
    let document = Document::new(doc(vec![paragraph(vec![text("Hello")])]));
    let map = PositionMap::build(&document);

    // doc 0 (before p, structural) -> snap to block start = scalar 0
    assert_eq!(
        map.doc_to_scalar(0, &document),
        0,
        "doc 0 (structural, before p) -> scalar 0"
    );
    // doc 1 -> scalar 0
    assert_eq!(map.doc_to_scalar(1, &document), 0, "doc 1 -> scalar 0");
    // doc 2 -> scalar 1
    assert_eq!(map.doc_to_scalar(2, &document), 1, "doc 2 -> scalar 1");
    // doc 3 -> scalar 2
    assert_eq!(map.doc_to_scalar(3, &document), 2, "doc 3 -> scalar 2");
    // doc 4 -> scalar 3
    assert_eq!(map.doc_to_scalar(4, &document), 3, "doc 4 -> scalar 3");
    // doc 5 -> scalar 4
    assert_eq!(map.doc_to_scalar(5, &document), 4, "doc 5 -> scalar 4");
    // doc 6 -> scalar 5
    assert_eq!(map.doc_to_scalar(6, &document), 5, "doc 6 -> scalar 5");
    // doc 7 (after p, structural) -> snap to end of last block = scalar 5
    assert_eq!(
        map.doc_to_scalar(7, &document),
        5,
        "doc 7 (structural, after p) -> scalar 5"
    );
}

#[test]
fn test_single_paragraph_roundtrip() {
    let document = Document::new(doc(vec![paragraph(vec![text("Hello")])]));
    let map = PositionMap::build(&document);

    // All cursorable positions should round-trip through scalar -> doc -> scalar.
    for scalar in 0..=5u32 {
        let doc_pos = map.scalar_to_doc(scalar, &document);
        let back = map.doc_to_scalar(doc_pos, &document);
        assert_eq!(
            back, scalar,
            "round-trip failed: scalar {} -> doc {} -> scalar {} (expected {})",
            scalar, doc_pos, back, scalar
        );
    }
}

// ===========================================================================
// Test 2: Two paragraphs — <doc><p>Hello</p><p>World</p></doc>
// ===========================================================================
//
// Doc layout:
//   p1.open | Hello | p1.close | p2.open | World | p2.close
//   pos: 0=before p1, 1..6=inside p1, 7=after p1/before p2, 8..13=inside p2, 14=after p2
//
// Block 0: doc_start=1, doc_end=6, scalar_start=0, scalar_len=5, break_after=1
// Block 1: doc_start=8, doc_end=13, scalar_start=6, scalar_len=5, break_after=0
// Rendered: "Hello\nWorld" = 11 scalars

#[test]
fn test_two_paragraphs_build() {
    let document = Document::new(doc(vec![
        paragraph(vec![text("Hello")]),
        paragraph(vec![text("World")]),
    ]));
    let map = PositionMap::build(&document);

    assert_eq!(map.block_count(), 2, "two paragraphs = 2 blocks");
    assert_eq!(
        map.total_scalars(),
        11,
        "'Hello\\nWorld' = 5 + 1 break + 5 = 11 scalars"
    );

    let b0 = map.block(0).unwrap();
    assert_eq!(b0.doc_start, 1);
    assert_eq!(b0.doc_end, 6);
    assert_eq!(b0.scalar_start, 0);
    assert_eq!(b0.scalar_len, 5);
    assert_eq!(
        b0.rendered_break_after, 1,
        "non-terminal block gets 1 break"
    );

    let b1 = map.block(1).unwrap();
    assert_eq!(b1.doc_start, 8);
    assert_eq!(b1.doc_end, 13);
    assert_eq!(b1.scalar_start, 6, "5 + 1 break = 6");
    assert_eq!(b1.scalar_len, 5);
    assert_eq!(b1.rendered_break_after, 0, "terminal block gets 0 break");
}

#[test]
fn test_two_paragraphs_scalar_to_doc() {
    let document = Document::new(doc(vec![
        paragraph(vec![text("Hello")]),
        paragraph(vec![text("World")]),
    ]));
    let map = PositionMap::build(&document);

    // First block: scalars 0..4 -> doc 1..5
    assert_eq!(map.scalar_to_doc(0, &document), 1, "scalar 0 -> doc 1 (H)");
    assert_eq!(map.scalar_to_doc(4, &document), 5, "scalar 4 -> doc 5 (o)");

    // Scalar 5 is the last position inside the first block content (after 'o')
    assert_eq!(
        map.scalar_to_doc(5, &document),
        6,
        "scalar 5 -> doc 6 (end of first p)"
    );

    // Scalar 6 is start of second block
    assert_eq!(map.scalar_to_doc(6, &document), 8, "scalar 6 -> doc 8 (W)");

    // Scalar 10 is last char of second block
    assert_eq!(
        map.scalar_to_doc(10, &document),
        12,
        "scalar 10 -> doc 12 (d)"
    );

    // Scalar 11 is end of second block
    assert_eq!(
        map.scalar_to_doc(11, &document),
        13,
        "scalar 11 -> doc 13 (end of second p)"
    );
}

#[test]
fn test_two_paragraphs_doc_to_scalar() {
    let document = Document::new(doc(vec![
        paragraph(vec![text("Hello")]),
        paragraph(vec![text("World")]),
    ]));
    let map = PositionMap::build(&document);

    // doc 0 (before first p) -> snap to start of first block = scalar 0
    assert_eq!(map.doc_to_scalar(0, &document), 0, "doc 0 -> scalar 0");

    // doc 1..6 -> scalar 0..5
    assert_eq!(map.doc_to_scalar(1, &document), 0, "doc 1 -> scalar 0");
    assert_eq!(map.doc_to_scalar(6, &document), 5, "doc 6 -> scalar 5");

    // doc 7 (between paragraphs, structural) -> snaps to nearest block
    // It's after p1's close and before p2's open. The gap is at doc 7.
    // Nearest block: end of block 0 (doc 6) or start of block 1 (doc 8).
    // Distance from 7 to 6 = 1, from 7 to 8 = 1 => tie, snap to previous
    let s7 = map.doc_to_scalar(7, &document);
    assert!(
        s7 == 5 || s7 == 6,
        "doc 7 (between paragraphs) should snap to scalar 5 or 6, got {}",
        s7
    );

    // doc 8..13 -> scalar 6..11
    assert_eq!(map.doc_to_scalar(8, &document), 6, "doc 8 -> scalar 6");
    assert_eq!(map.doc_to_scalar(13, &document), 11, "doc 13 -> scalar 11");
}

#[test]
fn test_two_paragraphs_roundtrip() {
    let document = Document::new(doc(vec![
        paragraph(vec![text("Hello")]),
        paragraph(vec![text("World")]),
    ]));
    let map = PositionMap::build(&document);

    // Cursorable scalars: 0..=5 (first block), 6..=11 (second block)
    for scalar in 0..=11u32 {
        let doc_pos = map.scalar_to_doc(scalar, &document);
        let back = map.doc_to_scalar(doc_pos, &document);
        assert_eq!(
            back, scalar,
            "round-trip failed: scalar {} -> doc {} -> scalar {} (expected {})",
            scalar, doc_pos, back, scalar
        );
    }
}

// ===========================================================================
// Test 3: Bullet list — <doc><ul><li><p>A</p></li><li><p>B</p></li></ul></doc>
// ===========================================================================
//
// Doc layout (positions inside doc content):
//   0: before bulletList
//   1: inside bulletList, before first listItem
//   2: inside first listItem, before paragraph
//   3: inside paragraph, before "A"
//   4: inside paragraph, after "A"
//   5: inside listItem, after paragraph
//   6: inside bulletList, after first listItem / before second listItem
//   7: inside second listItem, before paragraph
//   8: inside paragraph, before "B"
//   9: inside paragraph, after "B"
//   10: inside listItem, after paragraph
//   11: inside bulletList, after second listItem
//   12: after bulletList
//
// Blocks:
//   Block 0: paragraph in first listItem, path=[0,0,0], doc_start=3, doc_end=4, scalar_len=1
//   Block 1: paragraph in second listItem, path=[0,1,0], doc_start=8, doc_end=9, scalar_len=1
// With breaks: block 0 break=1, block 1 break=0
// scalar_start: block 0=0, block 1=2
// Total scalars: 2 + 1 = 3 ("A\nB")

#[test]
fn test_bullet_list_build() {
    let document = Document::new(doc(vec![bullet_list(vec![
        list_item(vec![paragraph(vec![text("A")])]),
        list_item(vec![paragraph(vec![text("B")])]),
    ])]));
    let map = PositionMap::build(&document);

    assert_eq!(map.block_count(), 2, "two list items = 2 blocks");
    assert_eq!(
        map.total_scalars(),
        7,
        "'• A\\n• B' = 2 + 1 + 1 break + 2 + 1 = 7"
    );

    let b0 = map.block(0).unwrap();
    assert_eq!(b0.doc_start, 3, "first paragraph content starts at 3");
    assert_eq!(b0.doc_end, 4, "first paragraph content ends at 4");
    assert_eq!(b0.scalar_start, 0);
    assert_eq!(
        b0.scalar_prefix_len, 2,
        "first item renders a bullet prefix"
    );
    assert_eq!(b0.scalar_len, 1);
    assert_eq!(b0.rendered_break_after, 1);
    assert_eq!(b0.node_path.as_slice(), &[0, 0, 0]);

    let b1 = map.block(1).unwrap();
    assert_eq!(b1.doc_start, 8, "second paragraph content starts at 8");
    assert_eq!(b1.doc_end, 9, "second paragraph content ends at 9");
    assert_eq!(b1.scalar_start, 4, "2 prefix + 1 content + 1 break = 4");
    assert_eq!(
        b1.scalar_prefix_len, 2,
        "second item renders a bullet prefix"
    );
    assert_eq!(b1.scalar_len, 1);
    assert_eq!(b1.rendered_break_after, 0);
    assert_eq!(b1.node_path.as_slice(), &[0, 1, 0]);
}

#[test]
fn test_bullet_list_scalar_to_doc() {
    let document = Document::new(doc(vec![bullet_list(vec![
        list_item(vec![paragraph(vec![text("A")])]),
        list_item(vec![paragraph(vec![text("B")])]),
    ])]));
    let map = PositionMap::build(&document);

    assert_eq!(
        map.scalar_to_doc(0, &document),
        3,
        "scalar 0 -> doc 3 (bullet prefix)"
    );
    assert_eq!(
        map.scalar_to_doc(1, &document),
        3,
        "scalar 1 -> doc 3 (bullet prefix)"
    );
    assert_eq!(map.scalar_to_doc(2, &document), 3, "scalar 2 -> doc 3 (A)");
    assert_eq!(
        map.scalar_to_doc(3, &document),
        4,
        "scalar 1 -> doc 4 (end of first p)"
    );
    assert_eq!(
        map.scalar_to_doc(4, &document),
        8,
        "scalar 4 -> doc 8 (bullet prefix)"
    );
    assert_eq!(
        map.scalar_to_doc(5, &document),
        8,
        "scalar 5 -> doc 8 (bullet prefix)"
    );
    assert_eq!(map.scalar_to_doc(6, &document), 8, "scalar 6 -> doc 8 (B)");
    assert_eq!(
        map.scalar_to_doc(7, &document),
        9,
        "scalar 3 -> doc 9 (end of second p)"
    );
}

#[test]
fn test_bullet_list_doc_to_scalar() {
    let document = Document::new(doc(vec![bullet_list(vec![
        list_item(vec![paragraph(vec![text("A")])]),
        list_item(vec![paragraph(vec![text("B")])]),
    ])]));
    let map = PositionMap::build(&document);

    assert_eq!(map.doc_to_scalar(3, &document), 2, "doc 3 -> scalar 2 (A)");
    assert_eq!(
        map.doc_to_scalar(4, &document),
        3,
        "doc 4 -> scalar 3 (end of first p)"
    );
    assert_eq!(map.doc_to_scalar(8, &document), 6, "doc 8 -> scalar 6 (B)");
    assert_eq!(
        map.doc_to_scalar(9, &document),
        7,
        "doc 9 -> scalar 7 (end of second p)"
    );
}

#[test]
fn test_bullet_list_roundtrip() {
    let document = Document::new(doc(vec![bullet_list(vec![
        list_item(vec![paragraph(vec![text("A")])]),
        list_item(vec![paragraph(vec![text("B")])]),
    ])]));
    let map = PositionMap::build(&document);

    for scalar in 0..=7u32 {
        let doc_pos = map.scalar_to_doc(scalar, &document);
        let canonical_scalar = map.doc_to_scalar(doc_pos, &document);
        let back = map.scalar_to_doc(canonical_scalar, &document);
        assert_eq!(
            back, doc_pos,
            "round-trip failed: scalar {} -> doc {} -> scalar {} -> doc {}",
            scalar, doc_pos, canonical_scalar, back
        );
    }
}

// ===========================================================================
// Test 4: Void nodes — <doc><p>He<br>llo</p></doc>
// ===========================================================================
//
// Doc layout:
//   p.open | H(1) e(1) | hardBreak(1) | l(1) l(1) o(1) | p.close
//   paragraph content size: 2 + 1 + 3 = 6
//   doc content_size: 1 + 6 + 1 = 8
//
// Positions inside doc:
//   0: before p
//   1: inside p, offset 0 (before H)
//   2: inside p, offset 1 (between H and e)
//   3: inside p, offset 2 (after e, at hardBreak)
//   4: inside p, offset 3 (after hardBreak, before l)
//   5: inside p, offset 4
//   6: inside p, offset 5
//   7: inside p, offset 6 (after o)
//   8: after p
//
// Block 0: doc_start=1, doc_end=7, scalar_len=6 (H, e, \n, l, l, o)
// Rendered: "He\nllo" = 6 scalars

#[test]
fn test_void_inline_build() {
    let document = Document::new(doc(vec![paragraph(vec![
        text("He"),
        hard_break(),
        text("llo"),
    ])]));
    let map = PositionMap::build(&document);

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
    let map = PositionMap::build(&document);

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
    let map = PositionMap::build(&document);

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
    let map = PositionMap::build(&document);

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

// ===========================================================================
// Test 5: Horizontal rule — <doc><p>A</p><hr><p>B</p></doc>
// ===========================================================================
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
    let map = PositionMap::build(&document);

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
    let map = PositionMap::build(&document);

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
    let map = PositionMap::build(&document);

    assert_eq!(map.doc_to_scalar(1, &document), 0, "doc 1 -> scalar 0");
    assert_eq!(map.doc_to_scalar(2, &document), 1, "doc 2 -> scalar 1");
    assert_eq!(map.doc_to_scalar(3, &document), 2, "doc 3 (hr) -> scalar 2");
    assert_eq!(map.doc_to_scalar(4, &document), 3, "doc 4 -> scalar 3");
    assert_eq!(map.doc_to_scalar(5, &document), 4, "doc 5 -> scalar 4");
    assert_eq!(map.doc_to_scalar(6, &document), 5, "doc 6 -> scalar 5");
}

// ===========================================================================
// Test 6: Emoji — <doc><p>Hi 👨‍👩‍👧‍👦!</p></doc>
// ===========================================================================
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
    let map = PositionMap::build(&document);

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
    let map = PositionMap::build(&document);

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

// ===========================================================================
// Test 7: Empty paragraph — <doc><p></p></doc>
// ===========================================================================
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
    let map = PositionMap::build(&document);

    assert_eq!(map.block_count(), 1, "empty paragraph = 1 block");
    assert_eq!(map.total_scalars(), 1, "empty paragraph = 1 placeholder scalar");

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
    let map = PositionMap::build(&document);

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
    let map = PositionMap::build(&document);

    assert_eq!(map.scalar_to_doc(0, &document), 1);
    assert_eq!(map.scalar_to_doc(1, &document), 1);
}

// ===========================================================================
// normalize_cursor_pos tests
// ===========================================================================

#[test]
fn test_normalize_cursor_pos_inside_text() {
    let document = Document::new(doc(vec![paragraph(vec![text("Hello")])]));
    let map = PositionMap::build(&document);

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
    let map = PositionMap::build(&document);

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
    let map = PositionMap::build(&document);

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

// ===========================================================================
// DeltaTree unit tests
// ===========================================================================

#[test]
fn test_delta_tree_empty() {
    let dt = editor_core::position::delta_tree::DeltaTree::empty();
    assert!(dt.is_empty());
    assert_eq!(dt.len(), 0);
    assert_eq!(dt.accumulated_delta(0), (0, 0));
    assert_eq!(dt.accumulated_delta(100), (0, 0));
}

#[test]
fn test_delta_tree_single_insert() {
    let mut dt = editor_core::position::delta_tree::DeltaTree::empty();
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
    let mut dt = editor_core::position::delta_tree::DeltaTree::empty();
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
    let mut dt = editor_core::position::delta_tree::DeltaTree::empty();
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
    let mut dt = editor_core::position::delta_tree::DeltaTree::empty();
    dt.insert(0, 1, 1);
    dt.insert(2, 3, 3);
    assert!(!dt.is_empty());

    dt.clear();
    assert!(dt.is_empty());
    assert_eq!(dt.accumulated_delta(5), (0, 0));
}

// ===========================================================================
// Incremental update test
// ===========================================================================

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
    let mut map = PositionMap::build(&document);

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
    map.update(&step_map, &new_doc);

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
    let mut map = PositionMap::build(&document);

    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::InsertText {
        pos: 2,
        text: "X".to_string(),
        marks: vec![],
    });
    let (new_doc, step_map) = tx.apply(&document, &schema).expect("insert should succeed");
    map.update(&step_map, &new_doc);
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

// ===========================================================================
// Compact tests
// ===========================================================================

#[test]
fn test_compact_folds_deltas() {
    let doc_node = doc(vec![
        paragraph(vec![text("AB")]),
        paragraph(vec![text("CD")]),
    ]);
    let document = Document::new(doc_node);
    let schema = tiptap_schema();
    let mut map = PositionMap::build(&document);

    // Insert "X" at pos 2 -> "AXB"
    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::InsertText {
        pos: 2,
        text: "X".to_string(),
        marks: vec![],
    });
    let (new_doc, step_map) = tx.apply(&document, &schema).unwrap();
    map.update(&step_map, &new_doc);

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

// ===========================================================================
// Edge case: multiple text nodes in one paragraph
// ===========================================================================

#[test]
fn test_multiple_text_nodes_single_paragraph() {
    // <doc><p>Hello World</p></doc> but split as two text nodes
    let document = Document::new(doc(vec![paragraph(vec![text("Hello "), text("World")])]));
    let map = PositionMap::build(&document);

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

// ===========================================================================
// Edge case: three paragraphs with breaks
// ===========================================================================

#[test]
fn test_three_paragraphs() {
    // <doc><p>A</p><p>B</p><p>C</p></doc>
    // Rendered: "A\nB\nC" = 1 + 1 + 1 + 1 + 1 = 5 scalars
    let document = Document::new(doc(vec![
        paragraph(vec![text("A")]),
        paragraph(vec![text("B")]),
        paragraph(vec![text("C")]),
    ]));
    let map = PositionMap::build(&document);

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

// ===========================================================================
// Full rebuild via update with structural change
// ===========================================================================

#[test]
fn test_update_fallback_to_rebuild() {
    // When the number of blocks changes, the update should fall back to a
    // full rebuild and still produce correct results.
    let doc_node = doc(vec![paragraph(vec![text("Hello")])]);
    let document = Document::new(doc_node);
    let mut map = PositionMap::build(&document);

    // Simulate a structural change by providing a different document.
    let new_doc_node = doc(vec![
        paragraph(vec![text("Hello")]),
        paragraph(vec![text("World")]),
    ]);
    let new_document = Document::new(new_doc_node);

    // Use a StepMap that won't match the single-range optimization.
    let step_map = StepMap::empty();
    map.update(&step_map, &new_document);

    assert_eq!(map.block_count(), 2, "should have rebuilt with 2 blocks");
    assert_eq!(
        map.total_scalars(),
        11,
        "'Hello\\nWorld' = 11 scalars after rebuild"
    );
}
