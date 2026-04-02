use std::collections::HashMap;

use editor_core::model::{Document, Fragment, Mark, Node};
use editor_core::render::generate::generate;
use editor_core::render::incremental::incremental;
use editor_core::render::{ListContext, RenderElement, RenderMark};
use editor_core::tiptap_schema;

// ---------------------------------------------------------------------------
// Helper builders (reuse patterns from model_test.rs)
// ---------------------------------------------------------------------------

fn bold() -> Mark {
    Mark::new("bold".to_string(), HashMap::new())
}

fn italic() -> Mark {
    Mark::new("italic".to_string(), HashMap::new())
}

fn render_mark(mark_type: &str) -> RenderMark {
    RenderMark {
        mark_type: mark_type.to_string(),
        attrs: HashMap::new(),
    }
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

fn ordered_list(start: u32, children: Vec<Node>) -> Node {
    let mut attrs = HashMap::new();
    attrs.insert("start".to_string(), serde_json::Value::Number(start.into()));
    Node::element("orderedList".to_string(), attrs, Fragment::from(children))
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

// ---------------------------------------------------------------------------
// Test 1: Plain paragraph
// <p>Hello</p> -> [BlockStart("paragraph"), TextRun("Hello", []), BlockEnd]
// ---------------------------------------------------------------------------
#[test]
fn test_plain_paragraph() {
    let schema = tiptap_schema();
    let root = doc(vec![paragraph(vec![text("Hello")])]);
    let document = Document::new(root);

    let elements = generate(&document, &schema);

    assert_eq!(
        elements,
        vec![
            RenderElement::BlockStart {
                node_type: "paragraph".to_string(),
                depth: 0,
                list_context: None,
            },
            RenderElement::TextRun {
                text: "Hello".to_string(),
                marks: vec![],
            },
            RenderElement::BlockEnd,
        ],
        "Plain paragraph should produce BlockStart, TextRun, BlockEnd"
    );
}

// ---------------------------------------------------------------------------
// Test 2: Formatted text with partial bold
// <p>H<b>ell</b>o</p>
// ---------------------------------------------------------------------------
#[test]
fn test_formatted_text_partial_bold() {
    let schema = tiptap_schema();
    let root = doc(vec![paragraph(vec![
        text("H"),
        text_with_marks("ell", vec![bold()]),
        text("o"),
    ])]);
    let document = Document::new(root);

    let elements = generate(&document, &schema);

    assert_eq!(
        elements,
        vec![
            RenderElement::BlockStart {
                node_type: "paragraph".to_string(),
                depth: 0,
                list_context: None,
            },
            RenderElement::TextRun {
                text: "H".to_string(),
                marks: vec![],
            },
            RenderElement::TextRun {
                text: "ell".to_string(),
                marks: vec![render_mark("bold")],
            },
            RenderElement::TextRun {
                text: "o".to_string(),
                marks: vec![],
            },
            RenderElement::BlockEnd,
        ],
        "Partial bold should produce three TextRun elements with correct marks"
    );
}

// ---------------------------------------------------------------------------
// Test 3: Multiple marks (bold + italic)
// <p><b><i>text</i></b></p>
// ---------------------------------------------------------------------------
#[test]
fn test_multiple_marks_bold_italic() {
    let schema = tiptap_schema();
    let root = doc(vec![paragraph(vec![text_with_marks(
        "text",
        vec![bold(), italic()],
    )])]);
    let document = Document::new(root);

    let elements = generate(&document, &schema);

    assert_eq!(
        elements,
        vec![
            RenderElement::BlockStart {
                node_type: "paragraph".to_string(),
                depth: 0,
                list_context: None,
            },
            RenderElement::TextRun {
                text: "text".to_string(),
                marks: vec![render_mark("bold"), render_mark("italic")],
            },
            RenderElement::BlockEnd,
        ],
        "Text with bold+italic marks should have both mark names in TextRun"
    );
}

// ---------------------------------------------------------------------------
// Test 4: Two paragraphs
// ---------------------------------------------------------------------------
#[test]
fn test_two_paragraphs() {
    let schema = tiptap_schema();
    let root = doc(vec![
        paragraph(vec![text("First")]),
        paragraph(vec![text("Second")]),
    ]);
    let document = Document::new(root);

    let elements = generate(&document, &schema);

    assert_eq!(
        elements,
        vec![
            RenderElement::BlockStart {
                node_type: "paragraph".to_string(),
                depth: 0,
                list_context: None,
            },
            RenderElement::TextRun {
                text: "First".to_string(),
                marks: vec![],
            },
            RenderElement::BlockEnd,
            RenderElement::BlockStart {
                node_type: "paragraph".to_string(),
                depth: 0,
                list_context: None,
            },
            RenderElement::TextRun {
                text: "Second".to_string(),
                marks: vec![],
            },
            RenderElement::BlockEnd,
        ],
        "Two paragraphs should produce two complete BlockStart/BlockEnd sequences"
    );
}

// ---------------------------------------------------------------------------
// Test 5: Bullet list with two items
// ---------------------------------------------------------------------------
#[test]
fn test_bullet_list_two_items() {
    let schema = tiptap_schema();
    let root = doc(vec![bullet_list(vec![
        list_item(vec![paragraph(vec![text("Item 1")])]),
        list_item(vec![paragraph(vec![text("Item 2")])]),
    ])]);
    let document = Document::new(root);

    let elements = generate(&document, &schema);

    assert_eq!(
        elements,
        vec![
            // First list item
            RenderElement::BlockStart {
                node_type: "listItem".to_string(),
                depth: 0,
                list_context: Some(ListContext {
                    ordered: false,
                    index: 1,
                    total: 2,
                    start: 1,
                    is_first: true,
                    is_last: false,
                }),
            },
            RenderElement::BlockStart {
                node_type: "paragraph".to_string(),
                depth: 1,
                list_context: None,
            },
            RenderElement::TextRun {
                text: "Item 1".to_string(),
                marks: vec![],
            },
            RenderElement::BlockEnd,
            RenderElement::BlockEnd,
            // Second list item
            RenderElement::BlockStart {
                node_type: "listItem".to_string(),
                depth: 0,
                list_context: Some(ListContext {
                    ordered: false,
                    index: 2,
                    total: 2,
                    start: 1,
                    is_first: false,
                    is_last: true,
                }),
            },
            RenderElement::BlockStart {
                node_type: "paragraph".to_string(),
                depth: 1,
                list_context: None,
            },
            RenderElement::TextRun {
                text: "Item 2".to_string(),
                marks: vec![],
            },
            RenderElement::BlockEnd,
            RenderElement::BlockEnd,
        ],
        "Bullet list should emit listItem BlockStart with ListContext for each item"
    );
}

// ---------------------------------------------------------------------------
// Test 6: Ordered list with start=3
// ---------------------------------------------------------------------------
#[test]
fn test_ordered_list_start_3() {
    let schema = tiptap_schema();
    let root = doc(vec![ordered_list(
        3,
        vec![
            list_item(vec![paragraph(vec![text("Three")])]),
            list_item(vec![paragraph(vec![text("Four")])]),
        ],
    )]);
    let document = Document::new(root);

    let elements = generate(&document, &schema);

    // First item should have index=3 (start + 0), second index=4 (start + 1)
    let first_block = &elements[0];
    assert_eq!(
        *first_block,
        RenderElement::BlockStart {
            node_type: "listItem".to_string(),
            depth: 0,
            list_context: Some(ListContext {
                ordered: true,
                index: 3,
                total: 2,
                start: 3,
                is_first: true,
                is_last: false,
            }),
        },
        "First item in ordered list starting at 3 should have index=3"
    );

    // Find the second listItem BlockStart
    let second_block = elements
        .iter()
        .filter(|e| {
            matches!(
                e,
                RenderElement::BlockStart {
                    node_type,
                    list_context: Some(_),
                    ..
                } if node_type == "listItem"
            )
        })
        .nth(1)
        .expect("Should have a second listItem BlockStart");

    assert_eq!(
        *second_block,
        RenderElement::BlockStart {
            node_type: "listItem".to_string(),
            depth: 0,
            list_context: Some(ListContext {
                ordered: true,
                index: 4,
                total: 2,
                start: 3,
                is_first: false,
                is_last: true,
            }),
        },
        "Second item in ordered list starting at 3 should have index=4"
    );
}

// ---------------------------------------------------------------------------
// Test 7: HardBreak in paragraph
// <p>He<br>llo</p>
// ---------------------------------------------------------------------------
#[test]
fn test_hard_break_in_paragraph() {
    let schema = tiptap_schema();
    let root = doc(vec![paragraph(vec![text("He"), hard_break(), text("llo")])]);
    let document = Document::new(root);

    let elements = generate(&document, &schema);

    assert_eq!(
        elements,
        vec![
            RenderElement::BlockStart {
                node_type: "paragraph".to_string(),
                depth: 0,
                list_context: None,
            },
            RenderElement::TextRun {
                text: "He".to_string(),
                marks: vec![],
            },
            RenderElement::VoidInline {
                node_type: "hardBreak".to_string(),
                doc_pos: 3, // after paragraph open (1) + "He" (2)
            },
            RenderElement::TextRun {
                text: "llo".to_string(),
                marks: vec![],
            },
            RenderElement::BlockEnd,
        ],
        "HardBreak should produce VoidInline between TextRuns"
    );
}

// ---------------------------------------------------------------------------
// Test 8: HorizontalRule between paragraphs
// ---------------------------------------------------------------------------
#[test]
fn test_horizontal_rule_between_paragraphs() {
    let schema = tiptap_schema();
    let root = doc(vec![
        paragraph(vec![text("Above")]),
        horizontal_rule(),
        paragraph(vec![text("Below")]),
    ]);
    let document = Document::new(root);

    let elements = generate(&document, &schema);

    assert_eq!(
        elements,
        vec![
            RenderElement::BlockStart {
                node_type: "paragraph".to_string(),
                depth: 0,
                list_context: None,
            },
            RenderElement::TextRun {
                text: "Above".to_string(),
                marks: vec![],
            },
            RenderElement::BlockEnd,
            RenderElement::VoidBlock {
                node_type: "horizontalRule".to_string(),
                doc_pos: 7, // after paragraph (1+5+1 = 7 tokens)
            },
            RenderElement::BlockStart {
                node_type: "paragraph".to_string(),
                depth: 0,
                list_context: None,
            },
            RenderElement::TextRun {
                text: "Below".to_string(),
                marks: vec![],
            },
            RenderElement::BlockEnd,
        ],
        "HorizontalRule should produce VoidBlock between paragraph sequences"
    );
}

// ---------------------------------------------------------------------------
// Test 9: Empty paragraph
// ---------------------------------------------------------------------------
#[test]
fn test_empty_paragraph() {
    let schema = tiptap_schema();
    let root = doc(vec![paragraph(vec![])]);
    let document = Document::new(root);

    let elements = generate(&document, &schema);

    assert_eq!(
        elements,
        vec![
            RenderElement::BlockStart {
                node_type: "paragraph".to_string(),
                depth: 0,
                list_context: None,
            },
            RenderElement::TextRun {
                text: "\u{200B}".to_string(),
                marks: vec![],
            },
            RenderElement::BlockEnd,
        ],
        "Empty paragraph should produce an invisible placeholder TextRun between BlockStart and BlockEnd"
    );
}

// ---------------------------------------------------------------------------
// Test 10: Incremental - modify first block only
// ---------------------------------------------------------------------------
#[test]
fn test_incremental_regenerate_first_block_only() {
    let schema = tiptap_schema();
    let root = doc(vec![
        paragraph(vec![text("Modified")]),
        paragraph(vec![text("Unchanged")]),
        paragraph(vec![text("Also unchanged")]),
    ]);
    let document = Document::new(root);

    // Only regenerate block 0
    let patches = incremental(&document, &schema, &[0]);

    assert_eq!(
        patches.len(),
        1,
        "Incremental should return exactly 1 patch for 1 affected index"
    );
    assert_eq!(patches[0].0, 0, "Patch should be for block index 0");
    assert_eq!(
        patches[0].1,
        vec![
            RenderElement::BlockStart {
                node_type: "paragraph".to_string(),
                depth: 0,
                list_context: None,
            },
            RenderElement::TextRun {
                text: "Modified".to_string(),
                marks: vec![],
            },
            RenderElement::BlockEnd,
        ],
        "Incremental patch for block 0 should match full generation for that block"
    );
}

// ---------------------------------------------------------------------------
// Test 11: Incremental - consistency with full generation
// ---------------------------------------------------------------------------
#[test]
fn test_incremental_matches_full_generation() {
    let schema = tiptap_schema();
    let root = doc(vec![
        paragraph(vec![text("First")]),
        paragraph(vec![text("Second")]),
        paragraph(vec![text("Third")]),
    ]);
    let document = Document::new(root);

    let full = generate(&document, &schema);
    let patches = incremental(&document, &schema, &[0, 1, 2]);

    // Flatten patches and compare to full generation
    let mut from_patches: Vec<RenderElement> = Vec::new();
    for (_, elements) in &patches {
        from_patches.extend(elements.iter().cloned());
    }

    assert_eq!(
        from_patches, full,
        "Regenerating all blocks incrementally should match full generation"
    );
}

// ---------------------------------------------------------------------------
// Test 12: Incremental - middle block only
// ---------------------------------------------------------------------------
#[test]
fn test_incremental_middle_block() {
    let schema = tiptap_schema();
    let root = doc(vec![
        paragraph(vec![text("A")]),
        paragraph(vec![text("B")]),
        paragraph(vec![text("C")]),
    ]);
    let document = Document::new(root);

    let patches = incremental(&document, &schema, &[1]);

    assert_eq!(patches.len(), 1, "Should have exactly 1 patch");
    assert_eq!(patches[0].0, 1, "Patch should be for block index 1");
    assert_eq!(
        patches[0].1,
        vec![
            RenderElement::BlockStart {
                node_type: "paragraph".to_string(),
                depth: 0,
                list_context: None,
            },
            RenderElement::TextRun {
                text: "B".to_string(),
                marks: vec![],
            },
            RenderElement::BlockEnd,
        ],
        "Middle block patch should have correct content"
    );
}

// ---------------------------------------------------------------------------
// Test 13: Nested list depth tracking
// ---------------------------------------------------------------------------
#[test]
fn test_list_item_paragraph_depth() {
    let schema = tiptap_schema();
    // bulletList > listItem > paragraph
    // The listItem is depth 0, paragraph inside listItem is depth 1
    let root = doc(vec![bullet_list(vec![list_item(vec![paragraph(vec![
        text("nested"),
    ])])])]);
    let document = Document::new(root);

    let elements = generate(&document, &schema);

    // listItem depth=0, paragraph depth=1
    assert_eq!(
        elements[0],
        RenderElement::BlockStart {
            node_type: "listItem".to_string(),
            depth: 0,
            list_context: Some(ListContext {
                ordered: false,
                index: 1,
                total: 1,
                start: 1,
                is_first: true,
                is_last: true,
            }),
        },
        "listItem should be at depth 0"
    );
    assert_eq!(
        elements[1],
        RenderElement::BlockStart {
            node_type: "paragraph".to_string(),
            depth: 1,
            list_context: None,
        },
        "paragraph inside listItem should be at depth 1"
    );
}

// ---------------------------------------------------------------------------
// Test 14: VoidBlock doc_pos is correct with multiple preceding blocks
// ---------------------------------------------------------------------------
#[test]
fn test_void_block_doc_pos_accuracy() {
    let schema = tiptap_schema();
    // Two paragraphs then a horizontal rule
    // p("AB") = 1 + 2 + 1 = 4 tokens
    // p("CD") = 1 + 2 + 1 = 4 tokens
    // hr = 1 token, at pos 8
    let root = doc(vec![
        paragraph(vec![text("AB")]),
        paragraph(vec![text("CD")]),
        horizontal_rule(),
    ]);
    let document = Document::new(root);

    let elements = generate(&document, &schema);

    let hr_element = elements
        .iter()
        .find(|e| matches!(e, RenderElement::VoidBlock { .. }))
        .expect("Should have a VoidBlock element");

    assert_eq!(
        *hr_element,
        RenderElement::VoidBlock {
            node_type: "horizontalRule".to_string(),
            doc_pos: 8,
        },
        "HorizontalRule should be at doc_pos 8 (after two 4-token paragraphs)"
    );
}

// ---------------------------------------------------------------------------
// Test 15: VoidInline doc_pos is correct
// ---------------------------------------------------------------------------
#[test]
fn test_void_inline_doc_pos_accuracy() {
    let schema = tiptap_schema();
    // p("ABC" + hardBreak + "DE")
    // paragraph open = pos 0, "ABC" occupies 3 chars
    // hardBreak at pos 0 + 1 (open) + 3 = 4
    let root = doc(vec![paragraph(vec![text("ABC"), hard_break(), text("DE")])]);
    let document = Document::new(root);

    let elements = generate(&document, &schema);

    let hb_element = elements
        .iter()
        .find(|e| matches!(e, RenderElement::VoidInline { .. }))
        .expect("Should have a VoidInline element");

    assert_eq!(
        *hb_element,
        RenderElement::VoidInline {
            node_type: "hardBreak".to_string(),
            doc_pos: 4,
        },
        "HardBreak should be at doc_pos 4 (paragraph open=1 + 'ABC'=3)"
    );
}

// ---------------------------------------------------------------------------
// Test 16: Incremental patches are sorted by block index
// ---------------------------------------------------------------------------
#[test]
fn test_incremental_patches_sorted() {
    let schema = tiptap_schema();
    let root = doc(vec![
        paragraph(vec![text("A")]),
        paragraph(vec![text("B")]),
        paragraph(vec![text("C")]),
        paragraph(vec![text("D")]),
    ]);
    let document = Document::new(root);

    // Request out of order
    let patches = incremental(&document, &schema, &[3, 1]);

    let indices: Vec<usize> = patches.iter().map(|(i, _)| *i).collect();
    assert_eq!(
        indices,
        vec![1, 3],
        "Patches should be sorted by block index even when requested out of order"
    );
}

// ---------------------------------------------------------------------------
// Test 17: Ordered list default start (no attr)
// ---------------------------------------------------------------------------
#[test]
fn test_ordered_list_default_start() {
    let schema = tiptap_schema();
    // orderedList with no explicit start attr (should default to 1)
    let ol = Node::element(
        "orderedList".to_string(),
        HashMap::new(), // no start attr
        Fragment::from(vec![list_item(vec![paragraph(vec![text("First")])])]),
    );
    let root = doc(vec![ol]);
    let document = Document::new(root);

    let elements = generate(&document, &schema);

    let first_li = &elements[0];
    assert_eq!(
        *first_li,
        RenderElement::BlockStart {
            node_type: "listItem".to_string(),
            depth: 0,
            list_context: Some(ListContext {
                ordered: true,
                index: 1,
                total: 1,
                start: 1,
                is_first: true,
                is_last: true,
            }),
        },
        "Ordered list with no start attr should default start to 1"
    );
}

// ---------------------------------------------------------------------------
// Test 18: Empty document (doc with no children)
// ---------------------------------------------------------------------------
#[test]
fn test_empty_document() {
    let schema = tiptap_schema();
    let root = doc(vec![]);
    let document = Document::new(root);

    let elements = generate(&document, &schema);

    assert!(
        elements.is_empty(),
        "Empty document should produce no render elements"
    );
}

// ---------------------------------------------------------------------------
// Test 19: Multiple marks ordering preserved
// ---------------------------------------------------------------------------
#[test]
fn test_mark_ordering_preserved() {
    let schema = tiptap_schema();
    let underline = Mark::new("underline".to_string(), HashMap::new());
    let root = doc(vec![paragraph(vec![text_with_marks(
        "styled",
        vec![italic(), bold(), underline],
    )])]);
    let document = Document::new(root);

    let elements = generate(&document, &schema);

    match &elements[1] {
        RenderElement::TextRun { marks, .. } => {
            assert_eq!(
                marks,
                &vec![
                    render_mark("italic"),
                    render_mark("bold"),
                    render_mark("underline")
                ],
                "Mark names should preserve the order from the node's marks"
            );
        }
        _ => panic!("Expected TextRun at index 1"),
    }
}

// ---------------------------------------------------------------------------
// Test 20: Complex document - list + paragraphs + hr
// ---------------------------------------------------------------------------
#[test]
fn test_complex_document_mixed_content() {
    let schema = tiptap_schema();
    let root = doc(vec![
        paragraph(vec![text("Intro")]),
        bullet_list(vec![
            list_item(vec![paragraph(vec![text("A")])]),
            list_item(vec![paragraph(vec![text("B")])]),
        ]),
        horizontal_rule(),
        paragraph(vec![text("End")]),
    ]);
    let document = Document::new(root);

    let elements = generate(&document, &schema);

    // Count element types
    let block_starts: usize = elements
        .iter()
        .filter(|e| matches!(e, RenderElement::BlockStart { .. }))
        .count();
    let block_ends: usize = elements
        .iter()
        .filter(|e| matches!(e, RenderElement::BlockEnd))
        .count();
    let text_runs: usize = elements
        .iter()
        .filter(|e| matches!(e, RenderElement::TextRun { .. }))
        .count();
    let void_blocks: usize = elements
        .iter()
        .filter(|e| matches!(e, RenderElement::VoidBlock { .. }))
        .count();

    assert_eq!(
        block_starts, block_ends,
        "BlockStart count should match BlockEnd count"
    );
    // 2 paragraphs at top level + 2 listItems + 2 paragraphs inside listItems = 6
    assert_eq!(block_starts, 6, "Should have 6 BlockStart elements");
    // "Intro", "A", "B", "End" = 4 text runs
    assert_eq!(text_runs, 4, "Should have 4 TextRun elements");
    assert_eq!(void_blocks, 1, "Should have 1 VoidBlock (horizontalRule)");
}

// ---------------------------------------------------------------------------
// Test 21: Incremental on list block
// ---------------------------------------------------------------------------
#[test]
fn test_incremental_list_block() {
    let schema = tiptap_schema();
    let root = doc(vec![
        paragraph(vec![text("Before")]),
        bullet_list(vec![
            list_item(vec![paragraph(vec![text("Item 1")])]),
            list_item(vec![paragraph(vec![text("Item 2")])]),
        ]),
        paragraph(vec![text("After")]),
    ]);
    let document = Document::new(root);

    // Regenerate only the list (block index 1)
    let patches = incremental(&document, &schema, &[1]);

    assert_eq!(patches.len(), 1, "Should have exactly 1 patch");
    assert_eq!(patches[0].0, 1, "Patch should be for block index 1");

    // The list block should produce listItem/paragraph elements
    let list_elements = &patches[0].1;
    let li_starts: usize = list_elements
        .iter()
        .filter(
            |e| matches!(e, RenderElement::BlockStart { node_type, .. } if node_type == "listItem"),
        )
        .count();
    assert_eq!(
        li_starts, 2,
        "List patch should have 2 listItem BlockStart elements"
    );
}
