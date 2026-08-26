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

fn no_image_schema_config() -> String {
    serde_json::json!({
        "schema": {
            "nodes": [
                {"name": "doc", "content": "block*", "role": "doc"},
                {"name": "paragraph", "content": "inline*", "group": "block", "role": "textBlock"},
                {"name": "text", "group": "inline", "role": "text"}
            ],
            "marks": []
        },
        "initialization": {"type": "localEmpty"}
    })
    .to_string()
}

fn optional_image_schema_config() -> String {
    serde_json::json!({
        "schema": {
            "nodes": [
                {"name": "doc", "content": "block*", "role": "doc"},
                {"name": "paragraph", "content": "inline*", "group": "block", "role": "textBlock"},
                {
                    "name": "image",
                    "content": "",
                    "group": "block",
                    "role": "block",
                    "isVoid": true,
                    "attrs": {"src": {}}
                },
                {"name": "blockquote", "content": "block*", "group": "block", "role": "block"},
                {"name": "text", "group": "inline", "role": "text"}
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

fn compile_json_with(
    document: serde_json::Value,
    config_json: String,
    images_enabled: bool,
) -> super::FfiViewerCompileResult {
    viewer_compile(FfiViewerCompileRequest {
        source_kind: FfiViewerSourceKind::Json,
        source: document.to_string(),
        config_json,
        images_enabled,
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
fn enabled_images_remain_semantically_non_empty() {
    let image_only = serde_json::json!({
        "type": "doc",
        "content": [{"type": "image", "attrs": {"src": "https://example.test/a.png"}}]
    });

    let compiled = compile_json(image_only)
        .value
        .expect("enabled image compiles");

    assert!(!compiled.is_empty());
}

#[test]
fn default_schema_rejects_an_empty_json_root() {
    let result = compile_json_with(
        serde_json::json!({"type": "doc", "content": []}),
        local_config(),
        false,
    );

    assert!(result.value.is_none());
    let error = result
        .error
        .expect("invalid root reports an admission error");
    assert_eq!(error.code, "DOCUMENT_INVALID");
    assert_eq!(
        error.message,
        "node 'doc' content [] does not match its content expression"
    );
}

#[test]
fn disabled_known_image_only_content_is_semantically_empty() {
    let image_only = serde_json::json!({
        "type": "doc",
        "content": [{"type": "image", "attrs": {"src": "https://example.test/a.png"}}]
    });
    let empty = serde_json::json!({"type": "doc", "content": []});
    let config = optional_image_schema_config();

    let hidden_image = compile_json_with(image_only, config.clone(), false)
        .value
        .expect("hidden image compiles");
    let empty_document = compile_json_with(empty, config, false)
        .value
        .expect("empty document compiles");

    assert!(hidden_image.elements().is_empty());
    assert!(hidden_image.is_empty());
    assert_eq!(hidden_image.elements(), empty_document.elements());
    assert_eq!(hidden_image.semantic_key(), empty_document.semantic_key());
    assert_eq!(hidden_image.is_empty(), empty_document.is_empty());
}

#[test]
fn disabled_images_preserve_empty_paragraph_semantics() {
    let empty_paragraph = serde_json::json!({
        "type": "doc",
        "content": [{"type": "paragraph"}]
    });
    let empty_paragraph_and_image = serde_json::json!({
        "type": "doc",
        "content": [
            {"type": "paragraph"},
            {"type": "image", "attrs": {"src": "https://example.test/a.png"}}
        ]
    });

    let paragraph = compile_json_with(empty_paragraph, local_config(), false)
        .value
        .expect("empty paragraph compiles");
    let paragraph_and_hidden_image =
        compile_json_with(empty_paragraph_and_image, local_config(), false)
            .value
            .expect("empty paragraph and hidden image compile");

    assert!(paragraph.is_empty());
    assert_eq!(paragraph_and_hidden_image.elements(), paragraph.elements());
    assert_eq!(
        paragraph_and_hidden_image.semantic_key(),
        paragraph.semantic_key()
    );
    assert_eq!(paragraph_and_hidden_image.is_empty(), paragraph.is_empty());
}

#[test]
fn authored_zero_width_text_is_not_semantically_empty() {
    let authored_zero_width_text = serde_json::json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{"type": "text", "text": "\u{200B}"}]
        }]
    });

    let compiled = compile_json_with(authored_zero_width_text, local_config(), false)
        .value
        .expect("authored zero-width text compiles");

    assert!(!compiled.is_empty());
}

#[test]
fn reports_only_structurally_empty_trailing_text_blocks() {
    let trailing_empty_paragraphs = serde_json::json!({
        "type": "doc",
        "content": [
            {"type": "paragraph", "content": [{"type": "text", "text": "first"}]},
            {"type": "paragraph"},
            {"type": "paragraph", "content": [{"type": "text", "text": "second"}]},
            {"type": "paragraph"},
            {"type": "paragraph"}
        ]
    });
    let authored_zero_width_text = serde_json::json!({
        "type": "doc",
        "content": [
            {"type": "paragraph", "content": [{"type": "text", "text": "first"}]},
            {"type": "paragraph", "content": [{"type": "text", "text": "\u{200B}"}]}
        ]
    });

    let trailing = compile_json_with(trailing_empty_paragraphs, local_config(), true)
        .value
        .expect("trailing empty paragraphs compile");
    let authored = compile_json_with(authored_zero_width_text, local_config(), true)
        .value
        .expect("authored zero-width text compiles");

    assert_eq!(trailing.trailing_empty_text_block_count(), 2);
    assert_eq!(trailing.preferred_text_block_name(), "paragraph");
    assert_eq!(authored.trailing_empty_text_block_count(), 0);
}

#[test]
fn disabled_images_do_not_hide_empty_structural_containers() {
    let empty_blockquote = serde_json::json!({
        "type": "doc",
        "content": [{"type": "blockquote"}]
    });

    let compiled = compile_json_with(empty_blockquote, optional_image_schema_config(), false)
        .value
        .expect("empty blockquote compiles");

    assert!(!compiled.is_empty());
}

#[test]
fn disabled_images_omit_opaque_json_images_from_custom_schemas() {
    let result = compile_json_with(
        serde_json::json!({
            "type": "doc",
            "content": [{"type": "image", "attrs": {"src": "https://example.test/a.png"}}]
        }),
        no_image_schema_config(),
        false,
    );

    let compiled = result.value.expect("opaque JSON image compiles");
    assert!(result.error.is_none());
    assert!(compiled.elements().is_empty());
    assert!(compiled.is_empty());
}

#[test]
fn disabled_images_omit_opaque_html_images_from_custom_schemas() {
    let result = viewer_compile(FfiViewerCompileRequest {
        source_kind: FfiViewerSourceKind::Html,
        source: r#"<img src="https://example.test/a.png">"#.into(),
        config_json: no_image_schema_config(),
        images_enabled: false,
        mention_prefix: None,
    });

    let compiled = result.value.expect("opaque HTML image compiles");
    assert!(result.error.is_none());
    assert!(compiled.elements().is_empty());
    assert!(compiled.is_empty());
}

#[test]
fn empty_html_and_empty_json_paragraph_have_matching_viewer_semantics() {
    let empty_json = compile_json(serde_json::json!({
        "type": "doc",
        "content": [{"type": "paragraph"}]
    }))
    .value
    .expect("empty JSON paragraph compiles");
    let empty_html = viewer_compile(FfiViewerCompileRequest {
        source_kind: FfiViewerSourceKind::Html,
        source: String::new(),
        config_json: local_config(),
        images_enabled: true,
        mention_prefix: None,
    })
    .value
    .expect("empty HTML compiles as an empty paragraph");

    assert_eq!(empty_html.elements(), empty_json.elements());
    assert_eq!(empty_html.semantic_key(), empty_json.semantic_key());
    assert_eq!(empty_html.is_empty(), empty_json.is_empty());
    assert!(empty_html.is_empty());
}

#[test]
fn opaque_inline_and_block_attrs_are_preserved_as_canonical_json() {
    let inline = compile_json(serde_json::json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{
                "type": "futureInline",
                "attrs": {"z": 2, "nested": {"b": true, "a": 1}}
            }]
        }]
    }))
    .value
    .expect("opaque inline compiles");
    let block = compile_json(serde_json::json!({
        "type": "doc",
        "content": [{
            "type": "futureBlock",
            "attrs": {"z": 2, "nested": {"b": true, "a": 1}}
        }]
    }))
    .value
    .expect("opaque block compiles");

    let expected_inline = r#"{"opaque_placement":"inline","original_json":{"attrs":{"nested":{"a":1,"b":true},"z":2},"type":"futureInline"},"original_type":"futureInline"}"#;
    let expected_block = r#"{"opaque_placement":"block","original_json":{"attrs":{"nested":{"a":1,"b":true},"z":2},"type":"futureBlock"},"original_type":"futureBlock"}"#;
    assert!(inline.elements().iter().any(|element| {
        matches!(element, FfiViewerElement::InlineAtom { node_type, attrs_json, .. }
            if node_type == "__opaque_json" && attrs_json == expected_inline)
    }));
    assert!(block.elements().iter().any(|element| {
        matches!(element, FfiViewerElement::BlockAtom { node_type, attrs_json, .. }
            if node_type == "__opaque_json" && attrs_json == expected_block)
    }));
}

#[test]
fn opaque_attrs_change_compiled_elements_and_semantic_identity() {
    let red = compile_json(serde_json::json!({
        "type": "doc",
        "content": [{"type": "futureBlock", "attrs": {"color": "red"}}]
    }))
    .value
    .expect("red opaque block compiles");
    let blue = compile_json(serde_json::json!({
        "type": "doc",
        "content": [{"type": "futureBlock", "attrs": {"color": "blue"}}]
    }))
    .value
    .expect("blue opaque block compiles");

    assert_ne!(red.elements(), blue.elements());
    assert_ne!(red.semantic_key(), blue.semantic_key());
}

#[test]
fn viewer_canonicalizes_importable_mark_order_before_compilation() {
    let canonical = compile_json(serde_json::json!({
        "type": "doc",
        "content": [{"type": "paragraph", "content": [{
            "type": "text",
            "text": "marked",
            "marks": [{"type": "bold"}, {"type": "italic"}]
        }]}]
    }))
    .value
    .expect("canonical marks compile");
    let out_of_order = compile_json(serde_json::json!({
        "type": "doc",
        "content": [{"type": "paragraph", "content": [{
            "type": "text",
            "text": "marked",
            "marks": [{"type": "italic"}, {"type": "bold"}]
        }]}]
    }))
    .value
    .expect("importable mark order is canonicalized");

    assert_eq!(out_of_order.elements(), canonical.elements());
    assert_eq!(out_of_order.semantic_key(), canonical.semantic_key());
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
                {"type": "hard_break"}
            ]},
            {"type": "horizontal_rule"},
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
        FfiViewerElement::InlineAtom { node_type, .. } if node_type == "hard_break"
    )));
    assert!(elements.iter().any(|element| matches!(element,
        FfiViewerElement::BlockAtom { node_type, .. } if node_type == "horizontal_rule"
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

#[test]
fn viewer_rejects_reserved_opaque_json_forgery_like_editor_import() {
    let result = compile_json(serde_json::json!({
        "type": "doc",
        "content": [{"type": "__opaque", "attrs": {"html_tag": "span"}}]
    }));

    let error = result.error.expect("reserved opaque JSON must reject");
    assert!(result.value.is_none());
    assert_eq!(error.code, "CODEC_INVARIANT_FAILED");
}

#[test]
fn viewer_enforces_editor_derived_output_limit() {
    let config = serde_json::json!({
        "initialization": {"type": "localEmpty"},
        "limits": {"editing": {"maxDerivedOutputBytes": 1}}
    });
    let result = compile_json_with(
        serde_json::json!({
            "type": "doc",
            "content": [{"type": "paragraph", "content": [{"type": "text", "text": "x"}]}]
        }),
        config.to_string(),
        true,
    );

    let error = result.error.expect("derived-output limit must reject");
    assert!(result.value.is_none());
    assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
    assert_eq!(
        error.details_json.as_deref(),
        Some(r#"{"field":"maxDerivedOutputBytes"}"#)
    );
}
