#[test]
fn opaque_block_insert_inside_text_rejects_without_mutating_yrs() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "AB" }]
        }]
    });
    let (doc, schema, limits, editing_limits, document) = diagnostic_doc(&source);
    let opaque = Node::void(
        "__opaque_json".into(),
        HashMap::from([
            ("original_type".into(), Value::String("mysteryBlock".into())),
            (
                "original_json".into(),
                json!({ "type": "mysteryBlock", "content": [{ "type": "paragraph" }] }),
            ),
            ("opaque_placement".into(), Value::String("block".into())),
        ]),
    );
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let before = txn.encode_state_as_update_v1(&StateVector::default());
    let error = compile_transaction_with_yrs(
        CompilationContext {
            document: &document,
            selection: None,
            schema: &schema,
            resource_limits: &limits,
            editing_limits: &editing_limits,
            document_revision: 0,
            max_length: None,
        },
        TypedTransaction {
            request_id: 171,
            base_document_revision: 0,
            origin: TransactionOrigin::LocalCommand,
            operations: vec![TypedOperation::InsertNode {
                at: point_for_test(1),
                node: opaque,
            }],
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Auto,
        },
        &txn,
        &fragment,
    )
    .unwrap_err();
    assert_eq!(error.code, "DOCUMENT_INVALID");
    assert_eq!(
        txn.encode_state_as_update_v1(&StateVector::default()),
        before
    );
    assert_eq!(
        YrsDocumentCodec::new(&schema, &limits)
            .read_json(&fragment, &txn)
            .unwrap(),
        source
    );
}

#[test]
fn opaque_html_inline_and_block_insertions_round_trip_canonical_metadata() {
    let inline_attrs = HashMap::from([
        ("html_tag".into(), Value::String("widget-inline".into())),
        ("opaque_placement".into(), Value::String("inline".into())),
        ("html_attrs".into(), json!({ "data-id": "7" })),
        ("text_content".into(), Value::String("raw".into())),
        ("inner_html".into(), Value::String("<b>raw</b>".into())),
    ]);
    let inline = Node::void("__opaque".into(), inline_attrs);
    let inline_source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "ab" }]
        }]
    });
    let (inline_doc, inline_schema, inline_limits, inline_compiled) =
        compile_operations_with_schema(
            &inline_source,
            vec![
                TypedOperation::InsertNode {
                    at: point_for_test(1),
                    node: inline,
                },
                TypedOperation::InsertText {
                    at: point_for_test(1),
                    text: "X".into(),
                    marks: vec![],
                },
            ],
            tiptap_schema(),
        );
    let inline_expected = to_prosemirror_json(&inline_compiled.preview, &inline_schema);
    assert_eq!(
        to_html(&inline_compiled.preview, &inline_schema),
        "<p>a<widget-inline data-id=\"7\"><b>raw</b></widget-inline>Xb</p>"
    );
    {
        let mut txn = inline_doc.transact_mut();
        execute_mutation_plan(inline_compiled.mutation_plan, &mut txn);
    }
    let txn = inline_doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let actual = YrsDocumentCodec::new(&inline_schema, &inline_limits)
        .read_json(&fragment, &txn)
        .unwrap();
    assert_eq!(actual, inline_expected);
    assert_eq!(actual["content"][0]["content"][1]["type"], "__opaque");
    assert_eq!(actual["content"][0]["content"][2]["text"], "Xb");

    let block_attrs = HashMap::from([
        ("html_tag".into(), Value::String("widget-block".into())),
        ("opaque_placement".into(), Value::String("block".into())),
        ("html_attrs".into(), json!({ "data-kind": "card" })),
        ("inner_html".into(), Value::String("<i>card</i>".into())),
    ]);
    let block = Node::void("__opaque".into(), block_attrs);
    let block_source = json!({
        "type": "doc",
        "content": [
            { "type": "paragraph", "content": [{ "type": "text", "text": "A" }] },
            { "type": "paragraph", "content": [{ "type": "text", "text": "B" }] }
        ]
    });
    let schema = tiptap_schema();
    let at = rendered_scalar_offset(&block_source, &schema, "B") - 1;
    let (doc, schema, limits, compiled) = compile_operations_with_schema(
        &block_source,
        vec![TypedOperation::InsertNode {
            at: point_for_test(at),
            node: block,
        }],
        schema,
    );
    assert_eq!(
        to_html(&compiled.preview, &schema),
        "<p>A</p><widget-block data-kind=\"card\"><i>card</i></widget-block><p>B</p>"
    );
    let expected = to_prosemirror_json(&compiled.preview, &schema);
    {
        let mut txn = doc.transact_mut();
        execute_mutation_plan(compiled.mutation_plan, &mut txn);
    }
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    assert_eq!(
        YrsDocumentCodec::new(&schema, &limits)
            .read_json(&fragment, &txn)
            .unwrap(),
        expected
    );
}

#[test]
fn opaque_sentinel_validator_rejects_forged_shapes_and_known_aliases() {
    let schema = tiptap_schema();
    let limits = ResourceLimits::default();
    let json_attrs = |original_type: &str, original_json: Value| {
        HashMap::from([
            ("original_type".into(), Value::String(original_type.into())),
            ("original_json".into(), original_json),
            ("opaque_placement".into(), Value::String("inline".into())),
        ])
    };
    let mut forged = vec![
        Node::element(
            "__opaque_json".into(),
            json_attrs("mystery", json!({ "type": "mystery" })),
            Fragment::from(vec![Node::text("child".into(), vec![])]),
        ),
        Node::void(
            "__opaque_json".into(),
            json_attrs("mystery", json!({ "type": "different" })),
        ),
        Node::void(
            "__opaque_json".into(),
            HashMap::from([
                ("original_type".into(), Value::String("mystery".into())),
                ("opaque_placement".into(), Value::String("inline".into())),
            ]),
        ),
        Node::void(
            "__opaque_json".into(),
            json_attrs("mystery", Value::String("not-an-object".into())),
        ),
        Node::void(
            "__opaque_json".into(),
            json_attrs("__opaque", json!({ "type": "__opaque" })),
        ),
        Node::void("__opaque_json".into(), {
            let mut attrs = json_attrs("mystery", json!({ "type": "mystery" }));
            attrs.insert("extra".into(), Value::Bool(true));
            attrs
        }),
        Node::void(
            "__opaque_json".into(),
            json_attrs("paragraph", json!({ "type": "paragraph" })),
        ),
        Node::void(
            "__opaque_json".into(),
            json_attrs(
                "heading",
                json!({ "type": "heading", "attrs": { "level": 2 } }),
            ),
        ),
        Node::void(
            "__opaque_json".into(),
            json_attrs(
                "heading",
                json!({ "type": "heading", "attrs": { "level": "2" } }),
            ),
        ),
        Node::void(
            "__opaque".into(),
            HashMap::from([
                ("html_tag".into(), Value::String("Bad<Tag".into())),
                ("opaque_placement".into(), Value::String("inline".into())),
            ]),
        ),
        Node::element(
            "__opaque".into(),
            HashMap::from([
                ("html_tag".into(), Value::String("widget-inline".into())),
                ("opaque_placement".into(), Value::String("inline".into())),
            ]),
            Fragment::from(vec![Node::text("child".into(), vec![])]),
        ),
        Node::void(
            "__opaque".into(),
            HashMap::from([
                ("html_tag".into(), Value::String("strong".into())),
                ("opaque_placement".into(), Value::String("inline".into())),
            ]),
        ),
        Node::void(
            "__opaque".into(),
            HashMap::from([
                ("html_tag".into(), Value::String("widget-inline".into())),
                ("opaque_placement".into(), Value::String("inline".into())),
                ("html_attrs".into(), json!({ "bad key": "value" })),
            ]),
        ),
        Node::void(
            "__opaque".into(),
            HashMap::from([
                ("html_tag".into(), Value::String("widget-inline".into())),
                ("opaque_placement".into(), Value::String("inline".into())),
                ("html_attrs".into(), json!({ "data-id": 7 })),
            ]),
        ),
        Node::void(
            "__opaque".into(),
            HashMap::from([
                ("html_tag".into(), Value::String("widget-inline".into())),
                ("opaque_placement".into(), Value::String("inline".into())),
                ("extra".into(), Value::Bool(true)),
            ]),
        ),
        Node::void(
            "__opaque".into(),
            HashMap::from([
                ("html_tag".into(), Value::String("img".into())),
                ("opaque_placement".into(), Value::String("block".into())),
                (
                    "html_attrs".into(),
                    json!({ "src": "https://example.test/image.png" }),
                ),
            ]),
        ),
    ];
    for tag in ["b", "i", "del", "strike"] {
        forged.push(Node::void(
            "__opaque".into(),
            HashMap::from([
                ("html_tag".into(), Value::String(tag.into())),
                ("opaque_placement".into(), Value::String("inline".into())),
            ]),
        ));
    }
    forged.push(Node::void(
        "__opaque".into(),
        HashMap::from([
            ("html_tag".into(), Value::String("span".into())),
            ("opaque_placement".into(), Value::String("inline".into())),
            (
                "html_attrs".into(),
                json!({ "data-native-editor-mark": "bold" }),
            ),
        ]),
    ));
    for (case_index, opaque) in forged.into_iter().enumerate() {
        let document = Document::new(Node::element(
            "doc".into(),
            HashMap::new(),
            Fragment::from(vec![Node::element(
                "paragraph".into(),
                HashMap::new(),
                Fragment::from(vec![opaque]),
            )]),
        ));
        let error = match DocumentValidator::validate(&document, &schema, &limits) {
            Ok(_) => panic!("forged opaque case {case_index} was accepted"),
            Err(error) => error,
        };
        assert_eq!(error.code, "DOCUMENT_INVALID");
    }

    for (tag, html_attrs, placement) in [
        ("paragraph", json!({}), "inline"),
        (
            "img",
            json!({ "src": "https://example.test/image.png", "alt": "Inline" }),
            "inline",
        ),
        (
            "img",
            json!({ "src": "data:image/png;base64,AAAA", "alt": "Inline" }),
            "block",
        ),
    ] {
        let opaque = Node::void(
            "__opaque".into(),
            HashMap::from([
                ("html_tag".into(), Value::String(tag.into())),
                ("opaque_placement".into(), Value::String(placement.into())),
                ("html_attrs".into(), html_attrs),
            ]),
        );
        let document = Document::new(Node::element(
            "doc".into(),
            HashMap::new(),
            Fragment::from(if placement == "block" {
                vec![opaque]
            } else {
                vec![Node::element(
                    "paragraph".into(),
                    HashMap::new(),
                    Fragment::from(vec![opaque]),
                )]
            }),
        ));
        DocumentValidator::validate(&document, &schema, &limits).unwrap();
    }

    let semantic_block_image = Document::new(Node::element(
        "doc".into(),
        HashMap::new(),
        Fragment::from(vec![Node::void(
            "__opaque".into(),
            HashMap::from([
                ("html_tag".into(), Value::String("img".into())),
                ("opaque_placement".into(), Value::String("block".into())),
                (
                    "html_attrs".into(),
                    json!({ "src": "https://example.test/image.png" }),
                ),
            ]),
        )]),
    ));
    assert_eq!(
        DocumentValidator::validate(&semantic_block_image, &schema, &limits)
            .unwrap_err()
            .code,
        "DOCUMENT_INVALID"
    );

    let mention_schema = Schema::from_json(&json!({
        "nodes": [
            { "name": "doc", "content": "block*", "role": "doc" },
            { "name": "paragraph", "content": "inline*", "group": "block", "role": "textBlock", "htmlTag": "p" },
            { "name": "mention", "content": "", "group": "inline", "role": "inline", "isVoid": true },
            { "name": "text", "group": "inline", "role": "text" }
        ],
        "marks": []
    }))
    .unwrap();
    let mention_forge = Document::new(Node::element(
        "doc".into(),
        HashMap::new(),
        Fragment::from(vec![Node::element(
            "paragraph".into(),
            HashMap::new(),
            Fragment::from(vec![Node::void(
                "__opaque".into(),
                HashMap::from([
                    ("html_tag".into(), Value::String("span".into())),
                    ("opaque_placement".into(), Value::String("inline".into())),
                    (
                        "html_attrs".into(),
                        json!({ "data-native-editor-mention": "true" }),
                    ),
                ]),
            )]),
        )]),
    ));
    assert_eq!(
        DocumentValidator::validate(&mention_forge, &mention_schema, &limits)
            .unwrap_err()
            .code,
        "DOCUMENT_INVALID"
    );

    let inline_void_schema = Schema::from_json(&json!({
        "nodes": [
            { "name": "doc", "content": "block+", "role": "doc" },
            { "name": "paragraph", "content": "inline*", "group": "block", "role": "textBlock", "htmlTag": "p" },
            { "name": "inlineVoid", "content": "", "group": "inline", "role": "inline", "htmlTag": "x-void", "isVoid": true },
            { "name": "hardBreak", "content": "", "group": "inline", "role": "hardBreak", "htmlTag": "br", "isVoid": true },
            { "name": "text", "group": "inline", "role": "text" }
        ],
        "marks": []
    }))
    .unwrap();
    for tag in ["x-void", "br"] {
        let forged = Document::new(Node::element(
            "doc".into(),
            HashMap::new(),
            Fragment::from(vec![Node::element(
                "paragraph".into(),
                HashMap::new(),
                Fragment::from(vec![Node::void(
                    "__opaque".into(),
                    HashMap::from([
                        ("html_tag".into(), Value::String(tag.into())),
                        ("opaque_placement".into(), Value::String("inline".into())),
                    ]),
                )]),
            )]),
        ));
        assert_eq!(
            DocumentValidator::validate(&forged, &inline_void_schema, &limits)
                .unwrap_err()
                .code,
            "DOCUMENT_INVALID"
        );
    }
}

#[test]
fn uppercase_private_opaque_html_attributes_are_rejected_before_reimport_normalizes_them() {
    let limits = ResourceLimits::default();
    let mark_schema = tiptap_schema();
    let mark_forge = Document::new(Node::element(
        "doc".into(),
        HashMap::new(),
        Fragment::from(vec![Node::element(
            "paragraph".into(),
            HashMap::new(),
            Fragment::from(vec![Node::void(
                "__opaque".into(),
                HashMap::from([
                    ("html_tag".into(), Value::String("span".into())),
                    ("opaque_placement".into(), Value::String("inline".into())),
                    (
                        "html_attrs".into(),
                        json!({ "DATA-NATIVE-EDITOR-MARK": "bold" }),
                    ),
                    ("inner_html".into(), Value::String("marked".into())),
                ]),
            )]),
        )]),
    ));
    assert_eq!(
        DocumentValidator::validate(&mark_forge, &mark_schema, &limits)
            .unwrap_err()
            .code,
        "DOCUMENT_INVALID"
    );
    let mark_html = to_html(&mark_forge, &mark_schema);
    let reparsed_mark = from_html(&mark_html, &mark_schema, &FromHtmlOptions::default()).unwrap();
    let reparsed_mark_json = to_prosemirror_json(&reparsed_mark, &mark_schema);
    assert_eq!(
        reparsed_mark_json["content"][0]["content"][0]["marks"][0]["type"],
        "bold"
    );

    let mention_schema = Schema::from_json(&json!({
        "nodes": [
            { "name": "doc", "content": "block*", "role": "doc" },
            { "name": "paragraph", "content": "inline*", "group": "block", "role": "textBlock", "htmlTag": "p" },
            { "name": "mention", "content": "", "group": "inline", "role": "inline", "isVoid": true },
            { "name": "text", "group": "inline", "role": "text" }
        ],
        "marks": []
    }))
    .unwrap();
    let mention_forge = Document::new(Node::element(
        "doc".into(),
        HashMap::new(),
        Fragment::from(vec![Node::element(
            "paragraph".into(),
            HashMap::new(),
            Fragment::from(vec![Node::void(
                "__opaque".into(),
                HashMap::from([
                    ("html_tag".into(), Value::String("span".into())),
                    ("opaque_placement".into(), Value::String("inline".into())),
                    (
                        "html_attrs".into(),
                        json!({ "DATA-NATIVE-EDITOR-MENTION": "true" }),
                    ),
                    ("inner_html".into(), Value::String("@Ada".into())),
                ]),
            )]),
        )]),
    ));
    assert_eq!(
        DocumentValidator::validate(&mention_forge, &mention_schema, &limits)
            .unwrap_err()
            .code,
        "DOCUMENT_INVALID"
    );
    let mention_html = to_html(&mention_forge, &mention_schema);
    let reparsed_mention =
        from_html(&mention_html, &mention_schema, &FromHtmlOptions::default()).unwrap();
    assert_eq!(
        to_prosemirror_json(&reparsed_mention, &mention_schema)["content"][0]["content"][0]["type"],
        "mention"
    );
}

#[test]
fn non_span_private_mention_metadata_remains_opaque_after_export_and_reimport() {
    let schema = Schema::from_json(&json!({
        "nodes": [
            { "name": "doc", "content": "block*", "role": "doc" },
            { "name": "paragraph", "content": "inline*", "group": "block", "role": "textBlock", "htmlTag": "p" },
            { "name": "mention", "content": "", "group": "inline", "role": "inline", "isVoid": true },
            { "name": "text", "group": "inline", "role": "text" }
        ],
        "marks": []
    }))
    .unwrap();
    let document = Document::new(Node::element(
        "doc".into(),
        HashMap::new(),
        Fragment::from(vec![Node::element(
            "paragraph".into(),
            HashMap::new(),
            Fragment::from(vec![Node::void(
                "__opaque".into(),
                HashMap::from([
                    ("html_tag".into(), Value::String("x-mention".into())),
                    ("opaque_placement".into(), Value::String("inline".into())),
                    (
                        "html_attrs".into(),
                        json!({ "data-native-editor-mention": "true" }),
                    ),
                    ("inner_html".into(), Value::String("@Ada".into())),
                ]),
            )]),
        )]),
    ));
    DocumentValidator::validate(&document, &schema, &ResourceLimits::default()).unwrap();
    let html = to_html(&document, &schema);
    let reparsed = from_html(&html, &schema, &FromHtmlOptions::default()).unwrap();
    let json = to_prosemirror_json(&reparsed, &schema);
    assert_eq!(json["content"][0]["content"][0]["type"], "__opaque");
    assert_eq!(
        json["content"][0]["content"][0]["attrs"]["html_attrs"]["data-native-editor-mention"],
        "true"
    );
}

#[test]
fn canonical_foreign_mixed_case_attributes_validate_and_round_trip() {
    let schema = tiptap_schema();
    let limits = ResourceLimits::default();
    for (tag, key, value) in [
        ("svg", "viewBox", "0 0 10 10"),
        ("math", "definitionURL", "https://example.test/definition"),
    ] {
        let document = Document::new(Node::element(
            "doc".into(),
            HashMap::new(),
            Fragment::from(vec![Node::element(
                "paragraph".into(),
                HashMap::new(),
                Fragment::from(vec![Node::void(
                    "__opaque".into(),
                    HashMap::from([
                        ("html_tag".into(), Value::String(tag.into())),
                        ("opaque_placement".into(), Value::String("inline".into())),
                        ("html_attrs".into(), json!({ key: value })),
                    ]),
                )]),
            )]),
        ));
        DocumentValidator::validate(&document, &schema, &limits).unwrap();
        let html = to_html(&document, &schema);
        let reparsed = from_html(&html, &schema, &FromHtmlOptions::default()).unwrap();
        let json = to_prosemirror_json(&reparsed, &schema);
        assert_eq!(
            json["content"][0]["content"][0]["attrs"]["html_attrs"][key],
            value
        );
    }

    for (tag, key) in [
        ("a", "attributeName"),
        ("svg", "DATA-NATIVE-EDITOR-MENTION"),
    ] {
        let forged = Document::new(Node::element(
            "doc".into(),
            HashMap::new(),
            Fragment::from(vec![Node::element(
                "paragraph".into(),
                HashMap::new(),
                Fragment::from(vec![Node::void(
                    "__opaque".into(),
                    HashMap::from([
                        ("html_tag".into(), Value::String(tag.into())),
                        ("opaque_placement".into(), Value::String("inline".into())),
                        ("html_attrs".into(), json!({ key: "forged" })),
                    ]),
                )]),
            )]),
        ));
        assert_eq!(
            DocumentValidator::validate(&forged, &schema, &limits)
                .unwrap_err()
                .code,
            "DOCUMENT_INVALID"
        );
    }
    for (tag, html_attrs) in [
        (
            "svg",
            json!({ "viewBox": "0 0 10 10", "viewbox": "0 0 20 20" }),
        ),
        (
            "math",
            json!({ "definitionURL": "a", "definitionurl": "b" }),
        ),
    ] {
        let collision = Document::new(Node::element(
            "doc".into(),
            HashMap::new(),
            Fragment::from(vec![Node::element(
                "paragraph".into(),
                HashMap::new(),
                Fragment::from(vec![Node::void(
                    "__opaque".into(),
                    HashMap::from([
                        ("html_tag".into(), Value::String(tag.into())),
                        ("opaque_placement".into(), Value::String("inline".into())),
                        ("html_attrs".into(), html_attrs),
                    ]),
                )]),
            )]),
        ));
        assert_eq!(
            DocumentValidator::validate(&collision, &schema, &limits)
                .unwrap_err()
                .code,
            "DOCUMENT_INVALID"
        );
    }
}

#[test]
fn foreign_qualified_attributes_preserve_prefixes_without_colliding() {
    let schema = tiptap_schema();
    let limits = ResourceLimits::default();
    let document = Document::new(Node::element(
        "doc".into(),
        HashMap::new(),
        Fragment::from(vec![Node::element(
            "paragraph".into(),
            HashMap::new(),
            Fragment::from(vec![Node::void(
                "__opaque".into(),
                HashMap::from([
                    ("html_tag".into(), Value::String("svg".into())),
                    ("opaque_placement".into(), Value::String("inline".into())),
                    (
                        "html_attrs".into(),
                        json!({
                            "href": "plain",
                            "xlink:href": "linked",
                            "xml:lang": "en",
                            "xmlns:xlink": "http://www.w3.org/1999/xlink"
                        }),
                    ),
                ]),
            )]),
        )]),
    ));
    DocumentValidator::validate(&document, &schema, &limits).unwrap();
    let expected = to_prosemirror_json(&document, &schema);
    let html = to_html(&document, &schema);
    let reparsed = from_html(&html, &schema, &FromHtmlOptions::default()).unwrap();
    let actual = to_prosemirror_json(&reparsed, &schema);
    assert_eq!(actual, expected);
}
