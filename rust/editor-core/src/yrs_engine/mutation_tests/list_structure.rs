use super::*;

#[test]
fn wrap_in_list_replaces_one_complete_root_block_directly() {
    let source = json!({
        "type": "doc",
        "content": [
            { "type": "paragraph", "content": [{ "type": "text", "text": "one" }] },
            { "type": "paragraph", "content": [{ "type": "text", "text": "tail" }] }
        ]
    });
    let (actual, expected, _, _, _) = compile_and_execute(
        source,
        vec![TypedOperation::WrapInList {
            range: range_for_test(0, 3),
            list_type: "bulletList".into(),
            item_type: "listItem".into(),
            attrs: HashMap::new(),
            item_attrs: HashMap::new(),
        }],
    );
    assert_eq!(actual, expected);
    assert_eq!(actual["content"][0]["type"], "bulletList");
    assert_eq!(
        actual["content"][0]["content"][0]["content"][0]["content"][0]["text"],
        "one"
    );
    assert_eq!(actual["content"][1]["content"][0]["text"], "tail");
}

#[test]
fn direct_root_wrap_oracle_metrics_match_the_public_lowering_matrix() {
    let cases = [
        json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{ "type": "text", "text": "plain" }]
            }]
        }),
        json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{ "type": "text", "text": "Á🙂漢字" }]
            }]
        }),
        json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{
                    "type": "text",
                    "text": "marked",
                    "marks": [{ "type": "bold" }, { "type": "italic" }]
                }]
            }]
        }),
        json!({
            "type": "doc",
            "content": [
                { "type": "paragraph", "content": [{ "type": "text", "text": "one" }] },
                { "type": "paragraph", "content": [{ "type": "text", "text": "two" }] },
                { "type": "paragraph", "content": [{ "type": "text", "text": "three" }] }
            ]
        }),
        json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [
                    { "type": "text", "text": "before" },
                    { "type": "hardBreak" },
                    { "type": "text", "text": "after" }
                ]
            }]
        }),
    ];

    for (case_index, source) in cases.into_iter().enumerate() {
        let schema = tiptap_schema();
        let source_document =
            from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
        let to = u32::try_from(
            crate::render::rendered_text(&source_document, &schema)
                .chars()
                .count(),
        )
        .unwrap();
        let (doc, schema, limits, compiled) = compile_operations_with_schema(
            &source,
            vec![TypedOperation::WrapInList {
                range: range_for_test(0, to),
                list_type: "bulletList".into(),
                item_type: "listItem".into(),
                attrs: HashMap::new(),
                item_attrs: HashMap::new(),
            }],
            schema,
        );
        let list_node = compiled
            .preview
            .root()
            .content()
            .and_then(|content| content.child(0))
            .unwrap();
        let metrics = direct_root_wrap_metrics(122, 0, list_node, &schema, &limits).unwrap();
        assert_eq!(
            metrics.insertion_units, compiled.authored_clock_units,
            "case {case_index} insertion units"
        );
        let replaced_children = match &compiled.mutation_plan.actions[0] {
            YrsMutationAction::DeleteXmlChildren { child_count, .. } => *child_count,
            action => panic!("case {case_index} unexpected first action: {action:?}"),
        };
        let txn = doc.transact();
        let envelope = crdt_envelope(122, &txn, usize::MAX).unwrap();
        assert_eq!(
            direct_xml_replacement_growth(
                122,
                0,
                replaced_children,
                metrics.growth_bytes,
                metrics.insertion_units,
                &envelope,
            )
            .unwrap(),
            compiled.encoded_growth_bound,
            "case {case_index} encoded growth"
        );
    }
}

#[test]
fn direct_root_wrap_growth_oracle_preserves_public_overflow_error_contract() {
    let error = direct_xml_replacement_growth(
        12_203,
        7,
        1,
        usize::MAX,
        1,
        &super::mutation::CrdtEnvelope::default(),
    )
    .unwrap_err();

    assert_eq!(error.code, "OPERATION_LIMIT_EXCEEDED");
    assert_eq!(error.operation_index, Some(7));
    assert_eq!(
        error.details,
        Some(json!({ "field": "estimatedUpdateV1Growth" }))
    );
    assert_eq!(error.limit, Some(u64::MAX));
    assert_eq!(error.actual, Some(u64::MAX));
}

#[test]
fn wrap_in_list_canonicalizes_partial_duplicate_selection_with_typed_attrs() {
    let source = json!({
        "type": "doc",
        "content": [
            { "type": "paragraph", "content": [{ "type": "text", "text": "left" }] },
            { "type": "paragraph", "content": [{ "type": "text", "text": "same" }] },
            { "type": "paragraph", "content": [{ "type": "text", "text": "same" }] },
            { "type": "paragraph", "content": [{ "type": "text", "text": "tail" }] }
        ]
    });
    let schema = attribute_schema();
    let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let rendered = crate::render::rendered_text(&document, &schema);
    let occurrences = rendered
        .match_indices("same")
        .map(|(byte, _)| byte)
        .collect::<Vec<_>>();
    let from = u32::try_from(rendered[..occurrences[0]].chars().count() + 1).unwrap();
    let to = u32::try_from(rendered[..occurrences[1]].chars().count() + 2).unwrap();
    let attrs = HashMap::from([("listMeta".into(), json!({ "nested": [1, { "ok": true }] }))]);
    let item_attrs = HashMap::from([
        ("checked".into(), Value::Bool(true)),
        ("itemMeta".into(), json!({ "ids": [1, 2, 3] })),
    ]);
    let operations = || {
        vec![TypedOperation::WrapInList {
            range: range_for_test(from, to),
            list_type: "taskList".into(),
            item_type: "taskItem".into(),
            attrs: attrs.clone(),
            item_attrs: item_attrs.clone(),
        }]
    };
    let (doc, schema, limits, mut compiled) =
        compile_operations_with_schema(&source, operations(), schema);
    assert!(matches!(
        compiled.mutation_plan.actions.as_slice(),
        [
            YrsMutationAction::DeleteXmlChildren {
                child_index: 1,
                child_count: 2,
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
        left_block_id,
        left_text_id,
        moved_block_ids,
        tail_block_id,
        tail_text_id,
        tail_sticky,
        before_full_len,
        before_update,
    ) = {
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let children = fragment.children(&txn).collect::<Vec<_>>();
        let left_text = paragraph_text(&fragment, &txn, 0);
        let tail_text = paragraph_text(&fragment, &txn, 3);
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
            vec![children[1].id(), children[2].id()],
            children[3].id(),
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
    assert_eq!(actual["content"].as_array().unwrap().len(), 3);
    let list = &actual["content"][1];
    assert_eq!(list["type"], "taskList");
    assert_eq!(list["attrs"]["listMeta"], attrs["listMeta"]);
    assert_eq!(list["content"].as_array().unwrap().len(), 2);
    for item in list["content"].as_array().unwrap() {
        assert_eq!(item["attrs"]["checked"], true);
        assert_eq!(item["attrs"]["itemMeta"], item_attrs["itemMeta"]);
    }
    let root_children = fragment.children(&txn).collect::<Vec<_>>();
    assert_eq!(root_children[0].id(), left_block_id);
    assert_eq!(root_children[2].id(), tail_block_id);
    assert!(moved_block_ids
        .iter()
        .all(|id| root_children.iter().all(|child| child.id() != *id)));
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
    assert_eq!(
        compile_operations_with_undo_limit(&source, operations(), attribute_schema(), undo_exact)
            .unwrap()
            .undo_units_bound,
        undo_exact
    );
    let undo_error = compile_operations_with_undo_limit(
        &source,
        operations(),
        attribute_schema(),
        undo_exact - 1,
    )
    .unwrap_err();
    assert_eq!(undo_error.code, "OPERATION_LIMIT_EXCEEDED");
    assert_eq!(undo_error.actual, Some(undo_exact));
}

#[test]
fn wrap_in_list_handles_void_and_empty_blocks_without_existing_text_targets() {
    for source in [
        json!({
            "type": "doc",
            "content": [
                { "type": "paragraph", "content": [{ "type": "hardBreak" }] },
                { "type": "paragraph", "content": [{ "type": "text", "text": "tail" }] }
            ]
        }),
        json!({
            "type": "doc",
            "content": [
                { "type": "paragraph" },
                { "type": "paragraph", "content": [{ "type": "text", "text": "tail" }] }
            ]
        }),
    ] {
        let (actual, expected, _, _, _) = compile_and_execute(
            source,
            vec![TypedOperation::WrapInList {
                range: range_for_test(0, 1),
                list_type: "bulletList".into(),
                item_type: "listItem".into(),
                attrs: HashMap::new(),
                item_attrs: HashMap::new(),
            }],
        );
        assert_eq!(actual, expected);
        assert_eq!(actual["content"][0]["type"], "bulletList");
        assert_eq!(actual["content"][1]["content"][0]["text"], "tail");
    }
}

#[test]
fn wrap_empty_textblock_accepts_follow_up_text_in_prepared_blueprint() {
    let source = json!({
        "type": "doc",
        "content": [
            { "type": "paragraph" },
            { "type": "paragraph", "content": [{ "type": "text", "text": "tail" }] }
        ]
    });
    let (actual, expected, _, _, _) = compile_and_execute(
        source,
        vec![
            TypedOperation::WrapInList {
                range: range_for_test(0, 1),
                list_type: "bulletList".into(),
                item_type: "listItem".into(),
                attrs: HashMap::new(),
                item_attrs: HashMap::new(),
            },
            TypedOperation::InsertText {
                at: point_for_test(1),
                text: "filled".into(),
                marks: vec![],
            },
        ],
    );
    assert_eq!(actual, expected);
    assert_eq!(
        actual["content"][0]["content"][0]["content"][0]["content"][0]["text"],
        "filled"
    );
}

#[test]
fn wrap_then_unwrap_same_transaction_rewrites_owned_blueprint() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "one" }]
        }]
    });
    let (doc, schema, limits, compiled) = compile_operations_with_schema(
        &source,
        vec![
            TypedOperation::WrapInList {
                range: range_for_test(0, 3),
                list_type: "bulletList".into(),
                item_type: "listItem".into(),
                attrs: HashMap::new(),
                item_attrs: HashMap::new(),
            },
            TypedOperation::UnwrapFromList {
                at: point_for_test(1),
            },
        ],
        tiptap_schema(),
    );
    assert!(
        compiled.mutation_plan.actions.is_empty(),
        "wrapping and immediately unwrapping the same unchanged block must cancel"
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
    assert_eq!(actual, source);
    assert_eq!(actual, expected);
}

#[test]
fn wrap_edit_then_unwrap_rewrites_the_owned_blueprint_once() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "one" }]
        }]
    });
    let (doc, schema, limits, compiled) = compile_operations_with_schema(
        &source,
        vec![
            TypedOperation::WrapInList {
                range: range_for_test(0, 3),
                list_type: "bulletList".into(),
                item_type: "listItem".into(),
                attrs: HashMap::new(),
                item_attrs: HashMap::new(),
            },
            TypedOperation::InsertText {
                at: point_for_test(1),
                text: "X".into(),
                marks: vec![],
            },
            TypedOperation::UnwrapFromList {
                at: point_for_test(1),
            },
        ],
        tiptap_schema(),
    );
    assert_eq!(
        compiled
            .mutation_plan
            .actions
            .iter()
            .filter(|action| matches!(action, YrsMutationAction::DeleteXmlChildren { .. }))
            .count(),
        1
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
    assert_eq!(actual["content"][0]["content"][0]["text"], "oXne");
}

#[test]
fn wrap_in_list_folds_prior_edits_and_accepts_follow_up_prepared_edits() {
    let source = json!({
        "type": "doc",
        "content": [
            { "type": "paragraph", "content": [{ "type": "text", "text": "ab" }] },
            { "type": "paragraph", "content": [{ "type": "text", "text": "tail" }] }
        ]
    });
    let (doc, schema, limits, compiled) = compile_operations_with_schema(
        &source,
        vec![
            TypedOperation::InsertText {
                at: point_for_test(1),
                text: "X".into(),
                marks: vec![],
            },
            TypedOperation::WrapInList {
                range: range_for_test(0, 2),
                list_type: "bulletList".into(),
                item_type: "listItem".into(),
                attrs: HashMap::new(),
                item_attrs: HashMap::new(),
            },
            TypedOperation::InsertText {
                at: point_for_test(1),
                text: "Y".into(),
                marks: vec![],
            },
        ],
        tiptap_schema(),
    );
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
    let text = actual["content"][0]["content"][0]["content"][0]["content"][0]["text"]
        .as_str()
        .unwrap();
    assert!(text.contains('X') && text.contains('Y'));
}

#[test]
fn structural_insert_then_wrap_folds_blueprint_and_accepts_follow_up_text() {
    let source = json!({
        "type": "doc",
        "content": [
            { "type": "paragraph", "content": [{ "type": "text", "text": "ab" }] },
            { "type": "paragraph", "content": [{ "type": "text", "text": "tail" }] }
        ]
    });
    let (doc, schema, limits, compiled) = compile_operations_with_schema(
        &source,
        vec![
            TypedOperation::InsertNode {
                at: point_for_test(1),
                node: Node::void("hardBreak".into(), HashMap::new()),
            },
            TypedOperation::WrapInList {
                range: range_for_test(0, 2),
                list_type: "bulletList".into(),
                item_type: "listItem".into(),
                attrs: HashMap::new(),
                item_attrs: HashMap::new(),
            },
            TypedOperation::InsertText {
                at: point_for_test(1),
                text: "Z".into(),
                marks: vec![],
            },
        ],
        tiptap_schema(),
    );
    assert_eq!(compiled.mutation_plan.actions.len(), 2);
    assert!(matches!(
        compiled.mutation_plan.actions[0],
        YrsMutationAction::DeleteXmlChildren { .. }
    ));
    assert!(matches!(
        compiled.mutation_plan.actions[1],
        YrsMutationAction::InsertXmlChildren { .. }
    ));
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
    let paragraph_content = actual["content"][0]["content"][0]["content"][0]["content"]
        .as_array()
        .unwrap();
    assert!(paragraph_content
        .iter()
        .any(|node| node["type"] == "hardBreak"));
    assert!(paragraph_content
        .iter()
        .any(|node| node["text"].as_str().is_some_and(|text| text.contains('Z'))));
}

#[test]
fn unwrap_only_list_item_replaces_the_list_with_its_blocks_directly() {
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
    let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let rendered = crate::render::rendered_text(&document, &schema);
    let one = u32::try_from(rendered[..rendered.find("one").unwrap()].chars().count()).unwrap();
    let (actual, expected, _, _, _) = compile_and_execute(
        source,
        vec![TypedOperation::UnwrapFromList {
            at: point_for_test(one + 1),
        }],
    );
    assert_eq!(actual, expected);
    assert_eq!(actual["content"][0]["type"], "paragraph");
    assert_eq!(actual["content"][0]["content"][0]["text"], "one");
}

#[test]
fn unwrap_only_then_insert_text_folds_into_prepared_text() {
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
    let (doc, schema, limits, compiled) = compile_operations_with_schema(
        &source,
        vec![
            TypedOperation::UnwrapFromList {
                at: point_for_test(3),
            },
            TypedOperation::InsertText {
                at: point_for_test(3),
                text: "X".into(),
                marks: vec![],
            },
        ],
        tiptap_schema(),
    );
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
    assert_eq!(actual["content"][0]["content"][0]["text"], "oXne");
}

#[test]
fn edit_then_unwrap_tombstones_deleted_text_actions() {
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
    let (doc, schema, limits, compiled) = compile_operations_with_schema(
        &source,
        vec![
            TypedOperation::InsertText {
                at: point_for_test(one + 1),
                text: "X".into(),
                marks: vec![],
            },
            TypedOperation::UnwrapFromList {
                at: point_for_test(one + 1),
            },
        ],
        schema,
    );
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
    assert_eq!(actual["content"][0]["content"][0]["text"], "oXne");
}

#[test]
fn insert_node_then_unwrap_owns_one_canonical_prepared_batch() {
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
    let (doc, schema, limits, compiled) = compile_operations_with_schema(
        &source,
        vec![
            TypedOperation::InsertNode {
                at: point_for_test(one + 1),
                node: Node::void("hardBreak".into(), HashMap::new()),
            },
            TypedOperation::UnwrapFromList {
                at: point_for_test(one + 1),
            },
        ],
        schema,
    );
    assert!(matches!(
        compiled.mutation_plan.actions.as_slice(),
        [
            YrsMutationAction::DeleteXmlChildren {
                child_index: 0,
                child_count: 1,
                ..
            },
            YrsMutationAction::InsertXmlChildren { nodes, .. }
        ] if nodes.len() == 1
    ));
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
fn unwrap_only_then_update_extracted_block_attrs() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "taskList",
            "content": [{
                "type": "taskItem",
                "content": [
                    {
                        "type": "paragraph",
                        "content": [{ "type": "text", "text": "lead" }]
                    },
                    {
                        "type": "h2",
                        "attrs": { "id": "before" },
                        "content": [{ "type": "text", "text": "heading" }]
                    }
                ]
            }]
        }]
    });
    let schema = attribute_schema();
    let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let rendered = crate::render::rendered_text(&document, &schema);
    let lead = u32::try_from(rendered[..rendered.find("lead").unwrap()].chars().count()).unwrap();
    let heading = u32::try_from(
        rendered[..rendered.find("heading").unwrap()]
            .chars()
            .count(),
    )
    .unwrap();
    let (doc, schema, limits, compiled) = compile_operations_with_schema(
        &source,
        vec![
            TypedOperation::UnwrapFromList {
                at: point_for_test(lead + 1),
            },
            TypedOperation::UpdateNodeAttrs {
                at: point_for_test(heading),
                attrs: HashMap::from([("id".into(), Value::String("after".into()))]),
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
    assert_eq!(actual["content"][1]["attrs"]["id"], "after");
}

#[test]
fn attrs_then_unwrap_tombstones_deleted_element_attrs() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "taskList",
            "content": [{
                "type": "taskItem",
                "content": [
                    {
                        "type": "paragraph",
                        "content": [{ "type": "text", "text": "lead" }]
                    },
                    {
                        "type": "h2",
                        "attrs": { "id": "before" },
                        "content": [{ "type": "text", "text": "heading" }]
                    }
                ]
            }]
        }]
    });
    let schema = attribute_schema();
    let lead = rendered_scalar_offset(&source, &schema, "lead");
    let heading = rendered_scalar_offset(&source, &schema, "heading");
    let (doc, schema, limits, compiled) = compile_operations_with_schema(
        &source,
        vec![
            TypedOperation::UpdateNodeAttrs {
                at: point_for_test(heading),
                attrs: HashMap::from([("id".into(), Value::String("after".into()))]),
            },
            TypedOperation::UnwrapFromList {
                at: point_for_test(lead + 1),
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
    assert_eq!(actual["content"][1]["attrs"]["id"], "after");
}

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

#[test]
fn indent_then_outdent_cancels_the_unchanged_prepared_move() {
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
        vec![
            TypedOperation::IndentListItem {
                at: point_for_test(two),
            },
            TypedOperation::OutdentListItem {
                at: point_for_test(two),
            },
        ],
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
fn indent_edit_then_outdent_reinserts_only_the_changed_prepared_item() {
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
    let (doc, schema, limits, compiled) = compile_operations_with_schema(
        &source,
        vec![
            TypedOperation::IndentListItem {
                at: point_for_test(two),
            },
            TypedOperation::InsertText {
                at: point_for_test(two),
                text: "X".into(),
                marks: vec![],
            },
            TypedOperation::OutdentListItem {
                at: point_for_test(two),
            },
        ],
        schema,
    );
    assert!(matches!(
        compiled.mutation_plan.actions.as_slice(),
        [
            YrsMutationAction::DeleteXmlChildren { .. },
            YrsMutationAction::InsertXmlChildren { .. }
        ]
    ));
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
        "tXwo"
    );
}

#[test]
fn outdent_from_a_multi_item_prepared_nested_list_retains_its_prefix_blueprint() {
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
            TypedOperation::OutdentListItem {
                at: point_for_test(three),
            },
        ],
    );
    assert_eq!(actual, expected);
    let outer = actual["content"][0]["content"].as_array().unwrap();
    assert_eq!(outer.len(), 2);
    assert_eq!(
        outer[0]["content"][1]["content"][0]["content"][0]["content"][0]["text"],
        "two"
    );
    assert_eq!(outer[1]["content"][0]["content"][0]["text"], "three");
}

#[test]
fn outdent_rewrites_a_fully_prepared_outer_list_and_parent_batch() {
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
            TypedOperation::OutdentListItem {
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
fn outdent_first_nested_item_under_a_prepared_parent_rewrites_the_owned_batch() {
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
            TypedOperation::OutdentListItem {
                at: point_for_test(inner + 1),
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
    let items = actual["content"][0]["content"].as_array().unwrap();
    assert_eq!(items.len(), 3);
    assert_eq!(items[0]["content"][0]["content"][0]["text"], "ou");
    assert_eq!(items[1]["content"][0]["content"][0]["text"], "ter");
    assert_eq!(items[2]["content"][0]["content"][0]["text"], "inner");
}

#[test]
fn outdent_first_of_multiple_nested_items_under_a_prepared_parent_keeps_the_tail() {
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
                            { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "inner" }] }] },
                            { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "tail" }] }] }
                        ]
                    }
                ]
            }]
        }]
    });
    let schema = tiptap_schema();
    let outer = rendered_scalar_offset(&source, &schema, "outer");
    let inner = rendered_scalar_offset(&source, &schema, "inner");
    let (actual, expected, _, _, _) = compile_and_execute(
        source,
        vec![
            TypedOperation::SplitBlock {
                at: point_for_test(outer + 2),
                node_type: "paragraph".into(),
                attrs: HashMap::new(),
            },
            TypedOperation::OutdentListItem {
                at: point_for_test(inner + 1),
            },
        ],
    );
    assert_eq!(actual, expected);
    let items = actual["content"][0]["content"].as_array().unwrap();
    assert_eq!(items.len(), 3);
    assert_eq!(items[2]["content"][0]["content"][0]["text"], "inner");
    assert_eq!(
        items[2]["content"][1]["content"][0]["content"][0]["content"][0]["text"],
        "tail"
    );
}

#[test]
fn unwrap_first_of_multiple_nested_items_under_a_prepared_parent_keeps_the_tail() {
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
                            { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "inner" }] }] },
                            { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "tail" }] }] }
                        ]
                    }
                ]
            }]
        }]
    });
    let schema = tiptap_schema();
    let outer = rendered_scalar_offset(&source, &schema, "outer");
    let inner = rendered_scalar_offset(&source, &schema, "inner");
    let (actual, expected, _, _, _) = compile_and_execute(
        source,
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
    );
    assert_eq!(actual, expected);
    let outer_items = actual["content"][0]["content"].as_array().unwrap();
    assert_eq!(outer_items.len(), 2);
    let split_outer = outer_items[1]["content"].as_array().unwrap();
    assert_eq!(split_outer[0]["content"][0]["text"], "ter");
    assert_eq!(split_outer[1]["content"][0]["text"], "inner");
    assert_eq!(
        split_outer[2]["content"][0]["content"][0]["content"][0]["text"],
        "tail"
    );
}

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
