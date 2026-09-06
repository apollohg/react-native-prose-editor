#[test]
fn html_rules_node_serializes_per_rules() {
    let schema = atom_rules_schema();
    let document = from_prosemirror_json(
        &serde_json::json!({"type":"doc","content":[
            {"type":"counterCard","attrs":{"title":"Sam\"ple","count":5}},
            {"type":"paragraph","content":[{"type":"text","text":"after"}]}
        ]}),
        &schema,
        UnknownTypeMode::Error,
    )
    .unwrap();
    assert_eq!(
        to_html(&document, &schema),
        "<div data-type=\"counter-card\" data-count=\"5\" data-title=\"Sam&quot;ple\"></div><p>after</p>"
    );
}

#[test]
fn html_rules_json_encode_non_scalar_attrs() {
    let schema = atom_rules_schema_with_array_attr();
    let document = from_prosemirror_json(
        &serde_json::json!({"type":"doc","content":[
            {"type":"counterCard","attrs":{"sets":[{"count":5}]}}
        ]}),
        &schema,
        UnknownTypeMode::Error,
    )
    .unwrap();
    let html = to_html(&document, &schema);
    assert!(
        html.contains("data-sets=\"[{&quot;count&quot;:5}]\""),
        "got: {html}"
    );
}

#[test]
fn html_rules_element_parses_to_atom_node() {
    let schema = atom_rules_schema();
    let doc = from_html(
        "<div data-type=\"counter-card\" data-title=\"Sample item\" data-count=\"5\"></div><p>x</p>",
        &schema,
        &default_opts(),
    )
    .unwrap();
    let json = to_prosemirror_json(&doc, &schema);
    assert_eq!(
        json["content"][0],
        serde_json::json!({"type":"counterCard","attrs":{"title":"Sample item","count":5}})
    );
}

#[test]
fn html_rules_missing_mapped_attr_takes_declared_default() {
    let schema = atom_rules_schema();
    let doc = from_html(
        "<div data-type=\"counter-card\" data-title=\"Sample item\"></div>",
        &schema,
        &default_opts(),
    )
    .unwrap();
    assert_eq!(doc.root().child(0).unwrap().attrs()["count"], 0);
}

#[test]
fn html_rules_mismatched_discriminator_stays_opaque() {
    let schema = atom_rules_schema();
    let doc = from_html(
        "<div data-type=\"other-thing\" data-title=\"X\"></div>",
        &schema,
        &default_opts(),
    )
    .unwrap();
    let json = to_prosemirror_json(&doc, &schema);
    assert_ne!(json["content"][0]["type"], "counterCard");
}

#[test]
fn atom_document_round_trips_html_losslessly() {
    let schema = atom_rules_schema();
    let original = from_prosemirror_json(
        &serde_json::json!({"type":"doc","content":[
            {"type":"paragraph","content":[{"type":"text","text":"before"}]},
            {"type":"counterCard","attrs":{"title":"Sam<u>ple","count":12}},
            {"type":"paragraph","content":[{"type":"text","text":"after"}]}
        ]}),
        &schema,
        UnknownTypeMode::Error,
    )
    .unwrap();
    let html = to_html(&original, &schema);
    let reparsed = from_html(&html, &schema, &default_opts()).unwrap();
    assert_eq!(
        to_prosemirror_json(&reparsed, &schema),
        to_prosemirror_json(&original, &schema),
        "html was: {html}"
    );
}

fn projected_heading_schema() -> Schema {
    Schema::from_json(&serde_json::json!({
        "nodes": [
            { "name": "doc", "content": "block+", "role": "doc" },
            { "name": "paragraph", "content": "inline*", "group": "block", "role": "textBlock", "htmlTag": "p" },
            {
                "name": "h1", "content": "inline*", "group": "block heading",
                "role": "textBlock", "htmlTag": "h1",
                "json": { "type": "heading", "attrs": { "level": 1 } }
            },
            {
                "name": "h2", "content": "inline*", "group": "block heading",
                "role": "textBlock", "htmlTag": "h2",
                "json": { "type": "heading", "attrs": { "level": 2 } }
            },
            { "name": "text", "content": "", "group": "inline", "role": "text" }
        ],
        "marks": []
    }))
    .unwrap()
}

#[test]
fn legacy_flat_heading_schema_still_accepts_tiptap_json() {
    let schema = Schema::from_json(&serde_json::json!({
        "nodes": [
            { "name": "doc", "content": "block+", "role": "doc" },
            { "name": "h2", "content": "inline*", "group": "block", "role": "textBlock", "htmlTag": "h2" },
            { "name": "text", "content": "", "group": "inline", "role": "text" }
        ],
        "marks": []
    }))
    .unwrap();
    for level in [
        serde_json::json!(2),
        serde_json::json!(2.0),
        serde_json::json!("2"),
        serde_json::json!("+2"),
    ] {
        let source = serde_json::json!({
            "type": "doc",
            "content": [{ "type": "heading", "attrs": { "level": level } }]
        });

        let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Error).unwrap();
        assert_eq!(document.root().child(0).unwrap().node_type(), "h2");
    }
}

#[test]
fn projected_json_node_types_round_trip_through_native_variants() {
    let schema = projected_heading_schema();
    let source = serde_json::json!({
        "type": "doc",
        "content": [{
            "type": "heading",
            "attrs": { "level": 2 },
            "content": [{ "type": "text", "text": "Projected" }]
        }]
    });

    let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Error).unwrap();
    let heading = document.root().child(0).unwrap();
    assert_eq!(heading.node_type(), "h2");
    assert!(!heading.attrs().contains_key("level"));
    assert_eq!(to_prosemirror_json(&document, &schema), source);
}

#[test]
fn projected_json_node_types_match_equivalent_float_discriminators() {
    let schema = projected_heading_schema();
    let source = serde_json::json!({
        "type": "doc",
        "content": [{ "type": "heading", "attrs": { "level": 2.0 } }]
    });

    let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Error).unwrap();
    assert_eq!(document.root().child(0).unwrap().node_type(), "h2");
    assert_eq!(
        to_prosemirror_json(&document, &schema),
        serde_json::json!({
            "type": "doc",
            "content": [{ "type": "heading", "attrs": { "level": 2 } }]
        })
    );
}

#[test]
fn native_projection_type_rejects_a_conflicting_public_discriminator() {
    let source = serde_json::json!({
        "type": "doc",
        "content": [{ "type": "h2", "attrs": { "level": 3 } }]
    });

    let error = from_prosemirror_json(&source, &projected_heading_schema(), UnknownTypeMode::Error)
        .unwrap_err();

    assert!(matches!(
        error,
        JsonParseError::InvalidStructure(message)
            if message.contains("projection attribute 'level'")
    ));
}

#[test]
fn legacy_heading_alias_retains_a_declared_native_level_attribute() {
    let schema = Schema::from_json(&serde_json::json!({
        "nodes": [
            { "name": "doc", "content": "block+", "role": "doc" },
            {
                "name": "h2", "content": "inline*", "group": "block", "role": "textBlock",
                "attrs": { "level": { "default": 0 } }
            },
            { "name": "text", "content": "", "group": "inline", "role": "text" }
        ],
        "marks": []
    }))
    .unwrap();
    let source = serde_json::json!({
        "type": "doc",
        "content": [{ "type": "heading", "attrs": { "level": 2 } }]
    });

    let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Error).unwrap();

    assert_eq!(document.root().child(0).unwrap().attrs()["level"], 2);
}

#[test]
fn unresolved_legacy_heading_does_not_fall_back_to_its_raw_type() {
    let schema = Schema::from_json(&serde_json::json!({
        "nodes": [
            { "name": "doc", "content": "block+", "role": "doc" },
            { "name": "paragraph", "content": "inline*", "group": "block", "role": "textBlock" },
            { "name": "text", "content": "", "group": "inline", "role": "text" }
        ],
        "marks": []
    }))
    .unwrap();
    let source = serde_json::json!({
        "type": "doc",
        "content": [{ "type": "heading", "attrs": { "level": 2 } }]
    });

    assert!(matches!(
        from_prosemirror_json(&source, &schema, UnknownTypeMode::Error),
        Err(JsonParseError::UnknownType(name)) if name == "h2"
    ));

    let preserved = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let opaque = preserved.root().child(0).unwrap();
    assert_eq!(opaque.attrs()["original_type"], "heading");
    assert_eq!(opaque.attrs()["original_json"], source["content"][0]);
    DocumentValidator::validate(&preserved, &schema, &ResourceLimits::default()).unwrap();
}

fn mention_schema() -> Schema {
    let base = crate::tiptap_schema();
    let mut nodes: Vec<NodeSpec> = base.all_nodes().cloned().collect();
    if !nodes.iter().any(|node| node.name == "mention") {
        let mut attrs = HashMap::new();
        attrs.insert(
            "label".to_string(),
            AttrSpec {
                default: Some(serde_json::Value::Null),
                has_default: true,
                ..AttrSpec::default()
            },
        );
        nodes.push(NodeSpec {
            name: "mention".to_string(),
            content: crate::schema::content_rule::ContentRule::parse("")
                .expect("mention content rule should parse"),
            group: Some("inline".to_string()),
            attrs,
            role: NodeRole::Inline,
            html_tag: None,
            html_rules: None,
            json_projection: None,
            is_void: true,
            deletable_on_backspace: None,
            // Mirrors the real `mentionNodeSpec()` (src/addons.ts), which
            // intentionally round-trips arbitrary app-defined attrs.
            allow_undeclared_attrs: true,
        });
    }
    let marks = base.all_marks().cloned().collect();
    Schema::new(nodes, marks)
}

/// A mark spec with `allow_undeclared_attrs: true` (mirrors `mention_schema`'s
/// node-side opt-in) for exercising the mark-attr escape hatch.
fn comment_schema() -> Schema {
    let base = crate::tiptap_schema();
    let nodes: Vec<NodeSpec> = base.all_nodes().cloned().collect();
    let mut marks: Vec<MarkSpec> = base.all_marks().cloned().collect();
    if !marks.iter().any(|mark| mark.name == "comment") {
        marks.push(MarkSpec {
            name: "comment".to_string(),
            attrs: HashMap::new(),
            excludes: None,
            allow_undeclared_attrs: true,
            html_tag: None,
        });
    }
    Schema::new(nodes, marks)
}

#[test]
fn custom_mark_names_are_never_emitted_as_raw_html_tags() {
    let base = schema();
    let nodes = base.all_nodes().cloned().collect();
    let mut marks: Vec<MarkSpec> = base.all_marks().cloned().collect();
    marks.push(MarkSpec {
        name: "custom<script".to_string(),
        attrs: HashMap::new(),
        excludes: None,
        allow_undeclared_attrs: false,
        html_tag: None,
    });
    let schema = Schema::new(nodes, marks);
    let document = Document::new(Node::element(
        "doc".to_string(),
        HashMap::new(),
        Fragment::from(vec![paragraph(vec![Node::text(
            "safe".to_string(),
            vec![Mark::new("custom<script".to_string(), HashMap::new())],
        )])]),
    ));

    let html = to_html(&document, &schema);
    assert_eq!(
        html,
        "<p><span data-native-editor-mark=\"custom&lt;script\">safe</span></p>"
    );
    let reparsed = from_html(&html, &schema, &default_opts()).unwrap();
    assert_eq!(
        reparsed.root().child(0).unwrap().child(0).unwrap().marks()[0].mark_type(),
        "custom<script"
    );
}

#[test]
fn json_mark_attrs_receive_schema_defaults() {
    let schema = Schema::from_json(&serde_json::json!({
        "nodes": [
            { "name": "doc", "content": "paragraph", "role": "doc" },
            { "name": "paragraph", "content": "text*", "role": "textBlock", "htmlTag": "p" },
            { "name": "text", "content": "", "group": "inline", "role": "text" }
        ],
        "marks": [{
            "name": "link", "htmlTag": "a",
            "attrs": { "target": { "default": "_self" }, "href": { "default": "" } }
        }]
    }))
    .unwrap();
    let json = serde_json::json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "x", "marks": [{ "type": "link", "attrs": { "href": "/" } }] }]
        }]
    });

    let document = crate::serialize::from_prosemirror_json(
        &json,
        &schema,
        crate::serialize::UnknownTypeMode::Error,
    )
    .unwrap();
    let output = crate::serialize::to_prosemirror_json(&document, &schema);
    assert_eq!(
        output["content"][0]["content"][0]["marks"][0]["attrs"]["target"],
        "_self"
    );
    assert_eq!(
        output["content"][0]["content"][0]["marks"][0]["attrs"]["href"],
        "/"
    );
}

#[test]
fn synthetic_html_nodes_receive_the_same_default_attrs_as_json_nodes() {
    let schema = Schema::from_json(&serde_json::json!({
        "nodes": [
            { "name": "article", "content": "list", "role": "doc", "attrs": { "locale": { "default": "en" } } },
            { "name": "list", "content": "item+", "role": "list", "htmlTag": "ul" },
            { "name": "item", "content": "body", "role": "listItem", "htmlTag": "li", "attrs": { "checked": { "default": false } } },
            { "name": "body", "content": "text*", "role": "textBlock", "htmlTag": "p", "attrs": { "kind": { "default": "plain" } } },
            { "name": "text", "content": "", "group": "inline", "role": "text" }
        ],
        "marks": []
    }))
    .unwrap();
    let from_auto_html = from_html("<ul>loose</ul>", &schema, &default_opts()).unwrap();
    let from_explicit_html =
        from_html("<ul><li><p>loose</p></li></ul>", &schema, &default_opts()).unwrap();
    let from_json = from_prosemirror_json(
        &serde_json::json!({
            "type": "article",
            "content": [{
                "type": "list",
                "content": [{
                    "type": "item",
                    "content": [{
                        "type": "body",
                        "content": [{ "type": "text", "text": "loose" }]
                    }]
                }]
            }]
        }),
        &schema,
        UnknownTypeMode::Error,
    )
    .unwrap();

    assert_eq!(from_auto_html.root().attrs(), from_json.root().attrs());
    assert_eq!(
        from_auto_html.root().attrs(),
        from_explicit_html.root().attrs()
    );
    let html_item = from_auto_html.root().child(0).unwrap().child(0).unwrap();
    let explicit_html_item = from_explicit_html
        .root()
        .child(0)
        .unwrap()
        .child(0)
        .unwrap();
    let json_item = from_json.root().child(0).unwrap().child(0).unwrap();
    assert_eq!(html_item.attrs(), json_item.attrs());
    assert_eq!(html_item.attrs(), explicit_html_item.attrs());
    assert_eq!(
        html_item.child(0).unwrap().attrs(),
        json_item.child(0).unwrap().attrs()
    );
    assert_eq!(
        html_item.child(0).unwrap().attrs(),
        explicit_html_item.child(0).unwrap().attrs()
    );
}

#[test]
fn custom_block_and_list_attrs_round_trip_through_html() {
    let schema = Schema::from_json(&serde_json::json!({
        "nodes": [
            { "name": "doc", "content": "section", "role": "doc" },
            { "name": "section", "content": "customList", "role": "block", "htmlTag": "section", "attrs": { "data-kind": { "default": "" } } },
            { "name": "customList", "content": "listItem+", "role": "list", "htmlTag": "ul", "attrs": { "data-id": { "default": "" } } },
            { "name": "listItem", "content": "paragraph", "role": "listItem", "htmlTag": "li" },
            { "name": "paragraph", "content": "inline*", "role": "textBlock", "htmlTag": "p" },
            { "name": "text", "content": "", "group": "inline", "role": "text" }
        ],
        "marks": []
    }))
    .unwrap();
    let html = "<section data-kind=\"note\"><ul data-id=\"7\"><li><p>x</p></li></ul></section>";
    let document = from_html(html, &schema, &default_opts()).unwrap();

    assert_eq!(to_html(&document, &schema), html);
}

#[test]
fn declared_custom_mark_tag_and_attrs_round_trip_through_html() {
    let schema = Schema::from_json(&serde_json::json!({
        "nodes": [
            { "name": "doc", "content": "paragraph", "role": "doc" },
            { "name": "paragraph", "content": "inline*", "role": "textBlock", "htmlTag": "p" },
            { "name": "text", "content": "", "group": "inline", "role": "text" }
        ],
        "marks": [{
            "name": "highlight",
            "htmlTag": "mark",
            "attrs": { "data-tone": { "default": "yellow" } }
        }]
    }))
    .unwrap();
    let html = "<p><mark data-tone=\"blue\">x</mark></p>";
    let document = from_html(html, &schema, &default_opts()).unwrap();

    assert_eq!(to_html(&document, &schema), html);
}

#[test]
fn private_mark_wrapper_is_recognized_only_on_span() {
    let schema = Schema::from_json(&serde_json::json!({
        "nodes": [
            { "name": "doc", "content": "panel", "role": "doc" },
            { "name": "panel", "content": "paragraph", "role": "block", "htmlTag": "div", "attrs": { "data-native-editor-mark": { "default": "" } } },
            { "name": "paragraph", "content": "inline*", "role": "textBlock", "htmlTag": "p" },
            { "name": "text", "content": "", "group": "inline", "role": "text" }
        ],
        "marks": [{ "name": "bold", "htmlTag": "strong" }]
    }))
    .unwrap();
    let html = "<div data-native-editor-mark=\"bold\"><p>x</p></div>";
    let document = from_html(html, &schema, &default_opts()).unwrap();

    assert_eq!(document.root().child(0).unwrap().node_type(), "panel");
    assert_eq!(to_html(&document, &schema), html);
}

#[test]
fn ordered_list_start_is_only_parsed_when_declared() {
    let without_start = Schema::from_json(&serde_json::json!({
        "nodes": [
            { "name": "doc", "content": "ordered", "role": "doc" },
            { "name": "ordered", "content": "item+", "role": "list", "htmlTag": "ol" },
            { "name": "item", "content": "paragraph", "role": "listItem", "htmlTag": "li" },
            { "name": "paragraph", "content": "inline*", "role": "textBlock", "htmlTag": "p" },
            { "name": "text", "content": "", "group": "inline", "role": "text" }
        ], "marks": []
    }))
    .unwrap();
    let html = "<ol start=\"7\"><li><p>x</p></li></ol>";
    let document = from_html(html, &without_start, &default_opts()).unwrap();
    assert!(document
        .root()
        .child(0)
        .unwrap()
        .attrs()
        .get("start")
        .is_none());
    assert_eq!(
        to_html(&document, &without_start),
        "<ol><li><p>x</p></li></ol>"
    );

    let declared = crate::tiptap_schema();
    let document = from_html(html, &declared, &default_opts()).unwrap();
    assert_eq!(to_html(&document, &declared), html);
}

#[test]
fn non_void_empty_content_node_parses_without_opaque_inference() {
    let schema = Schema::from_json(&serde_json::json!({
        "nodes": [
            { "name": "doc", "content": "empty", "role": "doc" },
            { "name": "empty", "content": "", "role": "block" },
            { "name": "text", "content": "", "role": "text" }
        ], "marks": []
    }))
    .unwrap();
    let json = serde_json::json!({ "type": "doc", "content": [{ "type": "empty" }] });

    let document = from_prosemirror_json(&json, &schema, UnknownTypeMode::Preserve).unwrap();
    assert_eq!(to_prosemirror_json(&document, &schema), json);
}

#[test]
fn opaque_placement_uses_complete_mixed_sibling_sequence() {
    let schema = Schema::from_json(&serde_json::json!({
        "nodes": [
            { "name": "doc", "content": "inline block", "role": "doc" },
            { "name": "paragraph", "content": "inline*", "group": "block", "role": "textBlock" },
            { "name": "inlineAtom", "content": "", "group": "inline", "role": "inline", "isVoid": true },
            { "name": "text", "content": "", "group": "inline", "role": "text" }
        ], "marks": []
    })).unwrap();
    let json = serde_json::json!({
        "type": "doc",
        "content": [{ "type": "futureInline" }, { "type": "paragraph" }]
    });

    let document = from_prosemirror_json(&json, &schema, UnknownTypeMode::Preserve).unwrap();
    assert_eq!(
        document.root().child(0).unwrap().attrs()["opaque_placement"],
        "inline"
    );
    assert_eq!(to_prosemirror_json(&document, &schema), json);
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
    attrs.insert(
        "src".to_string(),
        serde_json::Value::String(src.to_string()),
    );
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
