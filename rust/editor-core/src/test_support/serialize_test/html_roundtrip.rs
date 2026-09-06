// Round-trip tests: from_html(to_html(doc)) structure equivalence

/// Assert that two documents have the same tree structure (type names, text,
/// marks). We compare recursively since we don't have PartialEq on Node.
fn assert_tree_eq(a: &Node, b: &Node, path: &str) {
    assert_eq!(
        a.node_type(),
        b.node_type(),
        "node_type mismatch at {}: {:?} vs {:?}",
        path,
        a.node_type(),
        b.node_type()
    );

    // Text content
    if a.is_text() {
        assert_eq!(
            a.text_str().unwrap(),
            b.text_str().unwrap(),
            "text mismatch at {}",
            path
        );
        // Compare marks by type name (order matters for nesting)
        let a_marks: Vec<&str> = a.marks().iter().map(|m| m.mark_type()).collect();
        let b_marks: Vec<&str> = b.marks().iter().map(|m| m.mark_type()).collect();
        assert_eq!(a_marks, b_marks, "marks mismatch at {}", path);
    }

    // Void nodes
    if a.is_void() && b.is_void() {
        // For opaque nodes, compare tag attrs
        if a.node_type() == "__opaque" {
            assert_eq!(
                a.attrs().get("html_tag"),
                b.attrs().get("html_tag"),
                "opaque html_tag mismatch at {}",
                path
            );
        }
        return;
    }

    // Compare attrs for ordered lists (start attribute)
    if a.attrs().contains_key("start") || b.attrs().contains_key("start") {
        assert_eq!(
            a.attrs().get("start"),
            b.attrs().get("start"),
            "start attr mismatch at {}",
            path
        );
    }

    // Children
    assert_eq!(
        a.child_count(),
        b.child_count(),
        "child_count mismatch at {} (type={}): {} vs {}",
        path,
        a.node_type(),
        a.child_count(),
        b.child_count()
    );
    for i in 0..a.child_count() {
        let child_path = format!("{}/{}", path, i);
        assert_tree_eq(a.child(i).unwrap(), b.child(i).unwrap(), &child_path);
    }
}

fn assert_doc_eq(a: &Document, b: &Document) {
    assert_tree_eq(a.root(), b.root(), "doc");
}

#[test]
fn test_roundtrip_plain_paragraph() {
    let original = doc(vec![paragraph(vec![text("Hello")])]);
    let html = to_html(&original, &schema());
    let parsed = from_html(&html, &schema(), &default_opts()).unwrap();
    assert_doc_eq(&original, &parsed);
}

#[test]
fn test_roundtrip_bold_text() {
    let original = doc(vec![paragraph(vec![text_with_marks(
        "Hello",
        vec![bold()],
    )])]);
    let html = to_html(&original, &schema());
    let parsed = from_html(&html, &schema(), &default_opts()).unwrap();
    assert_doc_eq(&original, &parsed);
}

#[test]
fn test_roundtrip_mixed_marks() {
    let original = doc(vec![paragraph(vec![
        text("H"),
        text_with_marks("ell", vec![bold(), italic()]),
        text("o"),
    ])]);
    let html = to_html(&original, &schema());
    let parsed = from_html(&html, &schema(), &default_opts()).unwrap();
    assert_doc_eq(&original, &parsed);
}

#[test]
fn test_roundtrip_bullet_list() {
    let original = doc(vec![bullet_list(vec![
        list_item(vec![paragraph(vec![text("A")])]),
        list_item(vec![paragraph(vec![text("B")])]),
    ])]);
    let html = to_html(&original, &schema());
    let parsed = from_html(&html, &schema(), &default_opts()).unwrap();
    assert_doc_eq(&original, &parsed);
}

#[test]
fn test_roundtrip_ordered_list_start_3() {
    let original = doc(vec![ordered_list(
        3,
        vec![list_item(vec![paragraph(vec![text("A")])])],
    )]);
    let html = to_html(&original, &schema());
    assert_eq!(html, "<ol start=\"3\"><li><p>A</p></li></ol>");
    let parsed = from_html(&html, &schema(), &default_opts()).unwrap();
    assert_doc_eq(&original, &parsed);
}

#[test]
fn test_roundtrip_ordered_list_start_1() {
    let original = doc(vec![ordered_list(
        1,
        vec![list_item(vec![paragraph(vec![text("A")])])],
    )]);
    let html = to_html(&original, &schema());
    let parsed = from_html(&html, &schema(), &default_opts()).unwrap();
    assert_doc_eq(&original, &parsed);
}

#[test]
fn test_roundtrip_hard_break() {
    let original = doc(vec![paragraph(vec![text("He"), hard_break(), text("llo")])]);
    let html = to_html(&original, &schema());
    let parsed = from_html(&html, &schema(), &default_opts()).unwrap();
    assert_doc_eq(&original, &parsed);
}

#[test]
fn test_roundtrip_horizontal_rule() {
    let original = doc(vec![
        paragraph(vec![text("Above")]),
        horizontal_rule(),
        paragraph(vec![text("Below")]),
    ]);
    let html = to_html(&original, &schema());
    let parsed = from_html(&html, &schema(), &default_opts()).unwrap();
    assert_doc_eq(&original, &parsed);
}

#[test]
fn test_roundtrip_empty_paragraph() {
    let original = doc(vec![paragraph(vec![])]);
    let html = to_html(&original, &schema());
    let parsed = from_html(&html, &schema(), &default_opts()).unwrap();
    assert_doc_eq(&original, &parsed);
}

#[test]
fn test_roundtrip_all_four_marks() {
    let original = doc(vec![paragraph(vec![text_with_marks(
        "all",
        vec![bold(), italic(), underline(), strike()],
    )])]);
    let html = to_html(&original, &schema());
    let parsed = from_html(&html, &schema(), &default_opts()).unwrap();
    assert_doc_eq(&original, &parsed);
}

#[test]
fn test_roundtrip_multiple_paragraphs() {
    let original = doc(vec![
        paragraph(vec![text("First")]),
        paragraph(vec![text("Second")]),
        paragraph(vec![text("Third")]),
    ]);
    let html = to_html(&original, &schema());
    let parsed = from_html(&html, &schema(), &default_opts()).unwrap();
    assert_doc_eq(&original, &parsed);
}

// HTML string round-trip: to_html(from_html(html)) == html

#[test]
fn test_html_roundtrip_plain_paragraph() {
    let html = "<p>Hello</p>";
    let d = from_html(html, &schema(), &default_opts()).unwrap();
    let result = to_html(&d, &schema());
    assert_eq!(
        result, html,
        "to_html(from_html(html)) should equal original HTML"
    );
}

#[test]
fn test_html_roundtrip_bold() {
    let html = "<p><strong>Hello</strong></p>";
    let d = from_html(html, &schema(), &default_opts()).unwrap();
    let result = to_html(&d, &schema());
    assert_eq!(result, html);
}

#[test]
fn test_html_roundtrip_mixed_marks() {
    let html = "<p>H<strong><em>ell</em></strong>o</p>";
    let d = from_html(html, &schema(), &default_opts()).unwrap();
    let result = to_html(&d, &schema());
    assert_eq!(result, html);
}

#[test]
fn test_html_roundtrip_bullet_list() {
    let html = "<ul><li><p>A</p></li><li><p>B</p></li></ul>";
    let d = from_html(html, &schema(), &default_opts()).unwrap();
    let result = to_html(&d, &schema());
    assert_eq!(result, html);
}

#[test]
fn test_html_roundtrip_ordered_list_start() {
    let html = "<ol start=\"3\"><li><p>A</p></li></ol>";
    let d = from_html(html, &schema(), &default_opts()).unwrap();
    let result = to_html(&d, &schema());
    assert_eq!(result, html);
}

#[test]
fn test_html_roundtrip_hard_break() {
    let html = "<p>He<br>llo</p>";
    let d = from_html(html, &schema(), &default_opts()).unwrap();
    let result = to_html(&d, &schema());
    assert_eq!(result, html);
}

#[test]
fn test_html_roundtrip_hr() {
    let html = "<p>Above</p><hr><p>Below</p>";
    let d = from_html(html, &schema(), &default_opts()).unwrap();
    let result = to_html(&d, &schema());
    assert_eq!(result, html);
}

#[test]
fn test_html_roundtrip_empty_paragraph() {
    let html = "<p></p>";
    let d = from_html(html, &schema(), &default_opts()).unwrap();
    let result = to_html(&d, &schema());
    assert_eq!(result, html);
}

#[test]
fn test_from_html_li_without_p_wraps_in_paragraph() {
    // <li>text</li> without a wrapping <p> should auto-wrap text in paragraph
    let d = from_html("<ul><li>A</li><li>B</li></ul>", &schema(), &default_opts()).unwrap();
    let list = d.root().child(0).unwrap();
    assert_eq!(list.node_type(), "bulletList");
    let li0 = list.child(0).unwrap();
    assert_eq!(li0.node_type(), "listItem");
    // The text "A" should be wrapped in a paragraph
    let p = li0.child(0).unwrap();
    assert_eq!(
        p.node_type(),
        "paragraph",
        "bare text in li should be wrapped in paragraph"
    );
    assert_eq!(p.text_content(), "A");
}

#[test]
fn test_from_html_empty_string() {
    let d = from_html("", &schema(), &default_opts()).unwrap();
    let root = d.root();
    assert_eq!(
        root.child_count(),
        1,
        "empty input should produce one empty paragraph"
    );
    assert_eq!(root.child(0).unwrap().node_type(), "paragraph");
}

#[test]
fn test_to_html_complex_document() {
    // A realistic document with mixed content
    let original = doc(vec![
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
    let html = to_html(&original, &schema());
    let parsed = from_html(&html, &schema(), &default_opts()).unwrap();
    assert_doc_eq(&original, &parsed);
}

#[test]
fn test_from_html_whitespace_between_li_ignored() {
    // Real-world HTML often has whitespace between <li> elements
    let html = "<ul>\n  <li><p>A</p></li>\n  <li><p>B</p></li>\n</ul>";
    let d = from_html(html, &schema(), &default_opts()).unwrap();
    let list = d.root().child(0).unwrap();
    assert_eq!(list.node_type(), "bulletList");
    assert_eq!(
        list.child_count(),
        2,
        "whitespace between li elements should be ignored"
    );
}

#[test]
fn test_from_html_alternative_bold_tag_roundtrips_to_strong() {
    // <b> is parsed as bold mark, and re-serialized as <strong>
    let d = from_html("<p><b>Hello</b></p>", &schema(), &default_opts()).unwrap();
    let html = to_html(&d, &schema());
    assert_eq!(
        html, "<p><strong>Hello</strong></p>",
        "<b> should normalize to <strong> on round-trip"
    );
}

#[test]
fn test_from_html_alternative_italic_tag_roundtrips_to_em() {
    let d = from_html("<p><i>Hello</i></p>", &schema(), &default_opts()).unwrap();
    let html = to_html(&d, &schema());
    assert_eq!(html, "<p><em>Hello</em></p>");
}

#[test]
fn test_from_html_del_tag_roundtrips_to_s() {
    let d = from_html("<p><del>Hello</del></p>", &schema(), &default_opts()).unwrap();
    let html = to_html(&d, &schema());
    assert_eq!(html, "<p><s>Hello</s></p>");
}

#[test]
fn code_language_survives_html_and_json_roundtrips() {
    let schema = schema();
    let document = Document::new(Node::element("doc".to_string(), HashMap::new(), Fragment::from(vec![
        Node::element("codeBlock".to_string(), HashMap::from([("language".to_string(), serde_json::json!("rust"))]),
            Fragment::from(vec![Node::text("let value = 1;".to_string(), vec![])])),
    ])));
    let html = to_html(&document, &schema);
    let restored = from_html(&html, &schema, &FromHtmlOptions::default()).unwrap();
    assert_eq!(restored.root().child(0).unwrap().attrs().get("language"), Some(&serde_json::json!("rust")));
    assert_eq!(to_prosemirror_json(&restored, &schema), to_prosemirror_json(&document, &schema));
}
