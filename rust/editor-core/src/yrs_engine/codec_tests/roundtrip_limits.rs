fn read_raw(doc: &Doc) -> Value {
    let schema = tiptap_schema();
    let limits = ResourceLimits::default();
    let codec = YrsDocumentCodec::new(&schema, &limits);
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    codec.read_json(&fragment, &txn).unwrap()
}

fn mark_attrs(mark: &str) -> Attrs {
    mark_attrs_value(mark, Any::Bool(true))
}

fn mark_attrs_value(mark: &str, value: Any) -> Attrs {
    let mut attrs = Attrs::default();
    attrs.insert(mark.into(), value);
    attrs
}

#[test]
fn read_json_coalesces_only_adjacent_equal_mark_text_storage() {
    let siblings = utf16_doc();
    {
        let mut txn = siblings.transact_mut();
        let fragment = txn.get_or_insert_xml_fragment("prosemirror");
        let paragraph = fragment.push_back(&mut txn, XmlElementPrelim::empty("paragraph"));
        paragraph.push_back(&mut txn, XmlTextPrelim::new("a"));
        paragraph.push_back(&mut txn, XmlTextPrelim::new("b"));
    }
    assert_eq!(
        read_raw(&siblings),
        json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{ "type": "text", "text": "ab" }]
            }]
        })
    );

    let diff_runs = utf16_doc();
    let text = {
        let mut txn = diff_runs.transact_mut();
        let fragment = txn.get_or_insert_xml_fragment("prosemirror");
        let paragraph = fragment.push_back(&mut txn, XmlElementPrelim::empty("paragraph"));
        let text = paragraph.push_back(&mut txn, XmlTextPrelim::new(""));
        text.insert_with_attributes(&mut txn, 0, "ab", mark_attrs("bold"));
        text.insert_embed_with_attributes(&mut txn, 1, Any::Bool(false), Attrs::default());
        text
    };
    let txn = diff_runs.transact();
    assert_eq!(text.diff(&txn, YChange::identity).len(), 3);
    let schema = tiptap_schema();
    let limits = ResourceLimits::default();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let error = YrsDocumentCodec::new(&schema, &limits)
        .read_json(&fragment, &txn)
        .unwrap_err();
    assert_eq!(error.code, "CODEC_INVARIANT_FAILED");
    assert_eq!(
        error.details,
        Some(json!({
            "phase": "candidateMaterialization",
            "field": "xmlTextRun"
        }))
    );
    drop(txn);

    let shared_embed = utf16_doc();
    {
        let mut txn = shared_embed.transact_mut();
        let fragment = txn.get_or_insert_xml_fragment("prosemirror");
        let paragraph = fragment.push_back(&mut txn, XmlElementPrelim::empty("paragraph"));
        let text = paragraph.push_back(&mut txn, XmlTextPrelim::new(""));
        text.insert_embed_with_attributes(
            &mut txn,
            0,
            ArrayPrelim::from(vec![Any::String("shared".into())]),
            Attrs::default(),
        );
    }
    let txn = shared_embed.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let error = YrsDocumentCodec::new(&schema, &limits)
        .read_json(&fragment, &txn)
        .unwrap_err();
    assert_eq!(error.code, "CODEC_INVARIANT_FAILED");
    assert_eq!(
        error.details,
        Some(json!({
            "phase": "candidateMaterialization",
            "field": "xmlTextRun"
        }))
    );

    let different_marks = utf16_doc();
    {
        let mut txn = different_marks.transact_mut();
        let fragment = txn.get_or_insert_xml_fragment("prosemirror");
        let paragraph = fragment.push_back(&mut txn, XmlElementPrelim::empty("paragraph"));
        let bold = paragraph.push_back(&mut txn, XmlTextPrelim::new(""));
        bold.insert_with_attributes(&mut txn, 0, "a", mark_attrs("bold"));
        let italic = paragraph.push_back(&mut txn, XmlTextPrelim::new(""));
        italic.insert_with_attributes(&mut txn, 0, "b", mark_attrs("italic"));
    }
    assert_eq!(
        read_raw(&different_marks)["content"][0]["content"]
            .as_array()
            .unwrap()
            .len(),
        2
    );

    let element_boundary = utf16_doc();
    {
        let mut txn = element_boundary.transact_mut();
        let fragment = txn.get_or_insert_xml_fragment("prosemirror");
        let paragraph = fragment.push_back(&mut txn, XmlElementPrelim::empty("paragraph"));
        paragraph.push_back(&mut txn, XmlTextPrelim::new("a"));
        paragraph.push_back(&mut txn, XmlElementPrelim::empty("hardBreak"));
        paragraph.push_back(&mut txn, XmlTextPrelim::new("b"));
    }
    let element_json = read_raw(&element_boundary);
    assert_eq!(
        element_json["content"][0]["content"],
        json!([
            { "type": "text", "text": "a" },
            { "type": "hardBreak" },
            { "type": "text", "text": "b" }
        ])
    );

    let schema = tiptap_schema();
    let exact_limits = ResourceLimits {
        max_document_nodes: 3,
        ..ResourceLimits::default()
    };
    let txn = siblings.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    assert!(YrsDocumentCodec::new(&schema, &exact_limits)
        .read_json(&fragment, &txn)
        .is_ok());
    let rejected_limits = ResourceLimits {
        max_document_nodes: 2,
        ..ResourceLimits::default()
    };
    let error = YrsDocumentCodec::new(&schema, &rejected_limits)
        .read_json(&fragment, &txn)
        .unwrap_err();
    assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
    assert_eq!(error.limit, Some(2));
    assert_eq!(error.actual, Some(3));

    let raw_work_limits = ResourceLimits {
        max_document_nodes: 3,
        ..ResourceLimits::default()
    };
    let exact_fragmented = utf16_doc();
    {
        let mut txn = exact_fragmented.transact_mut();
        let fragment = txn.get_or_insert_xml_fragment("prosemirror");
        let paragraph = fragment.push_back(&mut txn, XmlElementPrelim::empty("paragraph"));
        for _ in 0..384 {
            paragraph.push_back(&mut txn, XmlTextPrelim::new("x"));
        }
    }
    let txn = exact_fragmented.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let exact_fragmented_json = YrsDocumentCodec::new(&schema, &raw_work_limits)
        .read_json(&fragment, &txn)
        .unwrap();
    assert_eq!(
        exact_fragmented_json["content"][0]["content"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        exact_fragmented_json["content"][0]["content"][0]["text"]
            .as_str()
            .unwrap()
            .chars()
            .count(),
        384
    );

    let fragmented = utf16_doc();
    {
        let mut txn = fragmented.transact_mut();
        let fragment = txn.get_or_insert_xml_fragment("prosemirror");
        let paragraph = fragment.push_back(&mut txn, XmlElementPrelim::empty("paragraph"));
        for _ in 0..385 {
            paragraph.push_back(&mut txn, XmlTextPrelim::new("x"));
        }
    }
    let txn = fragmented.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let error = YrsDocumentCodec::new(&schema, &raw_work_limits)
        .read_json(&fragment, &txn)
        .unwrap_err();
    assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
    assert_eq!(error.limit, Some(384));
    assert_eq!(error.actual, Some(385));
    assert_eq!(
        error.details,
        Some(json!({
            "phase": "candidateMaterialization",
            "dimension": "rawTextRuns"
        }))
    );
}

#[test]
fn round_trips_heading_mark_attrs_emoji_and_combining_text() {
    let next = json!({
        "type": "doc",
        "content": [{
            "type": "heading",
            "attrs": { "level": 2 },
            "content": [{
                "type": "text",
                "text": "A😀e\u{301}",
                "marks": [{
                    "type": "link",
                    "attrs": {
                        "href": "https://example.test",
                        "target": "_blank"
                    }
                }]
            }]
        }]
    });

    assert_eq!(round_trip(next.clone()), next);
}

#[test]
fn prepared_builder_matches_codec_for_heading_void_and_nested_any() {
    let input = json!({
        "type": "doc",
        "content": [{
            "type": "h2",
            "attrs": {
                "data": { "nested": [true, null, "😀", { "value": 7 }] }
            },
            "content": [{ "type": "text", "text": "title" }]
        }, {
            "type": "hardBreak",
            "attrs": { "meta": ["void", 2] }
        }, {
            "type": "image",
            "attrs": {
                "src": "https://example.test/image.png",
                "metadata": { "widths": [320, 640] }
            }
        }]
    });
    let schema = tiptap_schema();
    let limits = ResourceLimits::default();
    let codec = YrsDocumentCodec::new(&schema, &limits);

    let imported = utf16_doc();
    {
        let mut txn = imported.transact_mut();
        let fragment = txn.get_or_insert_xml_fragment("prosemirror");
        codec
            .apply_json(&fragment, &mut txn, &empty_json("doc"), &input)
            .unwrap();
    }
    let expected = {
        let txn = imported.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        codec.read_json(&fragment, &txn).unwrap()
    };

    let prepared_doc = utf16_doc();
    {
        let mut txn = prepared_doc.transact_mut();
        let fragment = txn.get_or_insert_xml_fragment("prosemirror");
        let batch = prepare_xml_nodes(input["content"].as_array().unwrap(), &limits, 2).unwrap();
        for child in batch.nodes {
            insert_prepared_node(&fragment, &mut txn, child.index, child.node);
        }
    }
    let actual = {
        let txn = prepared_doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        codec.read_json(&fragment, &txn).unwrap()
    };
    assert_eq!(actual, expected);

    let unsafe_number = json!({
        "type": "image",
        "attrs": { "unsafe": u64::MAX }
    });
    let error = prepare_xml_nodes(&[unsafe_number], &limits, 2).unwrap_err();
    assert_eq!(error.code, "DOCUMENT_INVALID");
}

#[test]
fn borrowed_mark_conversion_preserves_exact_sorted_attrs() {
    let marks = vec![
        json!({ "type": "italic" }),
        json!({
            "type": "link",
            "attrs": {
                "href": "https://example.test/😀",
                "title": "e\u{301} אב"
            }
        }),
        json!({ "type": "bold" }),
    ];

    let attrs = marks_to_attrs(Some(&marks));
    let schema = tiptap_schema();
    let limits = ResourceLimits::default();
    let mut budget = super::ConversionBudget::new(&limits);
    assert_eq!(
        attrs_to_marks(Some(&attrs), &schema, &mut budget).unwrap(),
        vec![
            json!({ "type": "bold" }),
            json!({ "type": "italic" }),
            json!({
                "type": "link",
                "attrs": {
                    "href": "https://example.test/😀",
                    "title": "e\u{301} אב"
                }
            }),
        ]
    );
    let empty = Attrs::default();
    assert!(actual_marks_equal(None, Some(&empty)));
    assert!(actual_marks_equal(Some(&empty), None));
}

#[test]
fn shared_codec_preserves_multimark_unicode_and_opaque_payload_exactly() {
    let input = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{
                "type": "text",
                "text": "A😀e\u{301} אב",
                "marks": [
                    { "type": "italic" },
                    { "type": "link", "attrs": { "href": "https://example.test/😀" } },
                    { "type": "bold" }
                ]
            }]
        }, {
            "type": "__opaque_json",
            "attrs": {
                "original_type": "callout",
                "opaque_placement": "block",
                "original_json": {
                    "type": "callout",
                    "attrs": { "payload": ["😀", "e\u{301}", "אב", null] }
                }
            }
        }]
    });
    let expected = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{
                "type": "text",
                "text": "A😀e\u{301} אב",
                "marks": [
                    { "type": "bold" },
                    { "type": "italic" },
                    { "type": "link", "attrs": { "href": "https://example.test/😀" } }
                ]
            }]
        }, {
            "type": "__opaque_json",
            "attrs": {
                "original_type": "callout",
                "opaque_placement": "block",
                "original_json": {
                    "type": "callout",
                    "attrs": { "payload": ["😀", "e\u{301}", "אב", null] }
                }
            }
        }]
    });

    assert_eq!(round_trip(input), expected);
}

#[test]
fn round_trips_list_attrs_and_inline_and_block_void_nodes() {
    let next = json!({
        "type": "doc",
        "content": [
            {
                "type": "orderedList",
                "attrs": { "start": 3 },
                "content": [{
                    "type": "listItem",
                    "content": [{
                        "type": "paragraph",
                        "content": [
                            { "type": "text", "text": "before" },
                            { "type": "hardBreak" },
                            { "type": "text", "text": "after" }
                        ]
                    }]
                }]
            },
            { "type": "horizontalRule" },
            {
                "type": "image",
                "attrs": {
                    "src": "https://example.test/image.png",
                    "alt": "example"
                }
            }
        ]
    });

    assert_eq!(round_trip(next.clone()), next);
}

#[test]
fn round_trips_opaque_json_nodes_without_changing_payloads() {
    let next = json!({
        "type": "doc",
        "content": [{
            "type": "__opaque_json",
            "attrs": {
                "original_type": "callout",
                "opaque_placement": "block",
                "original_json": {
                    "type": "callout",
                    "attrs": {
                        "kind": "warning",
                        "metadata": [true, null, { "rank": 2 }]
                    },
                    "content": [
                        { "type": "text", "text": "preserve " },
                        { "type": "text", "text": "me" }
                    ]
                }
            }
        }]
    });

    assert_eq!(round_trip(next.clone()), next);
}

#[test]
fn read_and_write_share_exact_node_limit_accounting() {
    let schema = tiptap_schema();
    let next = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "three nodes" }]
        }]
    });
    let exact_limits = ResourceLimits {
        max_document_nodes: 3,
        ..Default::default()
    };
    let exact_codec = YrsDocumentCodec::new(&schema, &exact_limits);
    let doc = utf16_doc();
    {
        let mut txn = doc.transact_mut();
        let fragment = txn.get_or_insert_xml_fragment("prosemirror");
        exact_codec
            .apply_json(&fragment, &mut txn, &empty_json("doc"), &next)
            .unwrap();
    }
    {
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        assert_eq!(exact_codec.read_json(&fragment, &txn).unwrap(), next);
    }

    let mut rejected_limits = exact_limits.clone();
    rejected_limits.max_document_nodes = 2;
    let rejected_codec = YrsDocumentCodec::new(&schema, &rejected_limits);
    let write_doc = utf16_doc();
    let write_error = {
        let mut txn = write_doc.transact_mut();
        let fragment = txn.get_or_insert_xml_fragment("prosemirror");
        rejected_codec
            .apply_json(&fragment, &mut txn, &empty_json("doc"), &next)
            .unwrap_err()
    };
    let read_error = {
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        rejected_codec.read_json(&fragment, &txn).unwrap_err()
    };

    for error in [write_error, read_error] {
        assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
        assert_eq!(error.limit, Some(2));
        assert_eq!(error.actual, Some(3));
    }
}

#[test]
fn read_and_write_share_exact_depth_limit_accounting() {
    let schema = tiptap_schema();
    let next = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "depth three" }]
        }]
    });
    let exact_limits = ResourceLimits {
        max_document_depth: 3,
        ..Default::default()
    };
    let exact_codec = YrsDocumentCodec::new(&schema, &exact_limits);
    let doc = utf16_doc();
    {
        let mut txn = doc.transact_mut();
        let fragment = txn.get_or_insert_xml_fragment("prosemirror");
        exact_codec
            .apply_json(&fragment, &mut txn, &empty_json("doc"), &next)
            .unwrap();
    }
    {
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        assert_eq!(exact_codec.read_json(&fragment, &txn).unwrap(), next);
    }

    let mut rejected_limits = exact_limits.clone();
    rejected_limits.max_document_depth = 2;
    let rejected_codec = YrsDocumentCodec::new(&schema, &rejected_limits);
    let write_doc = utf16_doc();
    let write_error = {
        let mut txn = write_doc.transact_mut();
        let fragment = txn.get_or_insert_xml_fragment("prosemirror");
        rejected_codec
            .apply_json(&fragment, &mut txn, &empty_json("doc"), &next)
            .unwrap_err()
    };
    let read_error = {
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        rejected_codec.read_json(&fragment, &txn).unwrap_err()
    };

    for error in [write_error, read_error] {
        assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
        assert_eq!(error.limit, Some(2));
        assert_eq!(error.actual, Some(3));
    }
}

#[test]
fn any_materialization_has_exact_depth_work_and_output_boundaries() {
    let nested = yrs::Any::Array(vec![yrs::Any::Array(vec![yrs::Any::Null].into())].into());
    let exact_depth = ResourceLimits {
        max_document_depth: 3,
        ..ResourceLimits::default()
    };
    assert_eq!(
        any_to_json_bounded(&nested, &exact_depth).unwrap(),
        serde_json::json!([[null]])
    );
    let depth_error = any_to_json_bounded(
        &nested,
        &ResourceLimits {
            max_document_depth: 2,
            ..ResourceLimits::default()
        },
    )
    .unwrap_err();
    assert_eq!(depth_error.code, "DOCUMENT_LIMIT_EXCEEDED");
    assert_eq!(depth_error.limit, Some(2));
    assert_eq!(depth_error.actual, Some(3));

    let exact_output = ResourceLimits {
        max_input_bytes: 3,
        ..ResourceLimits::default()
    };
    assert_eq!(
        any_to_json_bounded(&yrs::Any::String("x".into()), &exact_output).unwrap(),
        serde_json::json!("x")
    );
    let output_error = any_to_json_bounded(
        &yrs::Any::String("x".into()),
        &ResourceLimits {
            max_input_bytes: 2,
            ..ResourceLimits::default()
        },
    )
    .unwrap_err();
    assert_eq!(output_error.code, "DOCUMENT_LIMIT_EXCEEDED");
    assert_eq!(output_error.limit, Some(2));
    assert_eq!(output_error.actual, Some(3));

    let exact_items = yrs::Any::Array(vec![yrs::Any::Null; 127].into());
    assert!(any_to_json_bounded(
        &exact_items,
        &ResourceLimits {
            max_document_nodes: 1,
            ..ResourceLimits::default()
        }
    )
    .is_ok());
    let over_items = yrs::Any::Array(vec![yrs::Any::Null; 128].into());
    let work_error = any_to_json_bounded(
        &over_items,
        &ResourceLimits {
            max_document_nodes: 1,
            ..ResourceLimits::default()
        },
    )
    .unwrap_err();
    assert_eq!(work_error.code, "DOCUMENT_LIMIT_EXCEEDED");
    assert_eq!(work_error.limit, Some(128));
    assert_eq!(work_error.actual, Some(129));
}
