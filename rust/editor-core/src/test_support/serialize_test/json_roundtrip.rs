// Round-trip tests: from_json(to_json(doc)) equivalence

/// Assert round-trip: serializing a document to JSON and parsing it back
/// produces the same tree structure.
fn assert_json_roundtrip(original: &Document, schema: &Schema, label: &str) {
    let json = to_prosemirror_json(original, schema);
    let parsed = from_prosemirror_json(&json, schema, UnknownTypeMode::Error).unwrap_or_else(|e| {
        panic!(
            "from_prosemirror_json failed for {}: {} (json: {})",
            label, e, json
        )
    });
    assert_tree_eq(
        original.root(),
        parsed.root(),
        &format!("json_rt:{}", label),
    );
}

#[test]
fn test_json_roundtrip_plain_paragraph() {
    let d = doc(vec![paragraph(vec![text("Hello")])]);
    assert_json_roundtrip(&d, &schema(), "plain_paragraph");
}

#[test]
fn test_json_roundtrip_bold_text() {
    let d = doc(vec![paragraph(vec![text_with_marks(
        "Hello",
        vec![bold()],
    )])]);
    assert_json_roundtrip(&d, &schema(), "bold_text");
}

#[test]
fn test_json_roundtrip_mixed_marks() {
    let d = doc(vec![paragraph(vec![
        text("H"),
        text_with_marks("ell", vec![bold(), italic()]),
        text("o"),
    ])]);
    assert_json_roundtrip(&d, &schema(), "mixed_marks");
}

#[test]
fn test_json_roundtrip_all_four_marks() {
    let d = doc(vec![paragraph(vec![text_with_marks(
        "all",
        vec![bold(), italic(), underline(), strike()],
    )])]);
    assert_json_roundtrip(&d, &schema(), "all_four_marks");
}

#[test]
fn test_json_roundtrip_bullet_list() {
    let d = doc(vec![bullet_list(vec![
        list_item(vec![paragraph(vec![text("A")])]),
        list_item(vec![paragraph(vec![text("B")])]),
    ])]);
    assert_json_roundtrip(&d, &schema(), "bullet_list");
}

#[test]
fn test_json_roundtrip_ordered_list_start_3() {
    let d = doc(vec![ordered_list(
        3,
        vec![list_item(vec![paragraph(vec![text("A")])])],
    )]);
    assert_json_roundtrip(&d, &schema(), "ordered_list_start_3");
}

#[test]
fn test_json_roundtrip_ordered_list_start_1() {
    let d = doc(vec![ordered_list(
        1,
        vec![list_item(vec![paragraph(vec![text("A")])])],
    )]);
    assert_json_roundtrip(&d, &schema(), "ordered_list_start_1");
}

#[test]
fn test_json_roundtrip_hard_break() {
    let d = doc(vec![paragraph(vec![text("A"), hard_break(), text("B")])]);
    assert_json_roundtrip(&d, &schema(), "hard_break");
}

#[test]
fn test_json_roundtrip_horizontal_rule() {
    let d = doc(vec![
        paragraph(vec![text("Above")]),
        horizontal_rule(),
        paragraph(vec![text("Below")]),
    ]);
    assert_json_roundtrip(&d, &schema(), "horizontal_rule");
}

#[test]
fn test_json_roundtrip_empty_paragraph() {
    let d = doc(vec![paragraph(vec![])]);
    assert_json_roundtrip(&d, &schema(), "empty_paragraph");
}

#[test]
fn test_json_roundtrip_multiple_paragraphs() {
    let d = doc(vec![
        paragraph(vec![text("First")]),
        paragraph(vec![text("Second")]),
        paragraph(vec![text("Third")]),
    ]);
    assert_json_roundtrip(&d, &schema(), "multiple_paragraphs");
}

#[test]
fn test_json_roundtrip_complex_document() {
    let d = doc(vec![
        paragraph(vec![
            text("Hello "),
            text_with_marks("world", vec![bold()]),
            text("!"),
        ]),
        bullet_list(vec![
            list_item(vec![paragraph(vec![text("Item one")])]),
            list_item(vec![paragraph(vec![
                text_with_marks("Item ", vec![italic()]),
                text_with_marks("two", vec![italic(), bold()]),
            ])]),
        ]),
        horizontal_rule(),
        paragraph(vec![text("End.")]),
    ]);
    assert_json_roundtrip(&d, &schema(), "complex_document");
}

fn pm_doc(children: Vec<Node>) -> Document {
    Document::new(Node::element(
        "doc".to_string(),
        HashMap::new(),
        Fragment::from(children),
    ))
}

fn pm_paragraph(children: Vec<Node>) -> Node {
    Node::element(
        "paragraph".to_string(),
        HashMap::new(),
        Fragment::from(children),
    )
}

fn pm_bullet_list(children: Vec<Node>) -> Node {
    Node::element(
        "bullet_list".to_string(),
        HashMap::new(),
        Fragment::from(children),
    )
}

fn pm_ordered_list(start: u64, children: Vec<Node>) -> Node {
    let mut attrs = HashMap::new();
    attrs.insert("start".to_string(), serde_json::Value::Number(start.into()));
    Node::element("ordered_list".to_string(), attrs, Fragment::from(children))
}

fn pm_list_item(children: Vec<Node>) -> Node {
    Node::element(
        "list_item".to_string(),
        HashMap::new(),
        Fragment::from(children),
    )
}

fn pm_hard_break() -> Node {
    Node::void("hard_break".to_string(), HashMap::new())
}

fn pm_horizontal_rule() -> Node {
    Node::void("horizontal_rule".to_string(), HashMap::new())
}

#[test]
fn test_json_roundtrip_prosemirror_plain_paragraph() {
    let d = pm_doc(vec![pm_paragraph(vec![text("Hello")])]);
    assert_json_roundtrip(&d, &pm_schema(), "pm_plain_paragraph");
}

#[test]
fn test_json_roundtrip_prosemirror_bold_text() {
    let d = pm_doc(vec![pm_paragraph(vec![text_with_marks(
        "Hello",
        vec![bold()],
    )])]);
    assert_json_roundtrip(&d, &pm_schema(), "pm_bold_text");
}

#[test]
fn test_json_roundtrip_prosemirror_bullet_list() {
    let d = pm_doc(vec![pm_bullet_list(vec![
        pm_list_item(vec![pm_paragraph(vec![text("A")])]),
        pm_list_item(vec![pm_paragraph(vec![text("B")])]),
    ])]);
    assert_json_roundtrip(&d, &pm_schema(), "pm_bullet_list");
}

#[test]
fn test_json_roundtrip_prosemirror_ordered_list() {
    let d = pm_doc(vec![pm_ordered_list(
        5,
        vec![pm_list_item(vec![pm_paragraph(vec![text("A")])])],
    )]);
    assert_json_roundtrip(&d, &pm_schema(), "pm_ordered_list");
}

#[test]
fn test_json_roundtrip_prosemirror_hard_break() {
    let d = pm_doc(vec![pm_paragraph(vec![
        text("A"),
        pm_hard_break(),
        text("B"),
    ])]);
    assert_json_roundtrip(&d, &pm_schema(), "pm_hard_break");
}

#[test]
fn test_json_roundtrip_prosemirror_horizontal_rule() {
    let d = pm_doc(vec![
        pm_paragraph(vec![text("Above")]),
        pm_horizontal_rule(),
        pm_paragraph(vec![text("Below")]),
    ]);
    assert_json_roundtrip(&d, &pm_schema(), "pm_horizontal_rule");
}

#[test]
fn test_json_roundtrip_prosemirror_complex() {
    let d = pm_doc(vec![
        pm_paragraph(vec![text("Hello "), text_with_marks("world", vec![bold()])]),
        pm_bullet_list(vec![
            pm_list_item(vec![pm_paragraph(vec![text("A")])]),
            pm_list_item(vec![pm_paragraph(vec![text_with_marks(
                "B",
                vec![italic(), strike()],
            )])]),
        ]),
        pm_horizontal_rule(),
        pm_ordered_list(
            3,
            vec![pm_list_item(vec![pm_paragraph(vec![text("Third")])])],
        ),
    ]);
    assert_json_roundtrip(&d, &pm_schema(), "pm_complex");
}

// JSON string round-trip: to_json produces correct JSON, from_json parses it

#[test]
fn test_json_string_roundtrip_verify_output_format() {
    let d = doc(vec![paragraph(vec![
        text("Hello "),
        text_with_marks("world", vec![bold()]),
    ])]);
    let json = to_prosemirror_json(&d, &schema());

    // Verify exact JSON structure
    assert_eq!(json["type"], "doc");
    let content = json["content"].as_array().unwrap();
    assert_eq!(content.len(), 1);
    let p = &content[0];
    assert_eq!(p["type"], "paragraph");
    let p_content = p["content"].as_array().unwrap();
    assert_eq!(p_content.len(), 2);

    assert_eq!(p_content[0]["type"], "text");
    assert_eq!(p_content[0]["text"], "Hello ");
    assert!(p_content[0].get("marks").is_none());

    assert_eq!(p_content[1]["type"], "text");
    assert_eq!(p_content[1]["text"], "world");
    let marks = p_content[1]["marks"].as_array().unwrap();
    assert_eq!(marks.len(), 1);
    assert_eq!(marks[0]["type"], "bold");
    assert!(marks[0].get("attrs").is_none(), "bold mark has no attrs");

    // Re-parse and verify equivalence
    let parsed = from_prosemirror_json(&json, &schema(), UnknownTypeMode::Error).unwrap();
    assert_tree_eq(d.root(), parsed.root(), "json_string_roundtrip");
}

// Cross-format round-trip: JSON -> Doc -> HTML -> Doc -> JSON equivalence

#[test]
fn test_cross_format_roundtrip_json_to_html_and_back() {
    let json_input = serde_json::json!({
        "type": "doc",
        "content": [
            {
                "type": "paragraph",
                "content": [
                    { "type": "text", "text": "Hello " },
                    { "type": "text", "text": "world", "marks": [{ "type": "bold" }] }
                ]
            },
            {
                "type": "bulletList",
                "content": [{
                    "type": "listItem",
                    "content": [{
                        "type": "paragraph",
                        "content": [{ "type": "text", "text": "item" }]
                    }]
                }]
            }
        ]
    });
    let s = schema();

    // JSON -> Document
    let doc1 = from_prosemirror_json(&json_input, &s, UnknownTypeMode::Error).unwrap();

    // Document -> HTML -> Document
    let html = to_html(&doc1, &s);
    let doc2 = from_html(&html, &s, &default_opts()).unwrap();

    // Document -> JSON
    let json_output = to_prosemirror_json(&doc2, &s);

    // Re-parse JSON output
    let doc3 = from_prosemirror_json(&json_output, &s, UnknownTypeMode::Error).unwrap();

    // All three documents should be structurally identical
    assert_tree_eq(doc1.root(), doc2.root(), "cross_format:json->html->doc");
    assert_tree_eq(
        doc1.root(),
        doc3.root(),
        "cross_format:json->html->json->doc",
    );
}
