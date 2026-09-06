#[test]
fn structural_delete_preflight_rejects_same_count_child_substitution_atomically() {
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
                request_id: 111,
                base_document_revision: 0,
                origin: TransactionOrigin::LocalInput,
                operations: vec![TypedOperation::DeleteRange {
                    range: range_for_test(0, 1),
                }],
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Auto,
            },
            &txn,
            &fragment,
        )
        .unwrap()
    };
    {
        let mut txn = doc.transact_mut();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let XmlOut::Element(paragraph) = fragment.get(&txn, 0).unwrap() else {
            panic!("expected paragraph")
        };
        paragraph.remove_range(&mut txn, 0, 1);
        paragraph.insert(&mut txn, 0, XmlElementPrelim::empty("hardBreak"));
    }
    let txn = doc.transact();
    let before = txn.state_vector();
    let error = preflight_mutation_plan(111, &compiled.mutation_plan, &txn).unwrap_err();
    assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
    assert_eq!(txn.state_vector(), before);
}

#[test]
fn pure_insert_preflight_rejects_same_count_parent_substitution_atomically() {
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
                request_id: 118,
                base_document_revision: 0,
                origin: TransactionOrigin::LocalCommand,
                operations: vec![TypedOperation::InsertNode {
                    at: point_for_test(0),
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
        compiled.mutation_plan.actions.as_slice(),
        [YrsMutationAction::InsertXmlChildren { .. }]
    ));
    {
        let mut txn = doc.transact_mut();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        fragment.remove_range(&mut txn, 0, 1);
        let paragraph = fragment.insert(&mut txn, 0, XmlElementPrelim::empty("paragraph"));
        paragraph.insert(&mut txn, 0, XmlElementPrelim::empty("hardBreak"));
    }
    let txn = doc.transact();
    let before = txn.encode_state_as_update_v1(&StateVector::default());
    let error = preflight_mutation_plan(118, &compiled.mutation_plan, &txn).unwrap_err();
    assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
    assert_eq!(
        txn.encode_state_as_update_v1(&StateVector::default()),
        before
    );
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    assert_eq!(
        YrsDocumentCodec::new(&schema, &limits)
            .read_json(&fragment, &txn)
            .unwrap(),
        source
    );
}

#[test]
fn nested_text_and_attribute_preflight_reject_gc_replaced_parents_without_panicking() {
    let text_source = json!({
        "type": "doc",
        "content": [{
            "type": "blockquote",
            "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "abc" }] }]
        }]
    });
    let (text_doc, schema, limits, editing_limits, document) = diagnostic_doc(&text_source);
    let mut text_compiled = {
        let txn = text_doc.transact();
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
                request_id: 119,
                base_document_revision: 0,
                origin: TransactionOrigin::LocalCommand,
                operations: vec![TypedOperation::InsertText {
                    at: point_for_test(1),
                    text: "X".into(),
                    marks: vec![],
                }],
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Auto,
            },
            &txn,
            &fragment,
        )
        .unwrap()
    };
    let text_exact = text_compiled.mutation_plan.compilation_work_for_test()
        + text_compiled
            .mutation_plan
            .expected_preflight_work_for_test();
    {
        let mut txn = text_doc.transact_mut();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        fragment.remove_range(&mut txn, 0, 1);
        let quote = fragment.insert(&mut txn, 0, XmlElementPrelim::empty("blockquote"));
        let paragraph = quote.insert(&mut txn, 0, XmlElementPrelim::empty("paragraph"));
        paragraph.insert(&mut txn, 0, XmlTextPrelim::new("abc"));
    }
    let txn = text_doc.transact();
    let before = txn.encode_state_as_update_v1(&StateVector::default());
    text_compiled
        .mutation_plan
        .set_work_limit_for_test(text_exact - 1);
    assert_eq!(
        preflight_mutation_plan(119, &text_compiled.mutation_plan, &txn)
            .unwrap_err()
            .code,
        "OPERATION_LIMIT_EXCEEDED"
    );
    text_compiled
        .mutation_plan
        .set_work_limit_for_test(text_exact);
    assert_eq!(
        preflight_mutation_plan(119, &text_compiled.mutation_plan, &txn)
            .unwrap_err()
            .code,
        "ENGINE_INVARIANT_FAILED"
    );
    assert_eq!(
        txn.encode_state_as_update_v1(&StateVector::default()),
        before
    );

    let attr_source = json!({
        "type": "doc",
        "content": [{
            "type": "blockquote",
            "content": [{
                "type": "h2",
                "attrs": { "id": "old" },
                "content": [{ "type": "text", "text": "heading" }]
            }]
        }]
    });
    let (attr_doc, attr_schema, attr_limits, attr_editing, attr_document) =
        diagnostic_doc_with_schema(&attr_source, attribute_schema());
    let attr_compiled = {
        let txn = attr_doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        compile_transaction_with_yrs(
            CompilationContext {
                document: &attr_document,
                selection: None,
                schema: &attr_schema,
                resource_limits: &attr_limits,
                editing_limits: &attr_editing,
                document_revision: 0,
                max_length: None,
            },
            TypedTransaction {
                request_id: 120,
                base_document_revision: 0,
                origin: TransactionOrigin::LocalCommand,
                operations: vec![TypedOperation::UpdateNodeAttrs {
                    at: point_for_test(1),
                    attrs: HashMap::from([("id".into(), Value::String("new".into()))]),
                }],
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Auto,
            },
            &txn,
            &fragment,
        )
        .unwrap()
    };
    {
        let mut txn = attr_doc.transact_mut();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        fragment.remove_range(&mut txn, 0, 1);
        let quote = fragment.insert(&mut txn, 0, XmlElementPrelim::empty("blockquote"));
        let heading = quote.insert(&mut txn, 0, XmlElementPrelim::empty("heading"));
        heading.insert_attribute(&mut txn, "level", Any::BigInt(2));
        heading.insert_attribute(&mut txn, "id", Any::String("old".into()));
        heading.insert(&mut txn, 0, XmlTextPrelim::new("heading"));
    }
    let txn = attr_doc.transact();
    let before = txn.encode_state_as_update_v1(&StateVector::default());
    assert_eq!(
        preflight_mutation_plan(120, &attr_compiled.mutation_plan, &txn)
            .unwrap_err()
            .code,
        "ENGINE_INVARIANT_FAILED"
    );
    assert_eq!(
        txn.encode_state_as_update_v1(&StateVector::default()),
        before
    );
}

#[test]
fn create_text_preflight_rejects_same_count_neighbor_replacement() {
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
                request_id: 86,
                base_document_revision: 0,
                origin: TransactionOrigin::LocalInput,
                operations: vec![TypedOperation::InsertText {
                    at: point_for_test(0),
                    text: "x".into(),
                    marks: vec![],
                }],
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Auto,
            },
            &txn,
            &fragment,
        )
        .unwrap()
    };
    {
        let mut txn = doc.transact_mut();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let XmlOut::Element(parent) = fragment.get(&txn, 0).unwrap() else {
            panic!("paragraph expected")
        };
        parent.remove_range(&mut txn, 0, 1);
        parent.insert(&mut txn, 0, XmlElementPrelim::empty("hardBreak"));
    }
    let txn = doc.transact();
    let before = txn.state_vector();
    let error = preflight_mutation_plan(86, &compiled.mutation_plan, &txn).unwrap_err();
    assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
    assert_eq!(txn.state_vector(), before);
}

#[test]
fn nested_empty_textblock_create_preserves_unaffected_sibling_identity_and_sticky() {
    let source = json!({
        "type": "doc",
        "content": [
            {
                "type": "bulletList",
                "content": [{
                    "type": "listItem",
                    "content": [{ "type": "paragraph" }]
                }]
            },
            {
                "type": "paragraph",
                "content": [{ "type": "text", "text": "omega" }]
            }
        ]
    });
    let schema = tiptap_schema();
    let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let map = PositionMap::build(&document, &schema);
    let insertion_offset = map.doc_to_scalar(map.block(0).unwrap().doc_start, &document);
    let limits = ResourceLimits::default();
    let editing_limits = EditingLimits::default();
    let doc = utf16_doc();
    let codec = YrsDocumentCodec::new(&schema, &limits);
    {
        let mut txn = doc.transact_mut();
        let fragment = txn.get_or_insert_xml_fragment("prosemirror");
        codec
            .apply_json(&fragment, &mut txn, &json!({ "type": "doc" }), &source)
            .unwrap();
    }
    let (sibling_id, sticky, before_full_len) = {
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let sibling = paragraph_text(&fragment, &txn, 1);
        let id = <XmlTextRef as AsRef<Branch>>::as_ref(&sibling).id();
        let sticky = StickyIndex::at(
            &txn,
            BranchPtr::from(<XmlTextRef as AsRef<Branch>>::as_ref(&sibling)),
            2,
            Assoc::After,
        )
        .unwrap();
        (
            id,
            sticky,
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
                request_id: 78,
                base_document_revision: 0,
                origin: TransactionOrigin::LocalInput,
                operations: vec![TypedOperation::InsertText {
                    at: point_for_test(insertion_offset),
                    text: "item".into(),
                    marks: vec![Mark::new("italic".into(), HashMap::new())],
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
        Some(YrsMutationAction::CreateText { .. })
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
    assert_eq!(codec.read_json(&fragment, &txn).unwrap(), expected);
    let sibling = paragraph_text(&fragment, &txn, 1);
    assert_eq!(
        <XmlTextRef as AsRef<Branch>>::as_ref(&sibling).id(),
        sibling_id
    );
    let resolved = sticky.get_offset(&txn).unwrap();
    assert_eq!(resolved.branch.id(), sibling_id);
    assert_eq!(resolved.index, 2);
    assert!(update.len() <= estimate);
    let after_full_len = txn.encode_state_as_update_v1(&StateVector::default()).len();
    assert!(after_full_len <= before_full_len + estimate);
}

#[test]
fn create_text_preflight_rejects_stale_parent_atomically() {
    let source = json!({
        "type": "doc",
        "content": [{ "type": "paragraph" }]
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
                request_id: 79,
                base_document_revision: 0,
                origin: TransactionOrigin::LocalApi,
                operations: vec![TypedOperation::InsertText {
                    at: point_for_test(1),
                    text: "x".into(),
                    marks: vec![],
                }],
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Auto,
            },
            &txn,
            &fragment,
        )
        .unwrap()
    };
    let before = doc.transact().state_vector();
    {
        let mut txn = doc.transact_mut();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let XmlOut::Element(parent) = fragment.get(&txn, 0).unwrap() else {
            panic!("paragraph expected")
        };
        parent.insert(&mut txn, 0, XmlElementPrelim::empty("hardBreak"));
    }
    let after_external = doc.transact().state_vector();
    assert_ne!(before, after_external);
    let txn = doc.transact();
    let error = preflight_mutation_plan(79, &compiled.mutation_plan, &txn).unwrap_err();
    assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
    assert_eq!(txn.state_vector(), after_external);
}
