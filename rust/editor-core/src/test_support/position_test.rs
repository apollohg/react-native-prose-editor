use std::collections::HashMap;

use crate::model::{Document, Fragment, Node};
use crate::position::build::BLOCK_BREAK_SCALARS;
use crate::position::update::UpdateMode;
use crate::position::PositionMap;
use crate::render::{list_marker_string, task_list_marker_string, RenderElement};
use crate::schema::presets::tiptap_schema;
use crate::schema::Schema;
use crate::transform::{Source, Step, StepMap, Transaction};

// Helper builders (matching model_test.rs conventions)

fn text(s: &str) -> Node {
    Node::text(s.to_string(), vec![])
}

#[test]
fn position_paths_do_not_truncate_sibling_indexes_above_u16() {
    let children = (0..=65_536).map(|_| paragraph(vec![])).collect();
    let document = Document::new(doc(children));
    let map = PositionMap::build(&document, &tiptap_schema());

    assert_eq!(map.block(65_535).unwrap().node_path.as_slice(), &[65_535]);
    assert_eq!(map.block(65_536).unwrap().node_path.as_slice(), &[65_536]);
    assert_ne!(
        map.block(65_535).unwrap().node_path,
        map.block(65_536).unwrap().node_path
    );
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

fn task_list(children: Vec<Node>) -> Node {
    Node::element(
        "taskList".to_string(),
        HashMap::new(),
        Fragment::from(children),
    )
}

fn task_item(checked: bool, children: Vec<Node>) -> Node {
    let mut attrs = HashMap::new();
    attrs.insert("checked".to_string(), serde_json::Value::Bool(checked));
    Node::element("taskItem".to_string(), attrs, Fragment::from(children))
}

fn blockquote(children: Vec<Node>) -> Node {
    Node::element(
        "blockquote".to_string(),
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

fn image() -> Node {
    Node::void(
        "image".to_string(),
        HashMap::from([(
            "src".to_string(),
            serde_json::Value::String("https://example.com/a.png".to_string()),
        )]),
    )
}

/// A minimal schema declaring `taskList`/`taskItem` (by `NodeRole`, not just
/// name) for tests that build documents using those node type names. The
/// standard `tiptap_schema()` preset has no task-list node types, since task
/// lists are an app-supplied schema extension in this editor.
fn task_list_schema() -> Schema {
    Schema::from_json(&serde_json::json!({
        "nodes": [
            { "name": "doc", "content": "block+", "role": "doc" },
            { "name": "paragraph", "content": "inline*", "group": "block", "role": "textBlock" },
            { "name": "taskList", "content": "taskItem+", "group": "block", "role": "list" },
            { "name": "taskItem", "content": "paragraph block*", "role": "listItem", "attrs": { "checked": { "default": false } } },
            { "name": "text", "content": "", "group": "inline", "role": "text" }
        ],
        "marks": []
    }))
    .expect("task list schema should parse")
}

// Test 1: Single paragraph — <doc><p>Hello</p></doc>
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
    let map = PositionMap::build(&document, &tiptap_schema());

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
    let map = PositionMap::build(&document, &tiptap_schema());

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
    let map = PositionMap::build(&document, &tiptap_schema());

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
    let map = PositionMap::build(&document, &tiptap_schema());

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

// Test 2: Two paragraphs — <doc><p>Hello</p><p>World</p></doc>
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
    let map = PositionMap::build(&document, &tiptap_schema());

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
    let map = PositionMap::build(&document, &tiptap_schema());

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
    let map = PositionMap::build(&document, &tiptap_schema());

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
fn test_blockquote_followed_by_paragraph_scalar_to_doc() {
    let document = Document::new(doc(vec![
        blockquote(vec![paragraph(vec![text("Hello")])]),
        paragraph(vec![text("World")]),
    ]));
    let map = PositionMap::build(&document, &tiptap_schema());

    assert_eq!(
        map.total_scalars(),
        11,
        "'Hello\\nWorld' should render to 11 scalars"
    );

    assert_eq!(
        map.scalar_to_doc(6, &document),
        10,
        "scalar 6 should map to the second paragraph start"
    );
    assert_eq!(
        map.scalar_to_doc(7, &document),
        11,
        "scalar 7 should map inside the second paragraph"
    );
    assert_eq!(
        map.scalar_to_doc(8, &document),
        12,
        "scalar 8 should map inside the second paragraph"
    );
    assert_eq!(
        map.scalar_to_doc(9, &document),
        13,
        "scalar 9 should land before the fourth character of the second paragraph"
    );
    assert_eq!(
        map.scalar_to_doc(10, &document),
        14,
        "scalar 10 should land before the fifth character of the second paragraph"
    );
    assert_eq!(
        map.scalar_to_doc(11, &document),
        15,
        "scalar 11 should map to the end of the second paragraph"
    );
}

#[test]
fn test_two_paragraphs_roundtrip() {
    let document = Document::new(doc(vec![
        paragraph(vec![text("Hello")]),
        paragraph(vec![text("World")]),
    ]));
    let map = PositionMap::build(&document, &tiptap_schema());

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

// Test 3: Bullet list — <doc><ul><li><p>A</p></li><li><p>B</p></li></ul></doc>
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
    let map = PositionMap::build(&document, &tiptap_schema());

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
    let map = PositionMap::build(&document, &tiptap_schema());

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
    let map = PositionMap::build(&document, &tiptap_schema());

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
    let map = PositionMap::build(&document, &tiptap_schema());

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

#[test]
fn test_task_list_build_accounts_for_checkbox_prefixes() {
    let document = Document::new(doc(vec![task_list(vec![
        task_item(true, vec![paragraph(vec![text("A")])]),
        task_item(false, vec![paragraph(vec![text("B")])]),
    ])]));
    let map = PositionMap::build(&document, &task_list_schema());

    assert_eq!(
        map.block_count(),
        2,
        "two task items should produce two blocks"
    );
    assert_eq!(
        map.total_scalars(),
        7,
        "'☑ A\\n☐ B' should be seven scalars"
    );

    let first = map.block(0).unwrap();
    assert_eq!(first.scalar_prefix_len, 2);
    assert_eq!(first.scalar_start, 0);

    let second = map.block(1).unwrap();
    assert_eq!(second.scalar_prefix_len, 2);
    assert_eq!(second.scalar_start, 4);
}

#[test]
fn test_task_list_content_positions_account_for_checkbox_prefixes() {
    let document = Document::new(doc(vec![task_list(vec![
        task_item(true, vec![paragraph(vec![text("A")])]),
        task_item(false, vec![paragraph(vec![text("B")])]),
    ])]));
    let map = PositionMap::build(&document, &task_list_schema());

    let first = map.block(0).unwrap();
    let second = map.block(1).unwrap();

    assert_eq!(map.doc_to_scalar(first.doc_start, &document), 2);
    assert_eq!(map.scalar_to_doc(2, &document), first.doc_start);
    assert_eq!(map.doc_to_scalar(first.doc_end, &document), 3);
    assert_eq!(map.scalar_to_doc(3, &document), first.doc_end);

    assert_eq!(map.doc_to_scalar(second.doc_start, &document), 6);
    assert_eq!(map.scalar_to_doc(6, &document), second.doc_start);
    assert_eq!(map.doc_to_scalar(second.doc_end, &document), 7);
    assert_eq!(map.scalar_to_doc(7, &document), second.doc_end);
}

// Test 4: Void nodes — <doc><p>He<br>llo</p></doc>
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

include!("position_test/void_and_unicode.rs");

include!("position_test/incremental.rs");
