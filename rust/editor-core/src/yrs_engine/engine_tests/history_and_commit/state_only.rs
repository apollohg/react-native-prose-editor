#[test]
fn empty_skip_selection_bypasses_mutation_preflight_but_not_admission_or_boundaries() {
    use crate::yrs_engine::compiler::{set_atomic_failpoint_for_test, AtomicFailpoint};
    use crate::yrs_engine::observability::{
        reset_prepared_admission_counts_for_test, take_prepared_admission_counts_for_test,
    };

    let selection_transaction =
        |engine: &YrsDocumentEngine, request_id, history_policy| TypedTransaction {
            request_id,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: vec![],
            selection_intent: SelectionIntent::Set(SelectionInput::All),
            history_policy,
        };

    let mut skip = transaction_engine();
    reset_prepared_admission_counts_for_test();
    set_atomic_failpoint_for_test(Some(AtomicFailpoint::MutationPreflight));
    let result = skip
        .apply_typed_transaction_with_result(selection_transaction(&skip, 760, HistoryPolicy::Skip))
        .expect("empty Skip selection must not enter mutation preflight");
    set_atomic_failpoint_for_test(None);
    assert!(result.changed);
    assert_eq!(skip.revision(), 0);
    assert_eq!(skip.state_revision(), 1);
    let skip_counts = take_prepared_admission_counts_for_test();
    assert_eq!(skip_counts.staged_seed_preparations, 0);
    assert_eq!(skip_counts.installed_base_seed_publications, 0);

    let mut boundary = transaction_engine();
    let before_boundary = atomic_audit(&boundary);
    set_atomic_failpoint_for_test(Some(AtomicFailpoint::MutationPreflight));
    let boundary_error = boundary
        .apply_typed_transaction(selection_transaction(
            &boundary,
            761,
            HistoryPolicy::Boundary,
        ))
        .unwrap_err();
    set_atomic_failpoint_for_test(None);
    assert_eq!(
        boundary_error.details,
        Some(json!({ "failpoint": "mutationPreflight" }))
    );
    assert_eq!(atomic_audit(&boundary), before_boundary);

    let mut rejected = transaction_engine();
    let before_rejected = atomic_audit(&rejected);
    set_atomic_failpoint_for_test(Some(AtomicFailpoint::EnvelopeAdmission));
    let admission_error = rejected
        .apply_typed_transaction(selection_transaction(&rejected, 762, HistoryPolicy::Skip))
        .unwrap_err();
    set_atomic_failpoint_for_test(None);
    assert_eq!(
        admission_error.details,
        Some(json!({ "failpoint": "envelopeAdmission" }))
    );
    assert_eq!(atomic_audit(&rejected), before_rejected);
}

#[test]
fn empty_generic_state_only_transactions_do_not_prepare_lookup_seed() {
    use crate::yrs_engine::mutation::{
        set_lookup_seed_hydration_failpoint_for_test, LookupSeedHydrationFailpoint,
    };
    use crate::yrs_engine::observability::{
        reset_prepared_admission_counts_for_test, take_prepared_admission_counts_for_test,
    };

    for (offset, history_policy) in [
        HistoryPolicy::Skip,
        HistoryPolicy::Auto,
        HistoryPolicy::Boundary,
    ]
    .into_iter()
    .enumerate()
    {
        let request_id = 760_100 + u64::try_from(offset).unwrap();
        let mut engine = import_document_with_unavailable_lookup_seed();
        engine
            .apply_command(
                request_id,
                TypedCommand::ToggleMark {
                    mark_type: "bold".into(),
                },
            )
            .unwrap()
            .expect("collapsed toggle must set stored marks");
        assert_eq!(
            engine
                .stored_marks()
                .unwrap()
                .iter()
                .map(Mark::mark_type)
                .collect::<Vec<_>>(),
            vec!["bold"]
        );
        let installed = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
        let before_document_revision = engine.revision();
        let before_state_revision = engine.state_revision();
        reset_prepared_admission_counts_for_test();
        set_lookup_seed_hydration_failpoint_for_test(Some(
            LookupSeedHydrationFailpoint::InitialReservation,
        ));

        let result = engine
            .apply_typed_transaction_with_result(TypedTransaction {
                request_id: request_id + 10,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalApi,
                operations: Vec::new(),
                selection_intent: SelectionIntent::Set(SelectionInput::All),
                history_policy,
            })
            .expect("state-only generic transaction must not consume hydration failure");

        set_lookup_seed_hydration_failpoint_for_test(None);
        let counts = take_prepared_admission_counts_for_test();
        assert!(result.changed, "{history_policy:?}");
        assert_eq!(
            result.selection,
            ResolvedSelection::All,
            "{history_policy:?}"
        );
        assert_eq!(
            engine.revision(),
            before_document_revision,
            "{history_policy:?}"
        );
        assert_eq!(
            engine.state_revision(),
            before_state_revision + 1,
            "{history_policy:?}"
        );
        assert!(engine.stored_marks().is_none(), "{history_policy:?}");
        assert_eq!(counts.staged_seed_preparations, 0, "{history_policy:?}");
        assert_eq!(
            counts.installed_base_seed_publications, 0,
            "{history_policy:?}"
        );
        assert!(Arc::ptr_eq(
            &installed,
            &engine.derived_state.as_ref().unwrap().mutation_lookup_seed,
        ));
        assert!(engine
            .derived_state
            .as_ref()
            .unwrap()
            .mutation_lookup_seed
            .is_unavailable_for_test());
    }
}

#[test]
fn empty_generic_boundary_preserves_recorded_grouping_semantics() {
    let apply_insert = |engine: &mut YrsDocumentEngine, request_id, text: &str| {
        let at = engine.position_map().unwrap().total_scalars();
        engine
            .apply_typed_transaction(TypedTransaction {
                request_id,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalInput,
                operations: vec![TypedOperation::InsertText {
                    at: RevisionedPosition {
                        offset: at,
                        kind: EditorOffsetKind::Scalar,
                        affinity: Affinity::After,
                    },
                    text: text.into(),
                    marks: Vec::new(),
                }],
                selection_intent: SelectionIntent::Preserve,
                history_policy: HistoryPolicy::Auto,
            })
            .unwrap();
    };

    for (offset, state_only_policy) in [HistoryPolicy::Auto, HistoryPolicy::Boundary]
        .into_iter()
        .enumerate()
    {
        let request_id = 760_120 + u64::try_from(offset).unwrap() * 10;
        let mut engine = import_document_with_unavailable_lookup_seed();
        apply_insert(&mut engine, request_id, "x");
        force_lookup_seed_unavailable(&mut engine);
        engine
            .apply_typed_transaction(TypedTransaction {
                request_id: request_id + 1,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalApi,
                operations: Vec::new(),
                selection_intent: SelectionIntent::Set(SelectionInput::All),
                history_policy: state_only_policy,
            })
            .unwrap();
        apply_insert(&mut engine, request_id + 2, "y");
        assert_eq!(engine.document().unwrap().root().text_content(), "abcxy");

        engine
            .undo(request_id + 3)
            .unwrap()
            .expect("recorded insert must be undoable");
        let expected_after_first_pop = if state_only_policy == HistoryPolicy::Boundary {
            "abcx"
        } else {
            "abc"
        };
        assert_eq!(
            engine.document().unwrap().root().text_content(),
            expected_after_first_pop,
            "{state_only_policy:?}"
        );
        if state_only_policy == HistoryPolicy::Boundary {
            engine
                .undo(request_id + 4)
                .unwrap()
                .expect("Boundary must retain the earlier group");
            assert_eq!(engine.document().unwrap().root().text_content(), "abc");
        }
    }
}

#[test]
fn changed_state_boundary_revision_overflow_precedes_replay_allocation() {
    let mut engine = import_document_with_unavailable_lookup_seed();
    engine.history.compact_replay_event_capacity_for_test();
    engine.state_revision = u64::MAX;
    engine
        .derived_state
        .as_mut()
        .unwrap()
        .reseal_state_revision(u64::MAX);
    let before = atomic_audit(&engine);
    let replay_before = engine.history.replay_ledger_allocation_audit_for_test();

    let error = engine
        .apply_typed_transaction(TypedTransaction {
            request_id: 760_110,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: Vec::new(),
            selection_intent: SelectionIntent::Set(SelectionInput::All),
            history_policy: HistoryPolicy::Boundary,
        })
        .unwrap_err();

    assert_eq!(error.code, "ENGINE_INVARIANT_FAILED", "{error:?}");
    assert_eq!(
        error.message.as_ref(),
        "stateRevision cannot be incremented"
    );
    assert_eq!(error.details, Some(json!({ "field": "stateRevision" })));
    assert_eq!(atomic_audit(&engine), before);
    assert_eq!(
        engine.history.replay_ledger_allocation_audit_for_test(),
        replay_before
    );
}

#[test]
fn generic_structural_limit_drift_matches_preconfigured_current_and_reuses_evidence() {
    let source =
        r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"a"}]}]}"#;
    let schema = tiptap_schema();
    let base_document = from_prosemirror_json(
        &serde_json::from_str(source).unwrap(),
        &schema,
        UnknownTypeMode::Preserve,
    )
    .unwrap();
    let old_node_limit = crate::editor_state::document_node_count(base_document.root());
    let current_node_limit = old_node_limit + 1;
    let old_limits = ResourceLimits {
        max_document_nodes: old_node_limit,
        ..ResourceLimits::default()
    };
    let current_limits = ResourceLimits {
        max_document_nodes: current_node_limit,
        ..old_limits.clone()
    };

    let mut drifted = transaction_engine_with_resource_limits_and_mode(
        old_limits.clone(),
        crate::yrs_engine::InitializationMode::LocalEmpty,
    );
    let mut preconfigured = transaction_engine_with_resource_limits_and_mode(
        current_limits.clone(),
        crate::yrs_engine::InitializationMode::LocalEmpty,
    );
    let mut one_under = transaction_engine_with_resource_limits_and_mode(
        current_limits.clone(),
        crate::yrs_engine::InitializationMode::LocalEmpty,
    );
    for engine in [&mut drifted, &mut preconfigured, &mut one_under] {
        engine
            .import_json(source, TransactionOrigin::DocumentImport)
            .unwrap();
    }
    assert_eq!(
        drifted.derived_state.as_ref().unwrap().document_node_count,
        old_node_limit
    );
    assert!(derived_evidence_matches_runtime_limits(&drifted));
    drifted.resource_limits = current_limits.clone();
    assert!(!derived_evidence_matches_runtime_limits(&drifted));

    let drifted_commit = drifted
        .apply_typed_transaction(hard_break_insert_transaction(&drifted, 760_200))
        .expect("loosened runtime limit must admit the generic structural candidate");
    let preconfigured_commit = preconfigured
        .apply_typed_transaction(hard_break_insert_transaction(&preconfigured, 760_200))
        .unwrap();
    assert_eq!(drifted_commit, preconfigured_commit);
    assert_eq!(drifted_commit.document_revision, 2);
    assert_eq!(drifted_commit.state_revision, 2);
    assert_eq!(
        drifted.derived_state.as_ref().unwrap().document_node_count,
        current_node_limit
    );
    assert_limit_drift_semantic_parity(&drifted, &preconfigured);

    let drifted_followup = drifted
        .apply_typed_transaction(insert_transaction(&drifted, 760_201))
        .expect("current-limit evidence must be reusable by the following mutation");
    let preconfigured_followup = preconfigured
        .apply_typed_transaction(insert_transaction(&preconfigured, 760_201))
        .unwrap();
    assert_eq!(drifted_followup, preconfigured_followup);
    assert_limit_drift_semantic_parity(&drifted, &preconfigured);

    one_under.resource_limits = old_limits;
    let before = atomic_audit(&one_under);
    let error = one_under
        .apply_typed_transaction(hard_break_insert_transaction(&one_under, 760_202))
        .unwrap_err();
    assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
    assert_eq!(error.limit, Some(u64::try_from(old_node_limit).unwrap()));
    assert_eq!(
        error.actual,
        Some(u64::try_from(current_node_limit).unwrap())
    );
    assert_eq!(atomic_audit(&one_under), before);
}

#[test]
fn remote_limit_drift_matches_preconfigured_current_and_reuses_evidence() {
    let source_json =
        r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"a"}]}]}"#;
    let schema = tiptap_schema();
    let base_document = from_prosemirror_json(
        &serde_json::from_str(source_json).unwrap(),
        &schema,
        UnknownTypeMode::Preserve,
    )
    .unwrap();
    let old_node_limit = crate::editor_state::document_node_count(base_document.root());
    let current_node_limit = old_node_limit + 1;
    let old_limits = ResourceLimits {
        max_document_nodes: old_node_limit,
        ..ResourceLimits::default()
    };
    let current_limits = ResourceLimits {
        max_document_nodes: current_node_limit,
        ..old_limits.clone()
    };
    let mut source = transaction_engine_with_resource_limits_and_mode(
        current_limits.clone(),
        crate::yrs_engine::InitializationMode::LocalEmpty,
    );
    source
        .import_json(source_json, TransactionOrigin::DocumentImport)
        .unwrap();
    let base_update = source.encoded_state().unwrap();
    let mut drifted = transaction_engine_with_resource_limits_and_mode(
        old_limits,
        crate::yrs_engine::InitializationMode::AwaitRemote,
    );
    let mut preconfigured = transaction_engine_with_resource_limits_and_mode(
        current_limits.clone(),
        crate::yrs_engine::InitializationMode::AwaitRemote,
    );
    let drifted_base = drifted
        .apply_remote_update_v1(760_210, &base_update)
        .unwrap();
    let preconfigured_base = preconfigured
        .apply_remote_update_v1(760_210, &base_update)
        .unwrap();
    assert_eq!(drifted_base, preconfigured_base);
    assert!(derived_evidence_matches_runtime_limits(&drifted));
    drifted.resource_limits = current_limits;
    assert!(!derived_evidence_matches_runtime_limits(&drifted));

    let target_vector = drifted.doc.transact().state_vector();
    source
        .apply_typed_transaction(paragraph_insert_transaction(&source, 760_211))
        .unwrap();
    let structural_delta = source
        .doc
        .transact()
        .encode_state_as_update_v1(&target_vector);
    let drifted_commit = drifted
        .apply_remote_update_v1(760_212, &structural_delta)
        .expect("loosened runtime limit must admit the changed remote candidate");
    let preconfigured_commit = preconfigured
        .apply_remote_update_v1(760_212, &structural_delta)
        .unwrap();
    assert_eq!(drifted_commit, preconfigured_commit);
    assert_eq!(
        drifted.derived_state.as_ref().unwrap().document_node_count,
        current_node_limit
    );
    assert_limit_drift_semantic_parity(&drifted, &preconfigured);

    let target_vector = drifted.doc.transact().state_vector();
    source
        .apply_typed_transaction(insert_transaction(&source, 760_213))
        .unwrap();
    let followup_delta = source
        .doc
        .transact()
        .encode_state_as_update_v1(&target_vector);
    let drifted_followup = drifted
        .apply_remote_update_v1(760_214, &followup_delta)
        .expect("remote current-limit evidence must be reusable");
    let preconfigured_followup = preconfigured
        .apply_remote_update_v1(760_214, &followup_delta)
        .unwrap();
    assert_eq!(drifted_followup, preconfigured_followup);
    assert_limit_drift_semantic_parity(&drifted, &preconfigured);
}

#[test]
fn empty_skip_collapsed_text_prepares_one_forward_point_without_reverse_traversal() {
    use crate::yrs_engine::derived_state::{
        reset_relative_selection_traversal_counts_for_test,
        take_relative_selection_traversal_counts_for_test,
    };
    use crate::yrs_engine::position::{
        reset_relative_position_traversal_counts_for_test,
        take_relative_position_traversal_counts_for_test,
    };

    let mut engine = transaction_engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"prefix"}]},{"type":"paragraph","content":[{"type":"text","text":"a😀middle"}]},{"type":"paragraph","content":[{"type":"text","text":"suffix"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let point = RevisionedPosition {
        offset: 9,
        kind: EditorOffsetKind::Scalar,
        affinity: Affinity::Before,
    };
    reset_relative_position_traversal_counts_for_test();
    reset_relative_selection_traversal_counts_for_test();

    let result = engine
        .apply_typed_transaction_with_result(TypedTransaction {
            request_id: 759,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: vec![],
            selection_intent: SelectionIntent::Set(SelectionInput::Text {
                anchor: point,
                head: point,
            }),
            history_policy: HistoryPolicy::Skip,
        })
        .unwrap();

    assert!(result.changed);
    assert_eq!(result.document_revision, 1);
    assert_eq!(result.state_revision, 2);
    assert!(matches!(
        result.selection,
        ResolvedSelection::Text { anchor, head }
            if anchor == head && anchor.scalar == point.offset
    ));
    assert_eq!(
        take_relative_position_traversal_counts_for_test(),
        (0, 1, 0),
        "collapsed exact inputs must share one admitted forward materialization"
    );
    assert_eq!(
        take_relative_selection_traversal_counts_for_test(),
        (0, 0),
        "prepared resolved points must not round-trip through Yrs"
    );
}

#[test]
fn empty_skip_prepared_collapsed_text_preserves_overflow_and_output_atomicity() {
    fn populated_engine() -> YrsDocumentEngine {
        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abcdef"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        engine
    }

    fn transaction(engine: &YrsDocumentEngine, request_id: u64) -> TypedTransaction {
        let point = RevisionedPosition {
            offset: 3,
            kind: EditorOffsetKind::Scalar,
            affinity: Affinity::Before,
        };
        TypedTransaction {
            request_id,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: vec![],
            selection_intent: SelectionIntent::Set(SelectionInput::Text {
                anchor: point,
                head: point,
            }),
            history_policy: HistoryPolicy::Skip,
        }
    }

    let mut overflow = populated_engine();
    overflow.state_revision = u64::MAX;
    overflow.derived_state.as_mut().unwrap().state_revision = u64::MAX;
    let overflow_before = atomic_audit(&overflow);
    let overflow_transaction = transaction(&overflow, 759_001);

    let overflow_error = overflow
        .apply_typed_transaction_with_result(overflow_transaction)
        .unwrap_err();

    assert_eq!(overflow_error.code, "ENGINE_INVARIANT_FAILED");
    assert_eq!(
        overflow_error.details,
        Some(json!({ "field": "stateRevision" }))
    );
    assert_eq!(atomic_audit(&overflow), overflow_before);

    let mut output_limited = populated_engine();
    output_limited.editing_limits.max_derived_output_bytes = 1;
    let output_before = atomic_audit(&output_limited);
    let output_transaction = transaction(&output_limited, 759_002);

    let output_error = output_limited
        .apply_typed_transaction_with_result(output_transaction)
        .unwrap_err();

    assert_eq!(output_error.code, "DOCUMENT_LIMIT_EXCEEDED");
    assert_eq!(
        output_error.details,
        Some(json!({ "field": "maxDerivedOutputBytes" }))
    );
    assert_eq!(atomic_audit(&output_limited), output_before);
}

#[test]
fn empty_skip_fast_path_matches_full_compiler_at_yrs_scan_work_boundary() {
    fn populated_engine() -> YrsDocumentEngine {
        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"scan boundary"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        engine
    }

    fn scan_work(engine: &YrsDocumentEngine) -> usize {
        let document_text_bytes = engine.document().unwrap().root().text_content().len();
        let txn = engine.doc.transact();
        let crdt_clock_work = txn
            .state_vector()
            .iter()
            .map(|(_, clock)| usize::try_from(*clock).unwrap() + 1)
            .sum::<usize>();
        document_text_bytes * 2 + crdt_clock_work * 2
    }

    fn transaction(engine: &YrsDocumentEngine, request_id: u64) -> TypedTransaction {
        TypedTransaction {
            request_id,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: vec![],
            selection_intent: SelectionIntent::Set(SelectionInput::All),
            history_policy: HistoryPolicy::Skip,
        }
    }

    let required = scan_work(&populated_engine());

    let mut exact_fast = populated_engine();
    exact_fast.resource_limits.max_input_bytes = required;
    let exact_fast_result = exact_fast
        .apply_typed_transaction_with_result(transaction(&exact_fast, 763))
        .unwrap();
    let mut exact_slow = populated_engine();
    exact_slow.resource_limits.max_input_bytes = required;
    let exact_slow_transaction = transaction(&exact_slow, 763);
    let exact_slow_compiled = exact_slow
        .compile_typed_transaction(exact_slow_transaction)
        .unwrap();
    let exact_slow_result = exact_slow
        .apply_compiled_transaction(exact_slow_compiled, true)
        .unwrap()
        .1
        .unwrap();
    assert_eq!(exact_fast_result, exact_slow_result);
    assert_eq!(exact_fast.document_json(), exact_slow.document_json());
    assert_eq!(exact_fast.document_html(), exact_slow.document_html());
    assert_eq!(exact_fast.revision(), exact_slow.revision());
    assert_eq!(exact_fast.state_revision(), exact_slow.state_revision());
    assert_eq!(
        exact_fast.resolved_selection(),
        exact_slow.resolved_selection()
    );
    assert_eq!(exact_fast.stored_marks(), exact_slow.stored_marks());
    assert_eq!(exact_fast.can_undo(), exact_slow.can_undo());
    assert_eq!(exact_fast.can_redo(), exact_slow.can_redo());

    let mut one_under_slow = populated_engine();
    one_under_slow.resource_limits.max_input_bytes = required - 1;
    let before_slow = atomic_audit(&one_under_slow);
    let slow_error = one_under_slow
        .compile_typed_transaction(transaction(&one_under_slow, 764))
        .unwrap_err();
    assert_eq!(atomic_audit(&one_under_slow), before_slow);

    let mut one_under_fast = populated_engine();
    one_under_fast.resource_limits.max_input_bytes = required - 1;
    let before_fast = atomic_audit(&one_under_fast);
    let fast_error = one_under_fast
        .apply_typed_transaction_with_result(transaction(&one_under_fast, 764))
        .unwrap_err();
    assert_eq!(fast_error, slow_error);
    assert_eq!(atomic_audit(&one_under_fast), before_fast);

    let mut changed_document = populated_engine();
    changed_document
        .apply_command(765, TypedCommand::InsertText { text: "é".into() })
        .unwrap()
        .unwrap();
    let cached_text_bytes = changed_document
        .derived_state
        .as_ref()
        .unwrap()
        .document_text_bytes;
    assert_eq!(
        cached_text_bytes,
        changed_document
            .document()
            .unwrap()
            .root()
            .text_content()
            .len()
    );
    let changed_required = scan_work(&changed_document);
    changed_document.resource_limits.max_input_bytes = changed_required - 1;
    let before_changed = atomic_audit(&changed_document);
    let changed_error = changed_document
        .apply_typed_transaction_with_result(transaction(&changed_document, 766))
        .unwrap_err();
    assert_eq!(changed_error.code, "OPERATION_LIMIT_EXCEEDED");
    assert_eq!(
        changed_error.limit,
        Some(u64::try_from(changed_required - 1).unwrap())
    );
    assert_eq!(
        changed_error.actual,
        Some(u64::try_from(changed_required).unwrap())
    );
    assert_eq!(atomic_audit(&changed_document), before_changed);

    let invalid_selection = |engine: &YrsDocumentEngine| TypedTransaction {
        request_id: 767,
        base_document_revision: engine.revision(),
        origin: TransactionOrigin::LocalApi,
        operations: vec![],
        selection_intent: SelectionIntent::Set(SelectionInput::Text {
            anchor: RevisionedPosition {
                offset: u32::MAX,
                kind: EditorOffsetKind::Utf16,
                affinity: Affinity::Before,
            },
            head: RevisionedPosition {
                offset: u32::MAX,
                kind: EditorOffsetKind::Utf16,
                affinity: Affinity::After,
            },
        }),
        history_policy: HistoryPolicy::Skip,
    };
    let invalid_slow = populated_engine();
    let before_invalid_slow = atomic_audit(&invalid_slow);
    let invalid_slow_error = invalid_slow
        .compile_typed_transaction(invalid_selection(&invalid_slow))
        .unwrap_err();
    assert_eq!(atomic_audit(&invalid_slow), before_invalid_slow);
    let mut invalid_fast = populated_engine();
    let before_invalid_fast = atomic_audit(&invalid_fast);
    let invalid_fast_error = invalid_fast
        .apply_typed_transaction_with_result(invalid_selection(&invalid_fast))
        .unwrap_err();
    assert_eq!(invalid_fast_error, invalid_slow_error);
    assert_eq!(atomic_audit(&invalid_fast), before_invalid_fast);
}

#[test]
fn empty_skip_fast_path_matches_full_compiler_for_selection_forms_and_local_state() {
    fn populated_engine() -> YrsDocumentEngine {
        let mut engine = transaction_engine();
        engine
            .import_json(
                &json!({
                    "type": "doc",
                    "content": [
                        {"type": "paragraph", "content": [{"type": "text", "text": "a😀b"}]},
                        {"type": "horizontalRule"},
                        {"type": "paragraph", "content": [{"type": "text", "text": "tail"}]}
                    ]
                })
                .to_string(),
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        engine
    }

    fn transaction(
        engine: &YrsDocumentEngine,
        request_id: u64,
        selection_intent: SelectionIntent,
    ) -> TypedTransaction {
        TypedTransaction {
            request_id,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: vec![],
            selection_intent,
            history_policy: HistoryPolicy::Skip,
        }
    }

    fn slow_result(
        engine: &mut YrsDocumentEngine,
        transaction: TypedTransaction,
    ) -> crate::yrs_engine::TypedTransactionResult {
        let compiled = engine.compile_typed_transaction(transaction).unwrap();
        engine
            .apply_compiled_transaction(compiled, true)
            .unwrap()
            .1
            .unwrap()
    }

    let scalar = |offset, affinity| RevisionedPosition {
        offset,
        kind: EditorOffsetKind::Scalar,
        affinity,
    };
    let utf16 = |offset, affinity| RevisionedPosition {
        offset,
        kind: EditorOffsetKind::Utf16,
        affinity,
    };
    let intents = [
        SelectionIntent::Set(SelectionInput::Text {
            anchor: scalar(2, Affinity::Before),
            head: scalar(2, Affinity::Before),
        }),
        SelectionIntent::Set(SelectionInput::Text {
            anchor: scalar(2, Affinity::Before),
            head: scalar(2, Affinity::After),
        }),
        SelectionIntent::Set(SelectionInput::Text {
            anchor: utf16(3, Affinity::Before),
            head: utf16(3, Affinity::After),
        }),
        SelectionIntent::Set(SelectionInput::Node {
            at: scalar(4, Affinity::Before),
        }),
        SelectionIntent::Set(SelectionInput::All),
        SelectionIntent::Preserve,
        SelectionIntent::UseOperationResult,
    ];

    for (index, intent) in intents.into_iter().enumerate() {
        let mut fast = populated_engine();
        let mut slow = populated_engine();
        let fast_before = atomic_audit(&fast);
        let slow_before = atomic_audit(&slow);
        let fast_transaction = transaction(&fast, 770 + index as u64, intent.clone());
        let slow_transaction = transaction(&slow, 770 + index as u64, intent.clone());

        let fast_result = fast
            .apply_typed_transaction_with_result(fast_transaction)
            .unwrap();
        let slow_result = slow_result(&mut slow, slow_transaction);

        assert_eq!(fast_result, slow_result, "intent={intent:?}");
        assert_eq!(
            fast.document_json(),
            slow.document_json(),
            "intent={intent:?}"
        );
        assert_eq!(
            fast.document_html(),
            slow.document_html(),
            "intent={intent:?}"
        );
        assert_eq!(fast.revision(), slow.revision(), "intent={intent:?}");
        assert_eq!(
            fast.state_revision(),
            slow.state_revision(),
            "intent={intent:?}"
        );
        assert_eq!(
            fast.resolved_selection(),
            slow.resolved_selection(),
            "intent={intent:?}"
        );
        assert_eq!(
            fast.stored_marks(),
            slow.stored_marks(),
            "intent={intent:?}"
        );
        assert_eq!(fast.can_undo(), slow.can_undo(), "intent={intent:?}");
        assert_eq!(fast.can_redo(), slow.can_redo(), "intent={intent:?}");
        assert_eq!(fast.encoded_state().unwrap(), fast_before.encoded);
        assert_eq!(slow.encoded_state().unwrap(), slow_before.encoded);
        assert_eq!(fast.yrs_state_epoch, fast_before.yrs_state_epoch);
        assert_eq!(slow.yrs_state_epoch, slow_before.yrs_state_epoch);
        assert_eq!(
            fast.history.replay_audit_for_test(),
            fast_before.replay_audit
        );
        assert_eq!(
            slow.history.replay_audit_for_test(),
            slow_before.replay_audit
        );
    }

    let stored_mark_intents = [
        SelectionIntent::Set(SelectionInput::Text {
            anchor: scalar(1, Affinity::Before),
            head: scalar(1, Affinity::Before),
        }),
        SelectionIntent::Set(SelectionInput::Text {
            anchor: scalar(1, Affinity::Before),
            head: scalar(1, Affinity::After),
        }),
        SelectionIntent::Set(SelectionInput::Text {
            anchor: scalar(2, Affinity::Before),
            head: scalar(2, Affinity::Before),
        }),
        SelectionIntent::Set(SelectionInput::Node {
            at: scalar(4, Affinity::Before),
        }),
    ];
    for (index, intent) in stored_mark_intents.into_iter().enumerate() {
        let mut fast = populated_engine();
        let mut slow = populated_engine();
        select_text(&mut fast, 780, 1, 1);
        select_text(&mut slow, 780, 1, 1);
        for engine in [&mut fast, &mut slow] {
            engine
                .apply_command(
                    781,
                    TypedCommand::ToggleMark {
                        mark_type: "bold".into(),
                    },
                )
                .unwrap()
                .unwrap();
            assert!(engine.stored_marks().is_some());
        }
        let fast_transaction = transaction(&fast, 782 + index as u64, intent.clone());
        let slow_transaction = transaction(&slow, 782 + index as u64, intent.clone());

        let fast_result = fast
            .apply_typed_transaction_with_result(fast_transaction)
            .unwrap();
        let slow_result = slow_result(&mut slow, slow_transaction);

        assert_eq!(fast_result, slow_result, "stored intent={intent:?}");
        assert_eq!(
            fast.resolved_selection(),
            slow.resolved_selection(),
            "stored intent={intent:?}"
        );
        assert_eq!(
            fast.stored_marks(),
            slow.stored_marks(),
            "stored intent={intent:?}"
        );
        if index <= 1 {
            assert!(fast.stored_marks().is_some());
        } else {
            assert!(fast.stored_marks().is_none());
        }
    }
}
