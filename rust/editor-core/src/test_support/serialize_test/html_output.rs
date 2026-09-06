#[test]
fn test_to_html_plain_paragraph() {
    let d = doc(vec![paragraph(vec![text("Hello")])]);
    let html = to_html(&d, &schema());
    assert_eq!(
        html, "<p>Hello</p>",
        "plain paragraph should emit <p>Hello</p>"
    );
}

#[test]
fn test_to_html_bold_text() {
    let d = doc(vec![paragraph(vec![text_with_marks(
        "Hello",
        vec![bold()],
    )])]);
    let html = to_html(&d, &schema());
    assert_eq!(
        html, "<p><strong>Hello</strong></p>",
        "bold text should wrap in <strong>"
    );
}

#[test]
fn test_to_html_italic_text() {
    let d = doc(vec![paragraph(vec![text_with_marks(
        "Hello",
        vec![italic()],
    )])]);
    let html = to_html(&d, &schema());
    assert_eq!(html, "<p><em>Hello</em></p>");
}

#[test]
fn test_to_html_underline_text() {
    let d = doc(vec![paragraph(vec![text_with_marks(
        "Hello",
        vec![underline()],
    )])]);
    let html = to_html(&d, &schema());
    assert_eq!(html, "<p><u>Hello</u></p>");
}

#[test]
fn test_to_html_strike_text() {
    let d = doc(vec![paragraph(vec![text_with_marks(
        "Hello",
        vec![strike()],
    )])]);
    let html = to_html(&d, &schema());
    assert_eq!(html, "<p><s>Hello</s></p>");
}

#[test]
fn test_to_html_link_text() {
    let d = Document::new(Node::element(
        "doc".to_string(),
        HashMap::new(),
        Fragment::from(vec![paragraph(vec![text_with_marks(
            "OpenAI",
            vec![link("https://openai.com")],
        )])]),
    ));
    let html = to_html(&d, &schema());
    assert_eq!(html, "<p><a href=\"https://openai.com\">OpenAI</a></p>");
}

#[test]
fn test_to_html_image_node() {
    let d = doc(vec![image(
        "https://example.com/cat.png",
        Some("Cat"),
        Some("Preview"),
    )]);
    let html = to_html(&d, &schema());
    assert!(html.starts_with("<img "));
    assert!(html.contains("src=\"https://example.com/cat.png\""));
    assert!(html.contains("alt=\"Cat\""));
    assert!(html.contains("title=\"Preview\""));
    assert!(html.ends_with('>'));
}

#[test]
fn test_to_html_image_node_with_dimensions() {
    let d = doc(vec![image_with_dimensions(
        "https://example.com/cat.png",
        Some("Cat"),
        Some("Preview"),
        Some(320),
        Some(180),
    )]);
    let html = to_html(&d, &schema());
    assert!(html.starts_with("<img "));
    assert!(html.contains("src=\"https://example.com/cat.png\""));
    assert!(html.contains("width=\"320\""));
    assert!(html.contains("height=\"180\""));
}

#[test]
fn test_to_html_mixed_marks() {
    // "H" plain, "ell" bold+italic, "o" plain
    let d = doc(vec![paragraph(vec![
        text("H"),
        text_with_marks("ell", vec![bold(), italic()]),
        text("o"),
    ])]);
    let html = to_html(&d, &schema());
    assert_eq!(
        html, "<p>H<strong><em>ell</em></strong>o</p>",
        "nested marks should produce nested tags"
    );
}

#[test]
fn test_to_html_bullet_list() {
    let d = doc(vec![bullet_list(vec![
        list_item(vec![paragraph(vec![text("A")])]),
        list_item(vec![paragraph(vec![text("B")])]),
    ])]);
    let html = to_html(&d, &schema());
    assert_eq!(html, "<ul><li><p>A</p></li><li><p>B</p></li></ul>");
}

#[test]
fn test_to_html_blockquote() {
    let d = doc(vec![blockquote(vec![
        paragraph(vec![text("Hello")]),
        paragraph(vec![text("World")]),
    ])]);
    let html = to_html(&d, &schema());
    assert_eq!(html, "<blockquote><p>Hello</p><p>World</p></blockquote>");
}

#[test]
fn test_to_html_ordered_list_start_1() {
    let d = doc(vec![ordered_list(
        1,
        vec![list_item(vec![paragraph(vec![text("A")])])],
    )]);
    let html = to_html(&d, &schema());
    assert_eq!(
        html, "<ol><li><p>A</p></li></ol>",
        "ordered list with start=1 should omit the start attribute"
    );
}

#[test]
fn test_to_html_ordered_list_start_3() {
    let d = doc(vec![ordered_list(
        3,
        vec![list_item(vec![paragraph(vec![text("A")])])],
    )]);
    let html = to_html(&d, &schema());
    assert_eq!(
        html, "<ol start=\"3\"><li><p>A</p></li></ol>",
        "ordered list with start=3 should include start attribute"
    );
}

#[test]
fn test_from_html_blockquote() {
    let document = from_html(
        "<blockquote><p>Hello</p><p>World</p></blockquote>",
        &schema(),
        &default_opts(),
    )
    .expect("blockquote html should parse");

    let quote = document.root().child(0).expect("blockquote child");
    assert_eq!(quote.node_type(), "blockquote");
    assert_eq!(quote.child_count(), 2);
    assert_eq!(
        quote.child(0).expect("first paragraph").text_content(),
        "Hello"
    );
    assert_eq!(
        quote.child(1).expect("second paragraph").text_content(),
        "World"
    );
}

#[test]
fn test_to_html_hard_break() {
    let d = doc(vec![paragraph(vec![text("He"), hard_break(), text("llo")])]);
    let html = to_html(&d, &schema());
    assert_eq!(html, "<p>He<br>llo</p>");
}

#[test]
fn test_to_html_horizontal_rule_between_paragraphs() {
    let d = doc(vec![
        paragraph(vec![text("Above")]),
        horizontal_rule(),
        paragraph(vec![text("Below")]),
    ]);
    let html = to_html(&d, &schema());
    assert_eq!(html, "<p>Above</p><hr><p>Below</p>");
}

#[test]
fn test_to_html_mention_serializes_native_editor_roundtrip_span() {
    let d = doc(vec![paragraph(vec![
        text("Hello "),
        mention(&[
            ("id", serde_json::Value::String("u1".to_string())),
            ("kind", serde_json::Value::String("user".to_string())),
            ("label", serde_json::Value::String("@Alice".to_string())),
        ]),
        text("!"),
    ])]);

    let html = to_html(&d, &mention_schema());
    assert!(
        html.contains("data-native-editor-mention=\"true\""),
        "mention HTML should include the native mention marker, got: {html}"
    );
    assert!(
        html.contains("data-native-editor-mention-attrs="),
        "mention HTML should include serialized attrs, got: {html}"
    );
    assert!(
        html.contains("@Alice"),
        "mention HTML should render the visible label, got: {html}"
    );
}

#[test]
fn test_to_html_mention_applies_suggestion_trigger_to_bare_label() {
    let d = doc(vec![paragraph(vec![
        text("Hello "),
        mention(&[
            ("id", serde_json::Value::String("u1".to_string())),
            (
                "mentionSuggestionChar",
                serde_json::Value::String("@".to_string()),
            ),
            ("label", serde_json::Value::String("Alice".to_string())),
        ]),
        text("!"),
    ])]);

    let html = to_html(&d, &mention_schema());
    assert!(
        html.contains(">@Alice</span>"),
        "mention HTML should render the trigger-prefixed visible label, got: {html}"
    );
    assert!(
        html.contains("&quot;label&quot;:&quot;Alice&quot;"),
        "mention attrs should preserve the original bare label, got: {html}"
    );
}

#[test]
fn test_to_html_empty_paragraph() {
    let d = doc(vec![paragraph(vec![])]);
    let html = to_html(&d, &schema());
    assert_eq!(html, "<p></p>");
}

#[test]
fn test_to_html_all_four_marks_combined() {
    let d = doc(vec![paragraph(vec![text_with_marks(
        "all",
        vec![bold(), italic(), underline(), strike()],
    )])]);
    let html = to_html(&d, &schema());
    assert_eq!(
        html, "<p><strong><em><u><s>all</s></u></em></strong></p>",
        "all four marks should nest in order"
    );
}

#[test]
fn test_to_html_escapes_special_characters() {
    let d = doc(vec![paragraph(vec![text(
        "<script>alert(\"xss\")&</script>",
    )])]);
    let html = to_html(&d, &schema());
    assert_eq!(
        html, "<p>&lt;script&gt;alert(&quot;xss&quot;)&amp;&lt;/script&gt;</p>",
        "special HTML characters should be escaped"
    );
}

#[test]
fn test_to_html_multiple_paragraphs() {
    let d = doc(vec![
        paragraph(vec![text("First")]),
        paragraph(vec![text("Second")]),
    ]);
    let html = to_html(&d, &schema());
    assert_eq!(html, "<p>First</p><p>Second</p>");
}
