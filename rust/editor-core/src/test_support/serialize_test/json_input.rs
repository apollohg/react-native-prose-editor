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
fn test_from_json_standard_heading_alias_with_marks() {
    let json = serde_json::json!({
        "type": "doc",
        "content": [{
            "type": "heading",
            "attrs": { "level": 2 },
            "content": [{
                "type": "text",
                "text": "Heading",
                "marks": [
                    { "type": "bold" },
                    { "type": "link", "attrs": { "href": "https://example.com" } }
                ]
            }]
        }]
    });
    let d = from_prosemirror_json(&json, &schema(), UnknownTypeMode::Error).unwrap();
    let heading = d.root().child(0).unwrap();
    assert_eq!(heading.node_type(), "h2");
    let text = heading.child(0).unwrap();
    let mark_types: Vec<&str> = text.marks().iter().map(|m| m.mark_type()).collect();
    assert!(mark_types.contains(&"bold"));
    assert!(mark_types.contains(&"link"));
    assert_eq!(
        text.marks()
            .iter()
            .find(|mark| mark.mark_type() == "link")
            .and_then(|mark| mark.attrs().get("href"))
            .and_then(serde_json::Value::as_str),
        Some("https://example.com")
    );
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
    if let Err(JsonParseError::UnknownMark(name)) = result {
        assert_eq!(name, "superscript");
    }
}

#[test]
fn test_from_json_unknown_mark_skip_mode_still_rejects() {
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
    assert!(matches!(
        from_prosemirror_json(&json, &schema(), UnknownTypeMode::Skip),
        Err(JsonParseError::UnknownMark(name)) if name == "superscript"
    ));
}

#[test]
fn test_from_json_unknown_mark_preserve_mode_still_rejects() {
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
    assert!(matches!(
        from_prosemirror_json(&json, &schema(), UnknownTypeMode::Preserve),
        Err(JsonParseError::UnknownMark(name)) if name == "superscript"
    ));
}

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

// Undeclared attr filtering — parity with the HTML ingestion path
// (extract_node_attrs), hardened against attrs the schema does not declare
// for a given node type (e.g. "checked" on a plain listItem).

/// set_json must not admit attrs the schema does not declare for the node —
/// parity with the HTML ingestion path (extract_node_attrs). A declared attr
/// on a different node (orderedList's own "start") must still round-trip.
#[test]
fn set_json_drops_undeclared_attrs() {
    let json = serde_json::json!({
        "type": "doc",
        "content": [
            {
                "type": "orderedList",
                "attrs": { "start": 3 },
                "content": [
                    {
                        "type": "listItem",
                        "attrs": { "checked": true, "start": 3 },
                        "content": [
                            {
                                "type": "paragraph",
                                "content": [{ "type": "text", "text": "x" }]
                            }
                        ]
                    }
                ]
            }
        ]
    });

    let d = from_prosemirror_json(&json, &schema(), UnknownTypeMode::Error)
        .expect("orderedList > listItem > paragraph > text should parse");

    let ordered_list = d.root().child(0).unwrap();
    assert_eq!(
        ordered_list.node_type(),
        "orderedList",
        "sanity: first doc child should be the orderedList"
    );
    assert_eq!(
        ordered_list.attrs().get("start"),
        Some(&serde_json::Value::Number(3.into())),
        "orderedList declares 'start' — it must still round-trip on its own node"
    );

    let list_item = ordered_list.child(0).unwrap();
    assert_eq!(list_item.node_type(), "listItem");
    assert!(
        list_item.attrs().get("checked").is_none(),
        "listItem's schema spec declares no 'checked' attr — it must be dropped, got attrs: {:?}",
        list_item.attrs()
    );
    assert!(
        list_item.attrs().get("start").is_none(),
        "listItem's schema spec declares no 'start' attr (only orderedList does) — it must be dropped, got attrs: {:?}",
        list_item.attrs()
    );
}

/// A node spec with `allow_undeclared_attrs: true` (mirrors the real
/// `mention` node) must keep attrs the schema does not declare — this is the
/// approved escape hatch, not a regression of the filter above.
#[test]
fn set_json_keeps_undeclared_attrs_when_spec_opts_in() {
    let json = serde_json::json!({
        "type": "doc",
        "content": [
            {
                "type": "paragraph",
                "content": [
                    {
                        "type": "mention",
                        "attrs": {
                            "id": "u1",
                            "kind": "user",
                            "label": "@Alice"
                        }
                    }
                ]
            }
        ]
    });

    let d = from_prosemirror_json(&json, &mention_schema(), UnknownTypeMode::Error)
        .expect("paragraph > mention should parse");

    let mention = d.root().child(0).unwrap().child(0).unwrap();
    assert_eq!(mention.node_type(), "mention");
    assert_eq!(
        mention.attrs().get("id"),
        Some(&serde_json::Value::String("u1".to_string())),
        "mention opts into allow_undeclared_attrs — 'id' must survive"
    );
    assert_eq!(
        mention.attrs().get("kind"),
        Some(&serde_json::Value::String("user".to_string())),
        "mention opts into allow_undeclared_attrs — 'kind' must survive"
    );
    assert_eq!(
        mention.attrs().get("label"),
        Some(&serde_json::Value::String("@Alice".to_string())),
        "'label' is declared on the mention spec and must survive regardless"
    );
}

/// Undeclared mark attrs are dropped; declared ones survive; opted-in mark
/// specs keep arbitrary attrs — parity with the node-attr design.
#[test]
fn set_json_filters_mark_attrs_against_schema() {
    let json = serde_json::json!({
        "type": "doc",
        "content": [
            {
                "type": "paragraph",
                "content": [
                    {
                        "type": "text",
                        "text": "x",
                        "marks": [
                            {
                                "type": "bold",
                                "attrs": { "weight": 900 }
                            },
                            {
                                "type": "link",
                                "attrs": { "href": "https://e.x", "target": "_blank" }
                            }
                        ]
                    }
                ]
            }
        ]
    });

    let d = from_prosemirror_json(&json, &schema(), UnknownTypeMode::Error)
        .expect("paragraph > text with bold+link marks should parse");

    let text_node = d.root().child(0).unwrap().child(0).unwrap();
    let marks = text_node.marks();
    assert_eq!(marks.len(), 2, "both marks should still be attached");

    let bold = marks
        .iter()
        .find(|m| m.mark_type() == "bold")
        .expect("bold mark should survive");
    assert!(
        bold.attrs().get("weight").is_none(),
        "bold's schema spec declares no 'weight' attr — it must be dropped, got attrs: {:?}",
        bold.attrs()
    );

    let link = marks
        .iter()
        .find(|m| m.mark_type() == "link")
        .expect("link mark should survive");
    assert_eq!(
        link.attrs().get("href"),
        Some(&serde_json::Value::String("https://e.x".to_string())),
        "link declares 'href' — it must survive"
    );
    assert!(
        link.attrs().get("target").is_none(),
        "link's schema spec declares no 'target' attr — it must be dropped, got attrs: {:?}",
        link.attrs()
    );
}

/// A mark spec with `allow_undeclared_attrs: true` (mirrors the node-side
/// `mention` escape hatch) must keep attrs the schema does not declare.
#[test]
fn set_json_keeps_mark_attrs_when_spec_opts_in() {
    let json = serde_json::json!({
        "type": "doc",
        "content": [
            {
                "type": "paragraph",
                "content": [
                    {
                        "type": "text",
                        "text": "x",
                        "marks": [
                            {
                                "type": "comment",
                                "attrs": { "threadId": "t1" }
                            }
                        ]
                    }
                ]
            }
        ]
    });

    let d = from_prosemirror_json(&json, &comment_schema(), UnknownTypeMode::Error)
        .expect("paragraph > text with comment mark should parse");

    let text_node = d.root().child(0).unwrap().child(0).unwrap();
    let comment = text_node
        .marks()
        .iter()
        .find(|m| m.mark_type() == "comment")
        .expect("comment mark should survive")
        .clone();
    assert_eq!(
        comment.attrs().get("threadId"),
        Some(&serde_json::Value::String("t1".to_string())),
        "comment opts into allow_undeclared_attrs — 'threadId' must survive"
    );
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

/// Uses `link`'s declared `href` attr (not an arbitrary/undeclared one —
/// see `set_json_filters_mark_attrs_against_schema` for the filtering
/// behavior) since schema-declared mark attrs are what actually round-trip.
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
                    "type": "link",
                    "attrs": { "href": "https://example.com" }
                }]
            }]
        }]
    });
    let d = from_prosemirror_json(&json, &schema(), UnknownTypeMode::Error).unwrap();
    let t = d.root().child(0).unwrap().child(0).unwrap();
    assert_eq!(t.marks().len(), 1);
    assert_eq!(t.marks()[0].mark_type(), "link");
    let href = t.marks()[0].attrs().get("href").unwrap();
    assert_eq!(
        *href,
        serde_json::Value::String("https://example.com".to_string())
    );
}
