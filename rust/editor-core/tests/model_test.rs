use std::collections::HashMap;

use editor_core::model::{Document, Fragment, Mark, Node};

// ---------------------------------------------------------------------------
// Helper builders
// ---------------------------------------------------------------------------

fn bold() -> Mark {
    Mark::new("bold".to_string(), HashMap::new())
}

fn italic() -> Mark {
    Mark::new("italic".to_string(), HashMap::new())
}

fn link(href: &str) -> Mark {
    let mut attrs = HashMap::new();
    attrs.insert(
        "href".to_string(),
        serde_json::Value::String(href.to_string()),
    );
    Mark::new("link".to_string(), attrs)
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

fn hard_break() -> Node {
    Node::void("hardBreak".to_string(), HashMap::new())
}

fn horizontal_rule() -> Node {
    Node::void("horizontalRule".to_string(), HashMap::new())
}

// ===========================================================================
// Mark tests
// ===========================================================================

#[test]
fn test_mark_creation() {
    let m = bold();
    assert_eq!(m.mark_type(), "bold");
    assert!(m.attrs().is_empty());
}

#[test]
fn test_mark_with_attrs() {
    let m = link("https://example.com");
    assert_eq!(m.mark_type(), "link");
    assert_eq!(
        m.attrs().get("href"),
        Some(&serde_json::Value::String(
            "https://example.com".to_string()
        ))
    );
}

#[test]
fn test_mark_equality() {
    let a = bold();
    let b = bold();
    assert_eq!(a, b, "two bold marks with no attrs should be equal");

    let c = italic();
    assert_ne!(a, c, "bold and italic should not be equal");
}

#[test]
fn test_mark_equality_with_attrs() {
    let a = link("https://a.com");
    let b = link("https://a.com");
    let c = link("https://b.com");
    assert_eq!(a, b, "same link attrs should be equal");
    assert_ne!(a, c, "different link attrs should not be equal");
}

// ===========================================================================
// Node creation tests
// ===========================================================================

#[test]
fn test_text_node_creation() {
    let n = text("hello");
    assert!(n.is_text());
    assert!(!n.is_void());
    assert_eq!(n.text_content(), "hello");
    assert!(n.marks().is_empty());
}

#[test]
fn test_text_node_with_marks() {
    let n = text_with_marks("bold text", vec![bold()]);
    assert!(n.is_text());
    assert_eq!(n.marks().len(), 1);
    assert_eq!(n.marks()[0].mark_type(), "bold");
    assert_eq!(n.text_content(), "bold text");
}

#[test]
fn test_text_node_with_multiple_marks() {
    let n = text_with_marks("styled", vec![bold(), italic()]);
    assert_eq!(n.marks().len(), 2);
}

#[test]
fn test_void_node_creation() {
    let n = hard_break();
    assert!(n.is_void());
    assert!(!n.is_text());
    assert_eq!(n.node_type(), "hardBreak");
    assert_eq!(n.text_content(), "");
}

#[test]
fn test_element_node_creation() {
    let p = paragraph(vec![text("hello")]);
    assert!(!p.is_text());
    assert!(!p.is_void());
    assert_eq!(p.node_type(), "paragraph");
    assert_eq!(p.child_count(), 1);
}

// ===========================================================================
// Fragment tests
// ===========================================================================

#[test]
fn test_empty_fragment() {
    let f = Fragment::empty();
    assert_eq!(f.size(), 0);
    assert_eq!(f.child_count(), 0);
}

#[test]
fn test_fragment_size_text_only() {
    // A fragment containing a single text node "hello" (5 chars)
    let f = Fragment::from(vec![text("hello")]);
    assert_eq!(f.size(), 5, "fragment with 'hello' should have size 5");
}

#[test]
fn test_fragment_size_mixed_text() {
    // Two text nodes: "Hello " (6 chars) + "world" (5 chars) = 11
    let f = Fragment::from(vec![text("Hello "), text("world")]);
    assert_eq!(f.size(), 11);
}

#[test]
fn test_fragment_child_access() {
    let f = Fragment::from(vec![text("a"), text("b"), text("c")]);
    assert_eq!(f.child_count(), 3);
    assert_eq!(f.child(0).unwrap().text_content(), "a");
    assert_eq!(f.child(1).unwrap().text_content(), "b");
    assert_eq!(f.child(2).unwrap().text_content(), "c");
    assert!(f.child(3).is_none());
}

// ===========================================================================
// Node size tests
// ===========================================================================

#[test]
fn test_text_node_size() {
    assert_eq!(text("hello").node_size(), 5, "'hello' = 5 scalars");
    assert_eq!(text("").node_size(), 0, "empty text = 0 scalars");
    assert_eq!(text("a").node_size(), 1);
}

#[test]
fn test_text_node_size_unicode_scalars() {
    // Basic emoji: single codepoint
    assert_eq!(text("\u{1F600}").node_size(), 1, "grinning face = 1 scalar");

    // Family emoji: 7 scalars (4 person codepoints + 3 ZWJ)
    let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}\u{200D}\u{1F466}";
    assert_eq!(
        text(family).node_size(),
        7,
        "family emoji should be 7 Unicode scalars"
    );

    // Flag emoji: 2 regional indicator scalars
    let flag = "\u{1F1E6}\u{1F1FA}"; // AU flag
    assert_eq!(text(flag).node_size(), 2, "AU flag = 2 scalars");
}

#[test]
fn test_text_node_size_cjk() {
    // CJK characters are 1 scalar each
    assert_eq!(
        text("日本語").node_size(),
        3,
        "3 CJK characters = 3 scalars"
    );
}

#[test]
fn test_void_node_size() {
    assert_eq!(hard_break().node_size(), 1, "hardBreak = 1 token");
    assert_eq!(horizontal_rule().node_size(), 1, "horizontalRule = 1 token");
}

#[test]
fn test_element_node_size() {
    // paragraph with "hello":
    // 1 (open) + 5 (text) + 1 (close) = 7
    let p = paragraph(vec![text("hello")]);
    assert_eq!(p.node_size(), 7, "paragraph('hello') = 1 + 5 + 1 = 7");
}

#[test]
fn test_empty_paragraph_size() {
    // 1 (open) + 0 + 1 (close) = 2
    let p = paragraph(vec![]);
    assert_eq!(p.node_size(), 2, "empty paragraph = 1 + 0 + 1 = 2");
}

#[test]
fn test_nested_node_size() {
    // doc > paragraph > "Hi"
    // doc: 1 + (paragraph: 1 + 2 + 1) + 1 = 6
    let d = doc(vec![paragraph(vec![text("Hi")])]);
    assert_eq!(d.node_size(), 6, "doc(paragraph('Hi')) = 1 + 4 + 1 = 6");
}

#[test]
fn test_spec_example_doc_size() {
    // The canonical test from the task description:
    // <doc><paragraph>Hello <bold>world</bold>!</paragraph></doc>
    //
    // doc open (1) + paragraph open (1) + "Hello " (6) + "world" (5) + "!" (1)
    //   + paragraph close (1) + doc close (1) = 16
    let d = doc(vec![paragraph(vec![
        text("Hello "),
        text_with_marks("world", vec![bold()]),
        text("!"),
    ])]);
    assert_eq!(
        d.node_size(),
        16,
        "doc(paragraph('Hello ' + bold('world') + '!')) should be 16 tokens"
    );
}

// ===========================================================================
// Document wrapper and doc_size tests
// ===========================================================================

#[test]
fn test_document_creation() {
    let root = doc(vec![paragraph(vec![text("test")])]);
    let document = Document::new(root);
    assert_eq!(document.doc_size(), 8); // 1 + (1 + 4 + 1) + 1 = 8
}

#[test]
fn test_document_content_size() {
    // content_size excludes the root node's own open/close tags
    let root = doc(vec![paragraph(vec![text("Hi")])]);
    let document = Document::new(root);
    // content_size = paragraph node_size = 1 + 2 + 1 = 4
    assert_eq!(document.content_size(), 4);
}

// ===========================================================================
// Text content extraction
// ===========================================================================

#[test]
fn test_text_content_simple() {
    let d = doc(vec![paragraph(vec![text("Hello world")])]);
    assert_eq!(d.text_content(), "Hello world");
}

#[test]
fn test_text_content_mixed_marks() {
    let d = doc(vec![paragraph(vec![
        text("Hello "),
        text_with_marks("world", vec![bold()]),
        text("!"),
    ])]);
    assert_eq!(d.text_content(), "Hello world!");
}

#[test]
fn test_text_content_multiple_paragraphs() {
    let d = doc(vec![
        paragraph(vec![text("First")]),
        paragraph(vec![text("Second")]),
    ]);
    assert_eq!(d.text_content(), "FirstSecond");
}

#[test]
fn test_text_content_nested_list() {
    let d = doc(vec![bullet_list(vec![
        list_item(vec![paragraph(vec![text("Item 1")])]),
        list_item(vec![paragraph(vec![text("Item 2")])]),
    ])]);
    assert_eq!(d.text_content(), "Item 1Item 2");
}

#[test]
fn test_text_content_void_node() {
    // Void nodes contribute no text
    let p = paragraph(vec![text("before"), hard_break(), text("after")]);
    assert_eq!(p.text_content(), "beforeafter");
}

// ===========================================================================
// child / child_count tests
// ===========================================================================

#[test]
fn test_node_child_count() {
    let p = paragraph(vec![text("a"), text("b"), text("c")]);
    assert_eq!(p.child_count(), 3);
}

#[test]
fn test_node_child_access() {
    let p = paragraph(vec![text("first"), text("second")]);
    assert_eq!(p.child(0).unwrap().text_content(), "first");
    assert_eq!(p.child(1).unwrap().text_content(), "second");
    assert!(p.child(2).is_none());
}

#[test]
fn test_text_node_has_no_children() {
    let t = text("hello");
    assert_eq!(t.child_count(), 0);
    assert!(t.child(0).is_none());
}

#[test]
fn test_void_node_has_no_children() {
    let hr = horizontal_rule();
    assert_eq!(hr.child_count(), 0);
    assert!(hr.child(0).is_none());
}

// ===========================================================================
// Complex document size calculations
// ===========================================================================

#[test]
fn test_list_document_size() {
    // doc > bulletList > listItem > paragraph > "A"
    //
    // text "A":         1
    // paragraph:        1 + 1 + 1 = 3
    // listItem:         1 + 3 + 1 = 5
    // bulletList:       1 + 5 + 1 = 7
    // doc:              1 + 7 + 1 = 9
    let d = doc(vec![bullet_list(vec![list_item(vec![paragraph(vec![
        text("A"),
    ])])])]);
    assert_eq!(d.node_size(), 9);
}

#[test]
fn test_multi_item_list_size() {
    // doc > bulletList > [listItem > paragraph > "A", listItem > paragraph > "B"]
    //
    // listItem("A"): 1 + (1+1+1) + 1 = 5
    // listItem("B"): 1 + (1+1+1) + 1 = 5
    // bulletList:    1 + 5 + 5 + 1 = 12
    // doc:           1 + 12 + 1 = 14
    let d = doc(vec![bullet_list(vec![
        list_item(vec![paragraph(vec![text("A")])]),
        list_item(vec![paragraph(vec![text("B")])]),
    ])]);
    assert_eq!(d.node_size(), 14);
}

#[test]
fn test_doc_with_hr_size() {
    // doc > [paragraph("Hi"), horizontalRule]
    //
    // paragraph("Hi"): 1 + 2 + 1 = 4
    // horizontalRule:  1
    // doc:             1 + 4 + 1 + 1 = 7
    let d = doc(vec![paragraph(vec![text("Hi")]), horizontal_rule()]);
    assert_eq!(d.node_size(), 7);
}

#[test]
fn test_paragraph_with_hard_break_size() {
    // paragraph > ["before", hardBreak, "after"]
    // text "before": 6
    // hardBreak: 1
    // text "after": 5
    // paragraph: 1 + 6 + 1 + 5 + 1 = 14
    let p = paragraph(vec![text("before"), hard_break(), text("after")]);
    assert_eq!(p.node_size(), 14);
}

// ===========================================================================
// ResolvedPos tests
// ===========================================================================

#[test]
fn test_resolve_simple_doc() {
    // doc > paragraph > "Hi"
    // Positions:
    //   0: inside doc, before paragraph
    //   1: inside paragraph, before "H"
    //   2: inside paragraph, between "H" and "i"
    //   3: inside paragraph, after "i"
    //   4: inside doc, after paragraph
    let d = Document::new(doc(vec![paragraph(vec![text("Hi")])]));

    // pos 0: at doc level, before paragraph
    let r = d.resolve(0).expect("resolve(0) should succeed");
    assert_eq!(r.pos, 0, "pos should be 0");
    assert_eq!(r.depth, 1, "depth at pos 0 should be 1 (doc)");
    assert_eq!(r.parent_offset, 0, "parent_offset at pos 0 should be 0");

    // pos 1: inside paragraph, before text
    let r = d.resolve(1).expect("resolve(1) should succeed");
    assert_eq!(r.pos, 1);
    assert_eq!(r.depth, 2, "depth at pos 1 should be 2 (doc > paragraph)");
    assert_eq!(
        r.parent_offset, 0,
        "parent_offset at pos 1 = 0 (start of paragraph content)"
    );

    // pos 2: inside paragraph, between H and i
    let r = d.resolve(2).expect("resolve(2) should succeed");
    assert_eq!(r.pos, 2);
    assert_eq!(r.depth, 2);
    assert_eq!(
        r.parent_offset, 1,
        "parent_offset at pos 2 = 1 (1 char into text)"
    );

    // pos 3: inside paragraph, after "i"
    let r = d.resolve(3).expect("resolve(3) should succeed");
    assert_eq!(r.pos, 3);
    assert_eq!(r.depth, 2);
    assert_eq!(r.parent_offset, 2);

    // pos 4: inside doc, after paragraph
    let r = d.resolve(4).expect("resolve(4) should succeed");
    assert_eq!(r.pos, 4);
    assert_eq!(r.depth, 1);
    assert_eq!(
        r.parent_offset, 4,
        "parent_offset at pos 4 = 4 (after paragraph node)"
    );
}

#[test]
fn test_resolve_out_of_bounds() {
    let d = Document::new(doc(vec![paragraph(vec![text("Hi")])]));
    // content_size = 4, valid positions: 0..=4
    assert!(d.resolve(5).is_err(), "pos 5 should be out of bounds");
    // Large value
    assert!(d.resolve(100).is_err(), "pos 100 should be out of bounds");
}

#[test]
fn test_resolve_spec_example() {
    // <doc><paragraph>Hello <bold>world</bold>!</paragraph></doc>
    //
    // Positions inside doc content (0..14):
    //   0: before paragraph
    //   1: start of paragraph content (before "H")
    //   2-6: within "Hello " (offsets 1-5)
    //   7: after "Hello " / start of "world" (offset 6)
    //   8-11: within "world" (offsets 7-10)
    //   12: after "world" / start of "!" (offset 11)
    //   13: end of paragraph content (offset 12)
    //   14: after paragraph
    let d = Document::new(doc(vec![paragraph(vec![
        text("Hello "),
        text_with_marks("world", vec![bold()]),
        text("!"),
    ])]));

    // pos 0: doc level, before paragraph
    let r = d.resolve(0).unwrap();
    assert_eq!(r.depth, 1);
    assert_eq!(r.parent_offset, 0);

    // pos 1: inside paragraph, at start
    let r = d.resolve(1).unwrap();
    assert_eq!(r.depth, 2);
    assert_eq!(r.parent_offset, 0);

    // pos 7: after "Hello ", at start of "world"
    let r = d.resolve(7).unwrap();
    assert_eq!(r.depth, 2);
    assert_eq!(r.parent_offset, 6, "6 chars of 'Hello ' consumed");

    // pos 12: after "world", at start of "!"
    let r = d.resolve(12).unwrap();
    assert_eq!(r.depth, 2);
    assert_eq!(r.parent_offset, 11, "'Hello '(6) + 'world'(5) = 11");

    // pos 13: end of paragraph content
    let r = d.resolve(13).unwrap();
    assert_eq!(r.depth, 2);
    assert_eq!(
        r.parent_offset, 12,
        "'Hello '(6) + 'world'(5) + '!'(1) = 12"
    );

    // pos 14: doc level, after paragraph
    let r = d.resolve(14).unwrap();
    assert_eq!(r.depth, 1);
    assert_eq!(r.parent_offset, 14);
}

#[test]
fn test_resolve_empty_paragraph() {
    // doc > paragraph (empty)
    // content_size = 2 (paragraph open + close)
    // pos 0: before paragraph
    // pos 1: inside empty paragraph
    // pos 2: after paragraph
    let d = Document::new(doc(vec![paragraph(vec![])]));
    assert_eq!(d.content_size(), 2);

    let r = d.resolve(0).unwrap();
    assert_eq!(r.depth, 1);
    assert_eq!(r.parent_offset, 0);

    let r = d.resolve(1).unwrap();
    assert_eq!(r.depth, 2, "pos 1 is inside empty paragraph");
    assert_eq!(r.parent_offset, 0);

    let r = d.resolve(2).unwrap();
    assert_eq!(r.depth, 1);
    assert_eq!(r.parent_offset, 2);
}

#[test]
fn test_resolve_two_paragraphs() {
    // doc > [paragraph("A"), paragraph("B")]
    //
    // paragraph("A"): size 3 (1+1+1)
    // paragraph("B"): size 3 (1+1+1)
    // doc content_size: 6
    //
    // pos 0: doc level, before first paragraph
    // pos 1: inside first paragraph, before "A"
    // pos 2: inside first paragraph, after "A"
    // pos 3: doc level, between paragraphs
    // pos 4: inside second paragraph, before "B"
    // pos 5: inside second paragraph, after "B"
    // pos 6: doc level, after second paragraph
    let d = Document::new(doc(vec![
        paragraph(vec![text("A")]),
        paragraph(vec![text("B")]),
    ]));
    assert_eq!(d.content_size(), 6);

    let r = d.resolve(0).unwrap();
    assert_eq!(r.depth, 1);
    assert_eq!(r.parent_offset, 0);

    let r = d.resolve(1).unwrap();
    assert_eq!(r.depth, 2);
    assert_eq!(r.parent_offset, 0);

    let r = d.resolve(2).unwrap();
    assert_eq!(r.depth, 2);
    assert_eq!(r.parent_offset, 1);

    let r = d.resolve(3).unwrap();
    assert_eq!(r.depth, 1);
    assert_eq!(r.parent_offset, 3, "after first paragraph (size 3)");

    let r = d.resolve(4).unwrap();
    assert_eq!(r.depth, 2, "inside second paragraph");
    assert_eq!(r.parent_offset, 0);

    let r = d.resolve(5).unwrap();
    assert_eq!(r.depth, 2);
    assert_eq!(r.parent_offset, 1);

    let r = d.resolve(6).unwrap();
    assert_eq!(r.depth, 1);
    assert_eq!(r.parent_offset, 6);
}

#[test]
fn test_resolve_with_void_node() {
    // doc > paragraph > ["text", hardBreak, "more"]
    //
    // "text": 4
    // hardBreak: 1
    // "more": 4
    // paragraph: 1 + 4 + 1 + 4 + 1 = 11
    // doc content_size: 11
    //
    // pos 1: inside paragraph, start
    // pos 5: inside paragraph, offset 4 (after "text"), at hardBreak
    // pos 6: inside paragraph, offset 5 (after hardBreak), start of "more"
    let d = Document::new(doc(vec![paragraph(vec![
        text("text"),
        hard_break(),
        text("more"),
    ])]));

    let r = d.resolve(5).unwrap();
    assert_eq!(r.depth, 2, "at hardBreak position, inside paragraph");
    assert_eq!(r.parent_offset, 4, "4 chars of 'text' before hardBreak");

    let r = d.resolve(6).unwrap();
    assert_eq!(r.depth, 2);
    assert_eq!(r.parent_offset, 5, "4 chars + 1 hardBreak = 5");
}

#[test]
fn test_resolve_nested_list() {
    // doc > bulletList > listItem > paragraph > "X"
    //
    // text "X":       1
    // paragraph:      1 + 1 + 1 = 3
    // listItem:       1 + 3 + 1 = 5
    // bulletList:     1 + 5 + 1 = 7
    // doc content:    7
    //
    // pos 0: doc level, before bulletList
    // pos 1: inside bulletList, before listItem
    // pos 2: inside listItem, before paragraph
    // pos 3: inside paragraph, before "X"
    // pos 4: inside paragraph, after "X"
    // pos 5: inside listItem, after paragraph
    // pos 6: inside bulletList, after listItem
    // pos 7: doc level, after bulletList
    let d = Document::new(doc(vec![bullet_list(vec![list_item(vec![paragraph(
        vec![text("X")],
    )])])]));
    assert_eq!(d.content_size(), 7);

    let r = d.resolve(0).unwrap();
    assert_eq!(r.depth, 1);
    assert_eq!(r.parent_offset, 0);

    let r = d.resolve(1).unwrap();
    assert_eq!(r.depth, 2, "inside bulletList");
    assert_eq!(r.parent_offset, 0);

    let r = d.resolve(2).unwrap();
    assert_eq!(r.depth, 3, "inside listItem");
    assert_eq!(r.parent_offset, 0);

    let r = d.resolve(3).unwrap();
    assert_eq!(r.depth, 4, "inside paragraph");
    assert_eq!(r.parent_offset, 0);

    let r = d.resolve(4).unwrap();
    assert_eq!(r.depth, 4);
    assert_eq!(r.parent_offset, 1);

    let r = d.resolve(5).unwrap();
    assert_eq!(r.depth, 3, "back in listItem, after paragraph");
    assert_eq!(r.parent_offset, 3);

    let r = d.resolve(6).unwrap();
    assert_eq!(r.depth, 2, "back in bulletList, after listItem");
    assert_eq!(r.parent_offset, 5);

    let r = d.resolve(7).unwrap();
    assert_eq!(r.depth, 1, "back in doc, after bulletList");
    assert_eq!(r.parent_offset, 7);
}

#[test]
fn test_resolve_with_horizontal_rule() {
    // doc > [paragraph("A"), horizontalRule, paragraph("B")]
    //
    // paragraph("A"): 3
    // horizontalRule:  1
    // paragraph("B"): 3
    // doc content:    7
    //
    // pos 3: doc level, after first paragraph, before hr
    // pos 4: doc level, after hr, before second paragraph
    let d = Document::new(doc(vec![
        paragraph(vec![text("A")]),
        horizontal_rule(),
        paragraph(vec![text("B")]),
    ]));
    assert_eq!(d.content_size(), 7);

    // pos 3: between paragraph and hr
    let r = d.resolve(3).unwrap();
    assert_eq!(r.depth, 1, "doc level, at horizontal rule position");
    assert_eq!(r.parent_offset, 3);

    // pos 4: after hr, before second paragraph
    let r = d.resolve(4).unwrap();
    assert_eq!(r.depth, 1);
    assert_eq!(r.parent_offset, 4);

    // pos 5: inside second paragraph
    let r = d.resolve(5).unwrap();
    assert_eq!(r.depth, 2);
    assert_eq!(r.parent_offset, 0);
}

#[test]
fn test_resolve_node_path() {
    // doc > bulletList > listItem > paragraph > "X"
    let d = Document::new(doc(vec![bullet_list(vec![list_item(vec![paragraph(
        vec![text("X")],
    )])])]));

    // pos 3: inside paragraph (depth 4: doc > bulletList > listItem > paragraph)
    let r = d.resolve(3).unwrap();
    assert_eq!(r.depth, 4);
    // node_path should be [0, 0, 0] meaning:
    //   child 0 of doc (bulletList), child 0 of bulletList (listItem),
    //   child 0 of listItem (paragraph)
    assert_eq!(r.node_path.as_slice(), &[0, 0, 0]);
}

#[test]
fn test_resolve_node_path_second_child() {
    // doc > [paragraph("A"), paragraph("B")]
    let d = Document::new(doc(vec![
        paragraph(vec![text("A")]),
        paragraph(vec![text("B")]),
    ]));

    // pos 4: inside second paragraph
    let r = d.resolve(4).unwrap();
    assert_eq!(r.depth, 2);
    assert_eq!(
        r.node_path.as_slice(),
        &[1],
        "second paragraph is child index 1 of doc"
    );
}

// ===========================================================================
// Unicode edge cases in position resolution
// ===========================================================================

#[test]
fn test_resolve_unicode_emoji() {
    // doc > paragraph > family emoji (7 scalars)
    let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}\u{200D}\u{1F466}";
    let d = Document::new(doc(vec![paragraph(vec![text(family)])]));
    // paragraph content size = 7 (7 scalars)
    // doc content_size = 1 + 7 + 1 = 9
    assert_eq!(d.content_size(), 9);

    // pos 1: start of paragraph content
    let r = d.resolve(1).unwrap();
    assert_eq!(r.depth, 2);
    assert_eq!(r.parent_offset, 0);

    // pos 4: 3 scalars into the emoji
    let r = d.resolve(4).unwrap();
    assert_eq!(r.depth, 2);
    assert_eq!(r.parent_offset, 3);

    // pos 8: end of paragraph content (7 scalars)
    let r = d.resolve(8).unwrap();
    assert_eq!(r.depth, 2);
    assert_eq!(r.parent_offset, 7);
}

// ===========================================================================
// ResolvedPos parent accessor
// ===========================================================================

#[test]
fn test_resolve_parent_node() {
    // doc > paragraph > "Hello"
    let d = Document::new(doc(vec![paragraph(vec![text("Hello")])]));

    // pos 3: inside paragraph
    let r = d.resolve(3).unwrap();
    let parent = r.parent(&d);
    assert_eq!(parent.node_type(), "paragraph");

    // pos 0: inside doc
    let r = d.resolve(0).unwrap();
    let parent = r.parent(&d);
    assert_eq!(parent.node_type(), "doc");
}

// ===========================================================================
// Document: node_at
// ===========================================================================

#[test]
fn test_document_node_at() {
    // Verify that node_at retrieves a node by following the path indices
    let d = Document::new(doc(vec![
        paragraph(vec![text("first")]),
        paragraph(vec![text("second")]),
    ]));

    // Node at path [0] should be the first paragraph
    let node = d.node_at(&[0]).unwrap();
    assert_eq!(node.node_type(), "paragraph");
    assert_eq!(node.text_content(), "first");

    // Node at path [1] should be the second paragraph
    let node = d.node_at(&[1]).unwrap();
    assert_eq!(node.node_type(), "paragraph");
    assert_eq!(node.text_content(), "second");

    // Empty path = root
    let node = d.node_at(&[]).unwrap();
    assert_eq!(node.node_type(), "doc");

    // Invalid path
    assert!(d.node_at(&[5]).is_none());
}
