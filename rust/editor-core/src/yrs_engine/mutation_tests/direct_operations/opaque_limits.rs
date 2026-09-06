#[test]
fn malformed_reserved_opaque_insert_rejects_atomically_before_yrs_execution() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "AB" }]
        }]
    });
    let (doc, schema, limits, editing_limits, document) = diagnostic_doc(&source);
    let malformed = Node::void(
        "__opaque_json".into(),
        HashMap::from([
            ("original_type".into(), Value::String("mystery".into())),
            ("original_json".into(), Value::Null),
            ("opaque_placement".into(), Value::String("inline".into())),
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
            request_id: 172,
            base_document_revision: 0,
            origin: TransactionOrigin::LocalCommand,
            operations: vec![TypedOperation::InsertNode {
                at: point_for_test(1),
                node: malformed,
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
fn opaque_metadata_depth_and_width_limits_are_exact_and_atomic() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "AB" }]
        }]
    });
    let (doc, schema, limits, editing_limits, document) = diagnostic_doc(&source);
    let compile = |resource_limits: &ResourceLimits, node: Node| {
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
                request_id: 173,
                base_document_revision: 0,
                origin: TransactionOrigin::LocalCommand,
                operations: vec![TypedOperation::InsertNode {
                    at: point_for_test(1),
                    node,
                }],
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Auto,
            },
            &txn,
            &fragment,
        )
    };
    let nested = || {
        Node::void(
            "__opaque_json".into(),
            HashMap::from([
                ("original_type".into(), Value::String("mystery".into())),
                (
                    "original_json".into(),
                    json!({
                        "type": "mystery",
                        "attrs": { "payload": [[[0]]] }
                    }),
                ),
                ("opaque_placement".into(), Value::String("inline".into())),
            ]),
        )
    };
    let mut exact_depth = limits.clone();
    exact_depth.max_document_depth = 6;
    compile(&exact_depth, nested()).unwrap();
    let txn = doc.transact();
    let before = txn.encode_state_as_update_v1(&StateVector::default());
    drop(txn);
    exact_depth.max_document_depth = 5;
    let error = compile(&exact_depth, nested()).unwrap_err();
    assert_eq!(error.code, "OPERATION_LIMIT_EXCEEDED");
    assert_eq!(error.limit, Some(5));
    assert_eq!(error.actual, Some(6));

    let html_attrs = (0..100)
        .map(|index| (format!("data-{index}"), Value::String("x".into())))
        .collect::<serde_json::Map<_, _>>();
    let wide = || {
        Node::void(
            "__opaque".into(),
            HashMap::from([
                ("html_tag".into(), Value::String("widget-inline".into())),
                ("opaque_placement".into(), Value::String("inline".into())),
                ("html_attrs".into(), Value::Object(html_attrs.clone())),
            ]),
        )
    };
    let mut exact_width = limits.clone();
    exact_width.max_document_nodes = 103;
    compile(&exact_width, wide()).unwrap();
    exact_width.max_document_nodes = 102;
    let error = compile(&exact_width, wide()).unwrap_err();
    assert_eq!(error.code, "OPERATION_LIMIT_EXCEEDED");
    assert_eq!(error.limit, Some(102));
    assert_eq!(error.actual, Some(103));
    let txn = doc.transact();
    assert_eq!(
        txn.encode_state_as_update_v1(&StateVector::default()),
        before
    );
}

#[test]
fn opaque_metadata_max_input_bytes_is_exact_aggregated_and_atomic() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "AB" }]
        }]
    });
    let (doc, schema, limits, editing_limits, document) = diagnostic_doc(&source);
    let escaped_payload = "\u{0001}".repeat(4_096);
    let make_node = || {
        Node::void(
            "__opaque_json".into(),
            HashMap::from([
                ("original_type".into(), Value::String("mystery".into())),
                (
                    "original_json".into(),
                    json!({ "type": "mystery", "attrs": { "payload": escaped_payload } }),
                ),
                ("opaque_placement".into(), Value::String("inline".into())),
            ]),
        )
    };
    let exact_input = {
        let node = make_node();
        node.node_type().len() + serde_json::to_vec(node.attrs()).unwrap().len()
    };
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
                request_id: 174,
                base_document_revision: 0,
                origin: TransactionOrigin::LocalCommand,
                operations: vec![TypedOperation::InsertNode {
                    at: point_for_test(1),
                    node: make_node(),
                }],
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Auto,
            },
            &txn,
            &fragment,
        )
    };
    let initial_scan = {
        let txn = doc.transact();
        document.root().text_content().len() * 2
            + crdt_clock_scan_reservation(174, &txn, limits.max_encoded_state_bytes).unwrap() * 2
    };
    let baseline = compile(&limits).unwrap();
    let envelope_scan = {
        let txn = doc.transact();
        crdt_envelope(174, &txn, limits.max_encoded_state_bytes)
            .unwrap()
            .scan_work
    };
    let exact_total =
        (exact_input + initial_scan).max(baseline.mutation_plan.scan_work + envelope_scan);
    let mut exact_limits = limits.clone();
    exact_limits.max_input_bytes = exact_total;
    let admitted = compile(&exact_limits).unwrap();
    assert!(admitted.mutation_plan.scan_work < exact_total);

    let txn = doc.transact();
    let before = txn.encode_state_as_update_v1(&StateVector::default());
    drop(txn);
    exact_limits.max_input_bytes = exact_total - 1;
    let error = compile(&exact_limits).unwrap_err();
    assert_eq!(error.code, "OPERATION_LIMIT_EXCEEDED");
    assert_eq!(error.limit, Some(u64::try_from(exact_total - 1).unwrap()));
    assert_eq!(error.actual, Some(u64::try_from(exact_total).unwrap()));
    assert_eq!(error.details, Some(json!({ "field": "maxInputBytes" })));
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
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
fn sticky_reverse_mapping_rejects_unknown_wire_element_and_descendant_branches() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{
                "type": "mysteryInline",
                "content": [{ "type": "text", "text": "hidden" }]
            }]
        }]
    });
    let (doc, schema, _, _, _) = diagnostic_doc(&source);
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let XmlOut::Element(paragraph) = fragment.get(&txn, 0).unwrap() else {
        panic!("paragraph expected")
    };
    let XmlOut::Element(unknown) = paragraph.get(&txn, 0).unwrap() else {
        panic!("unknown element expected")
    };
    let XmlOut::Text(hidden) = unknown.get(&txn, 0).unwrap() else {
        panic!("hidden text expected")
    };
    let paragraph_branch = BranchPtr::from(
        <yrs::types::xml::XmlElementRef as AsRef<Branch>>::as_ref(&paragraph),
    );
    for (position, sticky) in [
        (
            1,
            StickyIndex::at(&txn, paragraph_branch, 0, Assoc::Before).unwrap(),
        ),
        (
            2,
            StickyIndex::at(&txn, paragraph_branch, 1, Assoc::Before).unwrap(),
        ),
    ] {
        assert_eq!(
            super::sticky_index_to_doc_pos(&txn, &fragment, &sticky, &schema),
            Some(position)
        );
    }
    for (position, affinity) in [
        (1, Affinity::Before),
        (1, Affinity::After),
        (2, Affinity::Before),
    ] {
        let point =
            super::doc_pos_to_relative_point(&txn, &fragment, position, affinity, &schema).unwrap();
        assert_eq!(point.affinity, affinity);
        assert_eq!(
            super::relative_point_to_doc_pos(&txn, &fragment, &point, &schema),
            Some(position)
        );
    }
    assert!(
        super::doc_pos_to_relative_point(&txn, &fragment, 2, Affinity::After, &schema).is_none()
    );
    for sticky in [
        StickyIndex::at(
            &txn,
            BranchPtr::from(<yrs::types::xml::XmlElementRef as AsRef<Branch>>::as_ref(
                &unknown,
            )),
            0,
            Assoc::After,
        )
        .unwrap(),
        StickyIndex::at(
            &txn,
            BranchPtr::from(<XmlTextRef as AsRef<Branch>>::as_ref(&hidden)),
            1,
            Assoc::After,
        )
        .unwrap(),
    ] {
        assert!(super::sticky_index_to_doc_pos(&txn, &fragment, &sticky, &schema).is_none());
    }
}

#[test]
fn structural_insert_splits_one_marked_unicode_storage_text_exactly() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "A😀e\u{301}Z" }]
        }]
    });
    let (doc, schema, limits, editing_limits, _) = diagnostic_doc(&source);
    {
        let mut txn = doc.transact_mut();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let text = paragraph_text(&fragment, &txn, 0);
        text.format(
            &mut txn,
            3,
            2,
            Attrs::from([(Arc::<str>::from("bold"), Any::Bool(true))]),
        );
    }
    let codec = YrsDocumentCodec::new(&schema, &limits);
    let (document, original_id, before_full_len) = {
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let json = codec.read_json(&fragment, &txn).unwrap();
        (
            from_prosemirror_json(&json, &schema, UnknownTypeMode::Preserve).unwrap(),
            <XmlTextRef as AsRef<Branch>>::as_ref(&paragraph_text(&fragment, &txn, 0)).id(),
            txn.encode_state_as_update_v1(&StateVector::default()).len(),
        )
    };
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
                request_id: 113,
                base_document_revision: 0,
                origin: TransactionOrigin::LocalInput,
                operations: vec![TypedOperation::InsertNode {
                    at: point_for_test(2),
                    node: Node::void("hardBreak".into(), HashMap::new()),
                }],
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Auto,
            },
            &txn,
            &fragment,
        )
        .unwrap()
    };
    assert!(matches!(
        compiled.mutation_plan.actions.first(),
        Some(YrsMutationAction::DeleteText {
            index_utf16: 3,
            len_utf16: 3,
            ..
        })
    ));
    let expected = to_prosemirror_json(&compiled.preview, &schema);
    let estimate = compiled.encoded_growth_bound;
    let update = {
        let mut txn = doc.transact_mut();
        execute_mutation_plan(compiled.mutation_plan, &mut txn);
        txn.commit();
        txn.encode_update_v1()
    };
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let XmlOut::Element(paragraph) = fragment.get(&txn, 0).unwrap() else {
        panic!("expected paragraph")
    };
    let children = paragraph.children(&txn).collect::<Vec<_>>();
    assert_eq!(children[0].id(), original_id);
    assert_ne!(children[2].id(), original_id);
    assert_eq!(codec.read_json(&fragment, &txn).unwrap(), expected);
    assert_eq!(expected["content"][0]["content"][0]["text"], "A😀");
    assert_eq!(expected["content"][0]["content"][2]["text"], "e\u{301}");
    assert_eq!(
        expected["content"][0]["content"][2]["marks"][0]["type"],
        "bold"
    );
    let update_len = update.len();
    assert!(update_len <= estimate, "{update_len} > {estimate}");
    let after_full_len = txn.encode_state_as_update_v1(&StateVector::default()).len();
    assert!(after_full_len <= before_full_len + estimate);
}

#[test]
fn structural_replace_swaps_an_inline_void_for_text_at_the_same_index() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "hardBreak" }]
        }]
    });

    let (actual, expected, html, update_len, estimate) = compile_and_execute(
        source,
        vec![TypedOperation::ReplaceRange {
            range: range_for_test(0, 1),
            content: Fragment::from(vec![Node::text("x".into(), vec![])]),
        }],
    );

    assert_eq!(actual, expected);
    assert_eq!(actual["content"][0]["content"][0]["text"], "x");
    assert!(html.contains(">x<"));
    assert!(update_len > 0);
    assert!(update_len <= estimate);
}

#[test]
fn structurally_identical_replace_is_a_document_no_op() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "hardBreak" }]
        }]
    });
    let (doc, schema, limits, editing_limits, document) = diagnostic_doc(&source);
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
                request_id: 116,
                base_document_revision: 0,
                origin: TransactionOrigin::LocalInput,
                operations: vec![TypedOperation::ReplaceRange {
                    range: range_for_test(0, 1),
                    content: Fragment::from(vec![Node::void("hardBreak".into(), HashMap::new())]),
                }],
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Auto,
            },
            &txn,
            &fragment,
        )
        .unwrap()
    };
    assert!(compiled.mutation_plan.actions.is_empty());
    assert_eq!(compiled.encoded_growth_bound, 0);
    assert_eq!(compiled.undo_units_bound, 0);
}
