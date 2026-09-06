#[test]
fn outdent_top_level_list_item_is_an_exact_compiler_noop() {
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
    let two = rendered_scalar_offset(&source, &schema, "two") + 1;
    let (_, _, _, compiled) = compile_operations_with_schema(
        &source,
        vec![TypedOperation::OutdentListItem {
            at: point_for_test(two),
        }],
        schema,
    );
    assert_eq!(
        to_prosemirror_json(&compiled.preview, &tiptap_schema()),
        source
    );
    assert!(compiled.mutation_plan.actions.is_empty());
    assert_eq!(compiled.encoded_growth_bound, 0);
    assert_eq!(compiled.undo_units_bound, 0);
}

#[test]
fn outdent_first_middle_and_last_nested_items_execute_directly() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "bulletList",
            "content": [
                {
                    "type": "listItem",
                    "content": [
                        { "type": "paragraph", "content": [{ "type": "text", "text": "parent" }] },
                        {
                            "type": "bulletList",
                            "content": [
                                { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "one" }] }] },
                                { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "two" }] }] },
                                { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "three" }] }] }
                            ]
                        }
                    ]
                },
                { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "tail" }] }] }
            ]
        }]
    });
    for (selected, expected_before, expected_after) in
        [("one", 0usize, 2usize), ("two", 1, 1), ("three", 2, 0)]
    {
        let schema = tiptap_schema();
        let at = rendered_scalar_offset(&source, &schema, selected) + 1;
        let (actual, expected, _, _, _) = compile_and_execute(
            source.clone(),
            vec![TypedOperation::OutdentListItem {
                at: point_for_test(at),
            }],
        );
        assert_eq!(actual, expected);
        let outer = actual["content"][0]["content"].as_array().unwrap();
        assert_eq!(outer.len(), 3);
        assert_eq!(outer[1]["content"][0]["content"][0]["text"], selected);
        let parent_content = outer[0]["content"].as_array().unwrap();
        if expected_before == 0 {
            assert_eq!(parent_content.len(), 1);
        } else {
            assert_eq!(
                parent_content[1]["content"].as_array().unwrap().len(),
                expected_before
            );
        }
        let moved_content = outer[1]["content"].as_array().unwrap();
        if expected_after == 0 {
            assert_eq!(moved_content.len(), 1);
        } else {
            assert_eq!(
                moved_content[1]["content"].as_array().unwrap().len(),
                expected_after
            );
        }
        assert_eq!(outer[2]["content"][0]["content"][0]["text"], "tail");
    }
}

#[test]
fn outdent_nested_prosemirror_list_item_executes_directly() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "bullet_list",
            "content": [{
                "type": "list_item",
                "content": [
                    { "type": "paragraph", "content": [{ "type": "text", "text": "parent" }] },
                    {
                        "type": "bullet_list",
                        "content": [{
                            "type": "list_item",
                            "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "nested" }] }]
                        }]
                    }
                ]
            }]
        }]
    });
    let schema = crate::schema::presets::prosemirror_schema();
    let at = rendered_scalar_offset(&source, &schema, "nested") + 1;
    let (doc, schema, limits, compiled) = compile_operations_with_schema(
        &source,
        vec![TypedOperation::OutdentListItem {
            at: point_for_test(at),
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
    assert_eq!(actual["content"][0]["content"].as_array().unwrap().len(), 2);
}

#[test]
fn outdent_preserves_existing_final_nested_list_attrs_when_merging_trailing_items() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "orderedList",
            "attrs": { "start": 10 },
            "content": [
                {
                    "type": "listItem",
                    "content": [
                        { "type": "paragraph", "content": [{ "type": "text", "text": "parent" }] },
                        {
                            "type": "orderedList",
                            "attrs": { "start": 5 },
                            "content": [
                                { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "before" }] }] },
                                {
                                    "type": "listItem",
                                    "content": [
                                        { "type": "paragraph", "content": [{ "type": "text", "text": "moved" }] },
                                        {
                                            "type": "orderedList",
                                            "attrs": { "start": 99 },
                                            "content": [{ "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "existing" }] }] }]
                                        }
                                    ]
                                },
                                { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "after-one" }] }] },
                                { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "after-two" }] }] }
                            ]
                        }
                    ]
                },
                { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "tail" }] }] }
            ]
        }]
    });
    let schema = tiptap_schema();
    let moved = rendered_scalar_offset(&source, &schema, "moved") + 1;
    let (actual, expected, _, _, _) = compile_and_execute(
        source,
        vec![TypedOperation::OutdentListItem {
            at: point_for_test(moved),
        }],
    );
    assert_eq!(actual, expected);
    let merged = &actual["content"][0]["content"][1]["content"][1];
    assert_eq!(merged["type"], "orderedList");
    assert_eq!(merged["attrs"]["start"], 99);
    assert_eq!(merged["content"].as_array().unwrap().len(), 3);
    assert_eq!(
        merged["content"][0]["content"][0]["content"][0]["text"],
        "existing"
    );
    assert_eq!(
        merged["content"][1]["content"][0]["content"][0]["text"],
        "after-one"
    );
    assert_eq!(
        merged["content"][2]["content"][0]["content"][0]["text"],
        "after-two"
    );
}

#[test]
fn outdent_preserves_stationary_parent_prefix_tail_ids_and_sticky() {
    let source = json!({
        "type": "doc",
        "content": [
            {
                "type": "bulletList",
                "content": [
                    {
                        "type": "listItem",
                        "content": [
                            { "type": "paragraph", "content": [{ "type": "text", "text": "parent" }] },
                            {
                                "type": "bulletList",
                                "content": [
                                    { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "one" }] }] },
                                    { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "two" }] }] },
                                    { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "three" }] }] }
                                ]
                            }
                        ]
                    },
                    { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "tail" }] }] }
                ]
            },
            { "type": "paragraph", "content": [{ "type": "text", "text": "after" }] }
        ]
    });
    let schema = tiptap_schema();
    let two = rendered_scalar_offset(&source, &schema, "two") + 1;
    let (doc, schema, limits, compiled) = compile_operations_with_schema(
        &source,
        vec![TypedOperation::OutdentListItem {
            at: point_for_test(two),
        }],
        schema,
    );
    let (outer_id, parent_id, nested_id, prefix_id, tail_id, after_id, sticky) = {
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let XmlOut::Element(outer) = fragment.get(&txn, 0).unwrap() else {
            panic!("outer list expected")
        };
        let items = outer.children(&txn).collect::<Vec<_>>();
        let XmlOut::Element(parent) = &items[0] else {
            panic!("parent item expected")
        };
        let XmlOut::Element(nested) = parent.get(&txn, 1).unwrap() else {
            panic!("nested list expected")
        };
        let prefix = nested.get(&txn, 0).unwrap();
        let prefix_text = list_item_text(&prefix, &txn);
        (
            AsRef::<Branch>::as_ref(&outer).id(),
            items[0].id(),
            AsRef::<Branch>::as_ref(&nested).id(),
            prefix.id(),
            items[1].id(),
            fragment.get(&txn, 1).unwrap().id(),
            StickyIndex::at(
                &txn,
                BranchPtr::from(<XmlTextRef as AsRef<Branch>>::as_ref(&prefix_text)),
                1,
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
    let XmlOut::Element(outer) = fragment.get(&txn, 0).unwrap() else {
        panic!("outer list expected")
    };
    let items = outer.children(&txn).collect::<Vec<_>>();
    let XmlOut::Element(parent) = &items[0] else {
        panic!("parent item expected")
    };
    let XmlOut::Element(nested) = parent.get(&txn, 1).unwrap() else {
        panic!("nested list expected")
    };
    assert_eq!(AsRef::<Branch>::as_ref(&outer).id(), outer_id);
    assert_eq!(items[0].id(), parent_id);
    assert_eq!(AsRef::<Branch>::as_ref(&nested).id(), nested_id);
    assert_eq!(nested.get(&txn, 0).unwrap().id(), prefix_id);
    assert_eq!(items[2].id(), tail_id);
    assert_eq!(fragment.get(&txn, 1).unwrap().id(), after_id);
    assert_eq!(sticky.get_offset(&txn).unwrap().index, 1);
    assert_eq!(
        YrsDocumentCodec::new(&schema, &limits)
            .read_json(&fragment, &txn)
            .unwrap(),
        expected
    );
}

#[test]
fn outdent_preflight_growth_undo_and_replica_bounds_are_exactly_enforced() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "bulletList",
            "content": [
                {
                    "type": "listItem",
                    "content": [
                        { "type": "paragraph", "content": [{ "type": "text", "text": "parent" }] },
                        {
                            "type": "bulletList",
                            "content": [
                                { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "one" }] }] },
                                { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "two" }] }] },
                                { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "three" }] }] }
                            ]
                        }
                    ]
                },
                { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "tail" }] }] }
            ]
        }]
    });
    let schema = tiptap_schema();
    let two = rendered_scalar_offset(&source, &schema, "two") + 1;
    let operations = || {
        vec![TypedOperation::OutdentListItem {
            at: point_for_test(two),
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
        compiled.mutation_plan.set_work_limit_for_test(exact);
        preflight_mutation_plan(122, &compiled.mutation_plan, &txn).unwrap();
        compiled.mutation_plan.set_work_limit_for_test(exact - 1);
        let error = preflight_mutation_plan(122, &compiled.mutation_plan, &txn).unwrap_err();
        assert_eq!(error.code, "OPERATION_LIMIT_EXCEEDED");
        assert_eq!(error.limit, Some(u64::try_from(exact - 1).unwrap()));
        assert!(error
            .actual
            .is_some_and(|actual| actual > u64::try_from(exact - 1).unwrap()));
        compiled.mutation_plan.set_work_limit_for_test(exact);
    }
    let expected = to_prosemirror_json(&compiled.preview, &schema);
    let growth_bound = compiled.encoded_growth_bound;
    let undo_bound = compiled.undo_units_bound;
    assert!(growth_bound > 0);
    assert!(undo_bound > 0);
    assert_eq!(
        compile_operations_with_undo_limit(&source, operations(), tiptap_schema(), undo_bound)
            .unwrap()
            .undo_units_bound,
        undo_bound
    );
    let undo_error =
        compile_operations_with_undo_limit(&source, operations(), tiptap_schema(), undo_bound - 1)
            .unwrap_err();
    assert_eq!(undo_error.code, "OPERATION_LIMIT_EXCEEDED");
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
    assert!(update.len() <= growth_bound);
    {
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        assert_eq!(
            YrsDocumentCodec::new(&schema, &limits)
                .read_json(&fragment, &txn)
                .unwrap(),
            expected
        );
        assert!(
            txn.encode_state_as_update_v1(&StateVector::default()).len()
                <= before_full_len + growth_bound
        );
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
fn outdent_uses_the_list_item_role_for_custom_schemas() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "taskList",
            "content": [{
                "type": "taskItem",
                "attrs": { "checked": false },
                "content": [
                    { "type": "paragraph", "content": [{ "type": "text", "text": "parent" }] },
                    {
                        "type": "taskList",
                        "content": [{
                            "type": "taskItem",
                            "attrs": { "checked": true },
                            "content": [{ "type": "paragraph" }]
                        }]
                    }
                ]
            }]
        }]
    });
    let schema = attribute_schema();
    let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let at = u32::try_from(
        crate::render::rendered_text(&document, &schema)
            .chars()
            .count(),
    )
    .unwrap();
    let (doc, schema, limits, compiled) = compile_operations_with_schema(
        &source,
        vec![TypedOperation::OutdentListItem {
            at: point_for_test(at),
        }],
        schema,
    );
    let expected = to_prosemirror_json(&compiled.preview, &schema);
    assert_ne!(expected, source);
    assert!(!compiled.mutation_plan.actions.is_empty());
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
fn outdent_folds_prior_and_follow_up_text_edits_into_the_moved_item() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "bulletList",
            "content": [{
                "type": "listItem",
                "content": [
                    { "type": "paragraph", "content": [{ "type": "text", "text": "parent" }] },
                    {
                        "type": "bulletList",
                        "content": [{
                            "type": "listItem",
                            "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "moved" }] }]
                        }]
                    }
                ]
            }]
        }]
    });
    let schema = tiptap_schema();
    let moved = rendered_scalar_offset(&source, &schema, "moved") + 1;
    for operations in [
        vec![
            TypedOperation::InsertText {
                at: point_for_test(moved),
                text: "X".into(),
                marks: vec![],
            },
            TypedOperation::OutdentListItem {
                at: point_for_test(moved),
            },
        ],
        vec![
            TypedOperation::OutdentListItem {
                at: point_for_test(moved),
            },
            TypedOperation::InsertText {
                at: point_for_test(moved),
                text: "X".into(),
                marks: vec![],
            },
        ],
    ] {
        let (doc, schema, limits, compiled) =
            compile_operations_with_schema(&source, operations, tiptap_schema());
        assert!(!compiled
            .mutation_plan
            .actions
            .iter()
            .any(|action| matches!(action, YrsMutationAction::InsertText { .. })));
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
            actual["content"][0]["content"][1]["content"][0]["content"][0]["text"],
            "mXoved"
        );
    }
}

#[test]
fn outdent_folds_prior_and_follow_up_attrs_into_a_literal_role_based_list_item() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "taskList",
            "attrs": { "listMeta": { "id": "outer" } },
            "content": [{
                "type": "listItem",
                "attrs": { "checked": false, "itemMeta": { "id": "parent" } },
                "content": [
                    { "type": "paragraph", "content": [{ "type": "text", "text": "parent" }] },
                    {
                        "type": "taskList",
                        "attrs": { "listMeta": { "id": "nested" } },
                        "content": [{
                            "type": "listItem",
                            "attrs": { "checked": true, "itemMeta": { "id": "moved" } },
                            "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "moved" }] }]
                        }]
                    }
                ]
            }]
        }]
    });
    let schema = literal_list_item_attr_schema();
    let moved = rendered_scalar_offset(&source, &schema, "moved") + 1;
    let attrs = HashMap::from([
        ("checked".into(), Value::Bool(false)),
        ("itemMeta".into(), json!({ "id": "updated" })),
    ]);
    for operations in [
        vec![
            TypedOperation::UpdateNodeAttrs {
                at: point_for_test(moved),
                attrs: attrs.clone(),
            },
            TypedOperation::OutdentListItem {
                at: point_for_test(moved),
            },
        ],
        vec![
            TypedOperation::OutdentListItem {
                at: point_for_test(moved),
            },
            TypedOperation::UpdateNodeAttrs {
                at: point_for_test(moved),
                attrs: attrs.clone(),
            },
        ],
    ] {
        let (doc, schema, limits, compiled) =
            compile_operations_with_schema(&source, operations, schema.clone());
        assert!(!compiled.mutation_plan.actions.iter().any(|action| {
            matches!(
                action,
                YrsMutationAction::SetXmlAttribute { .. }
                    | YrsMutationAction::RemoveXmlAttribute { .. }
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
        let moved_item = &actual["content"][0]["content"][1];
        assert_eq!(moved_item["attrs"]["checked"], false);
        assert_eq!(moved_item["attrs"]["itemMeta"]["id"], "updated");
    }
}

#[test]
fn outdent_then_insert_node_folds_into_the_moved_prepared_item() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "bulletList",
            "content": [{
                "type": "listItem",
                "content": [
                    { "type": "paragraph", "content": [{ "type": "text", "text": "parent" }] },
                    {
                        "type": "bulletList",
                        "content": [{
                            "type": "listItem",
                            "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "moved" }] }]
                        }]
                    }
                ]
            }]
        }]
    });
    let schema = tiptap_schema();
    let moved = rendered_scalar_offset(&source, &schema, "moved") + 2;
    let (doc, schema, limits, compiled) = compile_operations_with_schema(
        &source,
        vec![
            TypedOperation::OutdentListItem {
                at: point_for_test(moved),
            },
            TypedOperation::InsertNode {
                at: point_for_test(moved),
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
    assert!(actual["content"][0]["content"][1]["content"][0]["content"]
        .as_array()
        .unwrap()
        .iter()
        .any(|node| node["type"] == "hardBreak"));
}
