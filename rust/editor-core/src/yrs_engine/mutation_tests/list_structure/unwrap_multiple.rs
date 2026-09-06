#[test]
fn unwrap_first_item_retains_the_stationary_right_list_and_item_identities() {
    let source = json!({
        "type": "doc",
        "content": [
            {
                "type": "bulletList",
                "content": [
                    { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "one" }] }] },
                    { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "two" }] }] },
                    { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "three" }] }] }
                ]
            },
            { "type": "paragraph", "content": [{ "type": "text", "text": "tail" }] }
        ]
    });
    let schema = tiptap_schema();
    let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let rendered = crate::render::rendered_text(&document, &schema);
    let one = u32::try_from(rendered[..rendered.find("one").unwrap()].chars().count()).unwrap();
    let (doc, schema, limits, compiled) = compile_operations_with_schema(
        &source,
        vec![TypedOperation::UnwrapFromList {
            at: point_for_test(one + 1),
        }],
        schema,
    );
    let (list_id, remaining_item_ids, tail_id) = {
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let XmlOut::Element(list) = fragment.get(&txn, 0).unwrap() else {
            panic!("list expected")
        };
        let items = list.children(&txn).collect::<Vec<_>>();
        (
            AsRef::<Branch>::as_ref(&list).id(),
            vec![items[1].id(), items[2].id()],
            fragment.get(&txn, 1).unwrap().id(),
        )
    };
    let expected = to_prosemirror_json(&compiled.preview, &schema);
    {
        let mut txn = doc.transact_mut();
        execute_mutation_plan(compiled.mutation_plan, &mut txn);
    }
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let XmlOut::Element(list) = fragment.get(&txn, 1).unwrap() else {
        panic!("remaining list expected")
    };
    assert_eq!(AsRef::<Branch>::as_ref(&list).id(), list_id);
    assert_eq!(
        list.children(&txn)
            .map(|child| child.id())
            .collect::<Vec<_>>(),
        remaining_item_ids
    );
    assert_eq!(fragment.get(&txn, 2).unwrap().id(), tail_id);
    assert_eq!(
        YrsDocumentCodec::new(&schema, &limits)
            .read_json(&fragment, &txn)
            .unwrap(),
        expected
    );
}

#[test]
fn unwrap_first_then_insert_node_into_extracted_block() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "bulletList",
            "content": [
                {
                    "type": "listItem",
                    "content": [{
                        "type": "paragraph",
                        "content": [{ "type": "text", "text": "one" }]
                    }]
                },
                {
                    "type": "listItem",
                    "content": [{
                        "type": "paragraph",
                        "content": [{ "type": "text", "text": "two" }]
                    }]
                }
            ]
        }]
    });
    let schema = tiptap_schema();
    let one = rendered_scalar_offset(&source, &schema, "one");
    let (doc, schema, limits, compiled) = compile_operations_with_schema(
        &source,
        vec![
            TypedOperation::UnwrapFromList {
                at: point_for_test(one + 1),
            },
            TypedOperation::InsertNode {
                at: point_for_test(one + 1),
                node: Node::void("hardBreak".into(), HashMap::new()),
            },
        ],
        schema,
    );
    assert_eq!(
        compiled
            .mutation_plan
            .actions
            .iter()
            .filter(|action| matches!(action, YrsMutationAction::InsertXmlChildren { .. }))
            .count(),
        1,
        "the inline node should be owned by the prepared extracted block"
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
    assert!(actual["content"][0]["content"]
        .as_array()
        .unwrap()
        .iter()
        .any(|node| node["type"] == "hardBreak"));
}

#[test]
fn unwrap_last_and_middle_retain_the_left_list_and_stationary_item_identities() {
    for selected in [1usize, 2usize] {
        let source = json!({
            "type": "doc",
            "content": [
                {
                    "type": "bulletList",
                    "content": [
                        { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "one" }] }] },
                        { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "two" }] }] },
                        { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "three" }] }] }
                    ]
                },
                { "type": "paragraph", "content": [{ "type": "text", "text": "tail" }] }
            ]
        });
        let selected_text = if selected == 1 { "two" } else { "three" };
        let schema = tiptap_schema();
        let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
        let rendered = crate::render::rendered_text(&document, &schema);
        let at = u32::try_from(
            rendered[..rendered.find(selected_text).unwrap()]
                .chars()
                .count(),
        )
        .unwrap();
        let (doc, schema, limits, compiled) = compile_operations_with_schema(
            &source,
            vec![TypedOperation::UnwrapFromList {
                at: point_for_test(at + 1),
            }],
            schema,
        );
        let (list_id, item_ids, tail_id) = {
            let txn = doc.transact();
            let fragment = txn.get_xml_fragment("prosemirror").unwrap();
            let XmlOut::Element(list) = fragment.get(&txn, 0).unwrap() else {
                panic!("list expected")
            };
            (
                AsRef::<Branch>::as_ref(&list).id(),
                list.children(&txn)
                    .map(|child| child.id())
                    .collect::<Vec<_>>(),
                fragment.get(&txn, 1).unwrap().id(),
            )
        };
        let expected = to_prosemirror_json(&compiled.preview, &schema);
        {
            let mut txn = doc.transact_mut();
            execute_mutation_plan(compiled.mutation_plan, &mut txn);
        }
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let XmlOut::Element(left_list) = fragment.get(&txn, 0).unwrap() else {
            panic!("left list expected")
        };
        assert_eq!(AsRef::<Branch>::as_ref(&left_list).id(), list_id);
        let retained = left_list
            .children(&txn)
            .map(|child| child.id())
            .collect::<Vec<_>>();
        assert_eq!(retained, item_ids[..selected]);
        let tail_index = if selected == 1 { 3 } else { 2 };
        assert_eq!(fragment.get(&txn, tail_index).unwrap().id(), tail_id);
        assert_eq!(
            YrsDocumentCodec::new(&schema, &limits)
                .read_json(&fragment, &txn)
                .unwrap(),
            expected
        );
    }
}

#[test]
fn unwrap_middle_retains_the_larger_stationary_side_with_deterministic_left_ties() {
    let source = json!({
        "type": "doc",
        "content": [
            {
                "type": "bulletList",
                "content": [
                    { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "one" }] }] },
                    { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "two" }] }] },
                    { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "three" }] }] },
                    { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "four" }] }] },
                    { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "five" }] }] }
                ]
            },
            { "type": "paragraph", "content": [{ "type": "text", "text": "tail" }] }
        ]
    });
    for (selected, selected_text, retains_right, sticky_item) in [
        (1usize, "two", true, 2usize),
        (3usize, "four", false, 2usize),
        (2usize, "three", false, 1usize),
    ] {
        let schema = tiptap_schema();
        let at = rendered_scalar_offset(&source, &schema, selected_text);
        let (doc, schema, limits, compiled) = compile_operations_with_schema(
            &source,
            vec![TypedOperation::UnwrapFromList {
                at: point_for_test(at + 1),
            }],
            schema,
        );
        let (
            list_id,
            item_ids,
            stationary_text_id,
            stationary_sticky,
            tail_id,
            tail_text_id,
            tail_sticky,
        ) = {
            let txn = doc.transact();
            let fragment = txn.get_xml_fragment("prosemirror").unwrap();
            let XmlOut::Element(list) = fragment.get(&txn, 0).unwrap() else {
                panic!("list expected")
            };
            let items = list.children(&txn).collect::<Vec<_>>();
            let stationary_text = list_item_text(&items[sticky_item], &txn);
            let tail_text = paragraph_text(&fragment, &txn, 1);
            (
                AsRef::<Branch>::as_ref(&list).id(),
                items.iter().map(XmlOut::id).collect::<Vec<_>>(),
                <XmlTextRef as AsRef<Branch>>::as_ref(&stationary_text).id(),
                StickyIndex::at(
                    &txn,
                    BranchPtr::from(<XmlTextRef as AsRef<Branch>>::as_ref(&stationary_text)),
                    1,
                    Assoc::After,
                )
                .unwrap(),
                fragment.get(&txn, 1).unwrap().id(),
                <XmlTextRef as AsRef<Branch>>::as_ref(&tail_text).id(),
                StickyIndex::at(
                    &txn,
                    BranchPtr::from(<XmlTextRef as AsRef<Branch>>::as_ref(&tail_text)),
                    2,
                    Assoc::After,
                )
                .unwrap(),
            )
        };
        let expected = to_prosemirror_json(&compiled.preview, &schema);
        {
            let mut txn = doc.transact_mut();
            execute_mutation_plan(compiled.mutation_plan, &mut txn);
        }
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let stationary_root_index = if retains_right { 2 } else { 0 };
        let XmlOut::Element(stationary_list) = fragment.get(&txn, stationary_root_index).unwrap()
        else {
            panic!("stationary list expected")
        };
        assert_eq!(AsRef::<Branch>::as_ref(&stationary_list).id(), list_id);
        let expected_ids = if retains_right {
            &item_ids[selected + 1..]
        } else {
            &item_ids[..selected]
        };
        assert_eq!(
            stationary_list
                .children(&txn)
                .map(|child| child.id())
                .collect::<Vec<_>>(),
            expected_ids
        );
        let resolved_stationary = stationary_sticky.get_offset(&txn).unwrap();
        assert_eq!(resolved_stationary.branch.id(), stationary_text_id);
        assert_eq!(resolved_stationary.index, 1);
        assert_eq!(fragment.get(&txn, 3).unwrap().id(), tail_id);
        let resolved_tail = tail_sticky.get_offset(&txn).unwrap();
        assert_eq!(resolved_tail.branch.id(), tail_text_id);
        assert_eq!(resolved_tail.index, 2);
        assert_eq!(
            YrsDocumentCodec::new(&schema, &limits)
                .read_json(&fragment, &txn)
                .unwrap(),
            expected
        );
    }
}

#[test]
fn unwrap_middle_then_edit_retained_right_item_and_tail() {
    let source = json!({
        "type": "doc",
        "content": [
            {
                "type": "bulletList",
                "content": [
                    { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "one" }] }] },
                    { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "two" }] }] },
                    { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "three" }] }] }
                ]
            },
            { "type": "paragraph", "content": [{ "type": "text", "text": "tail" }] }
        ]
    });
    let schema = tiptap_schema();
    let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let rendered = crate::render::rendered_text(&document, &schema);
    let offset = |needle: &str| {
        u32::try_from(rendered[..rendered.find(needle).unwrap()].chars().count()).unwrap()
    };
    let two = offset("two");
    let three = offset("three");
    let tail = offset("tail");
    let (doc, schema, limits, compiled) = compile_operations_with_schema(
        &source,
        vec![
            TypedOperation::UnwrapFromList {
                at: point_for_test(two + 1),
            },
            TypedOperation::InsertText {
                at: point_for_test(three + 1),
                text: "X".into(),
                marks: vec![],
            },
            TypedOperation::InsertText {
                at: point_for_test(tail + 2),
                text: "Y".into(),
                marks: vec![],
            },
        ],
        schema,
    );
    assert_eq!(
        compiled
            .mutation_plan
            .actions
            .iter()
            .filter(|action| matches!(action, YrsMutationAction::InsertText { .. }))
            .count(),
        1,
        "the prepared right item edit should fold, leaving only the tail text action"
    );
    let tail_id = {
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        fragment.get(&txn, 1).unwrap().id()
    };
    let expected = to_prosemirror_json(&compiled.preview, &schema);
    {
        let mut txn = doc.transact_mut();
        execute_mutation_plan(compiled.mutation_plan, &mut txn);
    }
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    assert_eq!(fragment.get(&txn, 3).unwrap().id(), tail_id);
    let actual = YrsDocumentCodec::new(&schema, &limits)
        .read_json(&fragment, &txn)
        .unwrap();
    assert_eq!(actual, expected);
    assert_eq!(
        actual["content"][2]["content"][0]["content"][0]["content"][0]["text"],
        "tXhree"
    );
    assert_eq!(actual["content"][3]["content"][0]["text"], "taYil");
}

#[test]
fn unwrap_nested_list_item_splices_blocks_inside_the_outer_item() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "bulletList",
            "content": [{
                "type": "listItem",
                "content": [
                    { "type": "paragraph", "content": [{ "type": "text", "text": "outer" }] },
                    {
                        "type": "bulletList",
                        "content": [
                            { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "inner-one" }] }] },
                            { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "inner-two" }] }] }
                        ]
                    }
                ]
            }]
        }]
    });
    let schema = tiptap_schema();
    let inner = rendered_scalar_offset(&source, &schema, "inner-one");
    let (actual, expected, _, _, _) = compile_and_execute(
        source,
        vec![TypedOperation::UnwrapFromList {
            at: point_for_test(inner + 1),
        }],
    );
    assert_eq!(actual, expected);
    let outer_content = actual["content"][0]["content"][0]["content"]
        .as_array()
        .unwrap();
    assert_eq!(outer_content[0]["content"][0]["text"], "outer");
    assert_eq!(outer_content[1]["content"][0]["text"], "inner-one");
    assert_eq!(outer_content[2]["type"], "bulletList");
    assert_eq!(
        outer_content[2]["content"][0]["content"][0]["content"][0]["text"],
        "inner-two"
    );
}

#[test]
fn multiple_sibling_unwraps_preflight_in_both_operation_orders() {
    let source = json!({
        "type": "doc",
        "content": [
            {
                "type": "bulletList",
                "content": [{
                    "type": "listItem",
                    "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "one" }] }]
                }]
            },
            {
                "type": "bulletList",
                "content": [{
                    "type": "listItem",
                    "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "two" }] }]
                }]
            }
        ]
    });
    let schema = tiptap_schema();
    let one = rendered_scalar_offset(&source, &schema, "one") + 1;
    let two = rendered_scalar_offset(&source, &schema, "two") + 1;
    for positions in [[one, two], [two, one]] {
        let (actual, expected, _, _, _) = compile_and_execute(
            source.clone(),
            positions
                .into_iter()
                .map(|at| TypedOperation::UnwrapFromList {
                    at: point_for_test(at),
                })
                .collect(),
        );
        assert_eq!(actual, expected);
        assert_eq!(
            actual["content"]
                .as_array()
                .unwrap()
                .iter()
                .map(|node| node["type"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["paragraph", "paragraph"]
        );
    }
}

#[test]
fn unwrap_nested_list_under_a_prepared_parent_rewrites_one_owned_batch() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "bulletList",
            "content": [{
                "type": "listItem",
                "content": [
                    { "type": "paragraph", "content": [{ "type": "text", "text": "outer" }] },
                    {
                        "type": "bulletList",
                        "content": [{
                            "type": "listItem",
                            "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "inner" }] }]
                        }]
                    }
                ]
            }]
        }]
    });
    let schema = tiptap_schema();
    let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let rendered = crate::render::rendered_text(&document, &schema);
    let outer = u32::try_from(rendered[..rendered.find("outer").unwrap()].chars().count()).unwrap();
    let inner = u32::try_from(rendered[..rendered.find("inner").unwrap()].chars().count()).unwrap();
    let (doc, schema, limits, compiled) = compile_operations_with_schema(
        &source,
        vec![
            TypedOperation::SplitBlock {
                at: point_for_test(outer + 2),
                node_type: "paragraph".into(),
                attrs: HashMap::new(),
            },
            TypedOperation::UnwrapFromList {
                at: point_for_test(inner + 1),
            },
        ],
        schema,
    );
    assert_eq!(
        compiled
            .mutation_plan
            .actions
            .iter()
            .filter(|action| matches!(action, YrsMutationAction::InsertXmlChildren { .. }))
            .count(),
        1
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
    let prepared_item = &actual["content"][0]["content"][1];
    assert_eq!(prepared_item["content"][1]["type"], "paragraph");
    assert_eq!(prepared_item["content"][1]["content"][0]["text"], "inner");
}

#[test]
fn unwrap_supports_empty_items_and_preserves_void_and_task_attrs() {
    let empty = json!({
        "type": "doc",
        "content": [{
            "type": "bulletList",
            "content": [{
                "type": "listItem",
                "content": [{ "type": "paragraph" }]
            }]
        }]
    });
    let schema = tiptap_schema();
    let empty_document = from_prosemirror_json(&empty, &schema, UnknownTypeMode::Preserve).unwrap();
    let empty_at = u32::try_from(
        crate::render::rendered_text(&empty_document, &schema)
            .chars()
            .count(),
    )
    .unwrap();
    let (actual_empty, expected_empty, _, _, _) = compile_and_execute(
        empty,
        vec![TypedOperation::UnwrapFromList {
            at: point_for_test(empty_at),
        }],
    );
    assert_eq!(actual_empty, expected_empty);
    assert_eq!(
        actual_empty,
        json!({ "type": "doc", "content": [{ "type": "paragraph" }] })
    );

    let task = json!({
        "type": "doc",
        "content": [{
            "type": "taskList",
            "attrs": { "listMeta": { "owner": "team", "rank": 7 } },
            "content": [
                {
                    "type": "taskItem",
                    "attrs": { "checked": true, "itemMeta": { "id": "extract" } },
                    "content": [
                        { "type": "paragraph", "content": [{ "type": "text", "text": "extract" }] },
                        { "type": "image", "attrs": { "src": "asset://one", "alt": "typed" } }
                    ]
                },
                {
                    "type": "taskItem",
                    "attrs": { "checked": false, "itemMeta": { "id": "stationary", "score": 4 } },
                    "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "remain" }] }]
                }
            ]
        }]
    });
    let schema = attribute_schema();
    let extract = rendered_scalar_offset(&task, &schema, "extract");
    let (doc, schema, limits, compiled) = compile_operations_with_schema(
        &task,
        vec![TypedOperation::UnwrapFromList {
            at: point_for_test(extract + 1),
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
    assert_eq!(actual["content"][1]["type"], "image");
    assert_eq!(actual["content"][1]["attrs"]["src"], "asset://one");
    assert_eq!(actual["content"][1]["attrs"]["alt"], "typed");
    assert_eq!(
        actual["content"][2]["attrs"]["listMeta"],
        json!({ "owner": "team", "rank": 7 })
    );
    assert_eq!(
        actual["content"][2]["content"][0]["attrs"]["checked"],
        false
    );
    assert_eq!(
        actual["content"][2]["content"][0]["attrs"]["itemMeta"],
        json!({ "id": "stationary", "score": 4 })
    );
}

#[test]
fn unwrap_preflight_growth_undo_and_replica_bounds_are_exactly_enforced() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "bulletList",
            "content": [{
                "type": "listItem",
                "content": [{
                    "type": "paragraph",
                    "content": [{ "type": "text", "text": "one" }]
                }]
            }]
        }]
    });
    let schema = tiptap_schema();
    let one = rendered_scalar_offset(&source, &schema, "one");
    let operations = || {
        vec![TypedOperation::UnwrapFromList {
            at: point_for_test(one + 1),
        }]
    };
    let (doc, schema, limits, mut compiled) =
        compile_operations_with_schema(&source, operations(), schema);
    let (before_update, before_full_len) = {
        let txn = doc.transact();
        let update = txn.encode_state_as_update_v1(&StateVector::default());
        let len = update.len();
        (update, len)
    };
    {
        let txn = doc.transact();
        let preflight =
            preflight_mutation_work_for_test(122, &compiled.mutation_plan, &txn).unwrap();
        let exact = compiled.mutation_plan.compilation_work_for_test() + preflight;
        let exact_u64 = u64::try_from(exact).unwrap();
        compiled.mutation_plan.set_work_limit_for_test(exact);
        preflight_mutation_plan(122, &compiled.mutation_plan, &txn).unwrap();
        compiled.mutation_plan.set_work_limit_for_test(exact - 1);
        let error = preflight_mutation_plan(122, &compiled.mutation_plan, &txn).unwrap_err();
        assert_eq!(error.code, "OPERATION_LIMIT_EXCEEDED");
        assert_eq!(error.limit, Some(exact_u64 - 1));
        assert!(error.actual.is_some_and(|actual| actual > exact_u64 - 1));
        compiled.mutation_plan.set_work_limit_for_test(exact);
    }
    let expected = to_prosemirror_json(&compiled.preview, &schema);
    let growth_bound = compiled.encoded_growth_bound;
    let undo_bound = compiled.undo_units_bound;
    assert!(undo_bound > 0);
    assert_eq!(
        compile_operations_with_undo_limit(&source, operations(), tiptap_schema(), undo_bound,)
            .unwrap()
            .undo_units_bound,
        undo_bound
    );
    let undo_error =
        compile_operations_with_undo_limit(&source, operations(), tiptap_schema(), undo_bound - 1)
            .unwrap_err();
    assert_eq!(undo_error.code, "OPERATION_LIMIT_EXCEEDED");
    assert_eq!(undo_error.limit, Some(undo_bound - 1));
    assert_eq!(undo_error.actual, Some(undo_bound));

    let fragment = doc.transact().get_xml_fragment("prosemirror").unwrap();
    let mut undo = UndoManager::<()>::new();
    undo.expand_scope(&doc, &fragment);
    let update = {
        let mut txn = doc.transact_mut();
        execute_mutation_plan(compiled.mutation_plan, &mut txn);
        txn.commit();
        txn.encode_update_v1()
    };
    assert!(
        update.len() <= growth_bound,
        "{} > {growth_bound}",
        update.len()
    );
    {
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        assert_eq!(
            YrsDocumentCodec::new(&schema, &limits)
                .read_json(&fragment, &txn)
                .unwrap(),
            expected
        );
        let after_full_len = txn.encode_state_as_update_v1(&StateVector::default()).len();
        assert!(after_full_len <= before_full_len + growth_bound);
    }

    let replica = utf16_doc();
    {
        let mut txn = replica.transact_mut();
        txn.apply_update(Update::decode_v1(&before_update).unwrap())
            .unwrap();
        txn.apply_update(Update::decode_v1(&update).unwrap())
            .unwrap();
    }
    {
        let txn = replica.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        assert_eq!(
            YrsDocumentCodec::new(&schema, &limits)
                .read_json(&fragment, &txn)
                .unwrap(),
            expected
        );
    }

    assert!(undo.undo_blocking());
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    assert_eq!(
        YrsDocumentCodec::new(&schema, &limits)
            .read_json(&fragment, &txn)
            .unwrap(),
        source
    );
}

#[test]
fn wrap_multiple_remaps_following_attrs_and_structural_insertions() {
    let source = json!({
        "type": "doc",
        "content": [
            { "type": "paragraph", "content": [{ "type": "text", "text": "aa" }] },
            { "type": "paragraph", "content": [{ "type": "text", "text": "bb" }] },
            { "type": "h2", "attrs": { "id": "tail" }, "content": [{ "type": "text", "text": "cc" }] }
        ]
    });
    let schema = attribute_schema();
    let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let rendered = crate::render::rendered_text(&document, &schema);
    let tail_byte = rendered.find("cc").unwrap();
    let tail = u32::try_from(rendered[..tail_byte].chars().count()).unwrap();
    let (doc, schema, limits, compiled) = compile_operations_with_schema(
        &source,
        vec![
            TypedOperation::WrapInList {
                range: range_for_test(0, tail - 1),
                list_type: "taskList".into(),
                item_type: "taskItem".into(),
                attrs: HashMap::new(),
                item_attrs: HashMap::new(),
            },
            TypedOperation::UpdateNodeAttrs {
                at: point_for_test(tail),
                attrs: HashMap::from([("id".into(), Value::String("tail-new".into()))]),
            },
            TypedOperation::InsertNode {
                at: point_for_test(tail + 2),
                node: Node::void("hardBreak".into(), HashMap::new()),
            },
        ],
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
    assert_eq!(actual["content"][1]["attrs"]["id"], "tail-new");
    assert!(actual["content"][1]["content"]
        .as_array()
        .unwrap()
        .iter()
        .any(|node| node["type"] == "hardBreak"));
}

fn literal_list_item_attr_schema() -> crate::schema::Schema {
    crate::schema::Schema::from_json(&json!({
        "nodes": [
            { "name": "doc", "content": "block+", "role": "doc" },
            { "name": "paragraph", "content": "inline*", "group": "block", "role": "textBlock", "htmlTag": "p" },
            { "name": "hardBreak", "content": "", "group": "inline", "role": "hardBreak", "htmlTag": "br", "isVoid": true },
            { "name": "taskList", "content": "listItem+", "group": "block", "role": "list", "htmlTag": "ul", "attrs": { "listMeta": { "default": null } } },
            { "name": "listItem", "content": "paragraph block*", "role": "listItem", "htmlTag": "li", "attrs": { "checked": { "default": null }, "itemMeta": { "default": null } } },
            { "name": "text", "group": "inline", "role": "text" }
        ],
        "marks": []
    }))
    .unwrap()
}

fn list_item_text<T: ReadTxn>(item: &XmlOut, txn: &T) -> XmlTextRef {
    let XmlOut::Element(item) = item else {
        panic!("list item expected")
    };
    let XmlOut::Element(paragraph) = item.get(txn, 0).unwrap() else {
        panic!("paragraph expected")
    };
    let XmlOut::Text(text) = paragraph.get(txn, 0).unwrap() else {
        panic!("text expected")
    };
    text
}
