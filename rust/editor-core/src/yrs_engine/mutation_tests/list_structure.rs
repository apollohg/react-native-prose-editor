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

include!("list_structure/unwrap_single.rs");

include!("list_structure/indent.rs");

include!("list_structure/outdent.rs");

include!("list_structure/prepared_lists.rs");

include!("list_structure/unwrap_multiple.rs");
