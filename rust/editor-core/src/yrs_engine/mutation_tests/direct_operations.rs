use super::*;

fn engine() -> YrsDocumentEngine {
    let mut engine = YrsDocumentEngine::new(YrsEngineConfig {
        schema: tiptap_schema(),
        fragment_name: "prosemirror".into(),
        initialization_mode: InitializationMode::LocalEmpty,
        resource_limits: ResourceLimits::default(),
        editing_limits: EditingLimits::default(),
        max_length: None,
        scope: None,
    })
    .unwrap();
    engine
        .import_json(TWO_PARAGRAPHS, TransactionOrigin::DocumentImport)
        .unwrap();
    engine
}

#[test]
fn one_character_insert_compiles_a_direct_mutation_action() {
    let engine = engine();
    let compiled = engine
        .compile_typed_transaction(TypedTransaction {
            request_id: 51,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalInput,
            operations: vec![TypedOperation::InsertText {
                at: RevisionedPosition {
                    offset: 1,
                    kind: EditorOffsetKind::Scalar,
                    affinity: Affinity::After,
                },
                text: "!".into(),
                marks: vec![],
            }],
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Auto,
        })
        .unwrap();

    assert_eq!(compiled.preview.root().text_content(), "a!lphaomega");
    assert_eq!(compiled.mutation_plan.actions.len(), 1);
    assert!(compiled.encoded_growth_bound > 0);
}

#[test]
fn every_text_and_mark_operation_executes_to_its_exact_preview() {
    let plain = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "Hello" }]
        }]
    });
    let bold = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{
                "type": "text",
                "text": "Hello",
                "marks": [{ "type": "bold" }]
            }]
        }]
    });
    let link = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{
                "type": "text",
                "text": "Hello",
                "marks": [{ "type": "link", "attrs": { "href": "old" } }]
            }]
        }]
    });
    let cases = vec![
        (
            plain.clone(),
            TypedOperation::InsertText {
                at: point_for_test(2),
                text: "🙂".into(),
                marks: vec![Mark::new("bold".into(), HashMap::new())],
            },
        ),
        (
            plain.clone(),
            TypedOperation::DeleteRange {
                range: range_for_test(1, 4),
            },
        ),
        (
            plain.clone(),
            TypedOperation::ReplaceRange {
                range: range_for_test(1, 4),
                content: Fragment::from(vec![Node::text(
                    "XY".into(),
                    vec![Mark::new("italic".into(), HashMap::new())],
                )]),
            },
        ),
        (
            plain.clone(),
            TypedOperation::AddMark {
                range: range_for_test(1, 4),
                mark: Mark::new("bold".into(), HashMap::new()),
            },
        ),
        (
            bold,
            TypedOperation::RemoveMark {
                range: range_for_test(1, 4),
                mark_type: "bold".into(),
            },
        ),
        (
            link,
            TypedOperation::ReplaceMark {
                range: range_for_test(1, 4),
                mark: Mark::new(
                    "link".into(),
                    HashMap::from([("href".into(), Value::String("new".into()))]),
                ),
            },
        ),
    ];

    for (source, operation) in cases {
        compile_and_execute(source, vec![operation]);
    }
}

#[test]
fn empty_textblocks_create_direct_text_targets_for_insert_and_collapsed_replace() {
    let empty = json!({
        "type": "doc",
        "content": [{ "type": "paragraph" }]
    });
    let inserted = compile_and_execute(
        empty.clone(),
        vec![TypedOperation::InsertText {
            at: point_for_test(1),
            text: "hello".into(),
            marks: vec![Mark::new("bold".into(), HashMap::new())],
        }],
    );
    assert_eq!(inserted.0["content"][0]["content"][0]["text"], "hello");

    let replaced = compile_and_execute(
        empty,
        vec![TypedOperation::ReplaceRange {
            range: range_for_test(1, 1),
            content: Fragment::from(vec![Node::text("world".into(), vec![])]),
        }],
    );
    assert_eq!(replaced.0["content"][0]["content"][0]["text"], "world");
}

#[test]
fn created_text_target_executes_multi_piece_replacement_and_follow_up_edits() {
    let empty = json!({
        "type": "doc",
        "content": [{ "type": "paragraph" }]
    });
    let pieces = Fragment::from(vec![
        Node::text("ab".into(), vec![Mark::new("bold".into(), HashMap::new())]),
        Node::text(
            "cd".into(),
            vec![Mark::new("italic".into(), HashMap::new())],
        ),
    ]);
    let actual = compile_and_execute(
        empty,
        vec![TypedOperation::ReplaceRange {
            range: range_for_test(1, 1),
            content: pieces,
        }],
    );
    assert_eq!(actual.0["content"][0]["content"][0]["text"], "ab");
    assert_eq!(actual.0["content"][0]["content"][1]["text"], "cd");

    let source = json!({
        "type": "doc",
        "content": [{ "type": "paragraph" }]
    });
    let (doc, schema, limits, _editing_limits, document) = diagnostic_doc(&source);
    let plan = {
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let mut compiler = super::mutation::MutationCompiler::new(
            83,
            &txn,
            &fragment,
            &schema,
            1_000,
            limits.max_input_bytes,
            0,
        )
        .unwrap();
        compiler
            .insert(0, 1, "abcd", &[])
            .and_then(|_| compiler.insert(1, 3, "X", &[]))
            .and_then(|_| compiler.delete(2, 2, 3, &[]))
            .and_then(|_| {
                compiler.format(
                    3,
                    1,
                    5,
                    &[1, 5],
                    super::mutation::mark_attr(&Mark::new("bold".into(), HashMap::new())),
                )
            })
            .unwrap();
        let plan = compiler.finish(Some(3)).unwrap();
        preflight_mutation_plan(83, &plan, &txn).unwrap();
        plan
    };
    assert!(matches!(
        plan.actions.first(),
        Some(YrsMutationAction::CreateText { follow_up, .. }) if follow_up.len() == 3
    ));
    {
        let mut txn = doc.transact_mut();
        execute_mutation_plan(plan, &mut txn);
    }
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let actual = YrsDocumentCodec::new(&schema, &limits)
        .read_json(&fragment, &txn)
        .unwrap();
    let actual = from_prosemirror_json(&actual, &schema, UnknownTypeMode::Preserve).unwrap();
    assert_eq!(actual.root().text_content(), "aXcd");
    assert_eq!(document.root().text_content(), "");
    let marks = actual
        .root()
        .content()
        .unwrap()
        .iter()
        .next()
        .unwrap()
        .content()
        .unwrap();
    assert!(marks
        .iter()
        .all(|node| node.marks().iter().any(|mark| mark.mark_type() == "bold")));
}

#[test]
fn atom_only_textblock_supports_both_gaps_and_adjacent_opaque_atoms() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "hardBreak" }]
        }]
    });
    let before = compile_and_execute(
        source.clone(),
        vec![TypedOperation::InsertText {
            at: point_for_test(0),
            text: "before".into(),
            marks: vec![],
        }],
    );
    assert_eq!(before.0["content"][0]["content"][0]["text"], "before");
    assert_eq!(before.0["content"][0]["content"][1]["type"], "hardBreak");

    let after = compile_and_execute(
        source.clone(),
        vec![TypedOperation::InsertText {
            at: point_for_test(1),
            text: "after".into(),
            marks: vec![],
        }],
    );
    assert_eq!(after.0["content"][0]["content"][0]["type"], "hardBreak");
    assert_eq!(after.0["content"][0]["content"][1]["text"], "after");

    let both = compile_and_execute(
        source.clone(),
        vec![
            TypedOperation::InsertText {
                at: point_for_test(0),
                text: "L".into(),
                marks: vec![],
            },
            TypedOperation::InsertText {
                at: point_for_test(1),
                text: "R".into(),
                marks: vec![],
            },
        ],
    );
    assert_eq!(both.0["content"][0]["content"][0]["text"], "L");
    assert_eq!(both.0["content"][0]["content"][1]["type"], "hardBreak");
    assert_eq!(both.0["content"][0]["content"][2]["text"], "R");

    let mention_source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{
                "type": "mention",
                "attrs": { "id": "user-1", "label": "Alice" }
            }]
        }]
    });
    let (mention_doc, mention_schema, mention_limits, _, _) = diagnostic_doc(&mention_source);
    let mention_plan = {
        let txn = mention_doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let mut compiler = super::mutation::MutationCompiler::new(
            87,
            &txn,
            &fragment,
            &mention_schema,
            1_000,
            mention_limits.max_input_bytes,
            0,
        )
        .unwrap();
        let after_mention = compiler
            .target_positions_for_test()
            .unwrap()
            .last()
            .unwrap()
            .0;
        compiler.insert(0, after_mention, "tail", &[]).unwrap();
        let plan = compiler.finish(Some(0)).unwrap();
        preflight_mutation_plan(87, &plan, &txn).unwrap();
        plan
    };
    {
        let mut txn = mention_doc.transact_mut();
        execute_mutation_plan(mention_plan, &mut txn);
    }
    let txn = mention_doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let mention = YrsDocumentCodec::new(&mention_schema, &mention_limits)
        .read_json(&fragment, &txn)
        .unwrap();
    assert_eq!(mention["content"][0]["content"][0]["type"], "mention");
    assert_eq!(mention["content"][0]["content"][1]["text"], "tail");
}

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

#[test]
fn block_insert_node_at_rendered_inter_block_break_targets_the_root_boundary() {
    let source = json!({
        "type": "doc",
        "content": [
            { "type": "paragraph", "content": [{ "type": "text", "text": "A" }] },
            { "type": "paragraph", "content": [{ "type": "text", "text": "B" }] }
        ]
    });
    let schema = tiptap_schema();
    let break_offset = rendered_scalar_offset(&source, &schema, "B") - 1;
    let (actual, expected, _, _, _) = compile_and_execute(
        source,
        vec![TypedOperation::InsertNode {
            at: point_for_test(break_offset),
            node: Node::void("horizontalRule".into(), HashMap::new()),
        }],
    );
    assert_eq!(actual, expected);
    assert_eq!(
        actual["content"]
            .as_array()
            .unwrap()
            .iter()
            .map(|node| node["type"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["paragraph", "horizontalRule", "paragraph"]
    );
}

#[test]
fn wide_block_insert_resolver_admits_exact_work_and_rejects_one_under_atomically() {
    let large_text = format!("{}😀", "A".repeat(4_096));
    let mut wide_inline = vec![json!({ "type": "text", "text": large_text })];
    wide_inline.extend(
        (0..160)
            .map(|_| json!({ "type": "hardBreak" }))
            .collect::<Vec<_>>(),
    );
    wide_inline.push(json!({ "type": "text", "text": "end" }));
    let source = json!({
        "type": "doc",
        "content": [
            { "type": "paragraph", "content": wide_inline },
            { "type": "paragraph", "content": [{ "type": "text", "text": "tail" }] }
        ]
    });
    let (doc, schema, limits, editing_limits, document) = diagnostic_doc(&source);
    let at = rendered_scalar_offset(&source, &schema, "tail") - 1;
    let compile = |resource_limits: &ResourceLimits| {
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        compile_transaction_with_yrs(
            CompilationContext {
                document: &document,
                selection: None,
                schema: &schema,
                resource_limits,
                editing_limits: &editing_limits,
                document_revision: 0,
                max_length: None,
            },
            TypedTransaction {
                request_id: 116,
                base_document_revision: 0,
                origin: TransactionOrigin::LocalCommand,
                operations: vec![TypedOperation::InsertNode {
                    at: point_for_test(at),
                    node: Node::void("horizontalRule".into(), HashMap::new()),
                }],
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Auto,
            },
            &txn,
            &fragment,
        )
    };

    let baseline = compile(&limits).unwrap();
    let resolver_work = baseline.mutation_plan.position_resolver_work_for_test();
    assert!(resolver_work > 4_256);
    let exact = baseline.mutation_plan.scan_work;
    let mut exact_limits = limits.clone();
    exact_limits.max_input_bytes = exact;
    let admitted = compile(&exact_limits).unwrap();
    assert_eq!(admitted.mutation_plan.scan_work, exact);
    assert_eq!(
        admitted.mutation_plan.position_resolver_work_for_test(),
        resolver_work
    );

    let txn = doc.transact();
    let before = txn.encode_state_as_update_v1(&StateVector::default());
    drop(txn);
    exact_limits.max_input_bytes = exact - 1;
    let error = compile(&exact_limits).unwrap_err();
    assert_eq!(error.code, "OPERATION_LIMIT_EXCEEDED");
    assert_eq!(error.limit, Some(u64::try_from(exact - 1).unwrap()));
    assert_eq!(error.actual, Some(u64::try_from(exact).unwrap()));
    let txn = doc.transact();
    assert_eq!(
        txn.encode_state_as_update_v1(&StateVector::default()),
        before
    );
    drop(txn);

    let non_resolver_work = exact.checked_sub(resolver_work).unwrap();
    let early_limit = non_resolver_work.checked_add(20).unwrap();
    assert!(early_limit < exact);
    exact_limits.max_input_bytes = early_limit;
    let error = compile(&exact_limits).unwrap_err();
    assert_eq!(error.code, "OPERATION_LIMIT_EXCEEDED");
    assert_eq!(error.limit, Some(u64::try_from(early_limit).unwrap()));
    assert_eq!(error.actual, Some(u64::try_from(early_limit + 1).unwrap()));
    let txn = doc.transact();
    assert_eq!(
        txn.encode_state_as_update_v1(&StateVector::default()),
        before
    );
}

#[test]
fn opaque_block_insert_at_rendered_break_targets_root_and_preserves_wire_tree() {
    let source = json!({
        "type": "doc",
        "content": [
            { "type": "paragraph", "content": [{ "type": "text", "text": "A" }] },
            { "type": "paragraph", "content": [{ "type": "text", "text": "B" }] }
        ]
    });
    let original = json!({
        "type": "mysteryBlock",
        "attrs": { "payload": [1, 2, 3] },
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "wire-only" }]
        }]
    });
    let opaque = Node::void(
        "__opaque_json".into(),
        HashMap::from([
            ("original_type".into(), Value::String("mysteryBlock".into())),
            ("original_json".into(), original.clone()),
            ("opaque_placement".into(), Value::String("block".into())),
        ]),
    );
    let schema = tiptap_schema();
    let break_offset = rendered_scalar_offset(&source, &schema, "B") - 1;
    let (actual, expected, _, _, _) = compile_and_execute(
        source,
        vec![TypedOperation::InsertNode {
            at: point_for_test(break_offset),
            node: opaque,
        }],
    );
    assert_eq!(actual, expected);
    assert_eq!(actual["content"][1], original);
    assert_eq!(actual["content"][2]["content"][0]["text"], "B");
}

#[test]
fn block_insert_node_maps_public_start_end_and_empty_block_boundaries() {
    for (source, offset, affinity, expected_index) in [
        (
            json!({
                "type": "doc",
                "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "A" }] }]
            }),
            0,
            Affinity::Before,
            0,
        ),
        (
            json!({
                "type": "doc",
                "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "A" }] }]
            }),
            1,
            Affinity::After,
            1,
        ),
        (
            json!({ "type": "doc", "content": [{ "type": "paragraph" }] }),
            0,
            Affinity::Before,
            0,
        ),
        (
            json!({ "type": "doc", "content": [{ "type": "paragraph" }] }),
            1,
            Affinity::After,
            1,
        ),
    ] {
        let (actual, expected, _, _, _) = compile_and_execute(
            source,
            vec![TypedOperation::InsertNode {
                at: RevisionedPosition {
                    offset,
                    kind: EditorOffsetKind::Scalar,
                    affinity,
                },
                node: Node::void("horizontalRule".into(), HashMap::new()),
            }],
        );
        assert_eq!(actual, expected);
        assert_eq!(actual["content"][expected_index]["type"], "horizontalRule");
    }
}

#[test]
fn inline_insert_node_keeps_textblock_mapping_at_public_start_and_end() {
    for (source, offset, affinity, expected_inline_index) in [
        (
            json!({
                "type": "doc",
                "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "A" }] }]
            }),
            0,
            Affinity::Before,
            0,
        ),
        (
            json!({
                "type": "doc",
                "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "A" }] }]
            }),
            1,
            Affinity::After,
            1,
        ),
    ] {
        let (actual, expected, _, _, _) = compile_and_execute(
            source,
            vec![TypedOperation::InsertNode {
                at: RevisionedPosition {
                    offset,
                    kind: EditorOffsetKind::Scalar,
                    affinity,
                },
                node: Node::void("hardBreak".into(), HashMap::new()),
            }],
        );
        assert_eq!(actual, expected);
        assert_eq!(actual["content"].as_array().unwrap().len(), 1);
        assert_eq!(
            actual["content"][0]["content"][expected_inline_index]["type"],
            "hardBreak"
        );
    }
}

#[test]
fn custom_inline_roles_preserve_every_offset_mapping_for_direct_insert_node() {
    let schema = Schema::from_json(&json!({
        "nodes": [
            { "name": "root", "content": "block*", "role": "doc" },
            { "name": "body", "content": "inline*", "group": "block", "role": "textBlock" },
            { "name": "softBreak", "content": "", "group": "inline", "role": "hardBreak", "isVoid": true, "allowUndeclaredAttrs": true },
            { "name": "widget", "content": "", "group": "inline", "role": "inline", "isVoid": true, "allowUndeclaredAttrs": true },
            { "name": "text", "group": "inline", "role": "text" }
        ],
        "marks": []
    }))
    .unwrap();
    let long_label = "😀".repeat(2_048);
    let source = json!({
        "type": "root",
        "content": [{
            "type": "body",
            "content": [
                { "type": "softBreak", "attrs": { "label": "ignored-long-label" } },
                { "type": "widget", "attrs": { "label": long_label.clone() } }
            ]
        }]
    });
    let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Error).unwrap();
    let rendered = crate::render::rendered_text(&document, &schema);
    let map = PositionMap::build(&document, &schema);
    assert_eq!(rendered, format!("\n[{long_label}]"));
    let terminal_scalar = 1 + 2 + u32::try_from(long_label.chars().count()).unwrap();
    let mapped = (0..=terminal_scalar)
        .map(|offset| map.scalar_to_doc(offset, &document))
        .collect::<Vec<_>>();
    assert_eq!(mapped[0], 1);
    assert!(mapped[1..terminal_scalar as usize]
        .iter()
        .all(|position| *position == 2));
    assert_eq!(mapped[terminal_scalar as usize], 3);
    assert_eq!(
        (0..=3)
            .map(|position| map.doc_to_scalar(position, &document))
            .collect::<Vec<_>>(),
        vec![0, 0, 1, terminal_scalar]
    );

    let limits = ResourceLimits::default();
    let editing_limits = EditingLimits::default();
    let doc = utf16_doc();
    let codec = YrsDocumentCodec::new(&schema, &limits);
    {
        let mut txn = doc.transact_mut();
        let fragment = txn.get_or_insert_xml_fragment("prosemirror");
        codec
            .apply_json(&fragment, &mut txn, &json!({ "type": "root" }), &source)
            .unwrap();
    }
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
                request_id: 117,
                base_document_revision: 0,
                origin: TransactionOrigin::LocalCommand,
                operations: vec![TypedOperation::InsertNode {
                    at: RevisionedPosition {
                        offset: 2,
                        kind: EditorOffsetKind::Scalar,
                        affinity: Affinity::After,
                    },
                    node: Node::void(
                        "widget".into(),
                        HashMap::from([("label".into(), Value::String("Grace".into()))]),
                    ),
                }],
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Auto,
            },
            &txn,
            &fragment,
        )
        .unwrap()
    };
    assert_eq!(
        to_prosemirror_json(&compiled.preview, &schema)["content"][0]["content"]
            .as_array()
            .unwrap()
            .iter()
            .map(|node| node["attrs"]["label"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["ignored-long-label", "Grace", long_label.as_str()]
    );
    assert!(compiled.mutation_plan.position_resolver_work_for_test() > long_label.len());
    {
        let mut txn = doc.transact_mut();
        execute_mutation_plan(compiled.mutation_plan, &mut txn);
    }
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    assert_eq!(
        codec.read_json(&fragment, &txn).unwrap(),
        to_prosemirror_json(&compiled.preview, &schema)
    );
}

#[test]
fn block_insert_at_separator_between_list_items_uses_an_affinity_valid_item_boundary() {
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
    let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let rendered = crate::render::rendered_text(&document, &schema);
    let separator =
        u32::try_from(rendered[..rendered.find('\n').unwrap()].chars().count()).unwrap();
    for (affinity, item_index) in [(Affinity::Before, 0usize), (Affinity::After, 1usize)] {
        let (actual, expected, _, _, _) = compile_and_execute(
            source.clone(),
            vec![TypedOperation::InsertNode {
                at: RevisionedPosition {
                    offset: separator,
                    kind: EditorOffsetKind::Scalar,
                    affinity,
                },
                node: Node::void("horizontalRule".into(), HashMap::new()),
            }],
        );
        assert_eq!(actual, expected);
        assert_eq!(
            actual["content"][0]["content"][item_index]["content"][1]["type"],
            "horizontalRule"
        );
    }
}

#[test]
fn split_block_then_block_insert_at_same_revisioned_position_uses_created_boundary() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "AB" }]
        }]
    });
    let (actual, expected, _, _, _) = compile_and_execute(
        source,
        vec![
            TypedOperation::SplitBlock {
                at: point_for_test(1),
                node_type: "paragraph".into(),
                attrs: HashMap::new(),
            },
            TypedOperation::InsertNode {
                at: point_for_test(1),
                node: Node::void("horizontalRule".into(), HashMap::new()),
            },
        ],
    );
    assert_eq!(actual, expected);
    assert_eq!(
        actual["content"]
            .as_array()
            .unwrap()
            .iter()
            .map(|node| node["type"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["paragraph", "horizontalRule", "paragraph"]
    );
}

#[test]
fn nested_opaque_json_insert_remains_one_semantic_atom_for_follow_up_edits() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "ab" }]
        }]
    });
    let original = json!({
        "type": "mysteryInline",
        "attrs": { "payload": { "nested": true } },
        "content": [{ "type": "text", "text": "wire-only" }]
    });
    let opaque = Node::void(
        "__opaque_json".into(),
        HashMap::from([
            (
                "original_type".into(),
                Value::String("mysteryInline".into()),
            ),
            ("original_json".into(), original.clone()),
            ("opaque_placement".into(), Value::String("inline".into())),
        ]),
    );
    let (actual, expected, _, _, _) = compile_and_execute(
        source,
        vec![
            TypedOperation::InsertNode {
                at: point_for_test(1),
                node: opaque,
            },
            TypedOperation::InsertText {
                at: point_for_test(1),
                text: "X".into(),
                marks: vec![],
            },
        ],
    );
    assert_eq!(actual, expected);
    assert_eq!(actual["content"][0]["content"][1], original);
    assert_eq!(actual["content"][0]["content"][2]["text"], "Xb");
}

#[test]
fn existing_unknown_wire_element_with_descendants_has_void_semantic_size() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [
                { "type": "text", "text": "a" },
                {
                    "type": "mysteryInline",
                    "content": [{ "type": "text", "text": "wire-only" }]
                },
                { "type": "text", "text": "b" }
            ]
        }]
    });
    let schema = tiptap_schema();
    let b = rendered_scalar_offset(&source, &schema, "b");
    let (actual, expected, _, _, _) = compile_and_execute(
        source,
        vec![TypedOperation::InsertText {
            at: point_for_test(b),
            text: "X".into(),
            marks: vec![],
        }],
    );
    assert_eq!(actual, expected);
    assert_eq!(actual["content"][0]["content"][1]["type"], "mysteryInline");
    assert_eq!(actual["content"][0]["content"][2]["text"], "Xb");
}

#[test]
fn existing_unknown_block_wire_tree_is_one_semantic_atom_for_follow_up_text() {
    let source = json!({
        "type": "doc",
        "content": [
            {
                "type": "mysteryBlock",
                "content": [{
                    "type": "paragraph",
                    "content": [{ "type": "text", "text": "wire-only" }]
                }]
            },
            { "type": "paragraph", "content": [{ "type": "text", "text": "B" }] }
        ]
    });
    let schema = tiptap_schema();
    let b = rendered_scalar_offset(&source, &schema, "B");
    let (actual, expected, _, _, _) = compile_and_execute(
        source,
        vec![TypedOperation::InsertText {
            at: point_for_test(b),
            text: "X".into(),
            marks: vec![],
        }],
    );
    assert_eq!(actual, expected);
    assert_eq!(actual["content"][0]["type"], "mysteryBlock");
    assert_eq!(actual["content"][1]["content"][0]["text"], "XB");
}

#[test]
fn malformed_wire_headings_remain_one_opaque_atom_and_hide_descendants() {
    for attrs in [
        None,
        Some(json!({ "level": 7 })),
        Some(json!({ "level": 2.5 })),
    ] {
        let mut heading = json!({
            "type": "heading",
            "content": [{ "type": "text", "text": "wire-only" }]
        });
        if let Some(attrs) = attrs {
            heading["attrs"] = attrs;
        }
        let source = json!({
            "type": "doc",
            "content": [
                heading,
                { "type": "paragraph", "content": [{ "type": "text", "text": "B" }] }
            ]
        });
        let schema = tiptap_schema();
        let b = rendered_scalar_offset(&source, &schema, "B");
        let (actual, expected, _, _, _) = compile_and_execute(
            source,
            vec![TypedOperation::InsertText {
                at: point_for_test(b),
                text: "X".into(),
                marks: vec![],
            }],
        );
        assert_eq!(actual, expected);
        assert_eq!(actual["content"][0]["type"], "heading");
        assert_eq!(actual["content"][1]["content"][0]["text"], "XB");
    }

    let source = json!({
        "type": "doc",
        "content": [
            {
                "type": "heading",
                "attrs": { "level": 7 },
                "content": [{ "type": "text", "text": "hidden" }]
            },
            { "type": "paragraph", "content": [{ "type": "text", "text": "tail" }] }
        ]
    });
    let (doc, schema, _, _, _) = diagnostic_doc(&source);
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let XmlOut::Element(heading) = fragment.get(&txn, 0).unwrap() else {
        panic!("heading wire element expected")
    };
    let XmlOut::Text(hidden) = heading.get(&txn, 0).unwrap() else {
        panic!("hidden wire text expected")
    };
    let descendant = StickyIndex::at(
        &txn,
        BranchPtr::from(<XmlTextRef as AsRef<Branch>>::as_ref(&hidden)),
        1,
        Assoc::After,
    )
    .unwrap();
    assert!(super::sticky_index_to_doc_pos(&txn, &fragment, &descendant, &schema).is_none());

    let valid_source = json!({
        "type": "doc",
        "content": [{
            "type": "h2",
            "content": [{ "type": "text", "text": "visible" }]
        }]
    });
    let (valid_doc, valid_schema, _, _, _) = diagnostic_doc(&valid_source);
    let valid_txn = valid_doc.transact();
    let valid_fragment = valid_txn.get_xml_fragment("prosemirror").unwrap();
    let XmlOut::Element(valid_heading) = valid_fragment.get(&valid_txn, 0).unwrap() else {
        panic!("valid heading wire element expected")
    };
    assert_eq!(valid_heading.tag().as_ref(), "heading");
    let XmlOut::Text(visible) = valid_heading.get(&valid_txn, 0).unwrap() else {
        panic!("valid heading text expected")
    };
    let visible_sticky = StickyIndex::at(
        &valid_txn,
        BranchPtr::from(<XmlTextRef as AsRef<Branch>>::as_ref(&visible)),
        1,
        Assoc::After,
    )
    .unwrap();
    assert_eq!(
        super::sticky_index_to_doc_pos(&valid_txn, &valid_fragment, &visible_sticky, &valid_schema,),
        Some(2)
    );
}

#[test]
fn shared_and_oversized_heading_levels_are_bounded_opaque_atoms() {
    let source = json!({
        "type": "doc",
        "content": [
            {
                "type": "h2",
                "content": [{ "type": "text", "text": "hidden" }]
            },
            { "type": "paragraph", "content": [{ "type": "text", "text": "tail" }] }
        ]
    });

    for shared_kind in 0..2 {
        let (doc, schema, limits, _, _) = diagnostic_doc(&source);
        let hidden = {
            let txn = doc.transact();
            let fragment = txn.get_xml_fragment("prosemirror").unwrap();
            let XmlOut::Element(heading) = fragment.get(&txn, 0).unwrap() else {
                panic!("heading expected")
            };
            let XmlOut::Text(text) = heading.get(&txn, 0).unwrap() else {
                panic!("heading text expected")
            };
            StickyIndex::at(
                &txn,
                BranchPtr::from(<XmlTextRef as AsRef<Branch>>::as_ref(&text)),
                1,
                Assoc::After,
            )
            .unwrap()
        };
        {
            let mut txn = doc.transact_mut();
            let fragment = txn.get_xml_fragment("prosemirror").unwrap();
            let XmlOut::Element(heading) = fragment.get(&txn, 0).unwrap() else {
                panic!("heading expected")
            };
            if shared_kind == 0 {
                heading.insert_attribute(
                    &mut txn,
                    "level",
                    MapPrelim::from([("nested", Any::String("2".into()))]),
                );
            } else {
                heading.insert_attribute(
                    &mut txn,
                    "level",
                    ArrayPrelim::from(vec![Any::String("2".into())]),
                );
            }
        }
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let XmlOut::Element(heading) = fragment.get(&txn, 0).unwrap() else {
            panic!("heading expected")
        };
        assert_eq!(
            super::codec::normalized_wire_element_node_type(&heading, &txn),
            "heading"
        );
        assert!(super::sticky_index_to_doc_pos(&txn, &fragment, &hidden, &schema).is_none());
        let after_atom = StickyIndex::at(
            &txn,
            BranchPtr::from(<yrs::types::xml::XmlFragmentRef as AsRef<Branch>>::as_ref(
                &fragment,
            )),
            1,
            Assoc::After,
        )
        .unwrap();
        assert_eq!(
            super::sticky_index_to_doc_pos(&txn, &fragment, &after_atom, &schema),
            Some(1)
        );
        let error = YrsDocumentCodec::new(&schema, &limits)
            .read_json(&fragment, &txn)
            .unwrap_err();
        assert_eq!(error.code, "CODEC_INVARIANT_FAILED");
    }

    let (doc, schema, mut limits, _, _) = diagnostic_doc(&source);
    let oversized = format!("{}2", "0".repeat(128 * 1024));
    {
        let mut txn = doc.transact_mut();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let XmlOut::Element(heading) = fragment.get(&txn, 0).unwrap() else {
            panic!("heading expected")
        };
        heading.insert_attribute(&mut txn, "level", oversized);
    }
    limits.max_input_bytes = 64;
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let XmlOut::Element(heading) = fragment.get(&txn, 0).unwrap() else {
        panic!("heading expected")
    };
    assert_eq!(
        super::codec::normalized_wire_element_node_type(&heading, &txn),
        "heading"
    );
    let error = YrsDocumentCodec::new(&schema, &limits)
        .read_json(&fragment, &txn)
        .unwrap_err();
    assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
    assert_eq!(error.limit, Some(64));
}

#[test]
fn opaque_block_insert_inside_text_rejects_without_mutating_yrs() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "AB" }]
        }]
    });
    let (doc, schema, limits, editing_limits, document) = diagnostic_doc(&source);
    let opaque = Node::void(
        "__opaque_json".into(),
        HashMap::from([
            ("original_type".into(), Value::String("mysteryBlock".into())),
            (
                "original_json".into(),
                json!({ "type": "mysteryBlock", "content": [{ "type": "paragraph" }] }),
            ),
            ("opaque_placement".into(), Value::String("block".into())),
        ]),
    );
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let before = txn.encode_state_as_update_v1(&StateVector::default());
    let error = compile_transaction_with_yrs(
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
            request_id: 171,
            base_document_revision: 0,
            origin: TransactionOrigin::LocalCommand,
            operations: vec![TypedOperation::InsertNode {
                at: point_for_test(1),
                node: opaque,
            }],
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Auto,
        },
        &txn,
        &fragment,
    )
    .unwrap_err();
    assert_eq!(error.code, "DOCUMENT_INVALID");
    assert_eq!(
        txn.encode_state_as_update_v1(&StateVector::default()),
        before
    );
    assert_eq!(
        YrsDocumentCodec::new(&schema, &limits)
            .read_json(&fragment, &txn)
            .unwrap(),
        source
    );
}

#[test]
fn opaque_html_inline_and_block_insertions_round_trip_canonical_metadata() {
    let inline_attrs = HashMap::from([
        ("html_tag".into(), Value::String("widget-inline".into())),
        ("opaque_placement".into(), Value::String("inline".into())),
        ("html_attrs".into(), json!({ "data-id": "7" })),
        ("text_content".into(), Value::String("raw".into())),
        ("inner_html".into(), Value::String("<b>raw</b>".into())),
    ]);
    let inline = Node::void("__opaque".into(), inline_attrs);
    let inline_source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "ab" }]
        }]
    });
    let (inline_doc, inline_schema, inline_limits, inline_compiled) =
        compile_operations_with_schema(
            &inline_source,
            vec![
                TypedOperation::InsertNode {
                    at: point_for_test(1),
                    node: inline,
                },
                TypedOperation::InsertText {
                    at: point_for_test(1),
                    text: "X".into(),
                    marks: vec![],
                },
            ],
            tiptap_schema(),
        );
    let inline_expected = to_prosemirror_json(&inline_compiled.preview, &inline_schema);
    assert_eq!(
        to_html(&inline_compiled.preview, &inline_schema),
        "<p>a<widget-inline data-id=\"7\"><b>raw</b></widget-inline>Xb</p>"
    );
    {
        let mut txn = inline_doc.transact_mut();
        execute_mutation_plan(inline_compiled.mutation_plan, &mut txn);
    }
    let txn = inline_doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let actual = YrsDocumentCodec::new(&inline_schema, &inline_limits)
        .read_json(&fragment, &txn)
        .unwrap();
    assert_eq!(actual, inline_expected);
    assert_eq!(actual["content"][0]["content"][1]["type"], "__opaque");
    assert_eq!(actual["content"][0]["content"][2]["text"], "Xb");

    let block_attrs = HashMap::from([
        ("html_tag".into(), Value::String("widget-block".into())),
        ("opaque_placement".into(), Value::String("block".into())),
        ("html_attrs".into(), json!({ "data-kind": "card" })),
        ("inner_html".into(), Value::String("<i>card</i>".into())),
    ]);
    let block = Node::void("__opaque".into(), block_attrs);
    let block_source = json!({
        "type": "doc",
        "content": [
            { "type": "paragraph", "content": [{ "type": "text", "text": "A" }] },
            { "type": "paragraph", "content": [{ "type": "text", "text": "B" }] }
        ]
    });
    let schema = tiptap_schema();
    let at = rendered_scalar_offset(&block_source, &schema, "B") - 1;
    let (doc, schema, limits, compiled) = compile_operations_with_schema(
        &block_source,
        vec![TypedOperation::InsertNode {
            at: point_for_test(at),
            node: block,
        }],
        schema,
    );
    assert_eq!(
        to_html(&compiled.preview, &schema),
        "<p>A</p><widget-block data-kind=\"card\"><i>card</i></widget-block><p>B</p>"
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
fn opaque_sentinel_validator_rejects_forged_shapes_and_known_aliases() {
    let schema = tiptap_schema();
    let limits = ResourceLimits::default();
    let json_attrs = |original_type: &str, original_json: Value| {
        HashMap::from([
            ("original_type".into(), Value::String(original_type.into())),
            ("original_json".into(), original_json),
            ("opaque_placement".into(), Value::String("inline".into())),
        ])
    };
    let mut forged = vec![
        Node::element(
            "__opaque_json".into(),
            json_attrs("mystery", json!({ "type": "mystery" })),
            Fragment::from(vec![Node::text("child".into(), vec![])]),
        ),
        Node::void(
            "__opaque_json".into(),
            json_attrs("mystery", json!({ "type": "different" })),
        ),
        Node::void(
            "__opaque_json".into(),
            HashMap::from([
                ("original_type".into(), Value::String("mystery".into())),
                ("opaque_placement".into(), Value::String("inline".into())),
            ]),
        ),
        Node::void(
            "__opaque_json".into(),
            json_attrs("mystery", Value::String("not-an-object".into())),
        ),
        Node::void(
            "__opaque_json".into(),
            json_attrs("__opaque", json!({ "type": "__opaque" })),
        ),
        Node::void("__opaque_json".into(), {
            let mut attrs = json_attrs("mystery", json!({ "type": "mystery" }));
            attrs.insert("extra".into(), Value::Bool(true));
            attrs
        }),
        Node::void(
            "__opaque_json".into(),
            json_attrs("paragraph", json!({ "type": "paragraph" })),
        ),
        Node::void(
            "__opaque_json".into(),
            json_attrs(
                "heading",
                json!({ "type": "heading", "attrs": { "level": 2 } }),
            ),
        ),
        Node::void(
            "__opaque_json".into(),
            json_attrs(
                "heading",
                json!({ "type": "heading", "attrs": { "level": "2" } }),
            ),
        ),
        Node::void(
            "__opaque".into(),
            HashMap::from([
                ("html_tag".into(), Value::String("Bad<Tag".into())),
                ("opaque_placement".into(), Value::String("inline".into())),
            ]),
        ),
        Node::element(
            "__opaque".into(),
            HashMap::from([
                ("html_tag".into(), Value::String("widget-inline".into())),
                ("opaque_placement".into(), Value::String("inline".into())),
            ]),
            Fragment::from(vec![Node::text("child".into(), vec![])]),
        ),
        Node::void(
            "__opaque".into(),
            HashMap::from([
                ("html_tag".into(), Value::String("strong".into())),
                ("opaque_placement".into(), Value::String("inline".into())),
            ]),
        ),
        Node::void(
            "__opaque".into(),
            HashMap::from([
                ("html_tag".into(), Value::String("widget-inline".into())),
                ("opaque_placement".into(), Value::String("inline".into())),
                ("html_attrs".into(), json!({ "bad key": "value" })),
            ]),
        ),
        Node::void(
            "__opaque".into(),
            HashMap::from([
                ("html_tag".into(), Value::String("widget-inline".into())),
                ("opaque_placement".into(), Value::String("inline".into())),
                ("html_attrs".into(), json!({ "data-id": 7 })),
            ]),
        ),
        Node::void(
            "__opaque".into(),
            HashMap::from([
                ("html_tag".into(), Value::String("widget-inline".into())),
                ("opaque_placement".into(), Value::String("inline".into())),
                ("extra".into(), Value::Bool(true)),
            ]),
        ),
        Node::void(
            "__opaque".into(),
            HashMap::from([
                ("html_tag".into(), Value::String("img".into())),
                ("opaque_placement".into(), Value::String("block".into())),
                (
                    "html_attrs".into(),
                    json!({ "src": "https://example.test/image.png" }),
                ),
            ]),
        ),
    ];
    for tag in ["b", "i", "del", "strike"] {
        forged.push(Node::void(
            "__opaque".into(),
            HashMap::from([
                ("html_tag".into(), Value::String(tag.into())),
                ("opaque_placement".into(), Value::String("inline".into())),
            ]),
        ));
    }
    forged.push(Node::void(
        "__opaque".into(),
        HashMap::from([
            ("html_tag".into(), Value::String("span".into())),
            ("opaque_placement".into(), Value::String("inline".into())),
            (
                "html_attrs".into(),
                json!({ "data-native-editor-mark": "bold" }),
            ),
        ]),
    ));
    for (case_index, opaque) in forged.into_iter().enumerate() {
        let document = Document::new(Node::element(
            "doc".into(),
            HashMap::new(),
            Fragment::from(vec![Node::element(
                "paragraph".into(),
                HashMap::new(),
                Fragment::from(vec![opaque]),
            )]),
        ));
        let error = match DocumentValidator::validate(&document, &schema, &limits) {
            Ok(_) => panic!("forged opaque case {case_index} was accepted"),
            Err(error) => error,
        };
        assert_eq!(error.code, "DOCUMENT_INVALID");
    }

    for (tag, html_attrs, placement) in [
        ("paragraph", json!({}), "inline"),
        (
            "img",
            json!({ "src": "https://example.test/image.png", "alt": "Inline" }),
            "inline",
        ),
        (
            "img",
            json!({ "src": "data:image/png;base64,AAAA", "alt": "Inline" }),
            "block",
        ),
    ] {
        let opaque = Node::void(
            "__opaque".into(),
            HashMap::from([
                ("html_tag".into(), Value::String(tag.into())),
                ("opaque_placement".into(), Value::String(placement.into())),
                ("html_attrs".into(), html_attrs),
            ]),
        );
        let document = Document::new(Node::element(
            "doc".into(),
            HashMap::new(),
            Fragment::from(if placement == "block" {
                vec![opaque]
            } else {
                vec![Node::element(
                    "paragraph".into(),
                    HashMap::new(),
                    Fragment::from(vec![opaque]),
                )]
            }),
        ));
        DocumentValidator::validate(&document, &schema, &limits).unwrap();
    }

    let semantic_block_image = Document::new(Node::element(
        "doc".into(),
        HashMap::new(),
        Fragment::from(vec![Node::void(
            "__opaque".into(),
            HashMap::from([
                ("html_tag".into(), Value::String("img".into())),
                ("opaque_placement".into(), Value::String("block".into())),
                (
                    "html_attrs".into(),
                    json!({ "src": "https://example.test/image.png" }),
                ),
            ]),
        )]),
    ));
    assert_eq!(
        DocumentValidator::validate(&semantic_block_image, &schema, &limits)
            .unwrap_err()
            .code,
        "DOCUMENT_INVALID"
    );

    let mention_schema = Schema::from_json(&json!({
        "nodes": [
            { "name": "doc", "content": "block*", "role": "doc" },
            { "name": "paragraph", "content": "inline*", "group": "block", "role": "textBlock", "htmlTag": "p" },
            { "name": "mention", "content": "", "group": "inline", "role": "inline", "isVoid": true },
            { "name": "text", "group": "inline", "role": "text" }
        ],
        "marks": []
    }))
    .unwrap();
    let mention_forge = Document::new(Node::element(
        "doc".into(),
        HashMap::new(),
        Fragment::from(vec![Node::element(
            "paragraph".into(),
            HashMap::new(),
            Fragment::from(vec![Node::void(
                "__opaque".into(),
                HashMap::from([
                    ("html_tag".into(), Value::String("span".into())),
                    ("opaque_placement".into(), Value::String("inline".into())),
                    (
                        "html_attrs".into(),
                        json!({ "data-native-editor-mention": "true" }),
                    ),
                ]),
            )]),
        )]),
    ));
    assert_eq!(
        DocumentValidator::validate(&mention_forge, &mention_schema, &limits)
            .unwrap_err()
            .code,
        "DOCUMENT_INVALID"
    );

    let inline_void_schema = Schema::from_json(&json!({
        "nodes": [
            { "name": "doc", "content": "block+", "role": "doc" },
            { "name": "paragraph", "content": "inline*", "group": "block", "role": "textBlock", "htmlTag": "p" },
            { "name": "inlineVoid", "content": "", "group": "inline", "role": "inline", "htmlTag": "x-void", "isVoid": true },
            { "name": "hardBreak", "content": "", "group": "inline", "role": "hardBreak", "htmlTag": "br", "isVoid": true },
            { "name": "text", "group": "inline", "role": "text" }
        ],
        "marks": []
    }))
    .unwrap();
    for tag in ["x-void", "br"] {
        let forged = Document::new(Node::element(
            "doc".into(),
            HashMap::new(),
            Fragment::from(vec![Node::element(
                "paragraph".into(),
                HashMap::new(),
                Fragment::from(vec![Node::void(
                    "__opaque".into(),
                    HashMap::from([
                        ("html_tag".into(), Value::String(tag.into())),
                        ("opaque_placement".into(), Value::String("inline".into())),
                    ]),
                )]),
            )]),
        ));
        assert_eq!(
            DocumentValidator::validate(&forged, &inline_void_schema, &limits)
                .unwrap_err()
                .code,
            "DOCUMENT_INVALID"
        );
    }
}

#[test]
fn uppercase_private_opaque_html_attributes_are_rejected_before_reimport_normalizes_them() {
    let limits = ResourceLimits::default();
    let mark_schema = tiptap_schema();
    let mark_forge = Document::new(Node::element(
        "doc".into(),
        HashMap::new(),
        Fragment::from(vec![Node::element(
            "paragraph".into(),
            HashMap::new(),
            Fragment::from(vec![Node::void(
                "__opaque".into(),
                HashMap::from([
                    ("html_tag".into(), Value::String("span".into())),
                    ("opaque_placement".into(), Value::String("inline".into())),
                    (
                        "html_attrs".into(),
                        json!({ "DATA-NATIVE-EDITOR-MARK": "bold" }),
                    ),
                    ("inner_html".into(), Value::String("marked".into())),
                ]),
            )]),
        )]),
    ));
    assert_eq!(
        DocumentValidator::validate(&mark_forge, &mark_schema, &limits)
            .unwrap_err()
            .code,
        "DOCUMENT_INVALID"
    );
    let mark_html = to_html(&mark_forge, &mark_schema);
    let reparsed_mark = from_html(&mark_html, &mark_schema, &FromHtmlOptions::default()).unwrap();
    let reparsed_mark_json = to_prosemirror_json(&reparsed_mark, &mark_schema);
    assert_eq!(
        reparsed_mark_json["content"][0]["content"][0]["marks"][0]["type"],
        "bold"
    );

    let mention_schema = Schema::from_json(&json!({
        "nodes": [
            { "name": "doc", "content": "block*", "role": "doc" },
            { "name": "paragraph", "content": "inline*", "group": "block", "role": "textBlock", "htmlTag": "p" },
            { "name": "mention", "content": "", "group": "inline", "role": "inline", "isVoid": true },
            { "name": "text", "group": "inline", "role": "text" }
        ],
        "marks": []
    }))
    .unwrap();
    let mention_forge = Document::new(Node::element(
        "doc".into(),
        HashMap::new(),
        Fragment::from(vec![Node::element(
            "paragraph".into(),
            HashMap::new(),
            Fragment::from(vec![Node::void(
                "__opaque".into(),
                HashMap::from([
                    ("html_tag".into(), Value::String("span".into())),
                    ("opaque_placement".into(), Value::String("inline".into())),
                    (
                        "html_attrs".into(),
                        json!({ "DATA-NATIVE-EDITOR-MENTION": "true" }),
                    ),
                    ("inner_html".into(), Value::String("@Ada".into())),
                ]),
            )]),
        )]),
    ));
    assert_eq!(
        DocumentValidator::validate(&mention_forge, &mention_schema, &limits)
            .unwrap_err()
            .code,
        "DOCUMENT_INVALID"
    );
    let mention_html = to_html(&mention_forge, &mention_schema);
    let reparsed_mention =
        from_html(&mention_html, &mention_schema, &FromHtmlOptions::default()).unwrap();
    assert_eq!(
        to_prosemirror_json(&reparsed_mention, &mention_schema)["content"][0]["content"][0]["type"],
        "mention"
    );
}

#[test]
fn non_span_private_mention_metadata_remains_opaque_after_export_and_reimport() {
    let schema = Schema::from_json(&json!({
        "nodes": [
            { "name": "doc", "content": "block*", "role": "doc" },
            { "name": "paragraph", "content": "inline*", "group": "block", "role": "textBlock", "htmlTag": "p" },
            { "name": "mention", "content": "", "group": "inline", "role": "inline", "isVoid": true },
            { "name": "text", "group": "inline", "role": "text" }
        ],
        "marks": []
    }))
    .unwrap();
    let document = Document::new(Node::element(
        "doc".into(),
        HashMap::new(),
        Fragment::from(vec![Node::element(
            "paragraph".into(),
            HashMap::new(),
            Fragment::from(vec![Node::void(
                "__opaque".into(),
                HashMap::from([
                    ("html_tag".into(), Value::String("x-mention".into())),
                    ("opaque_placement".into(), Value::String("inline".into())),
                    (
                        "html_attrs".into(),
                        json!({ "data-native-editor-mention": "true" }),
                    ),
                    ("inner_html".into(), Value::String("@Ada".into())),
                ]),
            )]),
        )]),
    ));
    DocumentValidator::validate(&document, &schema, &ResourceLimits::default()).unwrap();
    let html = to_html(&document, &schema);
    let reparsed = from_html(&html, &schema, &FromHtmlOptions::default()).unwrap();
    let json = to_prosemirror_json(&reparsed, &schema);
    assert_eq!(json["content"][0]["content"][0]["type"], "__opaque");
    assert_eq!(
        json["content"][0]["content"][0]["attrs"]["html_attrs"]["data-native-editor-mention"],
        "true"
    );
}

#[test]
fn canonical_foreign_mixed_case_attributes_validate_and_round_trip() {
    let schema = tiptap_schema();
    let limits = ResourceLimits::default();
    for (tag, key, value) in [
        ("svg", "viewBox", "0 0 10 10"),
        ("math", "definitionURL", "https://example.test/definition"),
    ] {
        let document = Document::new(Node::element(
            "doc".into(),
            HashMap::new(),
            Fragment::from(vec![Node::element(
                "paragraph".into(),
                HashMap::new(),
                Fragment::from(vec![Node::void(
                    "__opaque".into(),
                    HashMap::from([
                        ("html_tag".into(), Value::String(tag.into())),
                        ("opaque_placement".into(), Value::String("inline".into())),
                        ("html_attrs".into(), json!({ key: value })),
                    ]),
                )]),
            )]),
        ));
        DocumentValidator::validate(&document, &schema, &limits).unwrap();
        let html = to_html(&document, &schema);
        let reparsed = from_html(&html, &schema, &FromHtmlOptions::default()).unwrap();
        let json = to_prosemirror_json(&reparsed, &schema);
        assert_eq!(
            json["content"][0]["content"][0]["attrs"]["html_attrs"][key],
            value
        );
    }

    for (tag, key) in [
        ("a", "attributeName"),
        ("svg", "DATA-NATIVE-EDITOR-MENTION"),
    ] {
        let forged = Document::new(Node::element(
            "doc".into(),
            HashMap::new(),
            Fragment::from(vec![Node::element(
                "paragraph".into(),
                HashMap::new(),
                Fragment::from(vec![Node::void(
                    "__opaque".into(),
                    HashMap::from([
                        ("html_tag".into(), Value::String(tag.into())),
                        ("opaque_placement".into(), Value::String("inline".into())),
                        ("html_attrs".into(), json!({ key: "forged" })),
                    ]),
                )]),
            )]),
        ));
        assert_eq!(
            DocumentValidator::validate(&forged, &schema, &limits)
                .unwrap_err()
                .code,
            "DOCUMENT_INVALID"
        );
    }
    for (tag, html_attrs) in [
        (
            "svg",
            json!({ "viewBox": "0 0 10 10", "viewbox": "0 0 20 20" }),
        ),
        (
            "math",
            json!({ "definitionURL": "a", "definitionurl": "b" }),
        ),
    ] {
        let collision = Document::new(Node::element(
            "doc".into(),
            HashMap::new(),
            Fragment::from(vec![Node::element(
                "paragraph".into(),
                HashMap::new(),
                Fragment::from(vec![Node::void(
                    "__opaque".into(),
                    HashMap::from([
                        ("html_tag".into(), Value::String(tag.into())),
                        ("opaque_placement".into(), Value::String("inline".into())),
                        ("html_attrs".into(), html_attrs),
                    ]),
                )]),
            )]),
        ));
        assert_eq!(
            DocumentValidator::validate(&collision, &schema, &limits)
                .unwrap_err()
                .code,
            "DOCUMENT_INVALID"
        );
    }
}

#[test]
fn foreign_qualified_attributes_preserve_prefixes_without_colliding() {
    let schema = tiptap_schema();
    let limits = ResourceLimits::default();
    let document = Document::new(Node::element(
        "doc".into(),
        HashMap::new(),
        Fragment::from(vec![Node::element(
            "paragraph".into(),
            HashMap::new(),
            Fragment::from(vec![Node::void(
                "__opaque".into(),
                HashMap::from([
                    ("html_tag".into(), Value::String("svg".into())),
                    ("opaque_placement".into(), Value::String("inline".into())),
                    (
                        "html_attrs".into(),
                        json!({
                            "href": "plain",
                            "xlink:href": "linked",
                            "xml:lang": "en",
                            "xmlns:xlink": "http://www.w3.org/1999/xlink"
                        }),
                    ),
                ]),
            )]),
        )]),
    ));
    DocumentValidator::validate(&document, &schema, &limits).unwrap();
    let expected = to_prosemirror_json(&document, &schema);
    let html = to_html(&document, &schema);
    let reparsed = from_html(&html, &schema, &FromHtmlOptions::default()).unwrap();
    let actual = to_prosemirror_json(&reparsed, &schema);
    assert_eq!(actual, expected);
}

#[test]
fn malformed_reserved_opaque_insert_rejects_atomically_before_yrs_execution() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "AB" }]
        }]
    });
    let (doc, schema, limits, editing_limits, document) = diagnostic_doc(&source);
    let malformed = Node::void(
        "__opaque_json".into(),
        HashMap::from([
            ("original_type".into(), Value::String("mystery".into())),
            ("original_json".into(), Value::Null),
            ("opaque_placement".into(), Value::String("inline".into())),
        ]),
    );
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let before = txn.encode_state_as_update_v1(&StateVector::default());
    let error = compile_transaction_with_yrs(
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
            request_id: 172,
            base_document_revision: 0,
            origin: TransactionOrigin::LocalCommand,
            operations: vec![TypedOperation::InsertNode {
                at: point_for_test(1),
                node: malformed,
            }],
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Auto,
        },
        &txn,
        &fragment,
    )
    .unwrap_err();
    assert_eq!(error.code, "DOCUMENT_INVALID");
    assert_eq!(
        txn.encode_state_as_update_v1(&StateVector::default()),
        before
    );
    assert_eq!(
        YrsDocumentCodec::new(&schema, &limits)
            .read_json(&fragment, &txn)
            .unwrap(),
        source
    );
}

#[test]
fn opaque_metadata_depth_and_width_limits_are_exact_and_atomic() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "AB" }]
        }]
    });
    let (doc, schema, limits, editing_limits, document) = diagnostic_doc(&source);
    let compile = |resource_limits: &ResourceLimits, node: Node| {
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        compile_transaction_with_yrs(
            CompilationContext {
                document: &document,
                selection: None,
                schema: &schema,
                resource_limits,
                editing_limits: &editing_limits,
                document_revision: 0,
                max_length: None,
            },
            TypedTransaction {
                request_id: 173,
                base_document_revision: 0,
                origin: TransactionOrigin::LocalCommand,
                operations: vec![TypedOperation::InsertNode {
                    at: point_for_test(1),
                    node,
                }],
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Auto,
            },
            &txn,
            &fragment,
        )
    };
    let nested = || {
        Node::void(
            "__opaque_json".into(),
            HashMap::from([
                ("original_type".into(), Value::String("mystery".into())),
                (
                    "original_json".into(),
                    json!({
                        "type": "mystery",
                        "attrs": { "payload": [[[0]]] }
                    }),
                ),
                ("opaque_placement".into(), Value::String("inline".into())),
            ]),
        )
    };
    let mut exact_depth = limits.clone();
    exact_depth.max_document_depth = 6;
    compile(&exact_depth, nested()).unwrap();
    let txn = doc.transact();
    let before = txn.encode_state_as_update_v1(&StateVector::default());
    drop(txn);
    exact_depth.max_document_depth = 5;
    let error = compile(&exact_depth, nested()).unwrap_err();
    assert_eq!(error.code, "OPERATION_LIMIT_EXCEEDED");
    assert_eq!(error.limit, Some(5));
    assert_eq!(error.actual, Some(6));

    let html_attrs = (0..100)
        .map(|index| (format!("data-{index}"), Value::String("x".into())))
        .collect::<serde_json::Map<_, _>>();
    let wide = || {
        Node::void(
            "__opaque".into(),
            HashMap::from([
                ("html_tag".into(), Value::String("widget-inline".into())),
                ("opaque_placement".into(), Value::String("inline".into())),
                ("html_attrs".into(), Value::Object(html_attrs.clone())),
            ]),
        )
    };
    let mut exact_width = limits.clone();
    exact_width.max_document_nodes = 103;
    compile(&exact_width, wide()).unwrap();
    exact_width.max_document_nodes = 102;
    let error = compile(&exact_width, wide()).unwrap_err();
    assert_eq!(error.code, "OPERATION_LIMIT_EXCEEDED");
    assert_eq!(error.limit, Some(102));
    assert_eq!(error.actual, Some(103));
    let txn = doc.transact();
    assert_eq!(
        txn.encode_state_as_update_v1(&StateVector::default()),
        before
    );
}

#[test]
fn opaque_metadata_max_input_bytes_is_exact_aggregated_and_atomic() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "AB" }]
        }]
    });
    let (doc, schema, limits, editing_limits, document) = diagnostic_doc(&source);
    let escaped_payload = "\u{0001}".repeat(4_096);
    let make_node = || {
        Node::void(
            "__opaque_json".into(),
            HashMap::from([
                ("original_type".into(), Value::String("mystery".into())),
                (
                    "original_json".into(),
                    json!({ "type": "mystery", "attrs": { "payload": escaped_payload } }),
                ),
                ("opaque_placement".into(), Value::String("inline".into())),
            ]),
        )
    };
    let exact_input = {
        let node = make_node();
        node.node_type().len() + serde_json::to_vec(node.attrs()).unwrap().len()
    };
    let compile = |resource_limits: &ResourceLimits| {
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        compile_transaction_with_yrs(
            CompilationContext {
                document: &document,
                selection: None,
                schema: &schema,
                resource_limits,
                editing_limits: &editing_limits,
                document_revision: 0,
                max_length: None,
            },
            TypedTransaction {
                request_id: 174,
                base_document_revision: 0,
                origin: TransactionOrigin::LocalCommand,
                operations: vec![TypedOperation::InsertNode {
                    at: point_for_test(1),
                    node: make_node(),
                }],
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Auto,
            },
            &txn,
            &fragment,
        )
    };
    let initial_scan = {
        let txn = doc.transact();
        document.root().text_content().len() * 2
            + crdt_clock_scan_reservation(174, &txn, limits.max_encoded_state_bytes).unwrap() * 2
    };
    let baseline = compile(&limits).unwrap();
    let envelope_scan = {
        let txn = doc.transact();
        crdt_envelope(174, &txn, limits.max_encoded_state_bytes)
            .unwrap()
            .scan_work
    };
    let exact_total =
        (exact_input + initial_scan).max(baseline.mutation_plan.scan_work + envelope_scan);
    let mut exact_limits = limits.clone();
    exact_limits.max_input_bytes = exact_total;
    let admitted = compile(&exact_limits).unwrap();
    assert!(admitted.mutation_plan.scan_work < exact_total);

    let txn = doc.transact();
    let before = txn.encode_state_as_update_v1(&StateVector::default());
    drop(txn);
    exact_limits.max_input_bytes = exact_total - 1;
    let error = compile(&exact_limits).unwrap_err();
    assert_eq!(error.code, "OPERATION_LIMIT_EXCEEDED");
    assert_eq!(error.limit, Some(u64::try_from(exact_total - 1).unwrap()));
    assert_eq!(error.actual, Some(u64::try_from(exact_total).unwrap()));
    assert_eq!(error.details, Some(json!({ "field": "maxInputBytes" })));
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    assert_eq!(
        txn.encode_state_as_update_v1(&StateVector::default()),
        before
    );
    assert_eq!(
        YrsDocumentCodec::new(&schema, &limits)
            .read_json(&fragment, &txn)
            .unwrap(),
        source
    );
}

#[test]
fn sticky_reverse_mapping_rejects_unknown_wire_element_and_descendant_branches() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{
                "type": "mysteryInline",
                "content": [{ "type": "text", "text": "hidden" }]
            }]
        }]
    });
    let (doc, schema, _, _, _) = diagnostic_doc(&source);
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let XmlOut::Element(paragraph) = fragment.get(&txn, 0).unwrap() else {
        panic!("paragraph expected")
    };
    let XmlOut::Element(unknown) = paragraph.get(&txn, 0).unwrap() else {
        panic!("unknown element expected")
    };
    let XmlOut::Text(hidden) = unknown.get(&txn, 0).unwrap() else {
        panic!("hidden text expected")
    };
    let paragraph_branch = BranchPtr::from(
        <yrs::types::xml::XmlElementRef as AsRef<Branch>>::as_ref(&paragraph),
    );
    for (position, sticky) in [
        (
            1,
            StickyIndex::at(&txn, paragraph_branch, 0, Assoc::Before).unwrap(),
        ),
        (
            2,
            StickyIndex::at(&txn, paragraph_branch, 1, Assoc::Before).unwrap(),
        ),
    ] {
        assert_eq!(
            super::sticky_index_to_doc_pos(&txn, &fragment, &sticky, &schema),
            Some(position)
        );
    }
    for (position, affinity) in [
        (1, Affinity::Before),
        (1, Affinity::After),
        (2, Affinity::Before),
    ] {
        let point =
            super::doc_pos_to_relative_point(&txn, &fragment, position, affinity, &schema).unwrap();
        assert_eq!(point.affinity, affinity);
        assert_eq!(
            super::relative_point_to_doc_pos(&txn, &fragment, &point, &schema),
            Some(position)
        );
    }
    assert!(
        super::doc_pos_to_relative_point(&txn, &fragment, 2, Affinity::After, &schema).is_none()
    );
    for sticky in [
        StickyIndex::at(
            &txn,
            BranchPtr::from(<yrs::types::xml::XmlElementRef as AsRef<Branch>>::as_ref(
                &unknown,
            )),
            0,
            Assoc::After,
        )
        .unwrap(),
        StickyIndex::at(
            &txn,
            BranchPtr::from(<XmlTextRef as AsRef<Branch>>::as_ref(&hidden)),
            1,
            Assoc::After,
        )
        .unwrap(),
    ] {
        assert!(super::sticky_index_to_doc_pos(&txn, &fragment, &sticky, &schema).is_none());
    }
}

#[test]
fn structural_insert_splits_one_marked_unicode_storage_text_exactly() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "A😀e\u{301}Z" }]
        }]
    });
    let (doc, schema, limits, editing_limits, _) = diagnostic_doc(&source);
    {
        let mut txn = doc.transact_mut();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let text = paragraph_text(&fragment, &txn, 0);
        text.format(
            &mut txn,
            3,
            2,
            Attrs::from([(Arc::<str>::from("bold"), Any::Bool(true))]),
        );
    }
    let codec = YrsDocumentCodec::new(&schema, &limits);
    let (document, original_id, before_full_len) = {
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let json = codec.read_json(&fragment, &txn).unwrap();
        (
            from_prosemirror_json(&json, &schema, UnknownTypeMode::Preserve).unwrap(),
            <XmlTextRef as AsRef<Branch>>::as_ref(&paragraph_text(&fragment, &txn, 0)).id(),
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
                request_id: 113,
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
    assert!(matches!(
        compiled.mutation_plan.actions.first(),
        Some(YrsMutationAction::DeleteText {
            index_utf16: 3,
            len_utf16: 3,
            ..
        })
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
    assert_eq!(children[0].id(), original_id);
    assert_ne!(children[2].id(), original_id);
    assert_eq!(codec.read_json(&fragment, &txn).unwrap(), expected);
    assert_eq!(expected["content"][0]["content"][0]["text"], "A😀");
    assert_eq!(expected["content"][0]["content"][2]["text"], "e\u{301}");
    assert_eq!(
        expected["content"][0]["content"][2]["marks"][0]["type"],
        "bold"
    );
    let update_len = update.len();
    assert!(update_len <= estimate, "{update_len} > {estimate}");
    let after_full_len = txn.encode_state_as_update_v1(&StateVector::default()).len();
    assert!(after_full_len <= before_full_len + estimate);
}

#[test]
fn structural_replace_swaps_an_inline_void_for_text_at_the_same_index() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "hardBreak" }]
        }]
    });

    let (actual, expected, html, update_len, estimate) = compile_and_execute(
        source,
        vec![TypedOperation::ReplaceRange {
            range: range_for_test(0, 1),
            content: Fragment::from(vec![Node::text("x".into(), vec![])]),
        }],
    );

    assert_eq!(actual, expected);
    assert_eq!(actual["content"][0]["content"][0]["text"], "x");
    assert!(html.contains(">x<"));
    assert!(update_len > 0);
    assert!(update_len <= estimate);
}

#[test]
fn structurally_identical_replace_is_a_document_no_op() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "hardBreak" }]
        }]
    });
    let (doc, schema, limits, editing_limits, document) = diagnostic_doc(&source);
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
                request_id: 116,
                base_document_revision: 0,
                origin: TransactionOrigin::LocalInput,
                operations: vec![TypedOperation::ReplaceRange {
                    range: range_for_test(0, 1),
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
    assert!(compiled.mutation_plan.actions.is_empty());
    assert_eq!(compiled.encoded_growth_bound, 0);
    assert_eq!(compiled.undo_units_bound, 0);
}

#[test]
fn update_node_attrs_toggles_and_removes_task_item_default() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "taskList",
            "content": [{
                "type": "taskItem",
                "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "todo" }] }]
            }]
        }]
    });
    let (actual, expected) = compile_and_execute_attribute_update(
        source,
        HashMap::from([("checked".into(), Value::Bool(true))]),
    );
    assert_eq!(actual, expected);
    assert_eq!(actual["content"][0]["content"][0]["attrs"]["checked"], true);

    let checked_source = json!({
        "type": "doc",
        "content": [{
            "type": "taskList",
            "content": [{
                "type": "taskItem",
                "attrs": { "checked": true },
                "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "done" }] }]
            }]
        }]
    });
    let (removed, removed_expected) =
        compile_and_execute_attribute_update(checked_source, HashMap::new());
    assert_eq!(removed, removed_expected);
    assert!(removed["content"][0]["content"][0]["attrs"]["checked"].is_null());
}

#[test]
fn update_node_attrs_sets_and_removes_image_attributes() {
    let source = json!({
        "type": "doc",
        "content": [{ "type": "image", "attrs": { "src": "old", "alt": "old alt" } }]
    });
    let (actual, expected) = compile_and_execute_attribute_update(
        source,
        HashMap::from([("src".into(), Value::String("new".into()))]),
    );
    assert_eq!(actual, expected);
    assert_eq!(actual["content"][0]["attrs"]["src"], "new");
    assert!(actual["content"][0]["attrs"]["alt"].is_null());
}

#[test]
fn update_node_attrs_preserves_nested_custom_any_values() {
    let source = json!({
        "type": "doc",
        "content": [{ "type": "customBlock", "attrs": { "old": true } }]
    });
    let attrs = HashMap::from([
        ("flag".into(), Value::Bool(true)),
        ("count".into(), json!(7)),
        ("label".into(), Value::String("custom".into())),
        ("items".into(), json!([1, false, "x"])),
        ("meta".into(), json!({ "nested": { "ok": true } })),
    ]);
    let (actual, expected) = compile_and_execute_attribute_update(source, attrs);
    assert_eq!(actual, expected);
    assert_eq!(
        actual["content"][0]["attrs"]["items"],
        json!([1, false, "x"])
    );
    assert_eq!(
        actual["content"][0]["attrs"]["meta"],
        json!({ "nested": { "ok": true } })
    );
}

#[test]
fn update_node_attrs_normalizes_sequential_same_key_changes() {
    let source = json!({
        "type": "doc",
        "content": [{ "type": "customBlock", "attrs": { "old": true } }]
    });
    let (_, _, _, compiled) = compile_attribute_operations(
        source.clone(),
        vec![
            HashMap::from([("label".into(), Value::String("first".into()))]),
            HashMap::from([("label".into(), Value::String("final".into()))]),
        ],
    );
    assert_eq!(compiled.mutation_plan.actions.len(), 2);
    assert!(matches!(
        compiled.mutation_plan.actions.as_slice(),
        [
            YrsMutationAction::SetXmlAttribute { key, value: Any::String(value), .. },
            YrsMutationAction::RemoveXmlAttribute { key: removed, .. }
        ] if key.as_ref() == "label" && value.as_ref() == "final" && removed.as_ref() == "old"
    ));

    let (_, _, _, removed) = compile_attribute_operations(
        source,
        vec![
            HashMap::from([("label".into(), Value::String("temporary".into()))]),
            HashMap::new(),
        ],
    );
    assert!(matches!(
        removed.mutation_plan.actions.as_slice(),
        [YrsMutationAction::RemoveXmlAttribute { key, .. }] if key.as_ref() == "old"
    ));
}

#[test]
fn update_node_attrs_identical_map_is_a_complete_no_op() {
    let source = json!({
        "type": "doc",
        "content": [{ "type": "customBlock", "attrs": { "flag": true } }]
    });
    let (_, _, _, compiled) = compile_attribute_operations(
        source,
        vec![HashMap::from([("flag".into(), Value::Bool(true))])],
    );
    assert!(compiled.mutation_plan.actions.is_empty());
    assert_eq!(compiled.encoded_growth_bound, 0);
    assert_eq!(compiled.undo_units_bound, 0);
}

#[test]
fn update_node_attrs_rejects_stale_same_count_attribute_substitution_atomically() {
    let source = json!({
        "type": "doc",
        "content": [{ "type": "image", "attrs": { "src": "old", "alt": "old alt" } }]
    });
    let (doc, _, _, compiled) = compile_attribute_operations(
        source,
        vec![HashMap::from([("src".into(), Value::String("new".into()))])],
    );
    {
        let mut txn = doc.transact_mut();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let XmlOut::Element(image) = fragment.get(&txn, 0).unwrap() else {
            panic!("expected image")
        };
        image.insert_attribute(&mut txn, "src", Any::String("raced".into()));
    }
    let before = doc
        .transact()
        .encode_state_as_update_v1(&StateVector::default());
    let error = {
        let txn = doc.transact();
        preflight_mutation_plan(118, &compiled.mutation_plan, &txn).unwrap_err()
    };
    assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
    assert_eq!(
        doc.transact()
            .encode_state_as_update_v1(&StateVector::default()),
        before
    );
}

#[test]
fn update_node_attrs_keeps_heading_synthetic_level_unchanged() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "h2",
            "attrs": { "id": "old" },
            "content": [{ "type": "text", "text": "Heading" }]
        }]
    });
    let (_, _, _, compiled) = compile_attribute_operations(
        source,
        vec![HashMap::from([("id".into(), Value::String("new".into()))])],
    );
    assert!(matches!(
        compiled.mutation_plan.actions.as_slice(),
        [YrsMutationAction::SetXmlAttribute { key, .. }] if key.as_ref() == "id"
    ));
}

#[test]
fn update_node_attrs_rejects_ambiguous_attrless_target() {
    let source = json!({
        "type": "doc",
        "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "x" }] }]
    });
    let (doc, schema, limits, editing_limits, document) =
        diagnostic_doc_with_schema(&source, attribute_schema());
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let error = compile_transaction_with_yrs(
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
            request_id: 119,
            base_document_revision: 0,
            origin: TransactionOrigin::LocalInput,
            operations: vec![TypedOperation::UpdateNodeAttrs {
                at: point_for_test(0),
                attrs: HashMap::new(),
            }],
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Auto,
        },
        &txn,
        &fragment,
    )
    .unwrap_err();
    assert_eq!(error.code, "POSITION_INVALID");
    assert_eq!(error.details.as_ref().unwrap()["field"], "at");
}

fn compile_and_execute_attribute_update(
    source: Value,
    attrs: HashMap<String, Value>,
) -> (Value, Value) {
    let (doc, schema, limits, editing_limits, document) =
        diagnostic_doc_with_schema(&source, attribute_schema());
    let (before_ids, before_full_len, sticky) = {
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let sticky = first_xml_text(&fragment, &txn).and_then(|text| {
            StickyIndex::at(
                &txn,
                BranchPtr::from(<XmlTextRef as AsRef<Branch>>::as_ref(&text)),
                0,
                Assoc::After,
            )
        });
        (
            collect_xml_ids(&fragment, &txn),
            txn.encode_state_as_update_v1(&StateVector::default()).len(),
            sticky,
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
                request_id: 117,
                base_document_revision: 0,
                origin: TransactionOrigin::LocalInput,
                operations: vec![TypedOperation::UpdateNodeAttrs {
                    at: point_for_test(0),
                    attrs,
                }],
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Auto,
            },
            &txn,
            &fragment,
        )
        .unwrap()
    };
    let keys = compiled
        .mutation_plan
        .actions
        .iter()
        .filter_map(|action| match action {
            YrsMutationAction::SetXmlAttribute { key, .. }
            | YrsMutationAction::RemoveXmlAttribute { key, .. } => Some(key.as_ref()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(keys.windows(2).all(|pair| pair[0] < pair[1]));
    assert_eq!(keys.len(), compiled.mutation_plan.actions.len());
    {
        let txn = doc.transact();
        let preflight =
            preflight_mutation_work_for_test(117, &compiled.mutation_plan, &txn).unwrap();
        let exact = compiled.mutation_plan.compilation_work_for_test() + preflight;
        compiled.mutation_plan.set_work_limit_for_test(exact);
        preflight_mutation_plan(117, &compiled.mutation_plan, &txn).unwrap();
        compiled.mutation_plan.set_work_limit_for_test(exact - 1);
        assert_eq!(
            preflight_mutation_plan(117, &compiled.mutation_plan, &txn)
                .unwrap_err()
                .code,
            "OPERATION_LIMIT_EXCEEDED"
        );
        compiled.mutation_plan.set_work_limit_for_test(exact);
    }
    let expected = to_prosemirror_json(&compiled.preview, &schema);
    let estimate = compiled.encoded_growth_bound;
    let has_actions = !compiled.mutation_plan.actions.is_empty();
    let update = if has_actions {
        let mut txn = doc.transact_mut();
        execute_mutation_plan(compiled.mutation_plan, &mut txn);
        txn.commit();
        txn.encode_update_v1()
    } else {
        Vec::new()
    };
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    assert_eq!(collect_xml_ids(&fragment, &txn), before_ids);
    if let Some(sticky) = sticky {
        assert!(sticky.get_offset(&txn).is_some());
    }
    let update_len = update.len();
    assert!(update_len <= estimate, "{update_len} > {estimate}");
    let after_full_len = txn.encode_state_as_update_v1(&StateVector::default()).len();
    assert!(after_full_len <= before_full_len + estimate);
    (
        YrsDocumentCodec::new(&schema, &limits)
            .read_json(&fragment, &txn)
            .unwrap(),
        expected,
    )
}

fn compile_attribute_operations(
    source: Value,
    updates: Vec<HashMap<String, Value>>,
) -> (
    Doc,
    crate::schema::Schema,
    ResourceLimits,
    CompiledTransaction,
) {
    let (doc, schema, limits, editing_limits, document) =
        diagnostic_doc_with_schema(&source, attribute_schema());
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
                request_id: 118,
                base_document_revision: 0,
                origin: TransactionOrigin::LocalInput,
                operations: updates
                    .into_iter()
                    .map(|attrs| TypedOperation::UpdateNodeAttrs {
                        at: point_for_test(0),
                        attrs,
                    })
                    .collect(),
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Auto,
            },
            &txn,
            &fragment,
        )
        .unwrap()
    };
    (doc, schema, limits, compiled)
}

fn collect_xml_ids<T: ReadTxn>(
    fragment: &yrs::types::xml::XmlFragmentRef,
    txn: &T,
) -> Vec<yrs::branch::BranchID> {
    fn visit<T: ReadTxn>(out: XmlOut, txn: &T, ids: &mut Vec<yrs::branch::BranchID>) {
        ids.push(out.id());
        match out {
            XmlOut::Element(element) => {
                for child in element.children(txn) {
                    visit(child, txn, ids);
                }
            }
            XmlOut::Fragment(fragment) => {
                for child in fragment.children(txn) {
                    visit(child, txn, ids);
                }
            }
            XmlOut::Text(_) => {}
        }
    }
    let mut ids = Vec::new();
    for child in fragment.children(txn) {
        visit(child, txn, &mut ids);
    }
    ids
}

fn first_xml_text<T: ReadTxn>(
    fragment: &yrs::types::xml::XmlFragmentRef,
    txn: &T,
) -> Option<XmlTextRef> {
    fn visit<T: ReadTxn>(out: XmlOut, txn: &T) -> Option<XmlTextRef> {
        match out {
            XmlOut::Text(text) => Some(text),
            XmlOut::Element(element) => element.children(txn).find_map(|child| visit(child, txn)),
            XmlOut::Fragment(fragment) => {
                fragment.children(txn).find_map(|child| visit(child, txn))
            }
        }
    }
    fragment.children(txn).find_map(|child| visit(child, txn))
}
