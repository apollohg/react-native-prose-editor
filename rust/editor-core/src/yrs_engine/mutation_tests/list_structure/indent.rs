#[test]
fn indent_first_list_item_is_an_exact_compiler_noop() {
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
    let one = rendered_scalar_offset(&source, &schema, "one") + 1;
    let (_, _, _, compiled) = compile_operations_with_schema(
        &source,
        vec![TypedOperation::IndentListItem {
            at: point_for_test(one),
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
fn indent_list_item_creates_a_direct_nested_list_and_matches_replica() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "orderedList",
            "attrs": { "start": 3 },
            "content": [
                { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "one" }] }] },
                { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "two" }] }] },
                { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "three" }] }] }
            ]
        }]
    });
    let schema = tiptap_schema();
    let two = rendered_scalar_offset(&source, &schema, "two") + 1;
    let (actual, expected, _, _, _) = compile_and_execute(
        source,
        vec![TypedOperation::IndentListItem {
            at: point_for_test(two),
        }],
    );
    assert_eq!(actual, expected);
    let outer = &actual["content"][0];
    assert_eq!(outer["content"].as_array().unwrap().len(), 2);
    let nested = &outer["content"][0]["content"][1];
    assert_eq!(nested["type"], "orderedList");
    assert_eq!(nested["attrs"]["start"], 3);
    assert_eq!(
        nested["content"][0]["content"][0]["content"][0]["text"],
        "two"
    );
    assert_eq!(
        outer["content"][1]["content"][0]["content"][0]["text"],
        "three"
    );
}

#[test]
fn indent_appends_to_existing_final_same_type_list_and_preserves_stationary_ids() {
    let source = json!({
        "type": "doc",
        "content": [
            {
                "type": "bulletList",
                "content": [
                    {
                        "type": "listItem",
                        "content": [
                            { "type": "paragraph", "content": [{ "type": "text", "text": "one" }] },
                            {
                                "type": "bulletList",
                                "content": [{
                                    "type": "listItem",
                                    "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "nested" }] }]
                                }]
                            }
                        ]
                    },
                    { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "two" }] }] },
                    { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "three" }] }] }
                ]
            },
            { "type": "paragraph", "content": [{ "type": "text", "text": "tail" }] }
        ]
    });
    let schema = tiptap_schema();
    let two = rendered_scalar_offset(&source, &schema, "two") + 1;
    let (doc, schema, limits, compiled) = compile_operations_with_schema(
        &source,
        vec![TypedOperation::IndentListItem {
            at: point_for_test(two),
        }],
        schema,
    );
    let (outer_id, first_id, nested_id, nested_item_id, tail_item_id, nested_sticky) = {
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let XmlOut::Element(outer) = fragment.get(&txn, 0).unwrap() else {
            panic!("outer list expected")
        };
        let items = outer.children(&txn).collect::<Vec<_>>();
        let XmlOut::Element(first) = &items[0] else {
            panic!("first item expected")
        };
        let XmlOut::Element(nested) = first.get(&txn, 1).unwrap() else {
            panic!("nested list expected")
        };
        let nested_item = nested.get(&txn, 0).unwrap();
        let nested_text = list_item_text(&nested_item, &txn);
        (
            AsRef::<Branch>::as_ref(&outer).id(),
            items[0].id(),
            AsRef::<Branch>::as_ref(&nested).id(),
            nested_item.id(),
            items[2].id(),
            StickyIndex::at(
                &txn,
                BranchPtr::from(<XmlTextRef as AsRef<Branch>>::as_ref(&nested_text)),
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
    let XmlOut::Element(outer) = fragment.get(&txn, 0).unwrap() else {
        panic!("outer list expected")
    };
    let items = outer.children(&txn).collect::<Vec<_>>();
    let XmlOut::Element(first) = &items[0] else {
        panic!("first item expected")
    };
    let XmlOut::Element(nested) = first.get(&txn, 1).unwrap() else {
        panic!("nested list expected")
    };
    assert_eq!(AsRef::<Branch>::as_ref(&outer).id(), outer_id);
    assert_eq!(items[0].id(), first_id);
    assert_eq!(AsRef::<Branch>::as_ref(&nested).id(), nested_id);
    assert_eq!(nested.get(&txn, 0).unwrap().id(), nested_item_id);
    assert_eq!(items[1].id(), tail_item_id);
    assert_eq!(nested_sticky.get_offset(&txn).unwrap().index, 2);
    assert_eq!(
        YrsDocumentCodec::new(&schema, &limits)
            .read_json(&fragment, &txn)
            .unwrap(),
        expected
    );
}

#[test]
fn indent_respects_different_and_nonfinal_nested_lists() {
    for previous_tail in [
        json!({
            "type": "bulletList",
            "content": [{
                "type": "listItem",
                "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "different" }] }]
            }]
        }),
        json!({
            "type": "blockquote",
            "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "after-list" }] }]
        }),
    ] {
        let mut previous_content =
            vec![json!({ "type": "paragraph", "content": [{ "type": "text", "text": "one" }] })];
        if previous_tail["type"] == "blockquote" {
            previous_content.push(json!({
                "type": "orderedList",
                "content": [{
                    "type": "listItem",
                    "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "existing" }] }]
                }]
            }));
        }
        previous_content.push(previous_tail);
        let source = json!({
            "type": "doc",
            "content": [{
                "type": "orderedList",
                "attrs": { "start": 4 },
                "content": [
                    { "type": "listItem", "content": previous_content },
                    { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "two" }] }] }
                ]
            }]
        });
        let schema = tiptap_schema();
        let two = rendered_scalar_offset(&source, &schema, "two") + 1;
        let (actual, expected, _, _, _) = compile_and_execute(
            source,
            vec![TypedOperation::IndentListItem {
                at: point_for_test(two),
            }],
        );
        assert_eq!(actual, expected);
        let children = actual["content"][0]["content"][0]["content"]
            .as_array()
            .unwrap();
        let appended = children.last().unwrap();
        assert_eq!(appended["type"], "orderedList");
        assert_eq!(appended["attrs"]["start"], 4);
        assert_eq!(
            appended["content"][0]["content"][0]["content"][0]["text"],
            "two"
        );
    }
}

#[test]
fn indent_is_role_driven_for_task_items_and_materializes_empty_textblocks() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "taskList",
            "attrs": { "listMeta": { "owner": "team" } },
            "content": [
                {
                    "type": "taskItem",
                    "attrs": { "checked": false, "itemMeta": { "id": "one" } },
                    "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "one" }] }]
                },
                {
                    "type": "taskItem",
                    "attrs": { "checked": true, "itemMeta": { "id": "empty" } },
                    "content": [{ "type": "paragraph" }]
                }
            ]
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
        vec![TypedOperation::IndentListItem {
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
    let nested = &actual["content"][0]["content"][0]["content"][1];
    assert_eq!(nested["type"], "taskList");
    assert_eq!(nested["attrs"]["listMeta"]["owner"], "team");
    assert_eq!(nested["content"][0]["type"], "taskItem");
    assert_eq!(nested["content"][0]["attrs"]["checked"], true);
    assert_eq!(nested["content"][0]["attrs"]["itemMeta"]["id"], "empty");
    assert_eq!(nested["content"][0]["content"][0]["type"], "paragraph");
}

#[test]
fn indent_folds_prior_and_follow_up_edits_into_the_moved_prepared_item() {
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
    for operations in [
        vec![
            TypedOperation::InsertText {
                at: point_for_test(two),
                text: "X".into(),
                marks: vec![],
            },
            TypedOperation::IndentListItem {
                at: point_for_test(two),
            },
        ],
        vec![
            TypedOperation::IndentListItem {
                at: point_for_test(two),
            },
            TypedOperation::InsertText {
                at: point_for_test(two),
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
            actual["content"][0]["content"][0]["content"][1]["content"][0]["content"][0]["content"]
                [0]["text"],
            "tXwo"
        );
    }
}

#[test]
fn wrap_then_indent_rewrites_the_single_owned_prepared_batch() {
    let source = json!({
        "type": "doc",
        "content": [
            { "type": "paragraph", "content": [{ "type": "text", "text": "one" }] },
            { "type": "paragraph", "content": [{ "type": "text", "text": "two" }] }
        ]
    });
    let schema = tiptap_schema();
    let two = rendered_scalar_offset(&source, &schema, "two");
    let (doc, schema, limits, compiled) = compile_operations_with_schema(
        &source,
        vec![
            TypedOperation::WrapInList {
                range: range_for_test(0, two + 3),
                list_type: "bulletList".into(),
                item_type: "listItem".into(),
                attrs: HashMap::new(),
                item_attrs: HashMap::new(),
            },
            TypedOperation::IndentListItem {
                at: point_for_test(two + 1),
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
    assert_eq!(
        YrsDocumentCodec::new(&schema, &limits)
            .read_json(&fragment, &txn)
            .unwrap(),
        expected
    );
}

#[test]
fn repeated_indent_appends_into_the_prepared_existing_nested_list() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "bulletList",
            "content": [
                { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "one" }] }] },
                { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "two" }] }] },
                { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "three" }] }] }
            ]
        }]
    });
    let schema = tiptap_schema();
    let two = rendered_scalar_offset(&source, &schema, "two") + 1;
    let three = rendered_scalar_offset(&source, &schema, "three") + 1;
    let (actual, expected, _, _, _) = compile_and_execute(
        source,
        vec![
            TypedOperation::IndentListItem {
                at: point_for_test(two),
            },
            TypedOperation::IndentListItem {
                at: point_for_test(three),
            },
        ],
    );
    assert_eq!(actual, expected);
    let nested_items = actual["content"][0]["content"][0]["content"][1]["content"]
        .as_array()
        .unwrap();
    assert_eq!(nested_items.len(), 2);
    assert_eq!(nested_items[0]["content"][0]["content"][0]["text"], "two");
    assert_eq!(nested_items[1]["content"][0]["content"][0]["text"], "three");
}

#[test]
fn indent_then_update_attrs_targets_the_moved_prepared_task_item() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "taskList",
            "content": [
                {
                    "type": "taskItem",
                    "attrs": { "checked": false, "itemMeta": { "id": "one" } },
                    "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "one" }] }]
                },
                {
                    "type": "taskItem",
                    "attrs": { "checked": true, "itemMeta": { "id": "two" } },
                    "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "two" }] }]
                }
            ]
        }]
    });
    let schema = attribute_schema();
    let two = rendered_scalar_offset(&source, &schema, "two") + 1;
    let (doc, schema, limits, compiled) = compile_operations_with_schema(
        &source,
        vec![
            TypedOperation::IndentListItem {
                at: point_for_test(two),
            },
            TypedOperation::UpdateNodeAttrs {
                at: point_for_test(two),
                attrs: HashMap::from([
                    ("checked".into(), Value::Bool(false)),
                    ("itemMeta".into(), json!({ "id": "updated" })),
                ]),
            },
        ],
        schema,
    );
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
    let moved = &actual["content"][0]["content"][0]["content"][1]["content"][0];
    assert_eq!(moved["attrs"]["checked"], false);
    assert_eq!(moved["attrs"]["itemMeta"]["id"], "updated");
}

#[test]
fn indent_preflight_growth_undo_and_replica_bounds_are_exactly_enforced() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "bulletList",
            "content": [
                { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "one" }] }] },
                { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "two" }] }] },
                { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "three" }] }] }
            ]
        }]
    });
    let schema = tiptap_schema();
    let two = rendered_scalar_offset(&source, &schema, "two") + 1;
    let operations = || {
        vec![TypedOperation::IndentListItem {
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
