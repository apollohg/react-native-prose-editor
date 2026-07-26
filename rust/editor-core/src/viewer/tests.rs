use super::{viewer_compile, FfiViewerCompileRequest, FfiViewerElement, FfiViewerSourceKind};

fn local_config() -> String {
    serde_json::json!({
        "initialization": {"type": "localEmpty"}
    })
    .to_string()
}

fn mention_config() -> String {
    serde_json::json!({
        "schema": {
            "nodes": [
                {"name": "doc", "content": "block+", "role": "doc"},
                {"name": "paragraph", "content": "inline*", "group": "block", "role": "textBlock"},
                {"name": "text", "group": "inline", "role": "text"},
                {
                    "name": "mention",
                    "content": "",
                    "group": "inline",
                    "role": "inline",
                    "isVoid": true,
                    "allowUndeclaredAttrs": true,
                    "attrs": {"label": {"default": null}}
                }
            ],
            "marks": []
        },
        "initialization": {"type": "localEmpty"}
    })
    .to_string()
}

fn compile_json(document: serde_json::Value) -> super::FfiViewerCompileResult {
    viewer_compile(FfiViewerCompileRequest {
        source_kind: FfiViewerSourceKind::Json,
        source: document.to_string(),
        config_json: local_config(),
        images_enabled: true,
        mention_prefix: None,
    })
}

#[test]
fn json_and_html_with_the_same_content_have_the_same_semantic_key() {
    let json = serde_json::json!({
        "type": "doc",
        "content": [{"type": "paragraph", "content": [{"type": "text", "text": "Hello"}]}]
    });
    let from_json = compile_json(json).value.expect("JSON document compiles");
    let from_html = viewer_compile(FfiViewerCompileRequest {
        source_kind: FfiViewerSourceKind::Html,
        source: "<p>Hello</p>".into(),
        config_json: local_config(),
        images_enabled: true,
        mention_prefix: None,
    })
    .value
    .expect("HTML document compiles");

    assert_eq!(from_json.semantic_key(), from_html.semantic_key());
    assert_eq!(from_json.elements(), from_html.elements());
}

#[test]
fn equivalent_requests_produce_a_stable_semantic_key() {
    let document = serde_json::json!({
        "type": "doc",
        "content": [{"type": "paragraph", "content": [{"type": "text", "text": "stable"}]}]
    });

    let first = compile_json(document.clone()).value.expect("first compile");
    let second = compile_json(document).value.expect("second compile");

    assert_eq!(first.semantic_key(), second.semantic_key());
}

#[test]
fn changed_input_changes_the_semantic_key() {
    let first = compile_json(serde_json::json!({
        "type": "doc",
        "content": [{"type": "paragraph", "content": [{"type": "text", "text": "first"}]}]
    }))
    .value
    .expect("first compile");
    let second = compile_json(serde_json::json!({
        "type": "doc",
        "content": [{"type": "paragraph", "content": [{"type": "text", "text": "second"}]}]
    }))
    .value
    .expect("second compile");

    assert_ne!(first.semantic_key(), second.semantic_key());
}

#[test]
fn disabled_images_are_absent_from_the_compiled_document() {
    let document = serde_json::json!({
        "type": "doc",
        "content": [{"type": "image", "attrs": {"src": "https://example.test/a.png"}}]
    });
    let config = serde_json::json!({
        "initialization": {"type": "localEmpty"}
    });
    let result = viewer_compile(FfiViewerCompileRequest {
        source_kind: FfiViewerSourceKind::Json,
        source: document.to_string(),
        config_json: config.to_string(),
        images_enabled: false,
        mention_prefix: Some("@".into()),
    });
    let compiled = result.value.expect("compiled document");
    assert!(result.error.is_none());
    assert!(compiled.elements().iter().all(|element| {
        !matches!(element, FfiViewerElement::InlineAtom { node_type, .. } |
            FfiViewerElement::BlockAtom { node_type, .. } if node_type == "image")
    }));
}

#[test]
fn declarative_mention_prefix_is_applied_to_mention_labels() {
    let document = serde_json::json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{"type": "mention", "attrs": {"label": "Ada"}}]
        }]
    });
    let result = viewer_compile(FfiViewerCompileRequest {
        source_kind: FfiViewerSourceKind::Json,
        source: document.to_string(),
        config_json: mention_config(),
        images_enabled: true,
        mention_prefix: Some("@".into()),
    });
    let compiled = result.value.expect("compiled document");

    assert!(compiled.elements().iter().any(|element| {
        matches!(element, FfiViewerElement::InlineAtom { node_type, label, .. }
            if node_type == "mention" && label == "@Ada")
    }));
}

#[test]
fn elements_preserve_blocks_text_marks_and_atoms() {
    let document = serde_json::json!({
        "type": "doc",
        "content": [
            {"type": "paragraph", "content": [
                {"type": "text", "text": "linked", "marks": [{"type": "link", "attrs": {"href": "https://example.test"}}]},
                {"type": "hardBreak"}
            ]},
            {"type": "horizontalRule"},
            {"type": "image", "attrs": {"src": "https://example.test/a.png"}}
        ]
    });
    let compiled = compile_json(document).value.expect("compiled document");
    let elements = compiled.elements();

    assert!(
        matches!(elements.first(), Some(FfiViewerElement::BlockStart { node_type, .. }) if node_type == "paragraph")
    );
    assert!(elements.iter().any(|element| matches!(element,
        FfiViewerElement::TextRun { text, marks } if text == "linked" && marks.iter().any(|mark| mark.mark_type == "link" && mark.attrs_json == r#"{"href":"https://example.test"}"#)
    )));
    assert!(elements.iter().any(|element| matches!(element,
        FfiViewerElement::InlineAtom { node_type, .. } if node_type == "hardBreak"
    )));
    assert!(elements.iter().any(|element| matches!(element,
        FfiViewerElement::BlockAtom { node_type, .. } if node_type == "horizontalRule"
    )));
    assert!(elements.iter().any(|element| matches!(element,
        FfiViewerElement::BlockAtom { node_type, .. } if node_type == "image"
    )));
}

#[test]
fn malformed_source_returns_a_structured_error() {
    let result = viewer_compile(FfiViewerCompileRequest {
        source_kind: FfiViewerSourceKind::Json,
        source: "{not json}".into(),
        config_json: local_config(),
        images_enabled: true,
        mention_prefix: None,
    });

    assert!(result.value.is_none());
    assert!(result.error.is_some());
}

#[test]
fn configured_resource_limits_apply_to_viewer_source() {
    let config = serde_json::json!({
        "initialization": {"type": "localEmpty"},
        "limits": {"resource": {"maxInputBytes": 32}}
    });
    let result = viewer_compile(FfiViewerCompileRequest {
        source_kind: FfiViewerSourceKind::Html,
        source: format!("<p>{}</p>", "x".repeat(64)),
        config_json: config.to_string(),
        images_enabled: true,
        mention_prefix: None,
    });

    assert!(result.value.is_none());
    assert!(result.error.is_some());
}
