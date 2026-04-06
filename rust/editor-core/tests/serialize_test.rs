use std::collections::HashMap;

use editor_core::model::{Document, Fragment, Mark, Node};
use editor_core::schema::{AttrSpec, NodeRole, NodeSpec, Schema};
use editor_core::serialize::{
    from_html, from_prosemirror_json, to_html, to_prosemirror_json, FromHtmlOptions,
    JsonParseError, UnknownTypeMode,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn schema() -> Schema {
    editor_core::tiptap_schema()
}

fn mention_schema() -> Schema {
    let base = editor_core::tiptap_schema();
    let mut nodes: Vec<NodeSpec> = base.all_nodes().cloned().collect();
    if !nodes.iter().any(|node| node.name == "mention") {
        let mut attrs = HashMap::new();
        attrs.insert(
            "label".to_string(),
            AttrSpec {
                default: Some(serde_json::Value::Null),
            },
        );
        nodes.push(NodeSpec {
            name: "mention".to_string(),
            content: editor_core::schema::content_rule::ContentRule::parse("")
                .expect("mention content rule should parse"),
            group: Some("inline".to_string()),
            attrs,
            role: NodeRole::Inline,
            html_tag: None,
            is_void: true,
        });
    }
    let marks = base.all_marks().cloned().collect();
    Schema::new(nodes, marks)
}

fn default_opts() -> FromHtmlOptions {
    FromHtmlOptions::default()
}

fn strict_opts() -> FromHtmlOptions {
    FromHtmlOptions {
        strict: true,
        allow_base64_images: false,
    }
}

fn bold() -> Mark {
    Mark::new("bold".to_string(), HashMap::new())
}

fn italic() -> Mark {
    Mark::new("italic".to_string(), HashMap::new())
}

fn underline() -> Mark {
    Mark::new("underline".to_string(), HashMap::new())
}

fn strike() -> Mark {
    Mark::new("strike".to_string(), HashMap::new())
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

fn blockquote(children: Vec<Node>) -> Node {
    Node::element(
        "blockquote".to_string(),
        HashMap::new(),
        Fragment::from(children),
    )
}

fn bullet_list(children: Vec<Node>) -> Node {
    Node::element(
        "bulletList".to_string(),
        HashMap::new(),
        Fragment::from(children),
    )
}

fn ordered_list(start: u64, children: Vec<Node>) -> Node {
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

fn image(src: &str, alt: Option<&str>, title: Option<&str>) -> Node {
    image_with_dimensions(src, alt, title, None, None)
}

fn image_with_dimensions(
    src: &str,
    alt: Option<&str>,
    title: Option<&str>,
    width: Option<u64>,
    height: Option<u64>,
) -> Node {
    let mut attrs = HashMap::new();
    attrs.insert("src".to_string(), serde_json::Value::String(src.to_string()));
    attrs.insert(
        "alt".to_string(),
        alt.map_or(serde_json::Value::Null, |value| {
            serde_json::Value::String(value.to_string())
        }),
    );
    attrs.insert(
        "title".to_string(),
        title.map_or(serde_json::Value::Null, |value| {
            serde_json::Value::String(value.to_string())
        }),
    );
    attrs.insert(
        "width".to_string(),
        width.map_or(serde_json::Value::Null, |value| {
            serde_json::Value::Number(value.into())
        }),
    );
    attrs.insert(
        "height".to_string(),
        height.map_or(serde_json::Value::Null, |value| {
            serde_json::Value::Number(value.into())
        }),
    );
    Node::void("image".to_string(), attrs)
}

fn mention(attrs: &[(&str, serde_json::Value)]) -> Node {
    let mut map = HashMap::new();
    for (key, value) in attrs {
        map.insert((*key).to_string(), value.clone());
    }
    Node::void("mention".to_string(), map)
}

fn doc(children: Vec<Node>) -> Document {
    Document::new(Node::element(
        "doc".to_string(),
        HashMap::new(),
        Fragment::from(children),
    ))
}

// ===========================================================================
// to_html tests
// ===========================================================================

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
    assert_eq!(quote.child(0).expect("first paragraph").text_content(), "Hello");
    assert_eq!(quote.child(1).expect("second paragraph").text_content(), "World");
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

// ===========================================================================
// from_html tests
// ===========================================================================

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
    assert_eq!(p.child_count(), 3, "paragraph should have text, mention, text");
    let mention = p.child(1).unwrap();
    assert!(mention.is_void(), "mention should be parsed as a void inline node");
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

// ---------------------------------------------------------------------------
// Alternative tag tests
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Auto-wrap bare text
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Unknown tag handling
// ---------------------------------------------------------------------------

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

// ===========================================================================
// Round-trip tests: from_html(to_html(doc)) structure equivalence
// ===========================================================================

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

// ===========================================================================
// HTML string round-trip: to_html(from_html(html)) == html
// ===========================================================================

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

// ===========================================================================
// Edge cases
// ===========================================================================

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

// ===========================================================================
// to_prosemirror_json tests
// ===========================================================================

fn pm_schema() -> Schema {
    editor_core::prosemirror_schema()
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

// ===========================================================================
// from_prosemirror_json tests
// ===========================================================================

#[test]
fn test_from_json_valid_plain_paragraph() {
    let json = serde_json::json!({
        "type": "doc",
        "content": [
            {
                "type": "paragraph",
                "content": [
                    { "type": "text", "text": "Hello" }
                ]
            }
        ]
    });
    let d = from_prosemirror_json(&json, &schema(), UnknownTypeMode::Error).unwrap();
    let root = d.root();
    assert_eq!(root.node_type(), "doc");
    assert_eq!(root.child_count(), 1);
    let p = root.child(0).unwrap();
    assert_eq!(p.node_type(), "paragraph");
    assert_eq!(p.child_count(), 1);
    let t = p.child(0).unwrap();
    assert!(t.is_text());
    assert_eq!(t.text_str().unwrap(), "Hello");
    assert!(t.marks().is_empty());
}

#[test]
fn test_from_json_bold_text() {
    let json = serde_json::json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{
                "type": "text",
                "text": "Bold",
                "marks": [{ "type": "bold" }]
            }]
        }]
    });
    let d = from_prosemirror_json(&json, &schema(), UnknownTypeMode::Error).unwrap();
    let t = d.root().child(0).unwrap().child(0).unwrap();
    assert_eq!(t.text_str().unwrap(), "Bold");
    assert_eq!(t.marks().len(), 1);
    assert_eq!(t.marks()[0].mark_type(), "bold");
}

#[test]
fn test_from_json_multiple_marks() {
    let json = serde_json::json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{
                "type": "text",
                "text": "styled",
                "marks": [
                    { "type": "bold" },
                    { "type": "italic" },
                    { "type": "underline" }
                ]
            }]
        }]
    });
    let d = from_prosemirror_json(&json, &schema(), UnknownTypeMode::Error).unwrap();
    let t = d.root().child(0).unwrap().child(0).unwrap();
    assert_eq!(t.marks().len(), 3);
    let mark_types: Vec<&str> = t.marks().iter().map(|m| m.mark_type()).collect();
    assert_eq!(mark_types, vec!["bold", "italic", "underline"]);
}

#[test]
fn test_from_json_bullet_list() {
    let json = serde_json::json!({
        "type": "doc",
        "content": [{
            "type": "bulletList",
            "content": [
                {
                    "type": "listItem",
                    "content": [{
                        "type": "paragraph",
                        "content": [{ "type": "text", "text": "A" }]
                    }]
                },
                {
                    "type": "listItem",
                    "content": [{
                        "type": "paragraph",
                        "content": [{ "type": "text", "text": "B" }]
                    }]
                }
            ]
        }]
    });
    let d = from_prosemirror_json(&json, &schema(), UnknownTypeMode::Error).unwrap();
    let list = d.root().child(0).unwrap();
    assert_eq!(list.node_type(), "bulletList");
    assert_eq!(list.child_count(), 2);
    assert_eq!(list.child(0).unwrap().node_type(), "listItem");
    assert_eq!(list.child(0).unwrap().child(0).unwrap().text_content(), "A");
}

#[test]
fn test_from_json_ordered_list_with_start() {
    let json = serde_json::json!({
        "type": "doc",
        "content": [{
            "type": "orderedList",
            "attrs": { "start": 5 },
            "content": [{
                "type": "listItem",
                "content": [{
                    "type": "paragraph",
                    "content": [{ "type": "text", "text": "A" }]
                }]
            }]
        }]
    });
    let d = from_prosemirror_json(&json, &schema(), UnknownTypeMode::Error).unwrap();
    let ol = d.root().child(0).unwrap();
    assert_eq!(ol.node_type(), "orderedList");
    let start = ol.attrs().get("start").unwrap();
    assert_eq!(*start, serde_json::Value::Number(5.into()));
}

#[test]
fn test_from_json_ordered_list_default_start() {
    let json = serde_json::json!({
        "type": "doc",
        "content": [{
            "type": "orderedList",
            "content": [{
                "type": "listItem",
                "content": [{
                    "type": "paragraph",
                    "content": [{ "type": "text", "text": "A" }]
                }]
            }]
        }]
    });
    let d = from_prosemirror_json(&json, &schema(), UnknownTypeMode::Error).unwrap();
    let ol = d.root().child(0).unwrap();
    // Missing attrs should fill in default start=1
    let start = ol.attrs().get("start").unwrap();
    assert_eq!(
        *start,
        serde_json::Value::Number(1.into()),
        "missing start attr should default to 1"
    );
}

#[test]
fn test_from_json_hard_break() {
    let json = serde_json::json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [
                { "type": "text", "text": "A" },
                { "type": "hardBreak" },
                { "type": "text", "text": "B" }
            ]
        }]
    });
    let d = from_prosemirror_json(&json, &schema(), UnknownTypeMode::Error).unwrap();
    let p = d.root().child(0).unwrap();
    assert_eq!(p.child_count(), 3);
    assert!(p.child(1).unwrap().is_void());
    assert_eq!(p.child(1).unwrap().node_type(), "hardBreak");
}

#[test]
fn test_from_json_horizontal_rule() {
    let json = serde_json::json!({
        "type": "doc",
        "content": [
            { "type": "paragraph", "content": [{ "type": "text", "text": "Above" }] },
            { "type": "horizontalRule" },
            { "type": "paragraph", "content": [{ "type": "text", "text": "Below" }] }
        ]
    });
    let d = from_prosemirror_json(&json, &schema(), UnknownTypeMode::Error).unwrap();
    assert_eq!(d.root().child_count(), 3);
    assert!(d.root().child(1).unwrap().is_void());
    assert_eq!(d.root().child(1).unwrap().node_type(), "horizontalRule");
}

#[test]
fn test_from_json_empty_paragraph_no_content() {
    let json = serde_json::json!({
        "type": "doc",
        "content": [
            { "type": "paragraph" }
        ]
    });
    let d = from_prosemirror_json(&json, &schema(), UnknownTypeMode::Error).unwrap();
    let p = d.root().child(0).unwrap();
    assert_eq!(p.node_type(), "paragraph");
    assert_eq!(p.child_count(), 0);
}

#[test]
fn test_from_json_empty_paragraph_empty_content_array() {
    let json = serde_json::json!({
        "type": "doc",
        "content": [
            { "type": "paragraph", "content": [] }
        ]
    });
    let d = from_prosemirror_json(&json, &schema(), UnknownTypeMode::Error).unwrap();
    let p = d.root().child(0).unwrap();
    assert_eq!(p.child_count(), 0);
}

// ---------------------------------------------------------------------------
// Unknown type handling
// ---------------------------------------------------------------------------

#[test]
fn test_from_json_unknown_type_error_mode() {
    let json = serde_json::json!({
        "type": "doc",
        "content": [
            { "type": "customWidget", "content": [] }
        ]
    });
    let result = from_prosemirror_json(&json, &schema(), UnknownTypeMode::Error);
    assert!(result.is_err(), "Error mode should reject unknown types");
    if let Err(JsonParseError::UnknownType(name)) = result {
        assert_eq!(
            name, "customWidget",
            "error should name the unknown type, got: {}",
            name
        );
    } else {
        panic!("expected JsonParseError::UnknownType, got: {:?}", result);
    }
}

#[test]
fn test_from_json_unknown_type_preserve_mode() {
    let json = serde_json::json!({
        "type": "doc",
        "content": [
            { "type": "paragraph", "content": [{ "type": "text", "text": "Before" }] },
            { "type": "customWidget", "attrs": { "color": "red" } },
            { "type": "paragraph", "content": [{ "type": "text", "text": "After" }] }
        ]
    });
    let d = from_prosemirror_json(&json, &schema(), UnknownTypeMode::Preserve).unwrap();
    assert_eq!(d.root().child_count(), 3);

    let opaque = d.root().child(1).unwrap();
    assert_eq!(
        opaque.node_type(),
        "__opaque_json",
        "preserved unknown type should become __opaque_json node"
    );
    assert!(opaque.is_void());
    let original_type = opaque
        .attrs()
        .get("original_type")
        .unwrap()
        .as_str()
        .unwrap();
    assert_eq!(original_type, "customWidget");
    // Original JSON should be preserved
    let original_json = opaque.attrs().get("original_json").unwrap();
    assert_eq!(original_json["attrs"]["color"], "red");
}

#[test]
fn test_from_json_unknown_type_skip_mode() {
    let json = serde_json::json!({
        "type": "doc",
        "content": [
            { "type": "paragraph", "content": [{ "type": "text", "text": "Keep" }] },
            { "type": "customWidget" },
            { "type": "paragraph", "content": [{ "type": "text", "text": "Also keep" }] }
        ]
    });
    let d = from_prosemirror_json(&json, &schema(), UnknownTypeMode::Skip).unwrap();
    assert_eq!(
        d.root().child_count(),
        2,
        "Skip mode should drop the unknown node, leaving 2 paragraphs"
    );
    assert_eq!(d.root().child(0).unwrap().text_content(), "Keep");
    assert_eq!(d.root().child(1).unwrap().text_content(), "Also keep");
}

#[test]
fn test_from_json_unknown_mark_error_mode() {
    let json = serde_json::json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{
                "type": "text",
                "text": "styled",
                "marks": [{ "type": "superscript" }]
            }]
        }]
    });
    let result = from_prosemirror_json(&json, &schema(), UnknownTypeMode::Error);
    assert!(result.is_err());
    if let Err(JsonParseError::UnknownType(name)) = result {
        assert_eq!(name, "superscript");
    }
}

#[test]
fn test_from_json_unknown_mark_skip_mode() {
    let json = serde_json::json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{
                "type": "text",
                "text": "styled",
                "marks": [
                    { "type": "bold" },
                    { "type": "superscript" },
                    { "type": "italic" }
                ]
            }]
        }]
    });
    let d = from_prosemirror_json(&json, &schema(), UnknownTypeMode::Skip).unwrap();
    let t = d.root().child(0).unwrap().child(0).unwrap();
    assert_eq!(t.marks().len(), 2, "unknown mark should be dropped");
    let mark_types: Vec<&str> = t.marks().iter().map(|m| m.mark_type()).collect();
    assert!(mark_types.contains(&"bold"));
    assert!(mark_types.contains(&"italic"));
    assert!(!mark_types.contains(&"superscript"));
}

#[test]
fn test_from_json_unknown_mark_preserve_mode() {
    let json = serde_json::json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{
                "type": "text",
                "text": "styled",
                "marks": [
                    { "type": "bold" },
                    { "type": "superscript" }
                ]
            }]
        }]
    });
    let d = from_prosemirror_json(&json, &schema(), UnknownTypeMode::Preserve).unwrap();
    let t = d.root().child(0).unwrap().child(0).unwrap();
    assert_eq!(
        t.marks().len(),
        2,
        "preserve mode should keep unknown marks"
    );
    let mark_types: Vec<&str> = t.marks().iter().map(|m| m.mark_type()).collect();
    assert!(mark_types.contains(&"bold"));
    assert!(mark_types.contains(&"superscript"));
}

// ---------------------------------------------------------------------------
// Invalid structure tests
// ---------------------------------------------------------------------------

#[test]
fn test_from_json_missing_type_field() {
    let json = serde_json::json!({
        "content": [{ "type": "paragraph" }]
    });
    let result = from_prosemirror_json(&json, &schema(), UnknownTypeMode::Error);
    assert!(result.is_err());
    if let Err(JsonParseError::InvalidStructure(msg)) = result {
        assert!(
            msg.contains("type"),
            "error should mention missing type field, got: {}",
            msg
        );
    }
}

#[test]
fn test_from_json_not_an_object() {
    let json = serde_json::json!("just a string");
    let result = from_prosemirror_json(&json, &schema(), UnknownTypeMode::Error);
    assert!(result.is_err());
    match result {
        Err(JsonParseError::InvalidStructure(_)) => {} // expected
        other => panic!("expected InvalidStructure, got: {:?}", other),
    }
}

#[test]
fn test_from_json_text_node_missing_text_field() {
    let json = serde_json::json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text" }]
        }]
    });
    let result = from_prosemirror_json(&json, &schema(), UnknownTypeMode::Error);
    assert!(result.is_err());
    match result {
        Err(JsonParseError::InvalidStructure(msg)) => {
            assert!(
                msg.contains("text"),
                "error should mention text field, got: {}",
                msg
            );
        }
        other => panic!("expected InvalidStructure, got: {:?}", other),
    }
}

#[test]
fn test_from_json_mark_attrs_preserved() {
    let json = serde_json::json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{
                "type": "text",
                "text": "link text",
                "marks": [{
                    "type": "bold",
                    "attrs": { "weight": 700 }
                }]
            }]
        }]
    });
    let d = from_prosemirror_json(&json, &schema(), UnknownTypeMode::Error).unwrap();
    let t = d.root().child(0).unwrap().child(0).unwrap();
    assert_eq!(t.marks().len(), 1);
    assert_eq!(t.marks()[0].mark_type(), "bold");
    let weight = t.marks()[0].attrs().get("weight").unwrap();
    assert_eq!(*weight, serde_json::Value::Number(700.into()));
}

// ===========================================================================
// Round-trip tests: from_json(to_json(doc)) equivalence
// ===========================================================================

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

// ---------------------------------------------------------------------------
// Round-trip with prosemirror schema
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// JSON string round-trip: to_json produces correct JSON, from_json parses it
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Cross-format round-trip: JSON -> Doc -> HTML -> Doc -> JSON equivalence
// ---------------------------------------------------------------------------

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
