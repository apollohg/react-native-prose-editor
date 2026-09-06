#[test]
fn structural_delete_removes_an_inline_atom_directly() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "hardBreak" }]
        }]
    });

    let (actual, expected, _, _, _) = compile_and_execute(
        source,
        vec![TypedOperation::DeleteRange {
            range: range_for_test(0, 1),
        }],
    );

    assert_eq!(actual, expected);
    assert_eq!(
        actual,
        json!({ "type": "doc", "content": [{ "type": "paragraph" }] })
    );
}

#[test]
fn duplicate_equal_atom_deletes_preserve_the_identity_selected_by_operation_intent() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "hardBreak" }, { "type": "hardBreak" }]
        }]
    });

    let remaining_after_delete = |from, to| {
        let (doc, schema, limits, editing_limits, document) = diagnostic_doc(&source);
        let before_ids = {
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
                    request_id: 108,
                    base_document_revision: 0,
                    origin: TransactionOrigin::LocalInput,
                    operations: vec![TypedOperation::DeleteRange {
                        range: range_for_test(from, to),
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
        let remaining_id = {
            let txn = doc.transact();
            let fragment = txn.get_xml_fragment("prosemirror").unwrap();
            let XmlOut::Element(paragraph) = fragment.get(&txn, 0).unwrap() else {
                panic!("expected paragraph")
            };
            paragraph.get(&txn, 0).unwrap().id()
        };
        (before_ids, remaining_id)
    };

    let (first_case_ids, after_delete_first) = remaining_after_delete(0, 1);
    assert_eq!(after_delete_first, first_case_ids[1]);
    let (second_case_ids, after_delete_second) = remaining_after_delete(1, 2);
    assert_eq!(after_delete_second, second_case_ids[0]);
}

#[test]
fn duplicate_equal_atom_inserts_preserve_existing_identities_before_and_after() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "hardBreak" }, { "type": "hardBreak" }]
        }]
    });

    let insert_and_ids = |at| {
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
                    request_id: 109,
                    base_document_revision: 0,
                    origin: TransactionOrigin::LocalInput,
                    operations: vec![TypedOperation::InsertNode {
                        at: point_for_test(at),
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

    let (before_insert, after_insert_before) = insert_and_ids(0);
    assert_eq!(&after_insert_before[1..], before_insert.as_slice());
    let (before_append, after_insert_after) = insert_and_ids(2);
    assert_eq!(&after_insert_after[..2], before_append.as_slice());
}

#[test]
fn structural_insert_inside_text_retains_left_storage_and_creates_atom_and_suffix() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "abcd" }]
        }]
    });
    let (doc, schema, limits, editing_limits, document) = diagnostic_doc(&source);
    let left_id = {
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        <XmlTextRef as AsRef<Branch>>::as_ref(&paragraph_text(&fragment, &txn, 0)).id()
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
                request_id: 112,
                base_document_revision: 0,
                origin: TransactionOrigin::LocalInput,
                operations: vec![TypedOperation::InsertNode {
                    at: point_for_test(2),
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
    {
        let mut txn = doc.transact_mut();
        execute_mutation_plan(compiled.mutation_plan, &mut txn);
    }
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let XmlOut::Element(paragraph) = fragment.get(&txn, 0).unwrap() else {
        panic!("expected paragraph")
    };
    let children = paragraph.children(&txn).collect::<Vec<_>>();
    assert_eq!(children.len(), 3);
    assert_eq!(children[0].id(), left_id);
    assert!(matches!(children[0], XmlOut::Text(_)));
    assert!(matches!(children[1], XmlOut::Element(_)));
    assert!(matches!(children[2], XmlOut::Text(_)));
    assert_eq!(
        YrsDocumentCodec::new(&schema, &limits)
            .read_json(&fragment, &txn)
            .unwrap(),
        to_prosemirror_json(&compiled.preview, &schema)
    );
}

#[test]
fn structural_insert_preserves_unaffected_identity_and_supports_replica_undo_redo() {
    let source = json!({
        "type": "doc",
        "content": [
            { "type": "paragraph", "content": [{ "type": "text", "text": "AB" }] },
            { "type": "paragraph", "content": [{ "type": "text", "text": "tail" }] }
        ]
    });
    let (doc, schema, limits, compiled) = compile_operations_with_schema(
        &source,
        vec![TypedOperation::InsertNode {
            at: point_for_test(1),
            node: Node::void("hardBreak".into(), HashMap::new()),
        }],
        tiptap_schema(),
    );
    let expected = to_prosemirror_json(&compiled.preview, &schema);
    let (before_update, tail_id, tail_text_id, sticky) = {
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let tail = fragment.get(&txn, 1).unwrap();
        let tail_text = paragraph_text(&fragment, &txn, 1);
        (
            txn.encode_state_as_update_v1(&StateVector::default()),
            tail.id(),
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
    let fragment = doc.transact().get_xml_fragment("prosemirror").unwrap();
    let mut undo = UndoManager::<()>::new();
    undo.expand_scope(&doc, &fragment);
    undo.include_origin(TransactionOrigin::LocalCommand.as_yrs_origin());
    let update = {
        let mut txn = doc.transact_mut_with(TransactionOrigin::LocalCommand.as_yrs_origin());
        execute_mutation_plan(compiled.mutation_plan, &mut txn);
        txn.commit();
        txn.encode_update_v1()
    };
    {
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        assert_eq!(fragment.get(&txn, 1).unwrap().id(), tail_id);
        assert_eq!(
            <XmlTextRef as AsRef<Branch>>::as_ref(&paragraph_text(&fragment, &txn, 1)).id(),
            tail_text_id
        );
        assert_eq!(
            super::sticky_index_to_doc_pos(&txn, &fragment, &sticky, &schema),
            Some(8)
        );
        assert_eq!(
            YrsDocumentCodec::new(&schema, &limits)
                .read_json(&fragment, &txn)
                .unwrap(),
            expected
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
    {
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        assert_eq!(
            YrsDocumentCodec::new(&schema, &limits)
                .read_json(&fragment, &txn)
                .unwrap(),
            source
        );
    }
    assert!(undo.redo_blocking());
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
fn insert_node_offset_node_class_and_recursive_attribute_matrix() {
    let unicode_source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "A😀B" }]
        }]
    });
    for (kind, offset) in [(EditorOffsetKind::Scalar, 2), (EditorOffsetKind::Utf16, 3)] {
        for affinity in [Affinity::Before, Affinity::After] {
            let (actual, expected, _, _, _) = compile_and_execute(
                unicode_source.clone(),
                vec![TypedOperation::InsertNode {
                    at: RevisionedPosition {
                        offset,
                        kind,
                        affinity,
                    },
                    node: Node::void("hardBreak".into(), HashMap::new()),
                }],
            );
            assert_eq!(actual, expected);
            assert_eq!(actual["content"][0]["content"][1]["type"], "hardBreak");
        }
    }

    let source = json!({
        "type": "doc",
        "content": [
            { "type": "paragraph", "content": [{ "type": "text", "text": "A😀" }] },
            { "type": "paragraph", "content": [{ "type": "text", "text": "B" }] }
        ]
    });
    let schema = attribute_schema();
    let scalar_at = rendered_scalar_offset(&source, &schema, "B") - 1;
    let image_attrs = HashMap::from([
        ("src".into(), Value::String("asset://direct".into())),
        ("alt".into(), Value::String("Direct image".into())),
    ]);
    let rich_quote = Node::element(
        "blockquote".into(),
        HashMap::new(),
        Fragment::from(vec![
            Node::element(
                "paragraph".into(),
                HashMap::new(),
                Fragment::from(vec![Node::text(
                    "quoted😀".into(),
                    vec![Mark::new("bold".into(), HashMap::new())],
                )]),
            ),
            Node::element(
                "taskList".into(),
                HashMap::from([("listMeta".into(), json!({ "owner": "team", "rank": 7 }))]),
                Fragment::from(vec![Node::element(
                    "taskItem".into(),
                    HashMap::from([
                        ("checked".into(), Value::Bool(true)),
                        (
                            "itemMeta".into(),
                            json!({ "id": "task-1", "flags": [1, false] }),
                        ),
                    ]),
                    Fragment::from(vec![
                        Node::element(
                            "paragraph".into(),
                            HashMap::new(),
                            Fragment::from(vec![Node::text("task".into(), vec![])]),
                        ),
                        Node::void(
                            "image".into(),
                            HashMap::from([
                                ("src".into(), Value::String("asset://nested".into())),
                                ("alt".into(), Value::String("Nested image".into())),
                            ]),
                        ),
                        Node::void(
                            "customBlock".into(),
                            HashMap::from([(
                                "meta".into(),
                                json!({ "nested": { "values": [1, "x", true] } }),
                            )]),
                        ),
                    ]),
                )]),
            ),
        ]),
    );

    for inserted in [Node::void("image".into(), image_attrs), rich_quote] {
        for (kind, offset) in [
            (EditorOffsetKind::Scalar, scalar_at),
            (EditorOffsetKind::Utf16, scalar_at + 1),
        ] {
            for affinity in [Affinity::Before, Affinity::After] {
                let (doc, schema, limits, compiled) = compile_operations_with_schema(
                    &source,
                    vec![TypedOperation::InsertNode {
                        at: RevisionedPosition {
                            offset,
                            kind,
                            affinity,
                        },
                        node: inserted.clone(),
                    }],
                    schema.clone(),
                );
                let expected = to_prosemirror_json(&compiled.preview, &schema);
                let before_update = doc
                    .transact()
                    .encode_state_as_update_v1(&StateVector::default());
                let update = {
                    let mut txn = doc.transact_mut();
                    execute_mutation_plan(compiled.mutation_plan, &mut txn);
                    txn.commit();
                    txn.encode_update_v1()
                };
                let txn = doc.transact();
                let fragment = txn.get_xml_fragment("prosemirror").unwrap();
                assert_eq!(
                    YrsDocumentCodec::new(&schema, &limits)
                        .read_json(&fragment, &txn)
                        .unwrap(),
                    expected
                );
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
            }
        }
    }
}

#[test]
fn structural_local_origin_undo_redo_and_bound_matrix() {
    let atom_source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "hardBreak" }]
        }]
    });
    let split_source = json!({
        "type": "doc",
        "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "ab" }] }]
    });
    let join_source = json!({
        "type": "doc",
        "content": [
            { "type": "paragraph", "content": [{ "type": "text", "text": "ab" }] },
            { "type": "paragraph", "content": [{ "type": "text", "text": "cd" }] }
        ]
    });
    let wrap_source = json!({
        "type": "doc",
        "content": [
            { "type": "paragraph", "content": [{ "type": "text", "text": "one" }] },
            { "type": "paragraph", "content": [{ "type": "text", "text": "tail" }] }
        ]
    });
    let indent_source = json!({
        "type": "doc",
        "content": [{
            "type": "bulletList",
            "content": [
                { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "one" }] }] },
                { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "two" }] }] }
            ]
        }]
    });
    let outdent_source = json!({
        "type": "doc",
        "content": [{
            "type": "bulletList",
            "content": [{
                "type": "listItem",
                "content": [
                    { "type": "paragraph", "content": [{ "type": "text", "text": "outer" }] },
                    { "type": "bulletList", "content": [{ "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "inner" }] }] }] }
                ]
            }]
        }]
    });
    let unwrap_source = json!({
        "type": "doc",
        "content": [{ "type": "bulletList", "content": [{ "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "one" }] }] }] }]
    });
    let schema = tiptap_schema();
    let cases = vec![
        (
            split_source.clone(),
            vec![TypedOperation::InsertNode {
                at: point_for_test(1),
                node: Node::void("hardBreak".into(), HashMap::new()),
            }],
        ),
        (
            atom_source.clone(),
            vec![TypedOperation::DeleteRange {
                range: range_for_test(0, 1),
            }],
        ),
        (
            atom_source,
            vec![TypedOperation::ReplaceRange {
                range: range_for_test(0, 1),
                content: Fragment::from(vec![Node::text("X".into(), vec![])]),
            }],
        ),
        (
            split_source,
            vec![TypedOperation::SplitBlock {
                at: point_for_test(1),
                node_type: "paragraph".into(),
                attrs: HashMap::new(),
            }],
        ),
        (
            join_source,
            vec![TypedOperation::JoinBlocks {
                at: point_for_test(2),
            }],
        ),
        (
            wrap_source,
            vec![TypedOperation::WrapInList {
                range: range_for_test(0, 3),
                list_type: "bulletList".into(),
                item_type: "listItem".into(),
                attrs: HashMap::new(),
                item_attrs: HashMap::new(),
            }],
        ),
        (
            indent_source.clone(),
            vec![TypedOperation::IndentListItem {
                at: point_for_test(rendered_scalar_offset(&indent_source, &schema, "two") + 1),
            }],
        ),
        (
            outdent_source.clone(),
            vec![TypedOperation::OutdentListItem {
                at: point_for_test(rendered_scalar_offset(&outdent_source, &schema, "inner") + 1),
            }],
        ),
        (
            unwrap_source.clone(),
            vec![TypedOperation::UnwrapFromList {
                at: point_for_test(rendered_scalar_offset(&unwrap_source, &schema, "one") + 1),
            }],
        ),
    ];

    for (case_index, (source, operations)) in cases.into_iter().enumerate() {
        let (doc, schema, limits, compiled) =
            compile_operations_with_schema(&source, operations, tiptap_schema());
        let expected = to_prosemirror_json(&compiled.preview, &schema);
        let undo_bound = compiled.undo_units_bound;
        let fragment = doc.transact().get_xml_fragment("prosemirror").unwrap();
        let mut undo = UndoManager::<()>::new();
        undo.expand_scope(&doc, &fragment);
        undo.include_origin(TransactionOrigin::LocalCommand.as_yrs_origin());
        {
            let mut txn = doc.transact_mut_with(TransactionOrigin::LocalCommand.as_yrs_origin());
            execute_mutation_plan(compiled.mutation_plan, &mut txn);
        }
        {
            let txn = doc.transact();
            let fragment = txn.get_xml_fragment("prosemirror").unwrap();
            assert_eq!(
                YrsDocumentCodec::new(&schema, &limits)
                    .read_json(&fragment, &txn)
                    .unwrap(),
                expected,
                "case {case_index} preview"
            );
        }
        let undo_item = undo
            .undo_stack()
            .last()
            .unwrap_or_else(|| panic!("case {case_index} was not captured by local origin"));
        let id_set_units = |set: &yrs::IdSet| {
            set.iter()
                .flat_map(|(_, ranges)| ranges.into_iter())
                .map(|range| u64::from(range.end - range.start))
                .sum::<u64>()
        };
        let actual_undo_units =
            id_set_units(undo_item.insertions()) + id_set_units(undo_item.deletions());
        assert!(actual_undo_units <= undo_bound, "case {case_index}");
        assert!(undo.undo_blocking(), "case {case_index} undo");
        {
            let txn = doc.transact();
            let fragment = txn.get_xml_fragment("prosemirror").unwrap();
            assert_eq!(
                YrsDocumentCodec::new(&schema, &limits)
                    .read_json(&fragment, &txn)
                    .unwrap(),
                source,
                "case {case_index} source"
            );
        }
        assert!(undo.redo_blocking(), "case {case_index} redo");
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        assert_eq!(
            YrsDocumentCodec::new(&schema, &limits)
                .read_json(&fragment, &txn)
                .unwrap(),
            expected,
            "case {case_index} redo preview"
        );
    }

    let source = json!({
        "type": "doc",
        "content": [{ "type": "image", "attrs": { "src": "old", "alt": "old alt" } }]
    });
    let (doc, schema, limits, compiled) = compile_operations_with_schema(
        &source,
        vec![TypedOperation::UpdateNodeAttrs {
            at: point_for_test(0),
            attrs: HashMap::from([
                ("src".into(), Value::String("new".into())),
                ("alt".into(), Value::String("new alt".into())),
            ]),
        }],
        attribute_schema(),
    );
    let expected = to_prosemirror_json(&compiled.preview, &schema);
    let undo_bound = compiled.undo_units_bound;
    let fragment = doc.transact().get_xml_fragment("prosemirror").unwrap();
    let mut undo = UndoManager::<()>::new();
    undo.expand_scope(&doc, &fragment);
    undo.include_origin(TransactionOrigin::LocalCommand.as_yrs_origin());
    {
        let mut txn = doc.transact_mut_with(TransactionOrigin::LocalCommand.as_yrs_origin());
        execute_mutation_plan(compiled.mutation_plan, &mut txn);
    }
    let item = undo.undo_stack().last().expect("attribute update captured");
    let id_set_units = |set: &yrs::IdSet| {
        set.iter()
            .flat_map(|(_, ranges)| ranges.into_iter())
            .map(|range| u64::from(range.end - range.start))
            .sum::<u64>()
    };
    assert!(id_set_units(item.insertions()) + id_set_units(item.deletions()) <= undo_bound);
    assert!(undo.undo_blocking());
    {
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        assert_eq!(
            YrsDocumentCodec::new(&schema, &limits)
                .read_json(&fragment, &txn)
                .unwrap(),
            source
        );
    }
    assert!(undo.redo_blocking());
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
fn insert_split_join_and_attribute_undo_limits_are_exact() {
    fn assert_exact(
        source: &Value,
        operations: Vec<TypedOperation>,
        schema: crate::schema::Schema,
    ) {
        let exact = compile_operations_with_undo_limit(
            source,
            operations.clone(),
            schema.clone(),
            u64::MAX,
        )
        .unwrap()
        .undo_units_bound;
        assert!(exact > 0);
        assert_eq!(
            compile_operations_with_undo_limit(source, operations.clone(), schema.clone(), exact,)
                .unwrap()
                .undo_units_bound,
            exact
        );
        let error =
            compile_operations_with_undo_limit(source, operations, schema, exact - 1).unwrap_err();
        assert_eq!(error.code, "OPERATION_LIMIT_EXCEEDED");
        assert_eq!(error.limit, Some(exact - 1));
        assert_eq!(error.actual, Some(exact));
    }

    let paragraph = json!({
        "type": "doc",
        "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "A😀B" }] }]
    });
    assert_exact(
        &paragraph,
        vec![TypedOperation::InsertNode {
            at: point_for_test(2),
            node: Node::void("hardBreak".into(), HashMap::new()),
        }],
        tiptap_schema(),
    );
    assert_exact(
        &paragraph,
        vec![TypedOperation::SplitBlock {
            at: point_for_test(2),
            node_type: "paragraph".into(),
            attrs: HashMap::new(),
        }],
        tiptap_schema(),
    );
    let join = json!({
        "type": "doc",
        "content": [
            { "type": "paragraph", "content": [{ "type": "text", "text": "ab" }] },
            { "type": "paragraph", "content": [{ "type": "text", "text": "cd" }] }
        ]
    });
    assert_exact(
        &join,
        vec![TypedOperation::JoinBlocks {
            at: point_for_test(2),
        }],
        tiptap_schema(),
    );
    let image = json!({
        "type": "doc",
        "content": [{ "type": "image", "attrs": { "src": "old", "alt": "old alt" } }]
    });
    assert_exact(
        &image,
        vec![TypedOperation::UpdateNodeAttrs {
            at: point_for_test(0),
            attrs: HashMap::from([
                ("src".into(), Value::String("new".into())),
                ("alt".into(), Value::String("new alt".into())),
            ]),
        }],
        attribute_schema(),
    );
}

#[test]
fn undo_limit_error_attributes_the_crossing_operation_before_a_trailing_noop() {
    let source = json!({
        "type": "doc",
        "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "AB" }] }]
    });
    let error = compile_operations_with_undo_limit(
        &source,
        vec![
            TypedOperation::InsertText {
                at: point_for_test(1),
                text: "XY".into(),
                marks: vec![],
            },
            TypedOperation::AddMark {
                range: range_for_test(0, 0),
                mark: Mark::new("bold".into(), HashMap::new()),
            },
        ],
        tiptap_schema(),
        1,
    )
    .unwrap_err();
    assert_eq!(error.code, "OPERATION_LIMIT_EXCEEDED");
    assert_eq!(error.operation_index, Some(0));
    assert_eq!(error.limit, Some(1));
    assert_eq!(error.actual, Some(2));
    assert_eq!(
        error.details,
        Some(json!({ "field": "maxUndoRetainedUnits" }))
    );
}
