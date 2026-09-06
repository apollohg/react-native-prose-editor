#[test]
fn block_insert_node_at_rendered_inter_block_break_targets_the_root_boundary() {
    let source = json!({
        "type": "doc",
        "content": [
            { "type": "paragraph", "content": [{ "type": "text", "text": "A" }] },
            { "type": "paragraph", "content": [{ "type": "text", "text": "B" }] }
        ]
    });
    let schema = tiptap_schema();
    let break_offset = rendered_scalar_offset(&source, &schema, "B") - 1;
    let (actual, expected, _, _, _) = compile_and_execute(
        source,
        vec![TypedOperation::InsertNode {
            at: point_for_test(break_offset),
            node: Node::void("horizontalRule".into(), HashMap::new()),
        }],
    );
    assert_eq!(actual, expected);
    assert_eq!(
        actual["content"]
            .as_array()
            .unwrap()
            .iter()
            .map(|node| node["type"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["paragraph", "horizontalRule", "paragraph"]
    );
}

#[test]
fn wide_block_insert_resolver_admits_exact_work_and_rejects_one_under_atomically() {
    let large_text = format!("{}😀", "A".repeat(4_096));
    let mut wide_inline = vec![json!({ "type": "text", "text": large_text })];
    wide_inline.extend(
        (0..160)
            .map(|_| json!({ "type": "hardBreak" }))
            .collect::<Vec<_>>(),
    );
    wide_inline.push(json!({ "type": "text", "text": "end" }));
    let source = json!({
        "type": "doc",
        "content": [
            { "type": "paragraph", "content": wide_inline },
            { "type": "paragraph", "content": [{ "type": "text", "text": "tail" }] }
        ]
    });
    let (doc, schema, limits, editing_limits, document) = diagnostic_doc(&source);
    let at = rendered_scalar_offset(&source, &schema, "tail") - 1;
    let compile = |resource_limits: &ResourceLimits| {
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        compile_transaction_with_yrs(
            CompilationContext {
                document: &document,
                selection: None,
                schema: &schema,
                resource_limits,
                editing_limits: &editing_limits,
                document_revision: 0,
                max_length: None,
            },
            TypedTransaction {
                request_id: 116,
                base_document_revision: 0,
                origin: TransactionOrigin::LocalCommand,
                operations: vec![TypedOperation::InsertNode {
                    at: point_for_test(at),
                    node: Node::void("horizontalRule".into(), HashMap::new()),
                }],
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Auto,
            },
            &txn,
            &fragment,
        )
    };

    let baseline = compile(&limits).unwrap();
    let resolver_work = baseline.mutation_plan.position_resolver_work_for_test();
    assert!(resolver_work > 4_256);
    let exact = baseline.mutation_plan.scan_work;
    let mut exact_limits = limits.clone();
    exact_limits.max_input_bytes = exact;
    let admitted = compile(&exact_limits).unwrap();
    assert_eq!(admitted.mutation_plan.scan_work, exact);
    assert_eq!(
        admitted.mutation_plan.position_resolver_work_for_test(),
        resolver_work
    );

    let txn = doc.transact();
    let before = txn.encode_state_as_update_v1(&StateVector::default());
    drop(txn);
    exact_limits.max_input_bytes = exact - 1;
    let error = compile(&exact_limits).unwrap_err();
    assert_eq!(error.code, "OPERATION_LIMIT_EXCEEDED");
    assert_eq!(error.limit, Some(u64::try_from(exact - 1).unwrap()));
    assert_eq!(error.actual, Some(u64::try_from(exact).unwrap()));
    let txn = doc.transact();
    assert_eq!(
        txn.encode_state_as_update_v1(&StateVector::default()),
        before
    );
    drop(txn);

    let non_resolver_work = exact.checked_sub(resolver_work).unwrap();
    let early_limit = non_resolver_work.checked_add(20).unwrap();
    assert!(early_limit < exact);
    exact_limits.max_input_bytes = early_limit;
    let error = compile(&exact_limits).unwrap_err();
    assert_eq!(error.code, "OPERATION_LIMIT_EXCEEDED");
    assert_eq!(error.limit, Some(u64::try_from(early_limit).unwrap()));
    assert_eq!(error.actual, Some(u64::try_from(early_limit + 1).unwrap()));
    let txn = doc.transact();
    assert_eq!(
        txn.encode_state_as_update_v1(&StateVector::default()),
        before
    );
}

#[test]
fn opaque_block_insert_at_rendered_break_targets_root_and_preserves_wire_tree() {
    let source = json!({
        "type": "doc",
        "content": [
            { "type": "paragraph", "content": [{ "type": "text", "text": "A" }] },
            { "type": "paragraph", "content": [{ "type": "text", "text": "B" }] }
        ]
    });
    let original = json!({
        "type": "mysteryBlock",
        "attrs": { "payload": [1, 2, 3] },
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "wire-only" }]
        }]
    });
    let opaque = Node::void(
        "__opaque_json".into(),
        HashMap::from([
            ("original_type".into(), Value::String("mysteryBlock".into())),
            ("original_json".into(), original.clone()),
            ("opaque_placement".into(), Value::String("block".into())),
        ]),
    );
    let schema = tiptap_schema();
    let break_offset = rendered_scalar_offset(&source, &schema, "B") - 1;
    let (actual, expected, _, _, _) = compile_and_execute(
        source,
        vec![TypedOperation::InsertNode {
            at: point_for_test(break_offset),
            node: opaque,
        }],
    );
    assert_eq!(actual, expected);
    assert_eq!(actual["content"][1], original);
    assert_eq!(actual["content"][2]["content"][0]["text"], "B");
}

#[test]
fn block_insert_node_maps_public_start_end_and_empty_block_boundaries() {
    for (source, offset, affinity, expected_index) in [
        (
            json!({
                "type": "doc",
                "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "A" }] }]
            }),
            0,
            Affinity::Before,
            0,
        ),
        (
            json!({
                "type": "doc",
                "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "A" }] }]
            }),
            1,
            Affinity::After,
            1,
        ),
        (
            json!({ "type": "doc", "content": [{ "type": "paragraph" }] }),
            0,
            Affinity::Before,
            0,
        ),
        (
            json!({ "type": "doc", "content": [{ "type": "paragraph" }] }),
            1,
            Affinity::After,
            1,
        ),
    ] {
        let (actual, expected, _, _, _) = compile_and_execute(
            source,
            vec![TypedOperation::InsertNode {
                at: RevisionedPosition {
                    offset,
                    kind: EditorOffsetKind::Scalar,
                    affinity,
                },
                node: Node::void("horizontalRule".into(), HashMap::new()),
            }],
        );
        assert_eq!(actual, expected);
        assert_eq!(actual["content"][expected_index]["type"], "horizontalRule");
    }
}

#[test]
fn inline_insert_node_keeps_textblock_mapping_at_public_start_and_end() {
    for (source, offset, affinity, expected_inline_index) in [
        (
            json!({
                "type": "doc",
                "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "A" }] }]
            }),
            0,
            Affinity::Before,
            0,
        ),
        (
            json!({
                "type": "doc",
                "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "A" }] }]
            }),
            1,
            Affinity::After,
            1,
        ),
    ] {
        let (actual, expected, _, _, _) = compile_and_execute(
            source,
            vec![TypedOperation::InsertNode {
                at: RevisionedPosition {
                    offset,
                    kind: EditorOffsetKind::Scalar,
                    affinity,
                },
                node: Node::void("hardBreak".into(), HashMap::new()),
            }],
        );
        assert_eq!(actual, expected);
        assert_eq!(actual["content"].as_array().unwrap().len(), 1);
        assert_eq!(
            actual["content"][0]["content"][expected_inline_index]["type"],
            "hardBreak"
        );
    }
}

#[test]
fn custom_inline_roles_preserve_every_offset_mapping_for_direct_insert_node() {
    let schema = Schema::from_json(&json!({
        "nodes": [
            { "name": "root", "content": "block*", "role": "doc" },
            { "name": "body", "content": "inline*", "group": "block", "role": "textBlock" },
            { "name": "softBreak", "content": "", "group": "inline", "role": "hardBreak", "isVoid": true, "allowUndeclaredAttrs": true },
            { "name": "widget", "content": "", "group": "inline", "role": "inline", "isVoid": true, "allowUndeclaredAttrs": true },
            { "name": "text", "group": "inline", "role": "text" }
        ],
        "marks": []
    }))
    .unwrap();
    let long_label = "😀".repeat(2_048);
    let source = json!({
        "type": "root",
        "content": [{
            "type": "body",
            "content": [
                { "type": "softBreak", "attrs": { "label": "ignored-long-label" } },
                { "type": "widget", "attrs": { "label": long_label.clone() } }
            ]
        }]
    });
    let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Error).unwrap();
    let rendered = crate::render::rendered_text(&document, &schema);
    let map = PositionMap::build(&document, &schema);
    assert_eq!(rendered, format!("\n[{long_label}]"));
    let terminal_scalar = 1 + 2 + u32::try_from(long_label.chars().count()).unwrap();
    let mapped = (0..=terminal_scalar)
        .map(|offset| map.scalar_to_doc(offset, &document))
        .collect::<Vec<_>>();
    assert_eq!(mapped[0], 1);
    assert!(mapped[1..terminal_scalar as usize]
        .iter()
        .all(|position| *position == 2));
    assert_eq!(mapped[terminal_scalar as usize], 3);
    assert_eq!(
        (0..=3)
            .map(|position| map.doc_to_scalar(position, &document))
            .collect::<Vec<_>>(),
        vec![0, 0, 1, terminal_scalar]
    );

    let limits = ResourceLimits::default();
    let editing_limits = EditingLimits::default();
    let doc = utf16_doc();
    let codec = YrsDocumentCodec::new(&schema, &limits);
    {
        let mut txn = doc.transact_mut();
        let fragment = txn.get_or_insert_xml_fragment("prosemirror");
        codec
            .apply_json(&fragment, &mut txn, &json!({ "type": "root" }), &source)
            .unwrap();
    }
    let compiled = {
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        compile_transaction_with_yrs(
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
                request_id: 117,
                base_document_revision: 0,
                origin: TransactionOrigin::LocalCommand,
                operations: vec![TypedOperation::InsertNode {
                    at: RevisionedPosition {
                        offset: 2,
                        kind: EditorOffsetKind::Scalar,
                        affinity: Affinity::After,
                    },
                    node: Node::void(
                        "widget".into(),
                        HashMap::from([("label".into(), Value::String("Grace".into()))]),
                    ),
                }],
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Auto,
            },
            &txn,
            &fragment,
        )
        .unwrap()
    };
    assert_eq!(
        to_prosemirror_json(&compiled.preview, &schema)["content"][0]["content"]
            .as_array()
            .unwrap()
            .iter()
            .map(|node| node["attrs"]["label"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["ignored-long-label", "Grace", long_label.as_str()]
    );
    assert!(compiled.mutation_plan.position_resolver_work_for_test() > long_label.len());
    {
        let mut txn = doc.transact_mut();
        execute_mutation_plan(compiled.mutation_plan, &mut txn);
    }
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    assert_eq!(
        codec.read_json(&fragment, &txn).unwrap(),
        to_prosemirror_json(&compiled.preview, &schema)
    );
}

#[test]
fn block_insert_at_separator_between_list_items_uses_an_affinity_valid_item_boundary() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "bulletList",
            "content": [
                { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "one" }] }] },
                { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "two" }] }] }
            ]
        }]
    });
    let schema = tiptap_schema();
    let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let rendered = crate::render::rendered_text(&document, &schema);
    let separator =
        u32::try_from(rendered[..rendered.find('\n').unwrap()].chars().count()).unwrap();
    for (affinity, item_index) in [(Affinity::Before, 0usize), (Affinity::After, 1usize)] {
        let (actual, expected, _, _, _) = compile_and_execute(
            source.clone(),
            vec![TypedOperation::InsertNode {
                at: RevisionedPosition {
                    offset: separator,
                    kind: EditorOffsetKind::Scalar,
                    affinity,
                },
                node: Node::void("horizontalRule".into(), HashMap::new()),
            }],
        );
        assert_eq!(actual, expected);
        assert_eq!(
            actual["content"][0]["content"][item_index]["content"][1]["type"],
            "horizontalRule"
        );
    }
}

#[test]
fn split_block_then_block_insert_at_same_revisioned_position_uses_created_boundary() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "AB" }]
        }]
    });
    let (actual, expected, _, _, _) = compile_and_execute(
        source,
        vec![
            TypedOperation::SplitBlock {
                at: point_for_test(1),
                node_type: "paragraph".into(),
                attrs: HashMap::new(),
            },
            TypedOperation::InsertNode {
                at: point_for_test(1),
                node: Node::void("horizontalRule".into(), HashMap::new()),
            },
        ],
    );
    assert_eq!(actual, expected);
    assert_eq!(
        actual["content"]
            .as_array()
            .unwrap()
            .iter()
            .map(|node| node["type"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["paragraph", "horizontalRule", "paragraph"]
    );
}

#[test]
fn nested_opaque_json_insert_remains_one_semantic_atom_for_follow_up_edits() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "ab" }]
        }]
    });
    let original = json!({
        "type": "mysteryInline",
        "attrs": { "payload": { "nested": true } },
        "content": [{ "type": "text", "text": "wire-only" }]
    });
    let opaque = Node::void(
        "__opaque_json".into(),
        HashMap::from([
            (
                "original_type".into(),
                Value::String("mysteryInline".into()),
            ),
            ("original_json".into(), original.clone()),
            ("opaque_placement".into(), Value::String("inline".into())),
        ]),
    );
    let (actual, expected, _, _, _) = compile_and_execute(
        source,
        vec![
            TypedOperation::InsertNode {
                at: point_for_test(1),
                node: opaque,
            },
            TypedOperation::InsertText {
                at: point_for_test(1),
                text: "X".into(),
                marks: vec![],
            },
        ],
    );
    assert_eq!(actual, expected);
    assert_eq!(actual["content"][0]["content"][1], original);
    assert_eq!(actual["content"][0]["content"][2]["text"], "Xb");
}

#[test]
fn existing_unknown_wire_element_with_descendants_has_void_semantic_size() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [
                { "type": "text", "text": "a" },
                {
                    "type": "mysteryInline",
                    "content": [{ "type": "text", "text": "wire-only" }]
                },
                { "type": "text", "text": "b" }
            ]
        }]
    });
    let schema = tiptap_schema();
    let b = rendered_scalar_offset(&source, &schema, "b");
    let (actual, expected, _, _, _) = compile_and_execute(
        source,
        vec![TypedOperation::InsertText {
            at: point_for_test(b),
            text: "X".into(),
            marks: vec![],
        }],
    );
    assert_eq!(actual, expected);
    assert_eq!(actual["content"][0]["content"][1]["type"], "mysteryInline");
    assert_eq!(actual["content"][0]["content"][2]["text"], "Xb");
}

#[test]
fn existing_unknown_block_wire_tree_is_one_semantic_atom_for_follow_up_text() {
    let source = json!({
        "type": "doc",
        "content": [
            {
                "type": "mysteryBlock",
                "content": [{
                    "type": "paragraph",
                    "content": [{ "type": "text", "text": "wire-only" }]
                }]
            },
            { "type": "paragraph", "content": [{ "type": "text", "text": "B" }] }
        ]
    });
    let schema = tiptap_schema();
    let b = rendered_scalar_offset(&source, &schema, "B");
    let (actual, expected, _, _, _) = compile_and_execute(
        source,
        vec![TypedOperation::InsertText {
            at: point_for_test(b),
            text: "X".into(),
            marks: vec![],
        }],
    );
    assert_eq!(actual, expected);
    assert_eq!(actual["content"][0]["type"], "mysteryBlock");
    assert_eq!(actual["content"][1]["content"][0]["text"], "XB");
}

#[test]
fn malformed_wire_headings_remain_one_opaque_atom_and_hide_descendants() {
    for attrs in [
        None,
        Some(json!({ "level": 7 })),
        Some(json!({ "level": 2.5 })),
    ] {
        let mut heading = json!({
            "type": "heading",
            "content": [{ "type": "text", "text": "wire-only" }]
        });
        if let Some(attrs) = attrs {
            heading["attrs"] = attrs;
        }
        let source = json!({
            "type": "doc",
            "content": [
                heading,
                { "type": "paragraph", "content": [{ "type": "text", "text": "B" }] }
            ]
        });
        let schema = tiptap_schema();
        let b = rendered_scalar_offset(&source, &schema, "B");
        let (actual, expected, _, _, _) = compile_and_execute(
            source,
            vec![TypedOperation::InsertText {
                at: point_for_test(b),
                text: "X".into(),
                marks: vec![],
            }],
        );
        assert_eq!(actual, expected);
        assert_eq!(actual["content"][0]["type"], "heading");
        assert_eq!(actual["content"][1]["content"][0]["text"], "XB");
    }

    let source = json!({
        "type": "doc",
        "content": [
            {
                "type": "heading",
                "attrs": { "level": 7 },
                "content": [{ "type": "text", "text": "hidden" }]
            },
            { "type": "paragraph", "content": [{ "type": "text", "text": "tail" }] }
        ]
    });
    let (doc, schema, _, _, _) = diagnostic_doc(&source);
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let XmlOut::Element(heading) = fragment.get(&txn, 0).unwrap() else {
        panic!("heading wire element expected")
    };
    let XmlOut::Text(hidden) = heading.get(&txn, 0).unwrap() else {
        panic!("hidden wire text expected")
    };
    let descendant = StickyIndex::at(
        &txn,
        BranchPtr::from(<XmlTextRef as AsRef<Branch>>::as_ref(&hidden)),
        1,
        Assoc::After,
    )
    .unwrap();
    assert!(super::sticky_index_to_doc_pos(&txn, &fragment, &descendant, &schema).is_none());

    let valid_source = json!({
        "type": "doc",
        "content": [{
            "type": "h2",
            "content": [{ "type": "text", "text": "visible" }]
        }]
    });
    let (valid_doc, valid_schema, _, _, _) = diagnostic_doc(&valid_source);
    let valid_txn = valid_doc.transact();
    let valid_fragment = valid_txn.get_xml_fragment("prosemirror").unwrap();
    let XmlOut::Element(valid_heading) = valid_fragment.get(&valid_txn, 0).unwrap() else {
        panic!("valid heading wire element expected")
    };
    assert_eq!(valid_heading.tag().as_ref(), "heading");
    let XmlOut::Text(visible) = valid_heading.get(&valid_txn, 0).unwrap() else {
        panic!("valid heading text expected")
    };
    let visible_sticky = StickyIndex::at(
        &valid_txn,
        BranchPtr::from(<XmlTextRef as AsRef<Branch>>::as_ref(&visible)),
        1,
        Assoc::After,
    )
    .unwrap();
    assert_eq!(
        super::sticky_index_to_doc_pos(&valid_txn, &valid_fragment, &visible_sticky, &valid_schema,),
        Some(2)
    );
}

#[test]
fn shared_and_oversized_heading_levels_are_bounded_opaque_atoms() {
    let source = json!({
        "type": "doc",
        "content": [
            {
                "type": "h2",
                "content": [{ "type": "text", "text": "hidden" }]
            },
            { "type": "paragraph", "content": [{ "type": "text", "text": "tail" }] }
        ]
    });

    for shared_kind in 0..2 {
        let (doc, schema, limits, _, _) = diagnostic_doc(&source);
        let hidden = {
            let txn = doc.transact();
            let fragment = txn.get_xml_fragment("prosemirror").unwrap();
            let XmlOut::Element(heading) = fragment.get(&txn, 0).unwrap() else {
                panic!("heading expected")
            };
            let XmlOut::Text(text) = heading.get(&txn, 0).unwrap() else {
                panic!("heading text expected")
            };
            StickyIndex::at(
                &txn,
                BranchPtr::from(<XmlTextRef as AsRef<Branch>>::as_ref(&text)),
                1,
                Assoc::After,
            )
            .unwrap()
        };
        {
            let mut txn = doc.transact_mut();
            let fragment = txn.get_xml_fragment("prosemirror").unwrap();
            let XmlOut::Element(heading) = fragment.get(&txn, 0).unwrap() else {
                panic!("heading expected")
            };
            if shared_kind == 0 {
                heading.insert_attribute(
                    &mut txn,
                    "level",
                    MapPrelim::from([("nested", Any::String("2".into()))]),
                );
            } else {
                heading.insert_attribute(
                    &mut txn,
                    "level",
                    ArrayPrelim::from(vec![Any::String("2".into())]),
                );
            }
        }
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let XmlOut::Element(heading) = fragment.get(&txn, 0).unwrap() else {
            panic!("heading expected")
        };
        assert_eq!(
            super::codec::normalized_wire_element_node_type(&heading, &txn),
            "heading"
        );
        assert!(super::sticky_index_to_doc_pos(&txn, &fragment, &hidden, &schema).is_none());
        let after_atom = StickyIndex::at(
            &txn,
            BranchPtr::from(<yrs::types::xml::XmlFragmentRef as AsRef<Branch>>::as_ref(
                &fragment,
            )),
            1,
            Assoc::After,
        )
        .unwrap();
        assert_eq!(
            super::sticky_index_to_doc_pos(&txn, &fragment, &after_atom, &schema),
            Some(1)
        );
        let error = YrsDocumentCodec::new(&schema, &limits)
            .read_json(&fragment, &txn)
            .unwrap_err();
        assert_eq!(error.code, "CODEC_INVARIANT_FAILED");
    }

    let (doc, schema, mut limits, _, _) = diagnostic_doc(&source);
    let oversized = format!("{}2", "0".repeat(128 * 1024));
    {
        let mut txn = doc.transact_mut();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let XmlOut::Element(heading) = fragment.get(&txn, 0).unwrap() else {
            panic!("heading expected")
        };
        heading.insert_attribute(&mut txn, "level", oversized);
    }
    limits.max_input_bytes = 64;
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let XmlOut::Element(heading) = fragment.get(&txn, 0).unwrap() else {
        panic!("heading expected")
    };
    assert_eq!(
        super::codec::normalized_wire_element_node_type(&heading, &txn),
        "heading"
    );
    let error = YrsDocumentCodec::new(&schema, &limits)
        .read_json(&fragment, &txn)
        .unwrap_err();
    assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
    assert_eq!(error.limit, Some(64));
}
