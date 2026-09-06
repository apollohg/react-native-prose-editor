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

include!("preflight_and_properties/target_identity.rs");

include!("preflight_and_properties/work_and_staleness.rs");

include!("preflight_and_properties/properties.rs");
