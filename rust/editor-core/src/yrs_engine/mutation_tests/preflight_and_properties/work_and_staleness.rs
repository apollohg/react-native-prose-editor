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
