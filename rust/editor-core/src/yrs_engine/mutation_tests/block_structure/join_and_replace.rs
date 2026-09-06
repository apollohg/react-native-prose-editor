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
