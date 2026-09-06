#[test]
fn test_from_html_plain_paragraph() {
    let d = from_html("<p>Hello</p>", &schema(), &default_opts()).unwrap();
    let root = d.root();
    assert_eq!(root.child_count(), 1, "doc should have 1 child");
    let p = root.child(0).unwrap();
    assert_eq!(p.node_type(), "paragraph");
    assert_eq!(p.child_count(), 1);
    assert_eq!(p.child(0).unwrap().text_str().unwrap(), "Hello");
}

#[test]
fn test_from_html_bold_text() {
    let d = from_html("<p><strong>Hello</strong></p>", &schema(), &default_opts()).unwrap();
    let p = d.root().child(0).unwrap();
    let text_node = p.child(0).unwrap();
    assert_eq!(text_node.text_str().unwrap(), "Hello");
    assert_eq!(text_node.marks().len(), 1);
    assert_eq!(text_node.marks()[0].mark_type(), "bold");
}

#[test]
fn test_from_html_italic_text() {
    let d = from_html("<p><em>Hello</em></p>", &schema(), &default_opts()).unwrap();
    let p = d.root().child(0).unwrap();
    let text_node = p.child(0).unwrap();
    assert_eq!(text_node.marks().len(), 1);
    assert_eq!(text_node.marks()[0].mark_type(), "italic");
}

#[test]
fn test_from_html_underline_text() {
    let d = from_html("<p><u>Hello</u></p>", &schema(), &default_opts()).unwrap();
    let p = d.root().child(0).unwrap();
    let text_node = p.child(0).unwrap();
    assert_eq!(text_node.marks().len(), 1);
    assert_eq!(text_node.marks()[0].mark_type(), "underline");
}

#[test]
fn test_from_html_strike_text() {
    let d = from_html("<p><s>Hello</s></p>", &schema(), &default_opts()).unwrap();
    let p = d.root().child(0).unwrap();
    let text_node = p.child(0).unwrap();
    assert_eq!(text_node.marks().len(), 1);
    assert_eq!(text_node.marks()[0].mark_type(), "strike");
}

#[test]
fn test_from_html_link_text() {
    let d = from_html(
        "<p><a href=\"https://openai.com\">OpenAI</a></p>",
        &schema(),
        &default_opts(),
    )
    .unwrap();
    let paragraph = d.root().child(0).expect("paragraph");
    let text = paragraph.child(0).expect("text");
    let marks = text.marks();
    assert_eq!(marks.len(), 1);
    assert_eq!(marks[0].mark_type(), "link");
    assert_eq!(
        marks[0].attrs().get("href"),
        Some(&serde_json::Value::String("https://openai.com".to_string()))
    );
}

#[test]
fn test_from_html_mixed_marks() {
    let d = from_html(
        "<p>H<strong><em>ell</em></strong>o</p>",
        &schema(),
        &default_opts(),
    )
    .unwrap();
    let p = d.root().child(0).unwrap();
    assert_eq!(p.child_count(), 3, "paragraph should have 3 text nodes");

    let h = p.child(0).unwrap();
    assert_eq!(h.text_str().unwrap(), "H");
    assert!(h.marks().is_empty());

    let ell = p.child(1).unwrap();
    assert_eq!(ell.text_str().unwrap(), "ell");
    assert_eq!(
        ell.marks().len(),
        2,
        "middle text should have bold+italic marks"
    );
    let mark_names: Vec<&str> = ell.marks().iter().map(|m| m.mark_type()).collect();
    assert!(mark_names.contains(&"bold"), "should have bold mark");
    assert!(mark_names.contains(&"italic"), "should have italic mark");

    let o = p.child(2).unwrap();
    assert_eq!(o.text_str().unwrap(), "o");
    assert!(o.marks().is_empty());
}

#[test]
fn test_from_html_bullet_list() {
    let d = from_html(
        "<ul><li><p>A</p></li><li><p>B</p></li></ul>",
        &schema(),
        &default_opts(),
    )
    .unwrap();
    let list = d.root().child(0).unwrap();
    assert_eq!(list.node_type(), "bulletList");
    assert_eq!(list.child_count(), 2, "bullet list should have 2 items");

    let li0 = list.child(0).unwrap();
    assert_eq!(li0.node_type(), "listItem");
    assert_eq!(li0.child(0).unwrap().node_type(), "paragraph");
    assert_eq!(li0.child(0).unwrap().text_content(), "A");

    let li1 = list.child(1).unwrap();
    assert_eq!(li1.child(0).unwrap().text_content(), "B");
}

#[test]
fn test_from_html_ordered_list_with_start() {
    let d = from_html(
        "<ol start=\"3\"><li><p>A</p></li></ol>",
        &schema(),
        &default_opts(),
    )
    .unwrap();
    let list = d.root().child(0).unwrap();
    assert_eq!(list.node_type(), "orderedList");
    let start = list.attrs().get("start").unwrap();
    assert_eq!(
        start,
        &serde_json::Value::Number(3.into()),
        "start should be 3"
    );
}

#[test]
fn test_from_html_ordered_list_default_start() {
    let d = from_html("<ol><li><p>A</p></li></ol>", &schema(), &default_opts()).unwrap();
    let list = d.root().child(0).unwrap();
    assert_eq!(list.node_type(), "orderedList");
    let start = list.attrs().get("start").unwrap();
    assert_eq!(
        start,
        &serde_json::Value::Number(1.into()),
        "default start should be 1"
    );
}

#[test]
fn test_from_html_hard_break() {
    let d = from_html("<p>He<br>llo</p>", &schema(), &default_opts()).unwrap();
    let p = d.root().child(0).unwrap();
    assert_eq!(p.child_count(), 3, "paragraph should have text, br, text");

    assert_eq!(p.child(0).unwrap().text_str().unwrap(), "He");
    assert!(p.child(1).unwrap().is_void());
    assert_eq!(p.child(1).unwrap().node_type(), "hardBreak");
    assert_eq!(p.child(2).unwrap().text_str().unwrap(), "llo");
}

#[test]
fn test_from_html_native_mention_roundtrip_preserves_custom_attrs() {
    let d = from_html(
        "<p>Hello <span data-native-editor-mention=\"true\" data-native-editor-mention-attrs=\"{&quot;id&quot;:&quot;u1&quot;,&quot;kind&quot;:&quot;user&quot;,&quot;label&quot;:&quot;@Alice&quot;}\">@Alice</span>!</p>",
        &mention_schema(),
        &default_opts(),
    )
    .unwrap();

    let p = d.root().child(0).unwrap();
    assert_eq!(
        p.child_count(),
        3,
        "paragraph should have text, mention, text"
    );
    let mention = p.child(1).unwrap();
    assert!(
        mention.is_void(),
        "mention should be parsed as a void inline node"
    );
    assert_eq!(mention.node_type(), "mention");
    assert_eq!(
        mention.attrs().get("id"),
        Some(&serde_json::Value::String("u1".to_string()))
    );
    assert_eq!(
        mention.attrs().get("kind"),
        Some(&serde_json::Value::String("user".to_string()))
    );
    assert_eq!(
        mention.attrs().get("label"),
        Some(&serde_json::Value::String("@Alice".to_string()))
    );

    let roundtrip_html = to_html(&d, &mention_schema());
    assert!(
        roundtrip_html.contains("data-native-editor-mention=\"true\""),
        "round-tripped mention HTML should preserve the native marker, got: {roundtrip_html}"
    );
    assert!(
        roundtrip_html.contains("@Alice"),
        "round-tripped mention HTML should preserve the visible label, got: {roundtrip_html}"
    );
}

#[test]
fn test_from_html_horizontal_rule() {
    let d = from_html("<p>Above</p><hr><p>Below</p>", &schema(), &default_opts()).unwrap();
    let root = d.root();
    assert_eq!(root.child_count(), 3, "doc should have para, hr, para");
    assert_eq!(root.child(0).unwrap().node_type(), "paragraph");
    assert_eq!(root.child(1).unwrap().node_type(), "horizontalRule");
    assert!(root.child(1).unwrap().is_void());
    assert_eq!(root.child(2).unwrap().node_type(), "paragraph");
}

#[test]
fn test_from_html_image_node() {
    let d = from_html(
        "<img src=\"https://example.com/cat.png\" alt=\"Cat\" title=\"Preview\">",
        &schema(),
        &default_opts(),
    )
    .unwrap();
    let root = d.root();
    assert_eq!(root.child_count(), 1, "doc should have one image node");
    let image_node = root.child(0).unwrap();
    assert_eq!(image_node.node_type(), "image");
    assert!(image_node.is_void());
    assert_eq!(
        image_node.attrs().get("src"),
        Some(&serde_json::Value::String(
            "https://example.com/cat.png".to_string()
        ))
    );
    assert_eq!(
        image_node.attrs().get("alt"),
        Some(&serde_json::Value::String("Cat".to_string()))
    );
    assert_eq!(
        image_node.attrs().get("title"),
        Some(&serde_json::Value::String("Preview".to_string()))
    );
    assert_eq!(image_node.attrs().get("width"), None);
    assert_eq!(image_node.attrs().get("height"), None);
}

#[test]
fn test_from_html_image_node_with_dimensions() {
    let d = from_html(
        "<img src=\"https://example.com/cat.png\" alt=\"Cat\" width=\"320\" height=\"180\">",
        &schema(),
        &default_opts(),
    )
    .unwrap();
    let image_node = d.root().child(0).unwrap();
    assert_eq!(image_node.node_type(), "image");
    assert_eq!(
        image_node.attrs().get("width"),
        Some(&serde_json::Value::Number(320u64.into()))
    );
    assert_eq!(
        image_node.attrs().get("height"),
        Some(&serde_json::Value::Number(180u64.into()))
    );
}

#[test]
fn test_from_html_base64_image_requires_opt_in() {
    let html = "<img src=\"data:image/png;base64,AAAA\" alt=\"Inline\">";

    let parsed_without_opt_in = from_html(html, &schema(), &default_opts()).unwrap();
    assert_eq!(
        parsed_without_opt_in.root().child(0).unwrap().node_type(),
        "__opaque"
    );

    let parsed_with_opt_in = from_html(
        html,
        &schema(),
        &FromHtmlOptions {
            strict: false,
            allow_base64_images: true,
        },
    )
    .unwrap();
    let image_node = parsed_with_opt_in.root().child(0).unwrap();
    assert_eq!(image_node.node_type(), "image");
    assert!(image_node.is_void());
    assert_eq!(
        image_node.attrs().get("src"),
        Some(&serde_json::Value::String(
            "data:image/png;base64,AAAA".to_string()
        ))
    );
    assert_eq!(
        image_node.attrs().get("alt"),
        Some(&serde_json::Value::String("Inline".to_string()))
    );
    assert_eq!(image_node.attrs().get("title"), None);
}

#[test]
fn test_from_html_empty_paragraph() {
    let d = from_html("<p></p>", &schema(), &default_opts()).unwrap();
    let p = d.root().child(0).unwrap();
    assert_eq!(p.node_type(), "paragraph");
    assert_eq!(
        p.child_count(),
        0,
        "empty paragraph should have no children"
    );
}

#[test]
fn test_from_html_b_tag_to_bold() {
    let d = from_html("<p><b>Hello</b></p>", &schema(), &default_opts()).unwrap();
    let text_node = d.root().child(0).unwrap().child(0).unwrap();
    assert_eq!(
        text_node.marks()[0].mark_type(),
        "bold",
        "<b> should map to bold"
    );
}

#[test]
fn test_from_html_i_tag_to_italic() {
    let d = from_html("<p><i>Hello</i></p>", &schema(), &default_opts()).unwrap();
    let text_node = d.root().child(0).unwrap().child(0).unwrap();
    assert_eq!(
        text_node.marks()[0].mark_type(),
        "italic",
        "<i> should map to italic"
    );
}

#[test]
fn test_from_html_del_tag_to_strike() {
    let d = from_html("<p><del>Hello</del></p>", &schema(), &default_opts()).unwrap();
    let text_node = d.root().child(0).unwrap().child(0).unwrap();
    assert_eq!(
        text_node.marks()[0].mark_type(),
        "strike",
        "<del> should map to strike"
    );
}

#[test]
fn test_from_html_strike_tag_to_strike() {
    let d = from_html("<p><strike>Hello</strike></p>", &schema(), &default_opts()).unwrap();
    let text_node = d.root().child(0).unwrap().child(0).unwrap();
    assert_eq!(
        text_node.marks()[0].mark_type(),
        "strike",
        "<strike> should map to strike"
    );
}

#[test]
fn test_from_html_bare_text_auto_wrapped() {
    let d = from_html("Hello world", &schema(), &default_opts()).unwrap();
    let root = d.root();
    assert_eq!(
        root.child_count(),
        1,
        "bare text should be wrapped in a paragraph"
    );
    let p = root.child(0).unwrap();
    assert_eq!(p.node_type(), "paragraph");
    assert_eq!(p.text_content(), "Hello world");
}

#[test]
fn test_from_html_unknown_inline_tag_preserved_as_opaque() {
    let d = from_html(
        "<p>Hello <span>world</span></p>",
        &schema(),
        &default_opts(),
    )
    .unwrap();
    let p = d.root().child(0).unwrap();
    // Should have text "Hello ", opaque node for <span>, in some form
    // The opaque node should preserve the tag name
    let mut found_opaque = false;
    for i in 0..p.child_count() {
        let child = p.child(i).unwrap();
        if child.node_type() == "__opaque" {
            found_opaque = true;
            let tag = child.attrs().get("html_tag").unwrap().as_str().unwrap();
            assert_eq!(tag, "span", "opaque node should preserve tag name");
            let text = child.attrs().get("text_content").unwrap().as_str().unwrap();
            assert_eq!(text, "world", "opaque node should preserve text content");
        }
    }
    assert!(
        found_opaque,
        "unknown <span> should be preserved as opaque node"
    );
}

#[test]
fn test_opaque_nested_attributes_are_safely_serialized() {
    let s = schema();
    let input = r#"<mystery><nested title='quoted &amp; &quot;value&quot;' data-break='&quot; onmouseover=&quot;alert(1)'>safe &amp; sound</nested></mystery>"#;
    let doc = from_html(input, &s, &default_opts()).unwrap();
    let output = to_html(&doc, &s);

    assert!(output.contains("title=\"quoted &amp; &quot;value&quot;\""));
    assert!(output.contains("data-break=\"&quot; onmouseover=&quot;alert(1)\""));
    assert!(!output.contains("data-break=\"\" onmouseover="));
    assert!(output.contains("safe &amp; sound"));

    let reparsed = from_html(&output, &s, &default_opts()).unwrap();
    let second_output = to_html(&reparsed, &s);
    assert!(second_output.contains("title=\"quoted &amp; &quot;value&quot;\""));
    assert!(second_output.contains("data-break=\"&quot; onmouseover=&quot;alert(1)\""));
    assert!(!second_output.contains("data-break=\"\" onmouseover="));
    assert!(second_output.contains("safe &amp; sound"));
}

#[test]
fn opaque_html_attribute_values_are_escaped_on_output() {
    let s = schema();
    let doc = from_html(
        r#"<p>before <widget title="&quot; onmouseover=&quot;x">inside</widget></p>"#,
        &s,
        &FromHtmlOptions::default(),
    )
    .expect("opaque HTML should parse");

    let output = to_html(&doc, &s);
    assert!(output.contains("title=\"&quot; onmouseover=&quot;x\""));
    assert!(!output.contains("title=\"\" onmouseover=\"x\""));
}

#[test]
fn test_from_html_unknown_block_tag_preserved_as_opaque() {
    let d = from_html("<div>content</div>", &schema(), &default_opts()).unwrap();
    let root = d.root();
    let mut found_opaque = false;
    for i in 0..root.child_count() {
        let child = root.child(i).unwrap();
        if child.node_type() == "__opaque" {
            found_opaque = true;
            let tag = child.attrs().get("html_tag").unwrap().as_str().unwrap();
            assert_eq!(tag, "div");
        }
    }
    assert!(
        found_opaque,
        "unknown <div> should be preserved as opaque node"
    );
}

#[test]
fn test_from_html_strict_mode_rejects_unknown_tag() {
    let result = from_html("<p><span>text</span></p>", &schema(), &strict_opts());
    assert!(result.is_err(), "strict mode should reject unknown tags");
    if let Err(e) = result {
        let msg = e.to_string();
        assert!(
            msg.contains("span"),
            "error message should mention the unknown tag, got: {}",
            msg
        );
    }
}
