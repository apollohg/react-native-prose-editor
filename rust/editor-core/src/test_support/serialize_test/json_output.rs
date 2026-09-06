fn pm_schema() -> Schema {
    crate::prosemirror_schema()
}

#[test]
fn test_to_json_plain_paragraph() {
    let d = doc(vec![paragraph(vec![text("Hello")])]);
    let json = to_prosemirror_json(&d, &schema());
    assert_eq!(json["type"], "doc");
    assert_eq!(json["content"][0]["type"], "paragraph");
    assert_eq!(json["content"][0]["content"][0]["type"], "text");
    assert_eq!(json["content"][0]["content"][0]["text"], "Hello");
    // No marks field for plain text
    assert!(
        json["content"][0]["content"][0].get("marks").is_none(),
        "plain text should not have marks field"
    );
}

#[test]
fn test_to_json_bold_text_with_marks_array() {
    let d = doc(vec![paragraph(vec![text_with_marks(
        "Hello",
        vec![bold()],
    )])]);
    let json = to_prosemirror_json(&d, &schema());
    let text_node = &json["content"][0]["content"][0];
    assert_eq!(text_node["type"], "text");
    assert_eq!(text_node["text"], "Hello");
    let marks = text_node["marks"]
        .as_array()
        .expect("marks should be an array");
    assert_eq!(marks.len(), 1, "should have exactly one mark");
    assert_eq!(marks[0]["type"], "bold");
}

#[test]
fn test_to_json_multiple_marks_on_text() {
    let d = doc(vec![paragraph(vec![text_with_marks(
        "styled",
        vec![bold(), italic(), underline()],
    )])]);
    let json = to_prosemirror_json(&d, &schema());
    let text_node = &json["content"][0]["content"][0];
    let marks = text_node["marks"].as_array().unwrap();
    assert_eq!(marks.len(), 3, "should have three marks");
    let mark_types: Vec<&str> = marks.iter().map(|m| m["type"].as_str().unwrap()).collect();
    assert!(mark_types.contains(&"bold"), "marks should include bold");
    assert!(
        mark_types.contains(&"italic"),
        "marks should include italic"
    );
    assert!(
        mark_types.contains(&"underline"),
        "marks should include underline"
    );
}

#[test]
fn test_to_json_bullet_list_uses_tiptap_schema_name() {
    let d = doc(vec![bullet_list(vec![list_item(vec![paragraph(vec![
        text("A"),
    ])])])]);
    let json = to_prosemirror_json(&d, &schema());
    assert_eq!(
        json["content"][0]["type"], "bulletList",
        "tiptap schema should use camelCase 'bulletList'"
    );
    assert_eq!(json["content"][0]["content"][0]["type"], "listItem");
}

#[test]
fn test_to_json_bullet_list_uses_prosemirror_schema_name() {
    // Build document with ProseMirror-style names
    let bl = Node::element(
        "bullet_list".to_string(),
        HashMap::new(),
        Fragment::from(vec![Node::element(
            "list_item".to_string(),
            HashMap::new(),
            Fragment::from(vec![paragraph(vec![text("A")])]),
        )]),
    );
    let d = Document::new(Node::element(
        "doc".to_string(),
        HashMap::new(),
        Fragment::from(vec![bl]),
    ));
    let json = to_prosemirror_json(&d, &pm_schema());
    assert_eq!(
        json["content"][0]["type"], "bullet_list",
        "prosemirror schema should use snake_case 'bullet_list'"
    );
    assert_eq!(json["content"][0]["content"][0]["type"], "list_item");
}

#[test]
fn test_to_json_ordered_list_with_start_attr() {
    let d = doc(vec![ordered_list(
        3,
        vec![list_item(vec![paragraph(vec![text("A")])])],
    )]);
    let json = to_prosemirror_json(&d, &schema());
    let ol = &json["content"][0];
    assert_eq!(ol["type"], "orderedList");
    assert_eq!(
        ol["attrs"]["start"], 3,
        "start=3 should be in attrs since it differs from default"
    );
}

#[test]
fn test_to_json_ordered_list_default_start_omitted() {
    let d = doc(vec![ordered_list(
        1,
        vec![list_item(vec![paragraph(vec![text("A")])])],
    )]);
    let json = to_prosemirror_json(&d, &schema());
    let ol = &json["content"][0];
    // start=1 is the default, so attrs should be omitted entirely
    assert!(
        ol.get("attrs").is_none(),
        "default start=1 should result in no attrs object, got: {:?}",
        ol.get("attrs")
    );
}

#[test]
fn test_to_json_hard_break_void_node() {
    let d = doc(vec![paragraph(vec![text("A"), hard_break(), text("B")])]);
    let json = to_prosemirror_json(&d, &schema());
    let p = &json["content"][0];
    assert_eq!(p["content"][0]["type"], "text");
    assert_eq!(p["content"][1]["type"], "hardBreak");
    // Void nodes should have no "content" or "text" fields
    assert!(
        p["content"][1].get("content").is_none(),
        "void node should not have content field"
    );
    assert!(
        p["content"][1].get("text").is_none(),
        "void node should not have text field"
    );
    assert_eq!(p["content"][2]["type"], "text");
}

#[test]
fn test_to_json_horizontal_rule_void_node() {
    let d = doc(vec![
        paragraph(vec![text("Above")]),
        horizontal_rule(),
        paragraph(vec![text("Below")]),
    ]);
    let json = to_prosemirror_json(&d, &schema());
    assert_eq!(json["content"][1]["type"], "horizontalRule");
    assert!(json["content"][1].get("content").is_none());
    assert!(json["content"][1].get("text").is_none());
}

#[test]
fn test_to_json_empty_paragraph() {
    let d = doc(vec![paragraph(vec![])]);
    let json = to_prosemirror_json(&d, &schema());
    let p = &json["content"][0];
    assert_eq!(p["type"], "paragraph");
    // Empty paragraph should have no content field
    assert!(
        p.get("content").is_none(),
        "empty paragraph should omit content field, got: {:?}",
        p.get("content")
    );
}

#[test]
fn test_to_json_mark_with_attrs() {
    // Create a link mark with href attr
    let link_mark = Mark::new("link".to_string(), {
        let mut attrs = HashMap::new();
        attrs.insert(
            "href".to_string(),
            serde_json::Value::String("https://example.com".to_string()),
        );
        attrs
    });
    let d = doc(vec![paragraph(vec![text_with_marks(
        "click me",
        vec![link_mark],
    )])]);
    let json = to_prosemirror_json(&d, &schema());
    let text_node = &json["content"][0]["content"][0];
    let marks = text_node["marks"].as_array().unwrap();
    assert_eq!(marks[0]["type"], "link");
    assert_eq!(marks[0]["attrs"]["href"], "https://example.com");
}
