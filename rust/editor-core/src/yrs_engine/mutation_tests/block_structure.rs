use super::*;

#[test]
fn split_block_directly_preserves_left_marked_unicode_storage() {
    let source = json!({
        "type": "doc",
        "content": [
            {
                "type": "paragraph",
                "content": [{ "type": "text", "text": "A😀B", "marks": [{ "type": "bold" }] }]
            },
            { "type": "paragraph", "content": [{ "type": "text", "text": "tail" }] }
        ]
    });
    let (doc, schema, limits, editing_limits, document) = diagnostic_doc(&source);
    let codec = YrsDocumentCodec::new(&schema, &limits);
    let (left_block_id, left_text_id, tail_block_id, tail_text_id, tail_sticky, before_full_len) = {
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let children = fragment.children(&txn).collect::<Vec<_>>();
        let left_text = paragraph_text(&fragment, &txn, 0);
        let tail_text = paragraph_text(&fragment, &txn, 1);
        let tail_sticky = StickyIndex::at(
            &txn,
            BranchPtr::from(<XmlTextRef as AsRef<Branch>>::as_ref(&tail_text)),
            2,
            Assoc::After,
        )
        .unwrap();
        (
            children[0].id(),
            <XmlTextRef as AsRef<Branch>>::as_ref(&left_text).id(),
            children[1].id(),
            <XmlTextRef as AsRef<Branch>>::as_ref(&tail_text).id(),
            tail_sticky,
            txn.encode_state_as_update_v1(&StateVector::default()).len(),
        )
    };
    let mut compiled = {
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
                request_id: 120,
                base_document_revision: 0,
                origin: TransactionOrigin::LocalCommand,
                operations: vec![TypedOperation::SplitBlock {
                    at: point_for_test(2),
                    node_type: "paragraph".into(),
                    attrs: HashMap::new(),
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
        [
            YrsMutationAction::DeleteText {
                index_utf16: 3,
                len_utf16: 1,
                operation_index: 0,
                ..
            },
            YrsMutationAction::InsertXmlChildren {
                child_index: 1,
                nodes,
                operation_index: 0,
                ..
            }
        ] if nodes.len() == 1 && nodes[0].index == 1
    ));
    {
        let txn = doc.transact();
        let preflight =
            preflight_mutation_work_for_test(120, &compiled.mutation_plan, &txn).unwrap();
        let exact = compiled.mutation_plan.compilation_work_for_test() + preflight;
        compiled.mutation_plan.set_work_limit_for_test(exact);
        preflight_mutation_plan(120, &compiled.mutation_plan, &txn).unwrap();
        compiled.mutation_plan.set_work_limit_for_test(exact - 1);
        assert_eq!(
            preflight_mutation_plan(120, &compiled.mutation_plan, &txn)
                .unwrap_err()
                .code,
            "OPERATION_LIMIT_EXCEEDED"
        );
        compiled.mutation_plan.set_work_limit_for_test(exact);
    }
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
    let children = fragment.children(&txn).collect::<Vec<_>>();
    assert_eq!(children.len(), 3);
    assert_eq!(children[0].id(), left_block_id);
    assert_eq!(children[2].id(), tail_block_id);
    assert_ne!(children[1].id(), left_block_id);
    assert_ne!(children[1].id(), tail_block_id);
    assert_eq!(
        <XmlTextRef as AsRef<Branch>>::as_ref(&paragraph_text(&fragment, &txn, 0)).id(),
        left_text_id
    );
    assert_eq!(
        <XmlTextRef as AsRef<Branch>>::as_ref(&paragraph_text(&fragment, &txn, 2)).id(),
        tail_text_id
    );
    let resolved_sticky = tail_sticky.get_offset(&txn).unwrap();
    assert_eq!(resolved_sticky.branch.id(), tail_text_id);
    assert_eq!(resolved_sticky.index, 2);
    let actual = codec.read_json(&fragment, &txn).unwrap();
    assert_eq!(actual, expected);
    assert_eq!(actual["content"].as_array().unwrap().len(), 3);
    assert_eq!(actual["content"][0]["content"][0]["text"], "A😀");
    assert_eq!(actual["content"][1]["content"][0]["text"], "B");
    assert!(update.len() <= estimate, "{} > {estimate}", update.len());
    let after_full_len = txn.encode_state_as_update_v1(&StateVector::default()).len();
    assert!(after_full_len <= before_full_len + estimate);
}

#[test]
fn split_block_then_insert_text_targets_the_created_right_block() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "A😀B" }]
        }]
    });
    let (actual, expected, _, _, _) = compile_and_execute(
        source,
        vec![
            TypedOperation::SplitBlock {
                at: point_for_test(2),
                node_type: "paragraph".into(),
                attrs: HashMap::new(),
            },
            TypedOperation::InsertText {
                at: point_for_test(2),
                text: "R".into(),
                marks: vec![],
            },
        ],
    );
    assert_eq!(actual, expected);
    assert_eq!(actual["content"][1]["content"][0]["text"], "RB");
}

#[test]
fn insert_text_then_split_block_folds_into_the_retained_left_text() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "A😀B" }]
        }]
    });
    let (actual, expected, _, _, _) = compile_and_execute(
        source,
        vec![
            TypedOperation::InsertText {
                at: point_for_test(2),
                text: "L".into(),
                marks: vec![],
            },
            TypedOperation::SplitBlock {
                at: point_for_test(2),
                node_type: "paragraph".into(),
                attrs: HashMap::new(),
            },
        ],
    );
    assert_eq!(actual, expected);
    assert_eq!(actual["content"][0]["content"][0]["text"], "A😀L");
    assert_eq!(actual["content"][1]["content"][0]["text"], "B");
}

#[test]
fn insert_text_in_copied_suffix_then_split_folds_into_the_prepared_right_block() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "A😀B" }]
        }]
    });
    let (actual, expected, _, _, _) = compile_and_execute(
        source,
        vec![
            TypedOperation::InsertText {
                at: point_for_test(3),
                text: "X".into(),
                marks: vec![],
            },
            TypedOperation::SplitBlock {
                at: point_for_test(2),
                node_type: "paragraph".into(),
                attrs: HashMap::new(),
            },
        ],
    );
    assert_eq!(actual, expected);
    assert_eq!(actual["content"][0]["content"][0]["text"], "A😀");
    assert_eq!(actual["content"][1]["content"][0]["text"], "BX");
}

#[test]
fn mark_in_copied_suffix_then_split_folds_into_the_prepared_right_block() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "A😀B" }]
        }]
    });
    let (actual, expected, _, _, _) = compile_and_execute(
        source,
        vec![
            TypedOperation::AddMark {
                range: range_for_test(2, 3),
                mark: Mark::new("bold".into(), HashMap::new()),
            },
            TypedOperation::SplitBlock {
                at: point_for_test(2),
                node_type: "paragraph".into(),
                attrs: HashMap::new(),
            },
        ],
    );
    assert_eq!(actual, expected);
    assert_eq!(actual["content"][1]["content"][0]["text"], "B");
    assert_eq!(
        actual["content"][1]["content"][0]["marks"][0]["type"],
        "bold"
    );
}

#[test]
fn split_block_same_boundary_affinity_selects_left_or_right_block() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "A😀B" }]
        }]
    });
    let operations = |affinity| {
        vec![
            TypedOperation::SplitBlock {
                at: point_for_test(2),
                node_type: "paragraph".into(),
                attrs: HashMap::new(),
            },
            TypedOperation::InsertText {
                at: RevisionedPosition {
                    offset: 2,
                    kind: EditorOffsetKind::Scalar,
                    affinity,
                },
                text: "X".into(),
                marks: vec![],
            },
        ]
    };
    let (before, expected_before, _, _, _) =
        compile_and_execute(source.clone(), operations(Affinity::Before));
    assert_eq!(before, expected_before);
    assert_eq!(before["content"][0]["content"][0]["text"], "A😀X");
    assert_eq!(before["content"][1]["content"][0]["text"], "B");

    let (after, expected_after, _, _, _) = compile_and_execute(source, operations(Affinity::After));
    assert_eq!(after, expected_after);
    assert_eq!(after["content"][0]["content"][0]["text"], "A😀");
    assert_eq!(after["content"][1]["content"][0]["text"], "XB");
}

#[test]
fn split_block_then_update_attrs_mutates_the_prepared_right_block() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "h2",
            "attrs": { "id": "left" },
            "content": [{ "type": "text", "text": "abcd" }]
        }]
    });
    let schema = attribute_schema();
    let (doc, schema, limits, editing_limits, document) =
        diagnostic_doc_with_schema(&source, schema);
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
                request_id: 121,
                base_document_revision: 0,
                origin: TransactionOrigin::LocalCommand,
                operations: vec![
                    TypedOperation::SplitBlock {
                        at: point_for_test(2),
                        node_type: "h2".into(),
                        attrs: HashMap::from([("id".into(), Value::String("right-old".into()))]),
                    },
                    TypedOperation::UpdateNodeAttrs {
                        at: point_for_test(2),
                        attrs: HashMap::from([("id".into(), Value::String("right-new".into()))]),
                    },
                ],
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Auto,
            },
            &txn,
            &fragment,
        )
        .unwrap()
    };
    let expected = to_prosemirror_json(&compiled.preview, &schema);
    {
        let mut txn = doc.transact_mut();
        execute_mutation_plan(compiled.mutation_plan, &mut txn);
    }
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let actual = YrsDocumentCodec::new(&schema, &limits)
        .read_json(&fragment, &txn)
        .unwrap();
    assert_eq!(actual, expected);
    assert_eq!(actual["content"][0]["attrs"]["id"], "left");
    assert_eq!(actual["content"][1]["attrs"]["id"], "right-new");
}

#[test]
fn split_block_immediately_before_and_after_an_atom_moves_only_the_suffix_children() {
    let source = json!({
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
    for (offset, retained_children, right_first_type) in
        [(1, 1usize, "hardBreak"), (2, 2usize, "text")]
    {
        let (doc, schema, limits, compiled) = compile_operations_with_schema(
            &source,
            vec![TypedOperation::SplitBlock {
                at: point_for_test(offset),
                node_type: "paragraph".into(),
                attrs: HashMap::new(),
            }],
            tiptap_schema(),
        );
        let (left_block_id, left_child_ids, before_full_len) = {
            let txn = doc.transact();
            let fragment = txn.get_xml_fragment("prosemirror").unwrap();
            let XmlOut::Element(left) = fragment.get(&txn, 0).unwrap() else {
                panic!("left paragraph expected")
            };
            (
                <yrs::types::xml::XmlElementRef as AsRef<Branch>>::as_ref(&left).id(),
                left.children(&txn)
                    .map(|child| child.id())
                    .collect::<Vec<_>>(),
                txn.encode_state_as_update_v1(&StateVector::default()).len(),
            )
        };
        assert!(matches!(
            compiled.mutation_plan.actions.as_slice(),
            [
                YrsMutationAction::DeleteXmlChildren { .. },
                YrsMutationAction::InsertXmlChildren { .. }
            ]
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
        let XmlOut::Element(left) = fragment.get(&txn, 0).unwrap() else {
            panic!("retained paragraph expected")
        };
        assert_eq!(
            <yrs::types::xml::XmlElementRef as AsRef<Branch>>::as_ref(&left).id(),
            left_block_id
        );
        let after_left_ids = left
            .children(&txn)
            .map(|child| child.id())
            .collect::<Vec<_>>();
        assert_eq!(after_left_ids, left_child_ids[..retained_children]);
        let XmlOut::Element(right) = fragment.get(&txn, 1).unwrap() else {
            panic!("prepared right paragraph expected")
        };
        let right_ids = right
            .children(&txn)
            .map(|child| child.id())
            .collect::<Vec<_>>();
        assert!(right_ids.iter().all(|id| !left_child_ids.contains(id)));
        let actual = YrsDocumentCodec::new(&schema, &limits)
            .read_json(&fragment, &txn)
            .unwrap();
        assert_eq!(actual, expected);
        assert_eq!(actual["content"][1]["content"][0]["type"], right_first_type);
        assert!(update.len() <= estimate);
        let after_full_len = txn.encode_state_as_update_v1(&StateVector::default()).len();
        assert!(after_full_len <= before_full_len + estimate);
    }
}

#[test]
fn split_atom_boundary_builds_canonical_h2_and_code_block_blueprints_with_follow_up_edits() {
    let source = json!({
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
    for (node_type, attrs, expected_type) in [
        (
            "h2",
            HashMap::from([("id".into(), Value::String("right".into()))]),
            "h2",
        ),
        (
            "codeBlock",
            HashMap::from([("language".into(), Value::String("rust".into()))]),
            "codeBlock",
        ),
    ] {
        let (doc, schema, limits, compiled) = compile_operations_with_schema(
            &source,
            vec![
                TypedOperation::SplitBlock {
                    at: point_for_test(2),
                    node_type: node_type.into(),
                    attrs,
                },
                TypedOperation::InsertText {
                    at: point_for_test(2),
                    text: "X".into(),
                    marks: vec![],
                },
            ],
            attribute_schema(),
        );
        let expected = to_prosemirror_json(&compiled.preview, &schema);
        {
            let mut txn = doc.transact_mut();
            execute_mutation_plan(compiled.mutation_plan, &mut txn);
        }
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let actual = YrsDocumentCodec::new(&schema, &limits)
            .read_json(&fragment, &txn)
            .unwrap();
        assert_eq!(actual, expected);
        assert_eq!(actual["content"][1]["type"], expected_type);
        assert_eq!(actual["content"][1]["content"][0]["text"], "XB");
    }
}

#[test]
fn split_atom_boundary_accounts_for_multiple_atoms_and_no_preceding_text() {
    let multiple = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [
                { "type": "text", "text": "A" },
                { "type": "hardBreak" },
                { "type": "hardBreak" },
                { "type": "text", "text": "B" }
            ]
        }]
    });
    let (actual, expected, _, _, _) = compile_and_execute(
        multiple,
        vec![
            TypedOperation::SplitBlock {
                at: point_for_test(3),
                node_type: "paragraph".into(),
                attrs: HashMap::new(),
            },
            TypedOperation::InsertText {
                at: point_for_test(3),
                text: "X".into(),
                marks: vec![],
            },
        ],
    );
    assert_eq!(actual, expected);
    assert_eq!(actual["content"][1]["content"][0]["text"], "XB");

    let no_prefix = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [
                { "type": "hardBreak" },
                { "type": "text", "text": "B" }
            ]
        }]
    });
    let (actual, expected, _, _, _) = compile_and_execute(
        no_prefix,
        vec![TypedOperation::SplitBlock {
            at: point_for_test(0),
            node_type: "paragraph".into(),
            attrs: HashMap::new(),
        }],
    );
    assert_eq!(actual, expected);
    assert!(actual["content"][0].get("content").is_none());
    assert_eq!(actual["content"][1]["content"][0]["type"], "hardBreak");
    assert_eq!(actual["content"][1]["content"][1]["text"], "B");
}

#[test]
fn split_block_inside_list_item_inserts_a_new_right_list_item() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "bulletList",
            "content": [
                {
                    "type": "listItem",
                    "content": [
                        {
                            "type": "paragraph",
                            "content": [{ "type": "text", "text": "A😀B" }]
                        },
                        {
                            "type": "paragraph",
                            "content": [{ "type": "text", "text": "post" }]
                        }
                    ]
                },
                {
                    "type": "listItem",
                    "content": [{
                        "type": "paragraph",
                        "content": [{ "type": "text", "text": "tail" }]
                    }]
                }
            ]
        }]
    });
    let schema = tiptap_schema();
    let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let rendered = crate::render::rendered_text(&document, &schema);
    let byte = rendered.find("A😀B").unwrap();
    let offset = u32::try_from(rendered[..byte].chars().count() + 2).unwrap();
    let (doc, schema, limits, mut compiled) = compile_operations_with_schema(
        &source,
        vec![TypedOperation::SplitBlock {
            at: point_for_test(offset),
            node_type: "paragraph".into(),
            attrs: HashMap::new(),
        }],
        tiptap_schema(),
    );
    assert!(matches!(
        compiled.mutation_plan.actions.as_slice(),
        [
            YrsMutationAction::DeleteText {
                index_utf16: 3,
                len_utf16: 1,
                operation_index: 0,
                ..
            },
            YrsMutationAction::DeleteXmlChildren {
                child_index: 1,
                child_count: 1,
                operation_index: 0,
                ..
            },
            YrsMutationAction::InsertXmlChildren {
                child_index: 1,
                nodes,
                operation_index: 0,
                ..
            }
        ] if nodes.len() == 1 && nodes[0].index == 1
    ));
    let (
        list_id,
        first_item_id,
        first_paragraph_id,
        first_text_id,
        post_paragraph_id,
        tail_item_id,
        tail_paragraph_id,
        tail_text_id,
        tail_sticky,
        before_full_len,
    ) = {
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let XmlOut::Element(list) = fragment.get(&txn, 0).unwrap() else {
            panic!("list expected")
        };
        let XmlOut::Element(first_item) = list.get(&txn, 0).unwrap() else {
            panic!("first list item expected")
        };
        let XmlOut::Element(first_paragraph) = first_item.get(&txn, 0).unwrap() else {
            panic!("first paragraph expected")
        };
        let XmlOut::Text(first_text) = first_paragraph.get(&txn, 0).unwrap() else {
            panic!("first text expected")
        };
        let post_paragraph_id = first_item.get(&txn, 1).unwrap().id();
        let XmlOut::Element(tail_item) = list.get(&txn, 1).unwrap() else {
            panic!("tail list item expected")
        };
        let XmlOut::Element(tail_paragraph) = tail_item.get(&txn, 0).unwrap() else {
            panic!("tail paragraph expected")
        };
        let XmlOut::Text(tail_text) = tail_paragraph.get(&txn, 0).unwrap() else {
            panic!("tail text expected")
        };
        let tail_sticky = StickyIndex::at(
            &txn,
            BranchPtr::from(<XmlTextRef as AsRef<Branch>>::as_ref(&tail_text)),
            2,
            Assoc::After,
        )
        .unwrap();
        (
            AsRef::<Branch>::as_ref(&list).id(),
            AsRef::<Branch>::as_ref(&first_item).id(),
            AsRef::<Branch>::as_ref(&first_paragraph).id(),
            <XmlTextRef as AsRef<Branch>>::as_ref(&first_text).id(),
            post_paragraph_id,
            AsRef::<Branch>::as_ref(&tail_item).id(),
            AsRef::<Branch>::as_ref(&tail_paragraph).id(),
            <XmlTextRef as AsRef<Branch>>::as_ref(&tail_text).id(),
            tail_sticky,
            txn.encode_state_as_update_v1(&StateVector::default()).len(),
        )
    };
    {
        let txn = doc.transact();
        let preflight =
            preflight_mutation_work_for_test(122, &compiled.mutation_plan, &txn).unwrap();
        let exact = compiled.mutation_plan.compilation_work_for_test() + preflight;
        compiled.mutation_plan.set_work_limit_for_test(exact);
        preflight_mutation_plan(122, &compiled.mutation_plan, &txn).unwrap();
        compiled.mutation_plan.set_work_limit_for_test(exact - 1);
        assert_eq!(
            preflight_mutation_plan(122, &compiled.mutation_plan, &txn)
                .unwrap_err()
                .code,
            "OPERATION_LIMIT_EXCEEDED"
        );
        compiled.mutation_plan.set_work_limit_for_test(exact);
    }
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
    let actual = YrsDocumentCodec::new(&schema, &limits)
        .read_json(&fragment, &txn)
        .unwrap();
    assert_eq!(actual, expected);
    let items = actual["content"][0]["content"].as_array().unwrap();
    assert_eq!(items.len(), 3);
    assert_eq!(items[0]["content"][0]["content"][0]["text"], "A😀");
    assert_eq!(items[1]["content"][0]["content"][0]["text"], "B");
    assert_eq!(items[1]["content"][1]["content"][0]["text"], "post");
    assert_eq!(items[2]["content"][0]["content"][0]["text"], "tail");

    let XmlOut::Element(list) = fragment.get(&txn, 0).unwrap() else {
        panic!("list expected")
    };
    assert_eq!(AsRef::<Branch>::as_ref(&list).id(), list_id);
    let XmlOut::Element(first_item) = list.get(&txn, 0).unwrap() else {
        panic!("first list item expected")
    };
    let XmlOut::Element(new_item) = list.get(&txn, 1).unwrap() else {
        panic!("new list item expected")
    };
    let XmlOut::Element(tail_item) = list.get(&txn, 2).unwrap() else {
        panic!("tail list item expected")
    };
    assert_eq!(AsRef::<Branch>::as_ref(&first_item).id(), first_item_id);
    assert_ne!(AsRef::<Branch>::as_ref(&new_item).id(), first_item_id);
    assert_ne!(AsRef::<Branch>::as_ref(&new_item).id(), tail_item_id);
    assert_eq!(AsRef::<Branch>::as_ref(&tail_item).id(), tail_item_id);
    let XmlOut::Element(first_paragraph) = first_item.get(&txn, 0).unwrap() else {
        panic!("retained first paragraph expected")
    };
    let XmlOut::Text(first_text) = first_paragraph.get(&txn, 0).unwrap() else {
        panic!("retained first text expected")
    };
    assert_eq!(
        AsRef::<Branch>::as_ref(&first_paragraph).id(),
        first_paragraph_id
    );
    assert_eq!(
        <XmlTextRef as AsRef<Branch>>::as_ref(&first_text).id(),
        first_text_id
    );
    assert_ne!(new_item.get(&txn, 1).unwrap().id(), post_paragraph_id);
    let XmlOut::Element(tail_paragraph) = tail_item.get(&txn, 0).unwrap() else {
        panic!("retained tail paragraph expected")
    };
    let XmlOut::Text(tail_text) = tail_paragraph.get(&txn, 0).unwrap() else {
        panic!("retained tail text expected")
    };
    assert_eq!(
        AsRef::<Branch>::as_ref(&tail_paragraph).id(),
        tail_paragraph_id
    );
    assert_eq!(
        <XmlTextRef as AsRef<Branch>>::as_ref(&tail_text).id(),
        tail_text_id
    );
    let resolved_sticky = tail_sticky.get_offset(&txn).unwrap();
    assert_eq!(resolved_sticky.branch.id(), tail_text_id);
    assert_eq!(resolved_sticky.index, 2);
    assert!(update.len() <= estimate, "{} > {estimate}", update.len());
    let after_full_len = txn.encode_state_as_update_v1(&StateVector::default()).len();
    assert!(after_full_len <= before_full_len + estimate);
}

include!("block_structure/cross_parent.rs");

include!("block_structure/folded_splits.rs");

include!("block_structure/join_and_replace.rs");
