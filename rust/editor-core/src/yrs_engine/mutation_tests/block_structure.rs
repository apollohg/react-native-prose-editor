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

#[test]
fn cross_parent_delete_merges_marked_unicode_paragraph_suffix_directly() {
    let source = json!({
        "type": "doc",
        "content": [
            {
                "type": "paragraph",
                "content": [{
                    "type": "text",
                    "text": "A😀B",
                    "marks": [{ "type": "bold" }]
                }]
            },
            {
                "type": "paragraph",
                "content": [{
                    "type": "text",
                    "text": "C😀D",
                    "marks": [{ "type": "bold" }]
                }]
            },
            {
                "type": "paragraph",
                "content": [{ "type": "text", "text": "tail" }]
            }
        ]
    });
    let schema = tiptap_schema();
    let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let rendered = crate::render::rendered_text(&document, &schema);
    let first_byte = rendered.find("A😀B").unwrap();
    let second_byte = rendered.find("C😀D").unwrap();
    let from = u32::try_from(rendered[..first_byte].chars().count() + 2).unwrap();
    let to = u32::try_from(rendered[..second_byte].chars().count() + 2).unwrap();
    let operations = || {
        vec![TypedOperation::DeleteRange {
            range: range_for_test(from, to),
        }]
    };
    let (doc, schema, limits, mut compiled) =
        compile_operations_with_schema(&source, operations(), tiptap_schema());
    assert!(matches!(
        compiled.mutation_plan.actions.as_slice(),
        [
            YrsMutationAction::DeleteText {
                index_utf16: 3,
                len_utf16: 1,
                operation_index: 0,
                ..
            },
            YrsMutationAction::InsertText {
                index_utf16: 3,
                text,
                len_utf16: 1,
                operation_index: 0,
                ..
            },
            YrsMutationAction::DeleteXmlChildren {
                child_index: 1,
                child_count: 1,
                operation_index: 0,
                ..
            }
        ] if text == "D"
    ));
    let (
        first_block_id,
        first_text_id,
        removed_block_id,
        tail_block_id,
        tail_text_id,
        tail_sticky,
        before_full_len,
        before_update,
    ) = {
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let children = fragment.children(&txn).collect::<Vec<_>>();
        let first_text = paragraph_text(&fragment, &txn, 0);
        let tail_text = paragraph_text(&fragment, &txn, 2);
        let tail_sticky = StickyIndex::at(
            &txn,
            BranchPtr::from(<XmlTextRef as AsRef<Branch>>::as_ref(&tail_text)),
            2,
            Assoc::After,
        )
        .unwrap();
        (
            children[0].id(),
            <XmlTextRef as AsRef<Branch>>::as_ref(&first_text).id(),
            children[1].id(),
            children[2].id(),
            <XmlTextRef as AsRef<Branch>>::as_ref(&tail_text).id(),
            tail_sticky,
            txn.encode_state_as_update_v1(&StateVector::default()).len(),
            txn.encode_state_as_update_v1(&StateVector::default()),
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
    let undo_exact = compiled.undo_units_bound;
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
    assert_eq!(actual["content"].as_array().unwrap().len(), 2);
    assert_eq!(actual["content"][0]["content"][0]["text"], "A😀D");
    assert_eq!(
        actual["content"][0]["content"][0]["marks"][0]["type"],
        "bold"
    );
    assert_eq!(actual["content"][1]["content"][0]["text"], "tail");
    let children = fragment.children(&txn).collect::<Vec<_>>();
    assert_eq!(children[0].id(), first_block_id);
    assert_eq!(children[1].id(), tail_block_id);
    assert!(!children.iter().any(|child| child.id() == removed_block_id));
    assert_eq!(
        <XmlTextRef as AsRef<Branch>>::as_ref(&paragraph_text(&fragment, &txn, 0)).id(),
        first_text_id
    );
    assert_eq!(
        <XmlTextRef as AsRef<Branch>>::as_ref(&paragraph_text(&fragment, &txn, 1)).id(),
        tail_text_id
    );
    let resolved_sticky = tail_sticky.get_offset(&txn).unwrap();
    assert_eq!(resolved_sticky.branch.id(), tail_text_id);
    assert_eq!(resolved_sticky.index, 2);
    assert!(update.len() <= estimate, "{} > {estimate}", update.len());
    let after_full_len = txn.encode_state_as_update_v1(&StateVector::default()).len();
    assert!(after_full_len <= before_full_len + estimate);
    let replica = utf16_doc();
    {
        let mut replica_txn = replica.transact_mut();
        replica_txn
            .apply_update(Update::decode_v1(&before_update).unwrap())
            .unwrap();
        replica_txn
            .apply_update(Update::decode_v1(&update).unwrap())
            .unwrap();
    }
    let replica_txn = replica.transact();
    let replica_fragment = replica_txn.get_xml_fragment("prosemirror").unwrap();
    assert_eq!(
        YrsDocumentCodec::new(&schema, &limits)
            .read_json(&replica_fragment, &replica_txn)
            .unwrap(),
        expected
    );

    assert!(undo_exact > 0);
    assert_eq!(
        compile_operations_with_undo_limit(&source, operations(), tiptap_schema(), undo_exact,)
            .unwrap()
            .undo_units_bound,
        undo_exact
    );
    let undo_error =
        compile_operations_with_undo_limit(&source, operations(), tiptap_schema(), undo_exact - 1)
            .unwrap_err();
    assert_eq!(undo_error.code, "OPERATION_LIMIT_EXCEEDED");
    assert_eq!(undo_error.limit, Some(undo_exact - 1));
    assert_eq!(undo_error.actual, Some(undo_exact));
}

#[test]
fn cross_parent_delete_uses_provenance_across_equal_blocks_and_a_middle_void() {
    let source = json!({
        "type": "doc",
        "content": [
            {
                "type": "paragraph",
                "content": [{
                    "type": "text",
                    "text": "A😀B",
                    "marks": [{ "type": "bold" }]
                }]
            },
            { "type": "horizontalRule" },
            {
                "type": "paragraph",
                "content": [{
                    "type": "text",
                    "text": "A😀B",
                    "marks": [{ "type": "bold" }]
                }]
            }
        ]
    });
    let schema = tiptap_schema();
    let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let rendered = crate::render::rendered_text(&document, &schema);
    let occurrences = rendered
        .match_indices("A😀B")
        .map(|(byte, _)| byte)
        .collect::<Vec<_>>();
    let from = u32::try_from(rendered[..occurrences[0]].chars().count() + 1).unwrap();
    let to = u32::try_from(rendered[..occurrences[1]].chars().count() + 2).unwrap();
    let (doc, schema, limits, compiled) = compile_operations_with_schema(
        &source,
        vec![TypedOperation::DeleteRange {
            range: range_for_test(from, to),
        }],
        tiptap_schema(),
    );
    assert!(matches!(
        compiled.mutation_plan.actions.as_slice(),
        [
            YrsMutationAction::DeleteText {
                index_utf16: 1,
                len_utf16: 3,
                ..
            },
            YrsMutationAction::InsertText {
                index_utf16: 1,
                text,
                ..
            },
            YrsMutationAction::DeleteXmlChildren {
                child_index: 1,
                child_count: 2,
                ..
            }
        ] if text == "B"
    ));
    let first_id = {
        let txn = doc.transact();
        txn.get_xml_fragment("prosemirror")
            .unwrap()
            .get(&txn, 0)
            .unwrap()
            .id()
    };
    let expected = to_prosemirror_json(&compiled.preview, &schema);
    {
        let mut txn = doc.transact_mut();
        execute_mutation_plan(compiled.mutation_plan, &mut txn);
    }
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    assert_eq!(fragment.get(&txn, 0).unwrap().id(), first_id);
    let actual = YrsDocumentCodec::new(&schema, &limits)
        .read_json(&fragment, &txn)
        .unwrap();
    assert_eq!(actual, expected);
    assert_eq!(actual["content"].as_array().unwrap().len(), 1);
    assert_eq!(actual["content"][0]["content"][0]["text"], "AB");
}

#[test]
fn cross_parent_replace_inserts_inline_text_and_atom_fragment_directly() {
    let source = json!({
        "type": "doc",
        "content": [
            { "type": "paragraph", "content": [{ "type": "text", "text": "ab" }] },
            { "type": "paragraph", "content": [{ "type": "text", "text": "cd" }] }
        ]
    });
    let schema = tiptap_schema();
    let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let rendered = crate::render::rendered_text(&document, &schema);
    let first = rendered.find("ab").unwrap();
    let second = rendered.find("cd").unwrap();
    let from = u32::try_from(rendered[..first].chars().count() + 1).unwrap();
    let to = u32::try_from(rendered[..second].chars().count() + 1).unwrap();
    let replacement = || {
        Fragment::from(vec![
            Node::text("X".into(), vec![Mark::new("bold".into(), HashMap::new())]),
            Node::void("hardBreak".into(), HashMap::new()),
            Node::text("Y".into(), vec![]),
        ])
    };
    let operations = || {
        vec![TypedOperation::ReplaceRange {
            range: range_for_test(from, to),
            content: replacement(),
        }]
    };
    let (doc, schema, limits, mut compiled) =
        compile_operations_with_schema(&source, operations(), tiptap_schema());
    assert!(matches!(
        compiled.mutation_plan.actions.as_slice(),
        [
            YrsMutationAction::DeleteXmlChildren {
                child_index: 1,
                child_count: 1,
                operation_index: 0,
                ..
            },
            YrsMutationAction::DeleteText {
                index_utf16: 1,
                len_utf16: 1,
                operation_index: 0,
                ..
            },
            YrsMutationAction::InsertText {
                index_utf16: 1,
                text,
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
        ] if text == "X"
            && matches!(nodes.as_slice(), [
                PreparedXmlChild { index: 1, node: PreparedXmlNode::Element { tag, .. } },
                PreparedXmlChild { index: 2, node: PreparedXmlNode::Text { runs } }
            ] if tag == "hardBreak" && prepared_text_for_test(runs) == "Yd")
    ));
    let (first_block_id, first_text_id, right_text_id, before_full_len) = {
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let first_text = paragraph_text(&fragment, &txn, 0);
        let right_text = paragraph_text(&fragment, &txn, 1);
        (
            fragment.get(&txn, 0).unwrap().id(),
            <XmlTextRef as AsRef<Branch>>::as_ref(&first_text).id(),
            <XmlTextRef as AsRef<Branch>>::as_ref(&right_text).id(),
            txn.encode_state_as_update_v1(&StateVector::default()).len(),
        )
    };
    assert!(compiled
        .mutation_plan
        .actions
        .iter()
        .all(|action| match action {
            YrsMutationAction::InsertText { target, .. }
            | YrsMutationAction::DeleteText { target, .. }
            | YrsMutationAction::FormatText { target, .. } =>
                AsRef::<Branch>::as_ref(target).id() != right_text_id,
            _ => true,
        }));
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
    let undo_exact = compiled.undo_units_bound;
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
    let content = actual["content"][0]["content"].as_array().unwrap();
    assert_eq!(content[0]["text"], "a");
    assert_eq!(content[1]["text"], "X");
    assert_eq!(content[1]["marks"][0]["type"], "bold");
    assert_eq!(content[2]["type"], "hardBreak");
    assert_eq!(content[3]["text"], "Yd");
    assert_eq!(fragment.get(&txn, 0).unwrap().id(), first_block_id);
    assert_eq!(
        <XmlTextRef as AsRef<Branch>>::as_ref(&paragraph_text(&fragment, &txn, 0)).id(),
        first_text_id
    );
    assert!(update.len() <= estimate, "{} > {estimate}", update.len());
    let after_full_len = txn.encode_state_as_update_v1(&StateVector::default()).len();
    assert!(after_full_len <= before_full_len + estimate);
    assert_eq!(
        compile_operations_with_undo_limit(&source, operations(), tiptap_schema(), undo_exact,)
            .unwrap()
            .undo_units_bound,
        undo_exact
    );
    let undo_error =
        compile_operations_with_undo_limit(&source, operations(), tiptap_schema(), undo_exact - 1)
            .unwrap_err();
    assert_eq!(undo_error.code, "OPERATION_LIMIT_EXCEEDED");
    assert_eq!(undo_error.actual, Some(undo_exact));
}

#[test]
fn cross_parent_replace_handles_empty_text_only_leading_and_multiple_atoms() {
    let source = json!({
        "type": "doc",
        "content": [
            { "type": "paragraph", "content": [{ "type": "text", "text": "ab" }] },
            { "type": "paragraph", "content": [{ "type": "text", "text": "cd" }] }
        ]
    });
    let schema = tiptap_schema();
    let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let rendered = crate::render::rendered_text(&document, &schema);
    let first = rendered.find("ab").unwrap();
    let second = rendered.find("cd").unwrap();
    let from = u32::try_from(rendered[..first].chars().count() + 1).unwrap();
    let to = u32::try_from(rendered[..second].chars().count() + 1).unwrap();
    let cases = vec![
        (Fragment::empty(), "ad", 1usize),
        (
            Fragment::from(vec![Node::text("Q".into(), vec![])]),
            "aQd",
            1,
        ),
        (
            Fragment::from(vec![
                Node::void("hardBreak".into(), HashMap::new()),
                Node::text("Y".into(), vec![]),
            ]),
            "aYd",
            3,
        ),
        (
            Fragment::from(vec![
                Node::void("hardBreak".into(), HashMap::new()),
                Node::text("X".into(), vec![]),
                Node::void("hardBreak".into(), HashMap::new()),
                Node::text("Y".into(), vec![]),
            ]),
            "aXYd",
            5,
        ),
    ];
    for (replacement, expected_text, expected_children) in cases {
        let (actual, expected, _, _, _) = compile_and_execute(
            source.clone(),
            vec![TypedOperation::ReplaceRange {
                range: range_for_test(from, to),
                content: replacement,
            }],
        );
        assert_eq!(actual, expected);
        assert_eq!(
            actual["content"][0]["content"].as_array().unwrap().len(),
            expected_children
        );
        let decoded = from_prosemirror_json(&actual, &schema, UnknownTypeMode::Preserve).unwrap();
        assert_eq!(decoded.root().text_content(), expected_text);
    }
}

#[test]
fn cross_parent_replace_folds_follow_up_edits_into_prepared_children() {
    let source = json!({
        "type": "doc",
        "content": [
            { "type": "paragraph", "content": [{ "type": "text", "text": "ab" }] },
            { "type": "paragraph", "content": [{ "type": "text", "text": "cd" }] }
        ]
    });
    let schema = tiptap_schema();
    let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let rendered = crate::render::rendered_text(&document, &schema);
    let first = rendered.find("ab").unwrap();
    let second = rendered.find("cd").unwrap();
    let from = u32::try_from(rendered[..first].chars().count() + 1).unwrap();
    let to = u32::try_from(rendered[..second].chars().count() + 1).unwrap();
    let (doc, schema, limits, compiled) = compile_operations_with_schema(
        &source,
        vec![
            TypedOperation::ReplaceRange {
                range: range_for_test(from, to),
                content: Fragment::from(vec![
                    Node::void("hardBreak".into(), HashMap::new()),
                    Node::text("Y".into(), vec![]),
                ]),
            },
            TypedOperation::InsertText {
                at: point_for_test(to),
                text: "Z".into(),
                marks: vec![],
            },
        ],
        tiptap_schema(),
    );
    assert!(
        !compiled.mutation_plan.actions.iter().any(|action| matches!(
            action,
            YrsMutationAction::InsertText {
                operation_index: 1,
                ..
            }
        ))
    );
    let prepared_text = compiled
        .mutation_plan
        .actions
        .iter()
        .find_map(|action| match action {
            YrsMutationAction::InsertXmlChildren { nodes, .. } => nodes.last(),
            _ => None,
        })
        .and_then(|child| match &child.node {
            PreparedXmlNode::Text { runs } => Some(prepared_text_for_test(runs)),
            _ => None,
        })
        .unwrap();
    assert!(prepared_text.contains('Z'));
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
fn cross_parent_replace_updates_a_prepared_inline_atom_blueprint() {
    let source = json!({
        "type": "doc",
        "content": [
            { "type": "paragraph", "content": [{ "type": "text", "text": "ab" }] },
            { "type": "paragraph", "content": [{ "type": "text", "text": "cd" }] }
        ]
    });
    let schema = attribute_schema();
    let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let rendered = crate::render::rendered_text(&document, &schema);
    let first = rendered.find("ab").unwrap();
    let second = rendered.find("cd").unwrap();
    let from = u32::try_from(rendered[..first].chars().count() + 1).unwrap();
    let to = u32::try_from(rendered[..second].chars().count() + 1).unwrap();
    let atom_position = RevisionedPosition {
        offset: from,
        kind: EditorOffsetKind::Scalar,
        affinity: Affinity::Before,
    };
    let (doc, schema, limits, compiled) = compile_operations_with_schema(
        &source,
        vec![
            TypedOperation::ReplaceRange {
                range: range_for_test(from, to),
                content: Fragment::from(vec![Node::void(
                    "inlineWidget".into(),
                    HashMap::from([
                        ("id".into(), Value::String("old".into())),
                        ("meta".into(), json!({ "nested": [1, true] })),
                    ]),
                )]),
            },
            TypedOperation::UpdateNodeAttrs {
                at: atom_position,
                attrs: HashMap::from([
                    ("id".into(), Value::String("new".into())),
                    ("meta".into(), json!({ "nested": [2, false] })),
                ]),
            },
        ],
        schema,
    );
    assert!(!compiled.mutation_plan.actions.iter().any(|action| {
        matches!(
            action,
            YrsMutationAction::SetXmlAttribute {
                operation_index: 1,
                ..
            } | YrsMutationAction::RemoveXmlAttribute {
                operation_index: 1,
                ..
            }
        )
    }));
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
    assert_eq!(actual["content"][0]["content"][1]["attrs"]["id"], "new");
    assert_eq!(
        actual["content"][0]["content"][1]["attrs"]["meta"],
        json!({ "nested": [2, false] })
    );
}

#[test]
fn cross_parent_replace_uses_virtual_runs_after_prior_endpoint_edits() {
    let source = json!({
        "type": "doc",
        "content": [
            { "type": "paragraph", "content": [{ "type": "text", "text": "abcd" }] },
            { "type": "paragraph", "content": [{ "type": "text", "text": "wxyz" }] }
        ]
    });
    let schema = tiptap_schema();
    let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let rendered = crate::render::rendered_text(&document, &schema);
    let first = u32::try_from(rendered[..rendered.find("abcd").unwrap()].chars().count()).unwrap();
    let second = u32::try_from(rendered[..rendered.find("wxyz").unwrap()].chars().count()).unwrap();
    let replacement = || TypedOperation::ReplaceRange {
        range: range_for_test(first + 2, second + 2),
        content: Fragment::from(vec![
            Node::void("hardBreak".into(), HashMap::new()),
            Node::text("Q".into(), vec![]),
        ]),
    };
    let bold = || Mark::new("bold".into(), HashMap::new());
    let cases = vec![
        vec![
            TypedOperation::InsertText {
                at: point_for_test(first + 1),
                text: "L".into(),
                marks: vec![],
            },
            replacement(),
        ],
        vec![
            TypedOperation::InsertText {
                at: point_for_test(second + 1),
                text: "R".into(),
                marks: vec![],
            },
            replacement(),
        ],
        vec![
            TypedOperation::DeleteRange {
                range: range_for_test(first, first + 1),
            },
            replacement(),
        ],
        vec![
            TypedOperation::DeleteRange {
                range: range_for_test(second + 2, second + 3),
            },
            replacement(),
        ],
        vec![
            TypedOperation::AddMark {
                range: range_for_test(first, first + 1),
                mark: bold(),
            },
            replacement(),
        ],
        vec![
            TypedOperation::AddMark {
                range: range_for_test(second + 2, second + 3),
                mark: bold(),
            },
            replacement(),
        ],
    ];
    for operations in cases {
        let (actual, expected, _, _, _) = compile_and_execute(source.clone(), operations);
        assert_eq!(actual, expected);
        assert_eq!(actual["content"].as_array().unwrap().len(), 1);
        assert!(actual["content"][0]["content"]
            .as_array()
            .unwrap()
            .iter()
            .any(|node| node["type"] == "hardBreak"));
    }
}

#[test]
fn cross_parent_replace_uses_provenance_and_a_nested_lca() {
    let equal = json!({
        "type": "doc",
        "content": [
            { "type": "paragraph", "content": [{ "type": "text", "text": "same" }] },
            { "type": "horizontalRule" },
            { "type": "paragraph", "content": [{ "type": "text", "text": "same" }] }
        ]
    });
    let schema = tiptap_schema();
    let document = from_prosemirror_json(&equal, &schema, UnknownTypeMode::Preserve).unwrap();
    let rendered = crate::render::rendered_text(&document, &schema);
    let occurrences = rendered
        .match_indices("same")
        .map(|(byte, _)| byte)
        .collect::<Vec<_>>();
    let from = u32::try_from(rendered[..occurrences[0]].chars().count() + 1).unwrap();
    let to = u32::try_from(rendered[..occurrences[1]].chars().count() + 1).unwrap();
    let (actual, expected, _, _, _) = compile_and_execute(
        equal,
        vec![TypedOperation::ReplaceRange {
            range: range_for_test(from, to),
            content: Fragment::from(vec![Node::text("X".into(), vec![])]),
        }],
    );
    assert_eq!(actual, expected);
    assert_eq!(actual["content"].as_array().unwrap().len(), 1);
    assert_eq!(actual["content"][0]["content"][0]["text"], "sXame");

    let nested = json!({
        "type": "doc",
        "content": [{
            "type": "bulletList",
            "content": [{
                "type": "listItem",
                "content": [
                    { "type": "paragraph", "content": [{ "type": "text", "text": "ab" }] },
                    { "type": "paragraph", "content": [{ "type": "text", "text": "cd" }] }
                ]
            }]
        }]
    });
    let document = from_prosemirror_json(&nested, &schema, UnknownTypeMode::Preserve).unwrap();
    let rendered = crate::render::rendered_text(&document, &schema);
    let first = rendered.find("ab").unwrap();
    let second = rendered.find("cd").unwrap();
    let from = u32::try_from(rendered[..first].chars().count() + 1).unwrap();
    let to = u32::try_from(rendered[..second].chars().count() + 1).unwrap();
    let (actual, expected, _, _, _) = compile_and_execute(
        nested,
        vec![TypedOperation::ReplaceRange {
            range: range_for_test(from, to),
            content: Fragment::from(vec![
                Node::void("hardBreak".into(), HashMap::new()),
                Node::text("Y".into(), vec![]),
            ]),
        }],
    );
    assert_eq!(actual, expected);
    let item = &actual["content"][0]["content"][0]["content"];
    assert_eq!(item.as_array().unwrap().len(), 1);
    assert_eq!(item[0]["content"][0]["text"], "a");
    assert_eq!(item[0]["content"][1]["type"], "hardBreak");
    assert_eq!(item[0]["content"][2]["text"], "Yd");
}

#[test]
fn cross_parent_delete_folds_right_edits_and_accepts_a_survivor_edit() {
    let source = json!({
        "type": "doc",
        "content": [
            { "type": "paragraph", "content": [{ "type": "text", "text": "ab" }] },
            { "type": "paragraph", "content": [{ "type": "text", "text": "cd" }] }
        ]
    });
    let schema = tiptap_schema();
    let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let rendered = crate::render::rendered_text(&document, &schema);
    let first = rendered.find("ab").unwrap();
    let second = rendered.find("cd").unwrap();
    let left = u32::try_from(rendered[..first].chars().count() + 1).unwrap();
    let right = u32::try_from(rendered[..second].chars().count() + 1).unwrap();
    let (actual, expected, _, _, _) = compile_and_execute(
        source,
        vec![
            TypedOperation::InsertText {
                at: point_for_test(right),
                text: "X".into(),
                marks: vec![],
            },
            TypedOperation::DeleteRange {
                range: range_for_test(left, right),
            },
            TypedOperation::InsertText {
                at: point_for_test(left),
                text: "Z".into(),
                marks: vec![],
            },
        ],
    );
    assert_eq!(actual, expected);
    assert_eq!(actual["content"].as_array().unwrap().len(), 1);
    assert_eq!(actual["content"][0]["content"][0]["text"], "aZd");
}

#[test]
fn structural_endpoint_resolution_uses_virtual_runs_after_prior_text_edits() {
    let source = json!({
        "type": "doc",
        "content": [
            { "type": "paragraph", "content": [{ "type": "text", "text": "abcd" }] },
            { "type": "paragraph", "content": [{ "type": "text", "text": "wxyz" }] }
        ]
    });
    let schema = tiptap_schema();
    let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let rendered = crate::render::rendered_text(&document, &schema);
    let first = u32::try_from(rendered[..rendered.find("abcd").unwrap()].chars().count()).unwrap();
    let second = u32::try_from(rendered[..rendered.find("wxyz").unwrap()].chars().count()).unwrap();
    let cross = || TypedOperation::DeleteRange {
        range: range_for_test(first + 2, second + 2),
    };
    let bold = || Mark::new("bold".into(), HashMap::new());
    let cases = vec![
        vec![
            TypedOperation::InsertText {
                at: point_for_test(first + 1),
                text: "L".into(),
                marks: vec![],
            },
            cross(),
        ],
        vec![
            TypedOperation::InsertText {
                at: point_for_test(second + 1),
                text: "R".into(),
                marks: vec![],
            },
            cross(),
        ],
        vec![
            TypedOperation::DeleteRange {
                range: range_for_test(first, first + 1),
            },
            cross(),
        ],
        vec![
            TypedOperation::DeleteRange {
                range: range_for_test(second + 2, second + 3),
            },
            cross(),
        ],
        vec![
            TypedOperation::AddMark {
                range: range_for_test(first, first + 1),
                mark: bold(),
            },
            cross(),
        ],
        vec![
            TypedOperation::AddMark {
                range: range_for_test(second + 2, second + 3),
                mark: bold(),
            },
            cross(),
        ],
    ];
    for operations in cases {
        let (actual, expected, _, _, _) = compile_and_execute(source.clone(), operations);
        assert_eq!(actual, expected);
        assert_eq!(actual["content"].as_array().unwrap().len(), 1);
    }
}

#[test]
fn cross_parent_delete_uses_a_nested_list_item_lca() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "bulletList",
            "content": [{
                "type": "listItem",
                "content": [
                    { "type": "paragraph", "content": [{ "type": "text", "text": "ab" }] },
                    { "type": "paragraph", "content": [{ "type": "text", "text": "cd" }] }
                ]
            }]
        }]
    });
    let schema = tiptap_schema();
    let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let rendered = crate::render::rendered_text(&document, &schema);
    let first = rendered.find("ab").unwrap();
    let second = rendered.find("cd").unwrap();
    let from = u32::try_from(rendered[..first].chars().count() + 1).unwrap();
    let to = u32::try_from(rendered[..second].chars().count() + 1).unwrap();
    let (actual, expected, _, _, _) = compile_and_execute(
        source,
        vec![TypedOperation::DeleteRange {
            range: range_for_test(from, to),
        }],
    );
    assert_eq!(actual, expected);
    let item_content = actual["content"][0]["content"][0]["content"]
        .as_array()
        .unwrap();
    assert_eq!(item_content.len(), 1);
    assert_eq!(item_content[0]["content"][0]["text"], "ad");
}

#[test]
fn split_sequences_materialize_only_the_compact_final_plan() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "A😀B" }]
        }]
    });
    let (_, _, _, inserted) = compile_operations_with_schema(
        &source,
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
        tiptap_schema(),
    );
    assert!(!inserted
        .mutation_plan
        .actions
        .iter()
        .any(|action| matches!(action, YrsMutationAction::InsertText { .. })));
    let inserted_nodes = inserted
        .mutation_plan
        .actions
        .iter()
        .find_map(|action| match action {
            YrsMutationAction::InsertXmlChildren { nodes, .. } => Some(nodes),
            _ => None,
        })
        .unwrap();
    let PreparedXmlNode::Element { children, .. } = &inserted_nodes[0].node else {
        panic!("prepared right block expected")
    };
    let PreparedXmlNode::Text { runs } = &children[0].node else {
        panic!("prepared right text expected")
    };
    assert_eq!(prepared_text_for_test(runs), "BX");

    let (_, _, _, marked) = compile_operations_with_schema(
        &source,
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
        tiptap_schema(),
    );
    assert!(!marked
        .mutation_plan
        .actions
        .iter()
        .any(|action| matches!(action, YrsMutationAction::FormatText { .. })));
    let marked_nodes = marked
        .mutation_plan
        .actions
        .iter()
        .find_map(|action| match action {
            YrsMutationAction::InsertXmlChildren { nodes, .. } => Some(nodes),
            _ => None,
        })
        .unwrap();
    let PreparedXmlNode::Element { children, .. } = &marked_nodes[0].node else {
        panic!("prepared marked block expected")
    };
    let PreparedXmlNode::Text { runs } = &children[0].node else {
        panic!("prepared marked text expected")
    };
    assert_eq!(prepared_text_for_test(runs), "B");
    assert_eq!(runs[0].attrs.get("bold"), Some(&Any::Bool(true)));

    let heading = json!({
        "type": "doc",
        "content": [{
            "type": "h2",
            "attrs": { "id": "left" },
            "content": [{ "type": "text", "text": "abcd" }]
        }]
    });
    let (_, _, _, attributed) = compile_operations_with_schema(
        &heading,
        vec![
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
        attribute_schema(),
    );
    assert!(!attributed.mutation_plan.actions.iter().any(|action| {
        matches!(
            action,
            YrsMutationAction::SetXmlAttribute { .. }
                | YrsMutationAction::RemoveXmlAttribute { .. }
        )
    }));
    let attributed_nodes = attributed
        .mutation_plan
        .actions
        .iter()
        .find_map(|action| match action {
            YrsMutationAction::InsertXmlChildren { nodes, .. } => Some(nodes),
            _ => None,
        })
        .unwrap();
    let PreparedXmlNode::Element { attrs, .. } = &attributed_nodes[0].node else {
        panic!("prepared attributed block expected")
    };
    assert!(attrs
        .iter()
        .any(|(key, value)| { key == "id" && value == &Any::String("right-new".into()) }));
}

#[test]
fn folded_split_undo_bound_uses_the_compact_final_plan_exactly() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "A😀B" }]
        }]
    });
    let operations = || {
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
        ]
    };
    let exact =
        compile_operations_with_undo_limit(&source, operations(), tiptap_schema(), u64::MAX)
            .unwrap()
            .undo_units_bound;
    assert!(exact > 0);
    let accepted =
        compile_operations_with_undo_limit(&source, operations(), tiptap_schema(), exact).unwrap();
    assert_eq!(accepted.undo_units_bound, exact);
    let rejected =
        compile_operations_with_undo_limit(&source, operations(), tiptap_schema(), exact - 1)
            .unwrap_err();
    assert_eq!(rejected.code, "OPERATION_LIMIT_EXCEEDED");
    assert_eq!(rejected.limit, Some(exact - 1));
    assert_eq!(rejected.actual, Some(exact));

    let plain = compile_operations_with_undo_limit(
        &source,
        vec![TypedOperation::InsertText {
            at: point_for_test(1),
            text: "x".into(),
            marks: vec![],
        }],
        tiptap_schema(),
        1,
    )
    .unwrap();
    assert_eq!(plain.undo_units_bound, 1);

    let emoji = compile_operations_with_undo_limit(
        &source,
        vec![TypedOperation::InsertText {
            at: point_for_test(1),
            text: "🙂".into(),
            marks: vec![],
        }],
        tiptap_schema(),
        2,
    )
    .unwrap();
    assert_eq!(emoji.undo_units_bound, 2);

    let (emoji_doc, _, _, emoji_compiled) = compile_operations_with_schema(
        &source,
        vec![TypedOperation::InsertText {
            at: point_for_test(1),
            text: "🙂".into(),
            marks: vec![],
        }],
        tiptap_schema(),
    );
    let fragment = emoji_doc
        .transact()
        .get_xml_fragment("prosemirror")
        .unwrap();
    let mut undo = UndoManager::<()>::new();
    undo.expand_scope(&emoji_doc, &fragment);
    {
        let mut txn = emoji_doc.transact_mut();
        execute_mutation_plan(emoji_compiled.mutation_plan, &mut txn);
    }
    let inserted = undo.undo_stack()[0]
        .insertions()
        .iter()
        .flat_map(|(_, ranges)| ranges.into_iter())
        .map(|range| u64::from(range.end - range.start))
        .sum::<u64>();
    assert_eq!(inserted, 2);
    assert!(inserted <= emoji_compiled.undo_units_bound);
}

#[test]
fn join_blocks_directly_keeps_the_left_block_and_text() {
    let source = json!({
        "type": "doc",
        "content": [
            { "type": "paragraph", "content": [{ "type": "text", "text": "ab" }] },
            { "type": "paragraph", "content": [{ "type": "text", "text": "cd" }] }
        ]
    });
    let (actual, expected, _, update_len, estimate) = compile_and_execute(
        source,
        vec![TypedOperation::JoinBlocks {
            at: point_for_test(2),
        }],
    );
    assert_eq!(actual, expected);
    assert_eq!(actual["content"].as_array().unwrap().len(), 1);
    assert_eq!(actual["content"][0]["content"][0]["text"], "abcd");
    assert!(update_len <= estimate);
}

#[test]
fn join_blocks_folds_edits_on_both_sides_and_targets_the_retained_text_afterward() {
    let source = json!({
        "type": "doc",
        "content": [
            { "type": "paragraph", "content": [{ "type": "text", "text": "ab" }] },
            { "type": "paragraph", "content": [{ "type": "text", "text": "cd" }] }
        ]
    });
    let (right, expected_right, _, _, _) = compile_and_execute(
        source.clone(),
        vec![
            TypedOperation::InsertText {
                at: point_for_test(4),
                text: "R".into(),
                marks: vec![],
            },
            TypedOperation::JoinBlocks {
                at: point_for_test(2),
            },
        ],
    );
    assert_eq!(right, expected_right);
    assert_eq!(right["content"][0]["content"][0]["text"], "abcRd");

    let (left, expected_left, _, _, _) = compile_and_execute(
        source.clone(),
        vec![
            TypedOperation::InsertText {
                at: point_for_test(1),
                text: "L".into(),
                marks: vec![],
            },
            TypedOperation::JoinBlocks {
                at: point_for_test(2),
            },
        ],
    );
    assert_eq!(left, expected_left);
    assert_eq!(left["content"][0]["content"][0]["text"], "aLbcd");

    let (after, expected_after, _, _, _) = compile_and_execute(
        source,
        vec![
            TypedOperation::JoinBlocks {
                at: point_for_test(2),
            },
            TypedOperation::InsertText {
                at: point_for_test(2),
                text: "X".into(),
                marks: vec![],
            },
            TypedOperation::AddMark {
                range: range_for_test(2, 4),
                mark: Mark::new("bold".into(), HashMap::new()),
            },
        ],
    );
    assert_eq!(after, expected_after);
    let pieces = after["content"][0]["content"].as_array().unwrap();
    assert_eq!(
        pieces
            .iter()
            .filter_map(|node| node["text"].as_str())
            .collect::<String>(),
        "abXcd"
    );
    assert!(pieces.iter().any(|node| {
        node["marks"]
            .as_array()
            .is_some_and(|marks| marks.iter().any(|mark| mark["type"] == "bold"))
    }));
}

#[test]
fn join_blocks_preserves_left_identity_marks_sticky_and_accepts_follow_up_attrs() {
    let source = json!({
        "type": "doc",
        "content": [
            { "type": "h2", "attrs": { "id": "left" }, "content": [{ "type": "text", "text": "ab" }] },
            { "type": "h2", "attrs": { "id": "right" }, "content": [{ "type": "text", "text": "😀c", "marks": [{ "type": "bold" }] }] },
            { "type": "h2", "attrs": { "id": "tail" }, "content": [{ "type": "text", "text": "tail" }] }
        ]
    });
    let (doc, schema, limits, mut compiled) = compile_operations_with_schema(
        &source,
        vec![
            TypedOperation::JoinBlocks {
                at: point_for_test(2),
            },
            TypedOperation::UpdateNodeAttrs {
                at: point_for_test(0),
                attrs: HashMap::from([("id".into(), Value::String("joined".into()))]),
            },
        ],
        attribute_schema(),
    );
    assert!(matches!(
        compiled.mutation_plan.actions.as_slice(),
        [
            YrsMutationAction::InsertText { .. },
            YrsMutationAction::DeleteXmlChildren {
                child_index: 1,
                child_count: 1,
                ..
            },
            YrsMutationAction::SetXmlAttribute { .. }
        ]
    ));
    let (left_block_id, left_text_id, tail_block_id, tail_text_id, sticky) = {
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let children = fragment.children(&txn).collect::<Vec<_>>();
        let left_text = paragraph_text(&fragment, &txn, 0);
        let tail_text = paragraph_text(&fragment, &txn, 2);
        let sticky = StickyIndex::at(
            &txn,
            BranchPtr::from(<XmlTextRef as AsRef<Branch>>::as_ref(&tail_text)),
            2,
            Assoc::After,
        )
        .unwrap();
        (
            children[0].id(),
            <XmlTextRef as AsRef<Branch>>::as_ref(&left_text).id(),
            children[2].id(),
            <XmlTextRef as AsRef<Branch>>::as_ref(&tail_text).id(),
            sticky,
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
    {
        let mut txn = doc.transact_mut();
        execute_mutation_plan(compiled.mutation_plan, &mut txn);
    }
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let children = fragment.children(&txn).collect::<Vec<_>>();
    assert_eq!(children[0].id(), left_block_id);
    assert_eq!(children[1].id(), tail_block_id);
    assert_eq!(
        <XmlTextRef as AsRef<Branch>>::as_ref(&paragraph_text(&fragment, &txn, 0)).id(),
        left_text_id
    );
    assert_eq!(
        <XmlTextRef as AsRef<Branch>>::as_ref(&paragraph_text(&fragment, &txn, 1)).id(),
        tail_text_id
    );
    assert_eq!(sticky.get_offset(&txn).unwrap().branch.id(), tail_text_id);
    let actual = YrsDocumentCodec::new(&schema, &limits)
        .read_json(&fragment, &txn)
        .unwrap();
    assert_eq!(actual, expected);
    assert_eq!(actual["content"][0]["attrs"]["id"], "joined");
    assert_eq!(actual["content"][0]["content"][1]["text"], "😀c");
    assert_eq!(
        actual["content"][0]["content"][1]["marks"][0]["type"],
        "bold"
    );
}

#[test]
fn join_blocks_disambiguates_equal_neighbors_and_ascends_to_nested_siblings() {
    let equal = json!({
        "type": "doc",
        "content": [
            { "type": "paragraph", "content": [{ "type": "text", "text": "xx" }] },
            { "type": "paragraph", "content": [{ "type": "text", "text": "xx" }] },
            { "type": "paragraph", "content": [{ "type": "text", "text": "xx" }] }
        ]
    });
    let (doc, _, _, compiled) = compile_operations_with_schema(
        &equal,
        vec![TypedOperation::JoinBlocks {
            at: point_for_test(5),
        }],
        tiptap_schema(),
    );
    let before_ids = {
        let txn = doc.transact();
        txn.get_xml_fragment("prosemirror")
            .unwrap()
            .children(&txn)
            .map(|child| child.id())
            .collect::<Vec<_>>()
    };
    {
        let mut txn = doc.transact_mut();
        execute_mutation_plan(compiled.mutation_plan, &mut txn);
    }
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let after_ids = fragment
        .children(&txn)
        .map(|child| child.id())
        .collect::<Vec<_>>();
    assert_eq!(
        after_ids,
        vec![before_ids[0].clone(), before_ids[1].clone()]
    );

    let nested = json!({
        "type": "doc",
        "content": [{
            "type": "bulletList",
            "content": [
                { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "ab" }] }] },
                { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "cd" }] }] }
            ]
        }]
    });
    let schema = tiptap_schema();
    let nested_document =
        from_prosemirror_json(&nested, &schema, UnknownTypeMode::Preserve).unwrap();
    let rendered = crate::render::rendered_text(&nested_document, &schema);
    let nested_byte = rendered.find("ab").unwrap();
    let nested_offset =
        u32::try_from(rendered[..nested_byte].chars().count().saturating_add(2)).unwrap();
    let (actual, expected, _, _, _) = compile_and_execute(
        nested,
        vec![TypedOperation::JoinBlocks {
            at: point_for_test(nested_offset),
        }],
    );
    assert_eq!(actual, expected);
    assert_eq!(actual["content"][0]["content"].as_array().unwrap().len(), 1);
    assert_eq!(
        actual["content"][0]["content"][0]["content"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn join_blocks_handles_empty_sides_and_keeps_the_first_block_type_and_attrs() {
    let empty_left = json!({
        "type": "doc",
        "content": [
            { "type": "paragraph" },
            { "type": "paragraph", "content": [{ "type": "text", "text": "right" }] }
        ]
    });
    let (actual, expected, _, _, _) = compile_and_execute(
        empty_left,
        vec![TypedOperation::JoinBlocks {
            at: point_for_test(0),
        }],
    );
    assert_eq!(actual, expected);
    assert_eq!(actual["content"][0]["content"][0]["text"], "right");

    let empty_right = json!({
        "type": "doc",
        "content": [
            { "type": "paragraph", "content": [{ "type": "text", "text": "left" }] },
            { "type": "paragraph" }
        ]
    });
    let (actual, expected, _, _, _) = compile_and_execute(
        empty_right,
        vec![TypedOperation::JoinBlocks {
            at: point_for_test(4),
        }],
    );
    assert_eq!(actual, expected);
    assert_eq!(actual["content"][0]["content"][0]["text"], "left");

    let differing = json!({
        "type": "doc",
        "content": [
            { "type": "h2", "attrs": { "id": "first" }, "content": [{ "type": "text", "text": "a" }] },
            { "type": "paragraph", "content": [{ "type": "text", "text": "b" }] }
        ]
    });
    let (doc, schema, limits, compiled) = compile_operations_with_schema(
        &differing,
        vec![TypedOperation::JoinBlocks {
            at: point_for_test(1),
        }],
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
    assert_eq!(actual["content"][0]["type"], "h2");
    assert_eq!(actual["content"][0]["attrs"]["id"], "first");
}

#[test]
fn join_blocks_copies_mixed_right_children_into_the_retained_left_element() {
    let source = json!({
        "type": "doc",
        "content": [
            {
                "type": "paragraph",
                "content": [
                    { "type": "text", "text": "L" },
                    { "type": "hardBreak" }
                ]
            },
            {
                "type": "paragraph",
                "content": [
                    { "type": "text", "text": "R", "marks": [{ "type": "bold" }] },
                    { "type": "hardBreak" },
                    { "type": "text", "text": "T" }
                ]
            }
        ]
    });
    let (doc, schema, limits, compiled) = compile_operations_with_schema(
        &source,
        vec![TypedOperation::JoinBlocks {
            at: point_for_test(2),
        }],
        tiptap_schema(),
    );
    let (left_block_id, left_child_ids, right_child_ids, before_full_len) = {
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let XmlOut::Element(left) = fragment.get(&txn, 0).unwrap() else {
            panic!("left paragraph expected")
        };
        let XmlOut::Element(right) = fragment.get(&txn, 1).unwrap() else {
            panic!("right paragraph expected")
        };
        (
            <yrs::types::xml::XmlElementRef as AsRef<Branch>>::as_ref(&left).id(),
            left.children(&txn)
                .map(|child| child.id())
                .collect::<Vec<_>>(),
            right
                .children(&txn)
                .map(|child| child.id())
                .collect::<Vec<_>>(),
            txn.encode_state_as_update_v1(&StateVector::default()).len(),
        )
    };
    assert!(matches!(
        compiled.mutation_plan.actions.as_slice(),
        [
            YrsMutationAction::InsertXmlChildren { .. },
            YrsMutationAction::DeleteXmlChildren { .. }
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
    let after_child_ids = left
        .children(&txn)
        .map(|child| child.id())
        .collect::<Vec<_>>();
    assert_eq!(&after_child_ids[..left_child_ids.len()], left_child_ids);
    assert!(after_child_ids[left_child_ids.len()..]
        .iter()
        .all(|id| !right_child_ids.contains(id)));
    let actual = YrsDocumentCodec::new(&schema, &limits)
        .read_json(&fragment, &txn)
        .unwrap();
    assert_eq!(actual, expected);
    assert_eq!(actual["content"][0]["content"][2]["text"], "R");
    assert_eq!(
        actual["content"][0]["content"][2]["marks"][0]["type"],
        "bold"
    );
    assert!(update.len() <= estimate);
    let after_full_len = txn.encode_state_as_update_v1(&StateVector::default()).len();
    assert!(after_full_len <= before_full_len + estimate);
}

#[test]
fn join_blocks_copies_nested_any_attributes_without_flattening() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "taskList",
            "content": [
                {
                    "type": "taskItem",
                    "attrs": { "checked": true },
                    "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "a" }] }]
                },
                {
                    "type": "taskItem",
                    "attrs": { "checked": false },
                    "content": [
                        { "type": "paragraph", "content": [{ "type": "text", "text": "b" }] },
                        { "type": "customBlock", "attrs": { "meta": { "nested": [1, false, "x"] } } }
                    ]
                }
            ]
        }]
    });
    let schema = attribute_schema();
    let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let rendered = crate::render::rendered_text(&document, &schema);
    let byte = rendered.find('a').unwrap();
    let offset = u32::try_from(rendered[..byte].chars().count() + 1).unwrap();
    let (doc, schema, limits, compiled) = compile_operations_with_schema(
        &source,
        vec![TypedOperation::JoinBlocks {
            at: point_for_test(offset),
        }],
        schema,
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
    assert_eq!(
        actual["content"][0]["content"][0]["content"][2]["attrs"]["meta"],
        json!({ "nested": [1, false, "x"] })
    );
}

#[test]
fn structural_replace_trims_marked_unicode_text_endpoints_around_an_atom() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [
                { "type": "text", "text": "A😀X" },
                { "type": "hardBreak" },
                { "type": "text", "text": "e\u{301}ZY" }
            ]
        }]
    });
    let (doc, schema, limits, editing_limits, _) = diagnostic_doc(&source);
    {
        let mut txn = doc.transact_mut();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let XmlOut::Element(paragraph) = fragment.get(&txn, 0).unwrap() else {
            panic!("expected paragraph")
        };
        let XmlOut::Text(left) = paragraph.get(&txn, 0).unwrap() else {
            panic!("expected left text")
        };
        let XmlOut::Text(right) = paragraph.get(&txn, 2).unwrap() else {
            panic!("expected right text")
        };
        left.format(
            &mut txn,
            1,
            3,
            Attrs::from([(Arc::<str>::from("bold"), Any::Bool(true))]),
        );
        right.format(
            &mut txn,
            0,
            2,
            Attrs::from([(Arc::<str>::from("italic"), Any::Bool(true))]),
        );
    }
    let codec = YrsDocumentCodec::new(&schema, &limits);
    let (document, left_id, old_atom_id, right_id, before_full_len) = {
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let XmlOut::Element(paragraph) = fragment.get(&txn, 0).unwrap() else {
            panic!("expected paragraph")
        };
        let children = paragraph.children(&txn).collect::<Vec<_>>();
        let json = codec.read_json(&fragment, &txn).unwrap();
        (
            from_prosemirror_json(&json, &schema, UnknownTypeMode::Preserve).unwrap(),
            children[0].id(),
            children[1].id(),
            children[2].id(),
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
                request_id: 114,
                base_document_revision: 0,
                origin: TransactionOrigin::LocalInput,
                operations: vec![TypedOperation::ReplaceRange {
                    range: range_for_test(2, 6),
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
    assert!(matches!(
        compiled.mutation_plan.actions.as_slice(),
        [
            YrsMutationAction::DeleteText {
                index_utf16: 0,
                len_utf16: 2,
                ..
            },
            YrsMutationAction::DeleteText {
                index_utf16: 3,
                len_utf16: 1,
                ..
            },
            YrsMutationAction::DeleteXmlChildren {
                child_index: 1,
                child_count: 1,
                ..
            },
            YrsMutationAction::InsertXmlChildren { child_index: 1, .. }
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
    let XmlOut::Element(paragraph) = fragment.get(&txn, 0).unwrap() else {
        panic!("expected paragraph")
    };
    let children = paragraph.children(&txn).collect::<Vec<_>>();
    assert_eq!(children.len(), 3);
    assert_eq!(children[0].id(), left_id);
    assert_ne!(children[1].id(), old_atom_id);
    assert_eq!(children[2].id(), right_id);
    assert_eq!(codec.read_json(&fragment, &txn).unwrap(), expected);
    assert_eq!(expected["content"][0]["content"][0]["text"], "A");
    assert_eq!(expected["content"][0]["content"][1]["text"], "😀");
    assert_eq!(
        expected["content"][0]["content"][1]["marks"][0]["type"],
        "bold"
    );
    assert_eq!(expected["content"][0]["content"][3]["text"], "ZY");
    assert!(update.len() <= estimate);
    let after_full_len = txn.encode_state_as_update_v1(&StateVector::default()).len();
    assert!(after_full_len <= before_full_len + estimate);
}

#[test]
fn structural_replace_disambiguates_duplicate_equal_atoms_by_position() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "hardBreak" }, { "type": "hardBreak" }]
        }]
    });
    let replace_and_ids = |from, to| {
        let (doc, schema, limits, editing_limits, document) = diagnostic_doc(&source);
        let original_ids = {
            let txn = doc.transact();
            let fragment = txn.get_xml_fragment("prosemirror").unwrap();
            let XmlOut::Element(paragraph) = fragment.get(&txn, 0).unwrap() else {
                panic!("expected paragraph")
            };
            paragraph
                .children(&txn)
                .map(|child| child.id())
                .collect::<Vec<_>>()
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
                    request_id: 115,
                    base_document_revision: 0,
                    origin: TransactionOrigin::LocalInput,
                    operations: vec![TypedOperation::ReplaceRange {
                        range: range_for_test(from, to),
                        content: Fragment::from(vec![Node::text("x".into(), vec![])]),
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
            execute_mutation_plan(compiled.mutation_plan, &mut txn);
        }
        let after_ids = {
            let txn = doc.transact();
            let fragment = txn.get_xml_fragment("prosemirror").unwrap();
            let XmlOut::Element(paragraph) = fragment.get(&txn, 0).unwrap() else {
                panic!("expected paragraph")
            };
            paragraph
                .children(&txn)
                .map(|child| child.id())
                .collect::<Vec<_>>()
        };
        (original_ids, after_ids)
    };

    let (before_first, after_first) = replace_and_ids(0, 1);
    assert_ne!(after_first[0], before_first[0]);
    assert_eq!(after_first[1], before_first[1]);
    let (before_second, after_second) = replace_and_ids(1, 2);
    assert_eq!(after_second[0], before_second[0]);
    assert_ne!(after_second[1], before_second[1]);
}

#[test]
fn structural_delete_maps_mark_runs_to_storage_and_preserves_unaffected_sticky() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [
                { "type": "text", "text": "ab" },
                { "type": "hardBreak" },
                { "type": "text", "text": "cd" }
            ]
        }]
    });
    let (doc, schema, limits, editing_limits, _) = diagnostic_doc(&source);
    {
        let mut txn = doc.transact_mut();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let first = paragraph_text(&fragment, &txn, 0);
        first.format(
            &mut txn,
            1,
            1,
            Attrs::from([(Arc::<str>::from("bold"), Any::Bool(true))]),
        );
    }
    let codec = YrsDocumentCodec::new(&schema, &limits);
    let (document, left_id, right_id, sticky, before_full_len) = {
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let json = codec.read_json(&fragment, &txn).unwrap();
        let document = from_prosemirror_json(&json, &schema, UnknownTypeMode::Preserve).unwrap();
        let XmlOut::Element(paragraph) = fragment.get(&txn, 0).unwrap() else {
            panic!("expected paragraph")
        };
        let children = paragraph.children(&txn).collect::<Vec<_>>();
        let left_id = children[0].id();
        let right_id = children[2].id();
        let XmlOut::Text(right) = &children[2] else {
            panic!("expected right text")
        };
        let sticky = StickyIndex::at(
            &txn,
            BranchPtr::from(<XmlTextRef as AsRef<Branch>>::as_ref(right)),
            1,
            Assoc::After,
        )
        .unwrap();
        (
            document,
            left_id,
            right_id,
            sticky,
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
                request_id: 110,
                base_document_revision: 0,
                origin: TransactionOrigin::LocalInput,
                operations: vec![TypedOperation::DeleteRange {
                    range: range_for_test(2, 3),
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
        let txn = doc.transact();
        let preflight =
            preflight_mutation_work_for_test(110, &compiled.mutation_plan, &txn).unwrap();
        let exact = compiled.mutation_plan.compilation_work_for_test() + preflight;
        compiled.mutation_plan.set_work_limit_for_test(exact);
        preflight_mutation_plan(110, &compiled.mutation_plan, &txn).unwrap();
        compiled.mutation_plan.set_work_limit_for_test(exact - 1);
        assert_eq!(
            preflight_mutation_plan(110, &compiled.mutation_plan, &txn)
                .unwrap_err()
                .code,
            "OPERATION_LIMIT_EXCEEDED"
        );
        compiled.mutation_plan.set_work_limit_for_test(exact);
    }
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
    assert_eq!(children.len(), 2);
    assert_eq!(children[0].id(), left_id);
    assert_eq!(children[1].id(), right_id);
    assert_eq!(sticky.get_offset(&txn).unwrap().index, 1);
    let actual = codec.read_json(&fragment, &txn).unwrap();
    assert_eq!(actual, to_prosemirror_json(&compiled.preview, &schema));
    assert!(update.len() <= estimate, "{} > {estimate}", update.len());
    let after_full_len = txn.encode_state_as_update_v1(&StateVector::default()).len();
    assert!(after_full_len <= before_full_len + estimate);
}
