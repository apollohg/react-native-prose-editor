use super::*;

#[test]
fn preflight_rejects_same_utf16_length_text_or_mark_changes() {
    fn compiled_insert(
        source: &Value,
    ) -> (
        Doc,
        crate::schema::Schema,
        ResourceLimits,
        CompiledTransaction,
    ) {
        let (doc, schema, limits, editing_limits, document) = diagnostic_doc(source);
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
                    request_id: 124,
                    base_document_revision: 0,
                    origin: TransactionOrigin::LocalInput,
                    operations: vec![TypedOperation::InsertText {
                        at: point_for_test(1),
                        text: "!".into(),
                        marks: vec![],
                    }],
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

    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "ab" }]
        }]
    });
    let (text_doc, _, _, text_compiled) = compiled_insert(&source);
    {
        let mut txn = text_doc.transact_mut();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let text = paragraph_text(&fragment, &txn, 0);
        text.remove_range(&mut txn, 0, 1);
        text.insert(&mut txn, 0, "z");
    }
    let txn = text_doc.transact();
    assert_eq!(
        preflight_mutation_plan(124, &text_compiled.mutation_plan, &txn)
            .unwrap_err()
            .code,
        "ENGINE_INVARIANT_FAILED"
    );

    let (mark_doc, _, _, mark_compiled) = compiled_insert(&source);
    {
        let mut txn = mark_doc.transact_mut();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let text = paragraph_text(&fragment, &txn, 0);
        text.format(
            &mut txn,
            0,
            1,
            Attrs::from([(Arc::<str>::from("bold"), Any::Bool(true))]),
        );
    }
    let txn = mark_doc.transact();
    assert_eq!(
        preflight_mutation_plan(124, &mark_compiled.mutation_plan, &txn)
            .unwrap_err()
            .code,
        "ENGINE_INVARIANT_FAILED"
    );
}

#[test]
fn crdt_envelope_bounds_fragmented_text_and_deep_xml_actual_costs() {
    fn id_set_units(set: &yrs::IdSet) -> u64 {
        set.iter()
            .flat_map(|(_, ranges)| ranges.into_iter())
            .map(|range| u64::from(range.end - range.start))
            .sum()
    }

    fn assert_actual_costs_bounded(
        source: Value,
        operations: Vec<TypedOperation>,
        schema: crate::schema::Schema,
        expect_legacy_underbound: bool,
    ) {
        let limits = ResourceLimits::default();
        let editing_limits = EditingLimits::default();
        let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
        let doc = utf16_doc();
        let codec = YrsDocumentCodec::new(&schema, &limits);
        {
            let mut txn = doc.transact_mut();
            let fragment = txn.get_or_insert_xml_fragment("prosemirror");
            codec
                .apply_json(&fragment, &mut txn, &json!({ "type": "doc" }), &source)
                .unwrap();
        }
        // Populate unrelated live and disjoint deleted clocks from independent
        // clients. They are outside the editor fragment but part of the same
        // transaction-wide Yrs snapshot envelope.
        for client in 10..14u64 {
            let auxiliary = Doc::with_client_id(client);
            let update = {
                let text = auxiliary.get_or_insert_text(format!("aux-{client}"));
                let mut txn = auxiliary.transact_mut();
                text.insert(&mut txn, 0, "abcdef");
                text.remove_range(&mut txn, 1, 1);
                text.remove_range(&mut txn, 3, 1);
                drop(txn);
                auxiliary
                    .transact()
                    .encode_state_as_update_v1(&StateVector::default())
            };
            doc.transact_mut()
                .apply_update(Update::decode_v1(&update).unwrap())
                .unwrap();
        }
        let before_full_len = doc
            .transact()
            .encode_state_as_update_v1(&StateVector::default())
            .len();
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
                    request_id: 125,
                    base_document_revision: 0,
                    origin: TransactionOrigin::LocalCommand,
                    operations,
                    selection_intent: SelectionIntent::UseOperationResult,
                    history_policy: HistoryPolicy::Auto,
                },
                &txn,
                &fragment,
            )
            .unwrap()
        };
        let legacy_visible_delete_bound =
            compiled
                .mutation_plan
                .actions
                .iter()
                .fold(0u64, |units, action| match action {
                    YrsMutationAction::DeleteXmlChildren { child_count, .. } => {
                        units + u64::from(*child_count)
                    }
                    YrsMutationAction::DeleteText { len_utf16, .. } => {
                        units + u64::from(*len_utf16)
                    }
                    _ => units,
                });
        let fragment = doc.transact().get_xml_fragment("prosemirror").unwrap();
        let mut undo = UndoManager::<()>::new();
        undo.expand_scope(&doc, &fragment);
        let actual_update = {
            let mut txn = doc.transact_mut();
            execute_mutation_plan(compiled.mutation_plan, &mut txn);
            txn.commit();
            txn.encode_update_v1()
        };
        assert!(actual_update.len() <= compiled.encoded_growth_bound);
        let after_full_len = doc
            .transact()
            .encode_state_as_update_v1(&StateVector::default())
            .len();
        assert!(after_full_len <= before_full_len + compiled.encoded_growth_bound);
        let item = undo.undo_stack().last().expect("mutation must be undoable");
        let actual_insertions = id_set_units(item.insertions());
        let actual_deletions = id_set_units(item.deletions());
        let actual_undo = actual_insertions + actual_deletions;
        if expect_legacy_underbound {
            assert!(
                actual_deletions > legacy_visible_delete_bound,
                "{actual_deletions} <= legacy bound {legacy_visible_delete_bound}"
            );
        } else {
            assert!(actual_insertions >= 6, "{actual_insertions}");
        }
        assert!(
            actual_undo <= compiled.undo_units_bound,
            "actual undo units {actual_undo} exceed bound {}",
            compiled.undo_units_bound
        );
    }

    let rich_runs = (0..256)
        .map(|index| {
            json!({
                "type": "text",
                "text": if index % 2 == 0 { "a" } else { "b" },
                "marks": [{ "type": if index % 2 == 0 { "bold" } else { "italic" } }]
            })
        })
        .collect::<Vec<_>>();
    let fragmented = json!({
        "type": "doc",
        "content": [
            { "type": "paragraph", "content": [{ "type": "text", "text": "LEFT" }] },
            { "type": "paragraph", "content": rich_runs },
            { "type": "paragraph", "content": [{ "type": "text", "text": "RIGHT" }] }
        ]
    });
    let schema = tiptap_schema();
    let semantic = from_prosemirror_json(&fragmented, &schema, UnknownTypeMode::Preserve).unwrap();
    let rendered = crate::render::rendered_text(&semantic, &schema);
    let from = u32::try_from(rendered.find("LEFT").unwrap() + 2).unwrap();
    let to = u32::try_from(rendered.find("RIGHT").unwrap() + 2).unwrap();
    assert_actual_costs_bounded(
        fragmented,
        vec![TypedOperation::DeleteRange {
            range: range_for_test(from, to),
        }],
        schema,
        true,
    );

    let prepared_emoji = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "🙂" }]
        }]
    });
    assert_actual_costs_bounded(
        prepared_emoji,
        vec![TypedOperation::WrapInList {
            range: range_for_test(0, 1),
            list_type: "bulletList".into(),
            attrs: HashMap::new(),
            item_type: "listItem".into(),
            item_attrs: HashMap::new(),
        }],
        tiptap_schema(),
        false,
    );
}

#[test]
fn pure_insert_skips_but_plain_mark_add_requires_a_snapshot_envelope() {
    let source = json!({
        "type": "doc",
        "content": (0..256)
            .map(|_| json!({
                "type": "paragraph",
                "content": [{ "type": "text", "text": "abcdefgh" }]
            }))
            .collect::<Vec<_>>()
    });
    let (doc, schema, limits, editing_limits, document) = diagnostic_doc(&source);
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let compiled = compile_transaction_with_yrs(
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
            request_id: 126,
            base_document_revision: 0,
            origin: TransactionOrigin::LocalInput,
            operations: vec![
                TypedOperation::InsertText {
                    at: point_for_test(1),
                    text: "!".into(),
                    marks: vec![],
                },
                TypedOperation::AddMark {
                    range: range_for_test(2, 4),
                    mark: Mark::new("bold".into(), HashMap::new()),
                },
            ],
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Auto,
        },
        &txn,
        &fragment,
    )
    .unwrap();
    assert!(compiled.mutation_plan.requires_crdt_envelope());

    let insert_only = compile_transaction_with_yrs(
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
            request_id: 127,
            base_document_revision: 0,
            origin: TransactionOrigin::LocalInput,
            operations: vec![TypedOperation::InsertText {
                at: point_for_test(1),
                text: "!".into(),
                marks: vec![],
            }],
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Auto,
        },
        &txn,
        &fragment,
    )
    .unwrap();
    assert!(!insert_only.mutation_plan.requires_crdt_envelope());
}

#[test]
fn pending_crdt_state_rejects_local_compilation_atomically() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "ready" }]
        }]
    });
    let (doc, schema, limits, editing_limits, document) = diagnostic_doc(&source);
    let remote = Doc::with_client_id(77);
    let remote_text = remote.get_or_insert_text("missing-prefix");
    {
        let mut txn = remote.transact_mut();
        remote_text.insert(&mut txn, 0, "a");
    }
    let suffix_update = {
        let mut txn = remote.transact_mut();
        remote_text.insert(&mut txn, 1, "b");
        txn.commit();
        txn.encode_update_v1()
    };
    doc.transact_mut()
        .apply_update(Update::decode_v1(&suffix_update).unwrap())
        .unwrap();
    let txn = doc.transact();
    assert!(txn.store().pending_update().is_some());
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let before = txn.state_vector();
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
            request_id: 128,
            base_document_revision: 0,
            origin: TransactionOrigin::LocalInput,
            operations: vec![TypedOperation::InsertText {
                at: point_for_test(1),
                text: "!".into(),
                marks: vec![],
            }],
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Auto,
        },
        &txn,
        &fragment,
    )
    .unwrap_err();
    assert_eq!(error.code, "ENGINE_NOT_READY");
    assert_eq!(txn.state_vector(), before);
}

#[test]
fn document_guard_rejects_pending_crdt_state_before_snapshot_validation() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "ready" }]
        }]
    });
    let (doc, _schema, _limits, compiled) = compile_operations_with_schema(
        &source,
        vec![TypedOperation::InsertText {
            at: point_for_test(1),
            text: "!".into(),
            marks: vec![],
        }],
        tiptap_schema(),
    );
    let remote = Doc::with_client_id(78);
    let remote_text = remote.get_or_insert_text("missing-prefix");
    {
        let mut txn = remote.transact_mut();
        remote_text.insert(&mut txn, 0, "a");
    }
    let suffix_update = {
        let mut txn = remote.transact_mut();
        remote_text.insert(&mut txn, 1, "b");
        txn.commit();
        txn.encode_update_v1()
    };
    doc.transact_mut()
        .apply_update(Update::decode_v1(&suffix_update).unwrap())
        .unwrap();
    let txn = doc.transact();
    assert!(txn.store().pending_update().is_some());
    let before = txn.encode_state_as_update_v1(&StateVector::default());
    let error = preflight_mutation_plan(178, &compiled.mutation_plan, &txn).unwrap_err();
    assert_eq!(error.code, "ENGINE_NOT_READY");
    assert_eq!(
        txn.encode_state_as_update_v1(&StateVector::default()),
        before
    );
}

#[test]
fn tombstone_scan_reservation_is_exact_and_compiler_charges_it() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "a" }]
        }]
    });
    let (doc, schema, limits, editing_limits, document) = diagnostic_doc(&source);
    {
        let mut txn = doc.transact_mut();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let text = paragraph_text(&fragment, &txn, 0);
        for _ in 0..512 {
            text.insert(&mut txn, 1, "x");
            text.remove_range(&mut txn, 1, 1);
        }
    }
    let txn = doc.transact();
    let exact_clock_work = crdt_clock_scan_reservation(129, &txn, usize::MAX).unwrap();
    assert!(exact_clock_work > 512);
    assert_eq!(
        crdt_clock_scan_reservation(129, &txn, exact_clock_work - 1)
            .unwrap_err()
            .code,
        "OPERATION_LIMIT_EXCEEDED"
    );
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let compiled = compile_transaction_with_yrs(
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
            request_id: 129,
            base_document_revision: 0,
            origin: TransactionOrigin::LocalInput,
            operations: vec![TypedOperation::InsertText {
                at: point_for_test(1),
                text: "!".into(),
                marks: vec![],
            }],
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Auto,
        },
        &txn,
        &fragment,
    )
    .unwrap();
    assert!(compiled.mutation_plan.scan_work >= exact_clock_work * 2);

    let compile_delete = |resource_limits: &ResourceLimits| {
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
                request_id: 131,
                base_document_revision: 0,
                origin: TransactionOrigin::LocalInput,
                operations: vec![TypedOperation::DeleteRange {
                    range: range_for_test(0, 1),
                }],
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Auto,
            },
            &txn,
            &fragment,
        )
    };
    let deletion = compile_delete(&limits).unwrap();
    assert!(deletion.mutation_plan.requires_crdt_envelope());
    let envelope = crdt_envelope(131, &txn, limits.max_encoded_state_bytes).unwrap();
    let exact_scan_limit = deletion
        .mutation_plan
        .scan_work
        .checked_add(envelope.scan_work)
        .unwrap();
    let mut exact_limits = limits.clone();
    exact_limits.max_input_bytes = exact_scan_limit;
    compile_delete(&exact_limits).unwrap();
    exact_limits.max_input_bytes = exact_scan_limit - 1;
    let one_under = compile_delete(&exact_limits).unwrap_err();
    assert_eq!(one_under.code, "OPERATION_LIMIT_EXCEEDED");
    assert_eq!(
        one_under.limit,
        Some(u64::try_from(exact_scan_limit - 1).unwrap())
    );
    assert_eq!(
        one_under.actual,
        Some(u64::try_from(exact_scan_limit).unwrap())
    );
}

#[test]
fn fully_deleted_clients_and_hidden_format_cleanup_are_bounded() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "ab" }]
        }]
    });
    let (doc, schema, limits, editing_limits, document) = diagnostic_doc(&source);
    for client in 200..204u64 {
        let remote = Doc::with_client_id(client);
        let text = remote.get_or_insert_text(format!("deleted-{client}"));
        {
            let mut txn = remote.transact_mut();
            text.insert(&mut txn, 0, "x");
            text.remove_range(&mut txn, 0, 1);
        }
        let update = remote
            .transact()
            .encode_state_as_update_v1(&StateVector::default());
        doc.transact_mut()
            .apply_update(Update::decode_v1(&update).unwrap())
            .unwrap();
    }
    // Add an adjacent true/null format pair at a zero-width gap. It is
    // semantically invisible but a later full-span format must clean it up.
    {
        let mut txn = doc.transact_mut();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let text = paragraph_text(&fragment, &txn, 0);
        text.format(
            &mut txn,
            1,
            0,
            Attrs::from([(Arc::<str>::from("bold"), Any::Bool(true))]),
        );
    }
    let envelope = crdt_envelope(130, &doc.transact(), limits.max_encoded_state_bytes).unwrap();
    assert!(envelope.client_count >= 5);
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
                request_id: 130,
                base_document_revision: 0,
                origin: TransactionOrigin::LocalInput,
                operations: vec![TypedOperation::AddMark {
                    range: range_for_test(0, 2),
                    mark: Mark::new("bold".into(), HashMap::new()),
                }],
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Auto,
            },
            &txn,
            &fragment,
        )
        .unwrap()
    };
    assert!(compiled.mutation_plan.requires_crdt_envelope());
    let before_full_len = doc
        .transact()
        .encode_state_as_update_v1(&StateVector::default())
        .len();
    let fragment = doc.transact().get_xml_fragment("prosemirror").unwrap();
    let mut undo = UndoManager::<()>::new();
    undo.expand_scope(&doc, &fragment);
    let tx_update = {
        let mut txn = doc.transact_mut();
        execute_mutation_plan(compiled.mutation_plan, &mut txn);
        txn.commit();
        txn.encode_update_v1()
    };
    assert!(tx_update.len() <= compiled.encoded_growth_bound);
    let after_full_len = doc
        .transact()
        .encode_state_as_update_v1(&StateVector::default())
        .len();
    assert!(after_full_len <= before_full_len + compiled.encoded_growth_bound);
    let item = undo.undo_stack().last().unwrap();
    let actual = item
        .deletions()
        .iter()
        .flat_map(|(_, ranges)| ranges.into_iter())
        .map(|range| u64::from(range.end - range.start))
        .sum::<u64>();
    assert!(actual > 0, "formatting should clean up hidden CRDT items");
    assert!(actual <= compiled.undo_units_bound);
}

#[test]
fn structural_delete_preflight_rejects_same_count_child_substitution_atomically() {
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
                request_id: 111,
                base_document_revision: 0,
                origin: TransactionOrigin::LocalInput,
                operations: vec![TypedOperation::DeleteRange {
                    range: range_for_test(0, 1),
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
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let XmlOut::Element(paragraph) = fragment.get(&txn, 0).unwrap() else {
            panic!("expected paragraph")
        };
        paragraph.remove_range(&mut txn, 0, 1);
        paragraph.insert(&mut txn, 0, XmlElementPrelim::empty("hardBreak"));
    }
    let txn = doc.transact();
    let before = txn.state_vector();
    let error = preflight_mutation_plan(111, &compiled.mutation_plan, &txn).unwrap_err();
    assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
    assert_eq!(txn.state_vector(), before);
}

#[test]
fn pure_insert_preflight_rejects_same_count_parent_substitution_atomically() {
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
                request_id: 118,
                base_document_revision: 0,
                origin: TransactionOrigin::LocalCommand,
                operations: vec![TypedOperation::InsertNode {
                    at: point_for_test(0),
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
        compiled.mutation_plan.actions.as_slice(),
        [YrsMutationAction::InsertXmlChildren { .. }]
    ));
    {
        let mut txn = doc.transact_mut();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        fragment.remove_range(&mut txn, 0, 1);
        let paragraph = fragment.insert(&mut txn, 0, XmlElementPrelim::empty("paragraph"));
        paragraph.insert(&mut txn, 0, XmlElementPrelim::empty("hardBreak"));
    }
    let txn = doc.transact();
    let before = txn.encode_state_as_update_v1(&StateVector::default());
    let error = preflight_mutation_plan(118, &compiled.mutation_plan, &txn).unwrap_err();
    assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
    assert_eq!(
        txn.encode_state_as_update_v1(&StateVector::default()),
        before
    );
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    assert_eq!(
        YrsDocumentCodec::new(&schema, &limits)
            .read_json(&fragment, &txn)
            .unwrap(),
        source
    );
}

#[test]
fn nested_text_and_attribute_preflight_reject_gc_replaced_parents_without_panicking() {
    let text_source = json!({
        "type": "doc",
        "content": [{
            "type": "blockquote",
            "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "abc" }] }]
        }]
    });
    let (text_doc, schema, limits, editing_limits, document) = diagnostic_doc(&text_source);
    let mut text_compiled = {
        let txn = text_doc.transact();
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
                request_id: 119,
                base_document_revision: 0,
                origin: TransactionOrigin::LocalCommand,
                operations: vec![TypedOperation::InsertText {
                    at: point_for_test(1),
                    text: "X".into(),
                    marks: vec![],
                }],
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Auto,
            },
            &txn,
            &fragment,
        )
        .unwrap()
    };
    let text_exact = text_compiled.mutation_plan.compilation_work_for_test()
        + text_compiled
            .mutation_plan
            .expected_preflight_work_for_test();
    {
        let mut txn = text_doc.transact_mut();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        fragment.remove_range(&mut txn, 0, 1);
        let quote = fragment.insert(&mut txn, 0, XmlElementPrelim::empty("blockquote"));
        let paragraph = quote.insert(&mut txn, 0, XmlElementPrelim::empty("paragraph"));
        paragraph.insert(&mut txn, 0, XmlTextPrelim::new("abc"));
    }
    let txn = text_doc.transact();
    let before = txn.encode_state_as_update_v1(&StateVector::default());
    text_compiled
        .mutation_plan
        .set_work_limit_for_test(text_exact - 1);
    assert_eq!(
        preflight_mutation_plan(119, &text_compiled.mutation_plan, &txn)
            .unwrap_err()
            .code,
        "OPERATION_LIMIT_EXCEEDED"
    );
    text_compiled
        .mutation_plan
        .set_work_limit_for_test(text_exact);
    assert_eq!(
        preflight_mutation_plan(119, &text_compiled.mutation_plan, &txn)
            .unwrap_err()
            .code,
        "ENGINE_INVARIANT_FAILED"
    );
    assert_eq!(
        txn.encode_state_as_update_v1(&StateVector::default()),
        before
    );

    let attr_source = json!({
        "type": "doc",
        "content": [{
            "type": "blockquote",
            "content": [{
                "type": "h2",
                "attrs": { "id": "old" },
                "content": [{ "type": "text", "text": "heading" }]
            }]
        }]
    });
    let (attr_doc, attr_schema, attr_limits, attr_editing, attr_document) =
        diagnostic_doc_with_schema(&attr_source, attribute_schema());
    let attr_compiled = {
        let txn = attr_doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        compile_transaction_with_yrs(
            CompilationContext {
                document: &attr_document,
                selection: None,
                schema: &attr_schema,
                resource_limits: &attr_limits,
                editing_limits: &attr_editing,
                document_revision: 0,
                max_length: None,
            },
            TypedTransaction {
                request_id: 120,
                base_document_revision: 0,
                origin: TransactionOrigin::LocalCommand,
                operations: vec![TypedOperation::UpdateNodeAttrs {
                    at: point_for_test(1),
                    attrs: HashMap::from([("id".into(), Value::String("new".into()))]),
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
        let mut txn = attr_doc.transact_mut();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        fragment.remove_range(&mut txn, 0, 1);
        let quote = fragment.insert(&mut txn, 0, XmlElementPrelim::empty("blockquote"));
        let heading = quote.insert(&mut txn, 0, XmlElementPrelim::empty("heading"));
        heading.insert_attribute(&mut txn, "level", Any::BigInt(2));
        heading.insert_attribute(&mut txn, "id", Any::String("old".into()));
        heading.insert(&mut txn, 0, XmlTextPrelim::new("heading"));
    }
    let txn = attr_doc.transact();
    let before = txn.encode_state_as_update_v1(&StateVector::default());
    assert_eq!(
        preflight_mutation_plan(120, &attr_compiled.mutation_plan, &txn)
            .unwrap_err()
            .code,
        "ENGINE_INVARIANT_FAILED"
    );
    assert_eq!(
        txn.encode_state_as_update_v1(&StateVector::default()),
        before
    );
}

#[test]
fn create_text_preflight_rejects_same_count_neighbor_replacement() {
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
                request_id: 86,
                base_document_revision: 0,
                origin: TransactionOrigin::LocalInput,
                operations: vec![TypedOperation::InsertText {
                    at: point_for_test(0),
                    text: "x".into(),
                    marks: vec![],
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
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let XmlOut::Element(parent) = fragment.get(&txn, 0).unwrap() else {
            panic!("paragraph expected")
        };
        parent.remove_range(&mut txn, 0, 1);
        parent.insert(&mut txn, 0, XmlElementPrelim::empty("hardBreak"));
    }
    let txn = doc.transact();
    let before = txn.state_vector();
    let error = preflight_mutation_plan(86, &compiled.mutation_plan, &txn).unwrap_err();
    assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
    assert_eq!(txn.state_vector(), before);
}

#[test]
fn nested_empty_textblock_create_preserves_unaffected_sibling_identity_and_sticky() {
    let source = json!({
        "type": "doc",
        "content": [
            {
                "type": "bulletList",
                "content": [{
                    "type": "listItem",
                    "content": [{ "type": "paragraph" }]
                }]
            },
            {
                "type": "paragraph",
                "content": [{ "type": "text", "text": "omega" }]
            }
        ]
    });
    let schema = tiptap_schema();
    let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let map = PositionMap::build(&document, &schema);
    let insertion_offset = map.doc_to_scalar(map.block(0).unwrap().doc_start, &document);
    let limits = ResourceLimits::default();
    let editing_limits = EditingLimits::default();
    let doc = utf16_doc();
    let codec = YrsDocumentCodec::new(&schema, &limits);
    {
        let mut txn = doc.transact_mut();
        let fragment = txn.get_or_insert_xml_fragment("prosemirror");
        codec
            .apply_json(&fragment, &mut txn, &json!({ "type": "doc" }), &source)
            .unwrap();
    }
    let (sibling_id, sticky, before_full_len) = {
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let sibling = paragraph_text(&fragment, &txn, 1);
        let id = <XmlTextRef as AsRef<Branch>>::as_ref(&sibling).id();
        let sticky = StickyIndex::at(
            &txn,
            BranchPtr::from(<XmlTextRef as AsRef<Branch>>::as_ref(&sibling)),
            2,
            Assoc::After,
        )
        .unwrap();
        (
            id,
            sticky,
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
                request_id: 78,
                base_document_revision: 0,
                origin: TransactionOrigin::LocalInput,
                operations: vec![TypedOperation::InsertText {
                    at: point_for_test(insertion_offset),
                    text: "item".into(),
                    marks: vec![Mark::new("italic".into(), HashMap::new())],
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
        Some(YrsMutationAction::CreateText { .. })
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
    assert_eq!(codec.read_json(&fragment, &txn).unwrap(), expected);
    let sibling = paragraph_text(&fragment, &txn, 1);
    assert_eq!(
        <XmlTextRef as AsRef<Branch>>::as_ref(&sibling).id(),
        sibling_id
    );
    let resolved = sticky.get_offset(&txn).unwrap();
    assert_eq!(resolved.branch.id(), sibling_id);
    assert_eq!(resolved.index, 2);
    assert!(update.len() <= estimate);
    let after_full_len = txn.encode_state_as_update_v1(&StateVector::default()).len();
    assert!(after_full_len <= before_full_len + estimate);
}

#[test]
fn create_text_preflight_rejects_stale_parent_atomically() {
    let source = json!({
        "type": "doc",
        "content": [{ "type": "paragraph" }]
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
                request_id: 79,
                base_document_revision: 0,
                origin: TransactionOrigin::LocalApi,
                operations: vec![TypedOperation::InsertText {
                    at: point_for_test(1),
                    text: "x".into(),
                    marks: vec![],
                }],
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Auto,
            },
            &txn,
            &fragment,
        )
        .unwrap()
    };
    let before = doc.transact().state_vector();
    {
        let mut txn = doc.transact_mut();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let XmlOut::Element(parent) = fragment.get(&txn, 0).unwrap() else {
            panic!("paragraph expected")
        };
        parent.insert(&mut txn, 0, XmlElementPrelim::empty("hardBreak"));
    }
    let after_external = doc.transact().state_vector();
    assert_ne!(before, after_external);
    let txn = doc.transact();
    let error = preflight_mutation_plan(79, &compiled.mutation_plan, &txn).unwrap_err();
    assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
    assert_eq!(txn.state_vector(), after_external);
}

#[test]
fn yrs_scan_accounting_accepts_exact_limit_rejects_one_over_and_amplification() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "abcdef" }]
        }]
    });
    let compile = |max_input_bytes: usize, source: &Value, operations: Vec<TypedOperation>| {
        let (doc, schema, mut limits, editing_limits, document) = diagnostic_doc(source);
        limits.max_input_bytes = max_input_bytes;
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
                request_id: 80,
                base_document_revision: 0,
                origin: TransactionOrigin::LocalInput,
                operations,
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Auto,
            },
            &txn,
            &fragment,
        )
    };
    let insertion = || TypedOperation::InsertText {
        at: point_for_test(1),
        text: "x".into(),
        marks: vec![],
    };
    // This is the exact admitted input plus reserved and reconciled Yrs
    // materialization, coordinate-index, and clock traversal work.
    compile(51, &source, vec![insertion()]).unwrap();
    let one_over = compile(50, &source, vec![insertion()]).unwrap_err();
    assert_eq!(one_over.code, "OPERATION_LIMIT_EXCEEDED");
    assert_eq!(one_over.limit, Some(50));

    let large = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "z".repeat(4_096) }]
        }]
    });
    let amplified = compile(20_000, &large, vec![insertion(); 8]).unwrap_err();
    assert_eq!(amplified.code, "OPERATION_LIMIT_EXCEEDED");
    assert_eq!(amplified.details, Some(json!({ "field": "maxInputBytes" })));

    let large_bold = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{
                "type": "text",
                "text": "z".repeat(4_096),
                "marks": [{ "type": "bold" }]
            }]
        }]
    });
    let noop = || TypedOperation::AddMark {
        range: range_for_test(0, 4_096),
        mark: Mark::new("bold".into(), HashMap::new()),
    };
    let noops = vec![noop(); 8];
    assert!(compile(16_442, &large_bold, noops.clone())
        .unwrap()
        .mutation_plan
        .actions
        .is_empty());
    assert_eq!(
        compile(16_441, &large_bold, noops).unwrap_err().limit,
        Some(16_441)
    );
}

#[test]
fn invalid_envelopes_reject_before_yrs_scan_and_semantic_noop_charges_initial_scan() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{
                "type": "text",
                "text": "abcdef",
                "marks": [{ "type": "bold" }]
            }]
        }]
    });
    let (doc, schema, mut limits, mut editing_limits, document) = diagnostic_doc(&source);
    limits.max_input_bytes = 1;
    editing_limits.max_operations_per_transaction = 1;
    let compile = |base_document_revision, origin, operations| {
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
                request_id: 81,
                base_document_revision,
                origin,
                operations,
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Auto,
            },
            &txn,
            &fragment,
        )
    };
    let noop = || TypedOperation::AddMark {
        range: range_for_test(0, 6),
        mark: Mark::new("bold".into(), HashMap::new()),
    };
    assert_eq!(
        compile(1, TransactionOrigin::LocalInput, vec![])
            .unwrap_err()
            .code,
        "REVISION_MISMATCH"
    );
    assert_eq!(
        compile(0, TransactionOrigin::RemoteSync, vec![])
            .unwrap_err()
            .code,
        "TRANSACTION_INVALID"
    );
    assert_eq!(
        compile(0, TransactionOrigin::LocalInput, vec![noop(), noop()])
            .unwrap_err()
            .details,
        Some(json!({ "field": "maxOperationsPerTransaction" }))
    );

    let (doc, schema, mut exact_limits, editing_limits, document) = diagnostic_doc(&source);
    // 6 admitted mark bytes + 12 bytes for materialization/coordinate indexing
    // + 22 reserved Yrs clock units across the two pre-lowering traversals.
    exact_limits.max_input_bytes = 40;
    let compile_noop = |limits: &ResourceLimits| {
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        compile_transaction_with_yrs(
            CompilationContext {
                document: &document,
                selection: None,
                schema: &schema,
                resource_limits: limits,
                editing_limits: &editing_limits,
                document_revision: 0,
                max_length: None,
            },
            TypedTransaction {
                request_id: 82,
                base_document_revision: 0,
                origin: TransactionOrigin::LocalCommand,
                operations: vec![noop()],
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Auto,
            },
            &txn,
            &fragment,
        )
    };
    assert!(compile_noop(&exact_limits)
        .unwrap()
        .mutation_plan
        .actions
        .is_empty());
    exact_limits.max_input_bytes = 39;
    assert_eq!(compile_noop(&exact_limits).unwrap_err().limit, Some(39));
}

#[test]
fn wide_target_paths_and_boundary_partition_work_stay_indexed_and_charged() {
    const BLOCKS: usize = 256;
    let source = json!({
        "type": "doc",
        "content": (0..BLOCKS)
            .map(|_| json!({
                "type": "paragraph",
                "content": [{ "type": "text", "text": "x" }]
            }))
            .collect::<Vec<_>>()
    });
    let (doc, schema, limits, _editing_limits, _document) = diagnostic_doc(&source);
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let mut compiler = super::mutation::MutationCompiler::new(
        85,
        &txn,
        &fragment,
        &schema,
        BLOCKS * 100,
        limits.max_input_bytes,
        0,
    )
    .unwrap();
    let traversal_work = compiler.total_mutation_work_for_test();
    // Exact normalized text/mark signatures add one charged run and content
    // comparison unit per target while preserving linear traversal.
    assert!(traversal_work <= BLOCKS * 20, "{traversal_work}");

    let boundary_count = BLOCKS * 3 + 1;
    let boundaries = (0..u32::try_from(boundary_count).unwrap()).collect::<Vec<_>>();
    let before = compiler.total_mutation_work_for_test();
    let disposition = compiler
        .delete(0, 1, u32::try_from(BLOCKS * 3 - 1).unwrap(), &boundaries)
        .unwrap();
    assert_eq!(disposition, TextRangeDisposition::Structural);
    let charged = compiler.total_mutation_work_for_test() - before;
    assert!(charged >= BLOCKS * 15, "{charged}");
    assert!(charged <= BLOCKS * 32, "{charged}");
}

#[test]
fn wide_preflight_and_virtual_delete_are_linear_in_children_and_spans() {
    const TARGETS: usize = 128;
    let source = json!({
        "type": "doc",
        "content": (0..TARGETS)
            .map(|_| json!({
                "type": "paragraph",
                "content": [{ "type": "text", "text": "x" }]
            }))
            .collect::<Vec<_>>()
    });
    let (doc, schema, limits, _editing_limits, _document) = diagnostic_doc(&source);
    let mut plan = {
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let mut compiler = super::mutation::MutationCompiler::new(
            88,
            &txn,
            &fragment,
            &schema,
            TARGETS * TARGETS * 4,
            limits.max_input_bytes,
            0,
        )
        .unwrap();
        for index in (0..TARGETS).rev() {
            compiler
                .insert(index, u32::try_from(1 + index * 3).unwrap(), "y", &[])
                .unwrap();
        }
        compiler.finish(Some(TARGETS - 1)).unwrap()
    };
    let txn = doc.transact();
    let preflight_work = preflight_mutation_work_for_test(88, &plan, &txn).unwrap();
    assert!(preflight_work <= TARGETS * 8, "{preflight_work}");
    let exact_limit = plan.compilation_work_for_test() + preflight_work;
    plan.set_work_limit_for_test(exact_limit);
    preflight_mutation_plan(88, &plan, &txn).unwrap();
    plan.set_work_limit_for_test(exact_limit - 1);
    let one_over = preflight_mutation_plan(88, &plan, &txn).unwrap_err();
    assert_eq!(one_over.code, "OPERATION_LIMIT_EXCEEDED");
    assert_eq!(
        one_over.limit,
        Some(u64::try_from(exact_limit - 1).unwrap())
    );
    assert_eq!(one_over.actual, Some(u64::try_from(exact_limit).unwrap()));

    let multi = utf16_doc();
    {
        let mut txn = multi.transact_mut();
        let fragment = txn.get_or_insert_xml_fragment("prosemirror");
        let paragraph = fragment.insert(&mut txn, 0, XmlElementPrelim::empty("paragraph"));
        for index in 0..TARGETS {
            paragraph.insert(
                &mut txn,
                u32::try_from(index).unwrap(),
                XmlTextPrelim::new("x"),
            );
        }
    }
    let txn = multi.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let mut compiler = super::mutation::MutationCompiler::new(
        89,
        &txn,
        &fragment,
        &schema,
        TARGETS * 20,
        limits.max_input_bytes,
        0,
    )
    .unwrap();
    compiler
        .delete(0, 1, u32::try_from(TARGETS + 1).unwrap(), &[])
        .unwrap();
    assert_eq!(compiler.virtual_delete_visits_for_test(), TARGETS);
}

#[test]
fn unicode_actions_store_checked_utf16_coordinates() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "🙂a" }]
        }]
    });
    let schema = tiptap_schema();
    let limits = ResourceLimits::default();
    let editing_limits = EditingLimits::default();
    let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let doc = utf16_doc();
    let codec = YrsDocumentCodec::new(&schema, &limits);
    {
        let mut txn = doc.transact_mut();
        let fragment = txn.get_or_insert_xml_fragment("prosemirror");
        codec
            .apply_json(&fragment, &mut txn, &json!({ "type": "doc" }), &source)
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
                request_id: 72,
                base_document_revision: 0,
                origin: TransactionOrigin::LocalInput,
                operations: vec![TypedOperation::InsertText {
                    at: point_for_test(1),
                    text: "x".into(),
                    marks: vec![],
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
        &compiled.mutation_plan.actions[0],
        YrsMutationAction::InsertText { index_utf16: 2, .. }
    ));
}

#[test]
fn unaffected_branch_identity_and_sticky_resolution_survive_local_edit() {
    let source: Value = serde_json::from_str(TWO_PARAGRAPHS).unwrap();
    let schema = tiptap_schema();
    let limits = ResourceLimits::default();
    let editing_limits = EditingLimits::default();
    let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let doc = utf16_doc();
    let codec = YrsDocumentCodec::new(&schema, &limits);
    {
        let mut txn = doc.transact_mut();
        let fragment = txn.get_or_insert_xml_fragment("prosemirror");
        codec
            .apply_json(&fragment, &mut txn, &json!({ "type": "doc" }), &source)
            .unwrap();
    }
    let (unaffected_id, sticky, before_full_len) = {
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let second = paragraph_text(&fragment, &txn, 1);
        let id = <XmlTextRef as AsRef<Branch>>::as_ref(&second).id();
        let sticky = StickyIndex::at(
            &txn,
            BranchPtr::from(<XmlTextRef as AsRef<Branch>>::as_ref(&second)),
            2,
            Assoc::After,
        )
        .unwrap();
        (
            id,
            sticky,
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
                request_id: 73,
                base_document_revision: 0,
                origin: TransactionOrigin::LocalInput,
                operations: vec![TypedOperation::InsertText {
                    at: point_for_test(1),
                    text: "!".into(),
                    marks: vec![],
                }],
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Auto,
            },
            &txn,
            &fragment,
        )
        .unwrap()
    };
    let estimate = compiled.encoded_growth_bound;
    let update = {
        let mut txn = doc.transact_mut();
        execute_mutation_plan(compiled.mutation_plan, &mut txn);
        txn.commit();
        txn.encode_update_v1()
    };
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let second = paragraph_text(&fragment, &txn, 1);
    assert_eq!(
        <XmlTextRef as AsRef<Branch>>::as_ref(&second).id(),
        unaffected_id
    );
    let resolved = sticky.get_offset(&txn).unwrap();
    assert_eq!(resolved.index, 2);
    assert_eq!(resolved.branch.id(), unaffected_id);
    assert!(!update.is_empty());
    assert!(update.len() <= estimate);
    assert!(update.len() < TWO_PARAGRAPHS.len());
    let after_full_len = txn.encode_state_as_update_v1(&StateVector::default()).len();
    assert!(after_full_len <= before_full_len + estimate);
}

#[test]
fn local_update_growth_is_independent_of_unaffected_article_size() {
    let source = |tail: String| {
        json!({
            "type": "doc",
            "content": [
                {"type": "paragraph", "content": [{"type": "text", "text": "alpha"}]},
                {"type": "paragraph", "content": [{"type": "text", "text": tail}]}
            ]
        })
    };
    let operation = || TypedOperation::InsertText {
        at: point_for_test(1),
        text: "!".into(),
        marks: vec![],
    };
    let (_, _, _, small_update, small_estimate) =
        compile_and_execute(source("omega".into()), vec![operation()]);
    let (_, _, _, large_update, large_estimate) =
        compile_and_execute(source("z".repeat(16_384)), vec![operation()]);

    assert!(small_update <= small_estimate);
    assert!(large_update <= large_estimate);
    assert!(large_update <= small_update + 16);
    assert!(large_update < 256);
}

#[test]
fn estimator_bounds_large_text_and_attribute_payloads() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "abcdef" }]
        }]
    });
    let payload = "x".repeat(8_192);
    compile_and_execute(
        source,
        vec![TypedOperation::InsertText {
            at: point_for_test(3),
            text: payload.clone(),
            marks: vec![Mark::new(
                "link".into(),
                HashMap::from([("href".into(), Value::String(payload))]),
            )],
        }],
    );
}

#[test]
fn format_actions_split_at_mark_boundaries_inside_one_xml_text() {
    let schema = tiptap_schema();
    let limits = ResourceLimits::default();
    let editing_limits = EditingLimits::default();
    let doc = utf16_doc();
    let source = {
        let mut txn = doc.transact_mut();
        let fragment = txn.get_or_insert_xml_fragment("prosemirror");
        let paragraph = fragment.insert(&mut txn, 0, XmlElementPrelim::empty("paragraph"));
        let text = paragraph.insert(&mut txn, 0, XmlTextPrelim::new("abcdef"));
        text.format(
            &mut txn,
            2,
            2,
            Attrs::from([("bold".into(), Any::Bool(true))]),
        );
        YrsDocumentCodec::new(&schema, &limits)
            .read_json(&fragment, &txn)
            .unwrap()
    };
    let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
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
                request_id: 74,
                base_document_revision: 0,
                origin: TransactionOrigin::LocalInput,
                operations: vec![TypedOperation::AddMark {
                    range: range_for_test(0, 6),
                    mark: Mark::new("italic".into(), HashMap::new()),
                }],
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Auto,
            },
            &txn,
            &fragment,
        )
        .unwrap()
    };
    assert_eq!(compiled.mutation_plan.actions.len(), 3);
    assert!(compiled
        .mutation_plan
        .actions
        .iter()
        .all(|action| matches!(action, YrsMutationAction::FormatText { .. })));
}

#[test]
fn preflight_rejects_a_target_changed_after_compilation() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "Hello" }]
        }]
    });
    let schema = tiptap_schema();
    let limits = ResourceLimits::default();
    let editing_limits = EditingLimits::default();
    let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let doc = utf16_doc();
    let codec = YrsDocumentCodec::new(&schema, &limits);
    {
        let mut txn = doc.transact_mut();
        let fragment = txn.get_or_insert_xml_fragment("prosemirror");
        codec
            .apply_json(&fragment, &mut txn, &json!({ "type": "doc" }), &source)
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
                request_id: 75,
                base_document_revision: 0,
                origin: TransactionOrigin::LocalInput,
                operations: vec![TypedOperation::InsertText {
                    at: point_for_test(1),
                    text: "x".into(),
                    marks: vec![],
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
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        paragraph_text(&fragment, &txn, 0).insert(&mut txn, 0, "!");
    }
    let txn = doc.transact();
    let error = preflight_mutation_plan(75, &compiled.mutation_plan, &txn).unwrap_err();
    assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
    assert_eq!(error.operation_index, Some(0));
}

#[test]
fn document_guard_rejects_deletion_only_staleness_with_an_unchanged_state_vector() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "Hello" }]
        }]
    });
    let (doc, _schema, _limits, compiled) = compile_operations_with_schema(
        &source,
        vec![TypedOperation::InsertText {
            at: point_for_test(1),
            text: "x".into(),
            marks: vec![],
        }],
        tiptap_schema(),
    );
    let before_delete_vector = doc.transact().state_vector();
    {
        let mut txn = doc.transact_mut();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        paragraph_text(&fragment, &txn, 0).remove_range(&mut txn, 0, 1);
    }
    let txn = doc.transact();
    assert_eq!(txn.state_vector(), before_delete_vector);
    let rejected_state = txn.encode_state_as_update_v1(&StateVector::default());
    let error = preflight_mutation_plan(175, &compiled.mutation_plan, &txn).unwrap_err();
    assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
    assert_eq!(error.operation_index, Some(0));
    assert_eq!(
        txn.encode_state_as_update_v1(&StateVector::default()),
        rejected_state
    );
}

#[test]
fn document_guard_rejects_a_foreign_same_content_yrs_store_without_mutation() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "Hello" }]
        }]
    });
    let (_source_doc, _schema, _limits, compiled) = compile_operations_with_schema(
        &source,
        vec![TypedOperation::InsertText {
            at: point_for_test(1),
            text: "x".into(),
            marks: vec![],
        }],
        tiptap_schema(),
    );
    let (foreign, _, _, _, _) = diagnostic_doc(&source);
    let txn = foreign.transact();
    let before = txn.encode_state_as_update_v1(&StateVector::default());
    let error = preflight_mutation_plan(176, &compiled.mutation_plan, &txn).unwrap_err();
    assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
    assert_eq!(error.operation_index, Some(0));
    assert_eq!(
        txn.encode_state_as_update_v1(&StateVector::default()),
        before
    );
}

#[test]
fn document_guard_rejects_hostile_stale_attributes_before_live_attribute_materialization() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "blockquote",
            "content": [{
                "type": "h2",
                "attrs": { "id": "old" },
                "content": [{ "type": "text", "text": "heading" }]
            }]
        }]
    });
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
                request_id: 177,
                base_document_revision: 0,
                origin: TransactionOrigin::LocalCommand,
                operations: vec![TypedOperation::UpdateNodeAttrs {
                    at: point_for_test(1),
                    attrs: HashMap::from([("id".into(), Value::String("new".into()))]),
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
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let XmlOut::Element(quote) = fragment.get(&txn, 0).unwrap() else {
            panic!("expected blockquote")
        };
        let XmlOut::Element(heading) = quote.get(&txn, 0).unwrap() else {
            panic!("expected heading")
        };
        heading.insert_attribute(
            &mut txn,
            "hostile",
            Any::String("x".repeat(1024 * 1024).into()),
        );
    }
    let txn = doc.transact();
    let before = txn.encode_state_as_update_v1(&StateVector::default());
    let error = preflight_mutation_plan(177, &compiled.mutation_plan, &txn).unwrap_err();
    assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
    assert_eq!(error.operation_index, Some(0));
    assert_eq!(
        txn.encode_state_as_update_v1(&StateVector::default()),
        before
    );
}

#[test]
fn replace_mark_then_replace_range_executes_to_preview() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "abcdef" }]
        }]
    });
    let mark = TypedOperation::ReplaceMark {
        range: range_for_test(1, 4),
        mark: Mark::new(
            "link".into(),
            HashMap::from([("href".into(), Value::String("a".into()))]),
        ),
    };
    let replace = TypedOperation::ReplaceRange {
        range: range_for_test(2, 3),
        content: Fragment::from(vec![Node::text("a".into(), vec![])]),
    };
    let (doc, schema, limits, editing_limits, document) = diagnostic_doc(&source);
    let compile = |operations| {
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
                request_id: 76,
                base_document_revision: 0,
                origin: TransactionOrigin::LocalInput,
                operations,
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Auto,
            },
            &txn,
            &fragment,
        )
        .unwrap()
    };
    let marked = compile(vec![mark.clone()]);
    let compiled = compile(vec![mark.clone(), replace.clone()]);
    assert_eq!(marked.preview.root().text_content(), "abcdef");
    assert_eq!(compiled.preview.root().text_content(), "abadef");

    let mut executed_text = String::new();
    for action in compiled.mutation_plan.actions.clone() {
        let mut txn = doc.transact_mut();
        execute_mutation_plan(
            super::mutation::YrsMutationPlan::single_action_for_test(action),
            &mut txn,
        );
        drop(txn);
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let decoded = YrsDocumentCodec::new(&schema, &limits)
            .read_json(&fragment, &txn)
            .unwrap();
        let decoded = from_prosemirror_json(&decoded, &schema, UnknownTypeMode::Preserve).unwrap();
        executed_text = decoded.root().text_content();
    }

    let (_, _, _, _, inverse_document) = diagnostic_doc(&source);
    let inverse_doc = utf16_doc();
    let codec = YrsDocumentCodec::new(&schema, &limits);
    {
        let mut txn = inverse_doc.transact_mut();
        let fragment = txn.get_or_insert_xml_fragment("prosemirror");
        codec
            .apply_json(&fragment, &mut txn, &json!({ "type": "doc" }), &source)
            .unwrap();
    }
    let inverse = {
        let txn = inverse_doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        compile_transaction_with_yrs(
            CompilationContext {
                document: &inverse_document,
                selection: None,
                schema: &schema,
                resource_limits: &limits,
                editing_limits: &editing_limits,
                document_revision: 0,
                max_length: None,
            },
            TypedTransaction {
                request_id: 77,
                base_document_revision: 0,
                origin: TransactionOrigin::LocalInput,
                operations: vec![replace, mark],
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Auto,
            },
            &txn,
            &fragment,
        )
        .unwrap()
    };
    assert_eq!(executed_text, compiled.preview.root().text_content());
    assert_eq!(inverse.preview.root().text_content(), "abadef");
}

#[test]
fn generated_structural_trees_bound_and_converge_for_256_fixed_seeds() {
    fn nested_doc(mut blocks: Vec<Value>, depth: usize) -> Value {
        for _ in 0..depth {
            blocks = vec![json!({ "type": "blockquote", "content": blocks })];
        }
        json!({ "type": "doc", "content": blocks })
    }

    let schema = tiptap_schema();
    for seed in 0usize..256 {
        let depth = (seed / 11) % 4;
        let (source, operations) = match seed % 11 {
            0 => {
                let source = nested_doc(
                    vec![
                        json!({ "type": "paragraph", "content": [{ "type": "text", "text": "A😀B" }] }),
                    ],
                    depth,
                );
                (
                    source,
                    vec![TypedOperation::InsertNode {
                        at: point_for_test(2),
                        node: Node::void("hardBreak".into(), HashMap::new()),
                    }],
                )
            }
            1 => {
                let source = nested_doc(
                    vec![
                        json!({ "type": "paragraph", "content": [{ "type": "text", "text": "A😀B" }] }),
                    ],
                    0,
                );
                (
                    source,
                    vec![TypedOperation::SplitBlock {
                        at: point_for_test(2),
                        node_type: "paragraph".into(),
                        attrs: HashMap::new(),
                    }],
                )
            }
            2 => {
                let source = nested_doc(
                    vec![
                        json!({ "type": "paragraph", "content": [{ "type": "text", "text": "ab" }] }),
                        json!({ "type": "paragraph", "content": [{ "type": "text", "text": "cd" }] }),
                    ],
                    depth,
                );
                (
                    source,
                    vec![TypedOperation::JoinBlocks {
                        at: point_for_test(2),
                    }],
                )
            }
            3 => {
                let source = nested_doc(
                    vec![
                        json!({ "type": "paragraph", "content": [{ "type": "text", "text": "one" }] }),
                    ],
                    0,
                );
                (
                    source,
                    vec![TypedOperation::WrapInList {
                        range: range_for_test(0, 3),
                        list_type: "bulletList".into(),
                        item_type: "listItem".into(),
                        attrs: HashMap::new(),
                        item_attrs: HashMap::new(),
                    }],
                )
            }
            4 => {
                let source = nested_doc(
                    vec![json!({
                        "type": "bulletList",
                        "content": [
                            { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "one" }] }] },
                            { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "two" }] }] }
                        ]
                    })],
                    depth,
                );
                let at = rendered_scalar_offset(&source, &schema, "two") + 1;
                (
                    source,
                    vec![TypedOperation::IndentListItem {
                        at: point_for_test(at),
                    }],
                )
            }
            5 => {
                let source = nested_doc(
                    vec![json!({
                        "type": "bulletList",
                        "content": [{
                            "type": "listItem",
                            "content": [
                                { "type": "paragraph", "content": [{ "type": "text", "text": "outer" }] },
                                { "type": "bulletList", "content": [{ "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "inner" }] }] }] }
                            ]
                        }]
                    })],
                    depth,
                );
                let at = rendered_scalar_offset(&source, &schema, "inner") + 1;
                (
                    source,
                    vec![TypedOperation::OutdentListItem {
                        at: point_for_test(at),
                    }],
                )
            }
            6 => {
                let source = nested_doc(
                    vec![
                        json!({ "type": "bulletList", "content": [{ "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "one" }] }] }] }),
                    ],
                    depth,
                );
                let at = rendered_scalar_offset(&source, &schema, "one") + 1;
                (
                    source,
                    vec![TypedOperation::UnwrapFromList {
                        at: point_for_test(at),
                    }],
                )
            }
            7 => {
                let source = nested_doc(
                    vec![json!({ "type": "paragraph", "content": [{ "type": "hardBreak" }] })],
                    depth,
                );
                (
                    source,
                    vec![TypedOperation::DeleteRange {
                        range: range_for_test(0, 1),
                    }],
                )
            }
            8 => {
                let source = nested_doc(
                    vec![json!({ "type": "paragraph", "content": [{ "type": "hardBreak" }] })],
                    depth,
                );
                (
                    source,
                    vec![TypedOperation::ReplaceRange {
                        range: range_for_test(0, 1),
                        content: Fragment::from(vec![Node::text(format!("seed-{seed}"), vec![])]),
                    }],
                )
            }
            9 => {
                let source = json!({
                    "type": "doc",
                    "content": [{
                        "type": "image",
                        "attrs": { "src": format!("old-{seed}"), "alt": "old alt" }
                    }]
                });
                (
                    source,
                    vec![TypedOperation::UpdateNodeAttrs {
                        at: point_for_test(0),
                        attrs: HashMap::from([
                            ("src".into(), Value::String(format!("new-{seed}"))),
                            ("alt".into(), Value::String("new alt".into())),
                            ("title".into(), Value::Null),
                            ("width".into(), Value::Null),
                            ("height".into(), Value::Null),
                        ]),
                    }],
                )
            }
            _ => {
                let source = nested_doc(
                    vec![
                        json!({ "type": "paragraph", "content": [{ "type": "text", "text": "A😀" }] }),
                        json!({ "type": "paragraph", "content": [{ "type": "text", "text": "B" }] }),
                    ],
                    depth,
                );
                let at = rendered_scalar_offset(&source, &schema, "B") - 1;
                (
                    source,
                    vec![TypedOperation::InsertNode {
                        at: point_for_test(at),
                        node: Node::void("horizontalRule".into(), HashMap::new()),
                    }],
                )
            }
        };
        let (actual, expected, _, update_len, estimate) = compile_and_execute(source, operations);
        assert_eq!(actual, expected, "fixed structural seed {seed}");
        assert!(update_len <= estimate, "fixed structural seed {seed}");
    }

    let source = json!({
        "type": "doc",
        "content": [
            { "type": "paragraph", "content": [{ "type": "text", "text": "AB" }] },
            { "type": "paragraph", "content": [{ "type": "text", "text": "sentinel" }] }
        ]
    });
    let (doc, schema, limits, mut compiled) = compile_operations_with_schema(
        &source,
        vec![TypedOperation::InsertNode {
            at: point_for_test(1),
            node: Node::void("hardBreak".into(), HashMap::new()),
        }],
        tiptap_schema(),
    );
    let sentinel_id = {
        let txn = doc.transact();
        txn.get_xml_fragment("prosemirror")
            .unwrap()
            .get(&txn, 1)
            .unwrap()
            .id()
    };
    {
        let txn = doc.transact();
        let preflight =
            preflight_mutation_work_for_test(71, &compiled.mutation_plan, &txn).unwrap();
        let exact = compiled.mutation_plan.compilation_work_for_test() + preflight;
        compiled.mutation_plan.set_work_limit_for_test(exact);
        preflight_mutation_plan(71, &compiled.mutation_plan, &txn).unwrap();
        compiled.mutation_plan.set_work_limit_for_test(exact - 1);
        assert_eq!(
            preflight_mutation_plan(71, &compiled.mutation_plan, &txn)
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
    assert_eq!(fragment.get(&txn, 1).unwrap().id(), sentinel_id);
    assert_eq!(
        YrsDocumentCodec::new(&schema, &limits)
            .read_json(&fragment, &txn)
            .unwrap(),
        expected
    );
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 64,
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    #[test]
    fn estimated_update_growth_bounds_supported_action_mixes(
        generated in prop::collection::vec((0u8..6, "[a-z]{1,3}"), 1..8)
    ) {
        let source = json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{ "type": "text", "text": "abcdef" }]
            }]
        });
        let operations = generated
            .into_iter()
            .map(|(kind, text)| match kind {
                0 => TypedOperation::InsertText {
                    at: point_for_test(2),
                    text,
                    marks: vec![],
                },
                1 => TypedOperation::DeleteRange {
                    range: range_for_test(2, 3),
                },
                2 => TypedOperation::ReplaceRange {
                    range: range_for_test(2, 3),
                    content: Fragment::from(vec![Node::text(text, vec![])]),
                },
                3 => TypedOperation::AddMark {
                    range: range_for_test(1, 4),
                    mark: Mark::new("bold".into(), HashMap::new()),
                },
                4 => TypedOperation::RemoveMark {
                    range: range_for_test(1, 4),
                    mark_type: "bold".into(),
                },
                _ => TypedOperation::ReplaceMark {
                    range: range_for_test(1, 4),
                    mark: Mark::new(
                        "link".into(),
                        HashMap::from([("href".into(), Value::String(text))]),
                    ),
                },
            })
            .collect();
        compile_and_execute(source, operations);
    }
}
