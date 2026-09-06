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
