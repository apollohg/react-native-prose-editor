#[test]
fn prepared_wrap_accepts_exact_output_limit_and_rejects_one_over_atomically() {
    use crate::yrs_engine::mutation::{
        reset_root_window_lowering_counts_for_test, take_root_window_lowering_counts_for_test,
    };

    let command = TypedCommand::WrapInList {
        list_type: "bulletList".into(),
        item_type: "listItem".into(),
    };
    let mut exact = 1;
    loop {
        let mut probe = transaction_engine();
        probe.editing_limits.max_derived_output_bytes = exact;
        match probe.apply_command(70_036_001, command.clone()) {
            Ok(Some(_)) => break,
            Err(error) if error.details == Some(json!({ "field": "maxDerivedOutputBytes" })) => {
                let required = usize::try_from(error.actual.unwrap()).unwrap();
                assert!(required > exact);
                exact = required;
            }
            outcome => panic!("unexpected output-limit probe result: {outcome:?}"),
        }
    }

    let mut exact_limit = transaction_engine();
    exact_limit.editing_limits.max_derived_output_bytes = exact;
    reset_root_window_lowering_counts_for_test();
    assert!(exact_limit
        .apply_command(70_036_002, command.clone())
        .unwrap()
        .is_some());
    let exact_counts = take_root_window_lowering_counts_for_test();
    assert_eq!((exact_counts.2, exact_counts.3), (1, 0));

    let mut one_over = transaction_engine();
    one_over.editing_limits.max_derived_output_bytes = exact - 1;
    let before = atomic_audit(&one_over);
    reset_root_window_lowering_counts_for_test();
    let error = one_over.apply_command(70_036_003, command).unwrap_err();
    let rejected_counts = take_root_window_lowering_counts_for_test();
    assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
    assert_eq!(error.actual, Some(exact as u64));
    assert_eq!(
        error.details,
        Some(json!({ "field": "maxDerivedOutputBytes" }))
    );
    assert_eq!((rejected_counts.2, rejected_counts.3), (1, 0));
    assert_eq!(atomic_audit(&one_over), before);
}

#[test]
fn prepared_wrap_undo_and_encoded_limits_match_public_eager_exactly() {
    use crate::yrs_engine::mutation::{
        reset_root_window_lowering_counts_for_test, take_root_window_lowering_counts_for_test,
    };
    use crate::yrs_engine::{EditingLimits, OperationResult, TypedTransactionResult};

    fn command() -> TypedCommand {
        TypedCommand::WrapInList {
            list_type: "bulletList".into(),
            item_type: "listItem".into(),
        }
    }

    fn fixture(field: &str, value: u64) -> YrsDocumentEngine {
        let mut resource_limits = ResourceLimits::default();
        let mut editing_limits = EditingLimits::default();
        match field {
            "maxUndoRetainedUnits" => editing_limits.max_undo_retained_units = value,
            "maxEncodedStateBytes" => {
                resource_limits.max_encoded_state_bytes = usize::try_from(value).unwrap()
            }
            _ => unreachable!(),
        }
        YrsDocumentEngine::new(YrsEngineConfig {
            schema: tiptap_schema(),
            fragment_name: "prosemirror".into(),
            initialization_mode: crate::yrs_engine::InitializationMode::LocalEmpty,
            resource_limits,
            editing_limits,
            max_length: None,
            scope: Some(crate::yrs_engine::DocumentScope {
                document_id: "doc".into(),
                lineage_id: "lineage".into(),
            }),
        })
        .unwrap()
    }

    fn public_eager_apply(
        engine: &mut YrsDocumentEngine,
        request_id: u64,
    ) -> OperationResult<TypedTransactionResult> {
        let CommandPlan::Transaction(transaction) = engine.plan_command(request_id, command())?
        else {
            panic!("WrapInList must produce a public typed transaction")
        };
        engine.apply_typed_transaction_with_result(transaction)
    }

    fn prepared_apply(
        engine: &mut YrsDocumentEngine,
        request_id: u64,
    ) -> OperationResult<TypedTransactionResult> {
        Ok(engine
            .apply_command(request_id, command())?
            .expect("WrapInList must produce a transaction result"))
    }

    fn exact_undo_limit() -> u64 {
        let field = "maxUndoRetainedUnits";
        let mut limit = 1;
        loop {
            let mut probe = fixture(field, limit);
            match public_eager_apply(&mut probe, 70_036_010) {
                Ok(_) => return limit,
                Err(error) => {
                    assert_eq!(error.details, Some(json!({ "field": field })), "{field}");
                    let actual = error.actual.expect("limit rejection must report actual");
                    assert!(actual > limit, "{field} probe must make progress");
                    limit = actual;
                }
            }
        }
    }

    let field = "maxUndoRetainedUnits";
    let exact = exact_undo_limit();
    let request_id = 70_036_020;

    let mut prepared = fixture(field, exact);
    reset_root_window_lowering_counts_for_test();
    let prepared_result = prepared
        .apply_command(request_id, command())
        .unwrap()
        .unwrap();
    assert_eq!(
        take_root_window_lowering_counts_for_test(),
        (0, 0, 1, 0, 0, 1),
        "{field} prepared exact"
    );

    let mut generic = fixture(field, exact);
    reset_root_window_lowering_counts_for_test();
    let generic_result = public_eager_apply(&mut generic, request_id).unwrap();
    assert_eq!(
        take_root_window_lowering_counts_for_test(),
        (1, 1, 0, 0, 1, 0),
        "{field} eager exact"
    );
    assert_eq!(prepared_result, generic_result, "{field} exact");
    assert_eq!(prepared.document_json(), generic.document_json(), "{field}");
    assert_eq!(prepared.document_html(), generic.document_html(), "{field}");
    assert_eq!(
        prepared.resolved_selection(),
        generic.resolved_selection(),
        "{field}"
    );
    assert_eq!(prepared.stored_marks(), generic.stored_marks(), "{field}");
    assert_eq!(prepared.can_undo(), generic.can_undo(), "{field}");
    assert_eq!(prepared.can_redo(), generic.can_redo(), "{field}");

    let limit = exact.checked_sub(1).expect("wrap limits must be nonzero");
    let mut rejected_prepared = fixture(field, limit);
    let prepared_before = atomic_audit(&rejected_prepared);
    reset_root_window_lowering_counts_for_test();
    let prepared_error = rejected_prepared
        .apply_command(request_id, command())
        .unwrap_err();
    let prepared_counts = take_root_window_lowering_counts_for_test();
    assert_eq!(atomic_audit(&rejected_prepared), prepared_before, "{field}");

    let mut rejected_generic = fixture(field, limit);
    let generic_before = atomic_audit(&rejected_generic);
    reset_root_window_lowering_counts_for_test();
    let generic_error = public_eager_apply(&mut rejected_generic, request_id).unwrap_err();
    let generic_counts = take_root_window_lowering_counts_for_test();
    assert_eq!(atomic_audit(&rejected_generic), generic_before, "{field}");
    assert_eq!(prepared_error, generic_error, "{field}");
    assert_eq!(
        prepared_error.details,
        Some(json!({ "field": field })),
        "{field}"
    );
    assert_eq!(prepared_error.limit, Some(limit), "{field}");
    assert_eq!(prepared_error.actual, Some(exact), "{field}");

    let expected_prepared_counts = (0, 0, 1, 0, 0, 0);
    let expected_generic_counts = (1, 1, 0, 0, 0, 0);
    assert_eq!(
        prepared_counts, expected_prepared_counts,
        "{field} prepared reject"
    );
    assert_eq!(
        generic_counts, expected_generic_counts,
        "{field} eager reject"
    );

    fn exercise_max_encoded_state_boundary(
        request_id: u64,
        apply: fn(&mut YrsDocumentEngine, u64) -> OperationResult<TypedTransactionResult>,
        probe_counts: (usize, usize, usize, usize, usize, usize),
        rejected_counts: (usize, usize, usize, usize, usize, usize),
        success_counts: (usize, usize, usize, usize, usize, usize),
    ) -> (YrsDocumentEngine, TypedTransactionResult) {
        let field = "maxEncodedStateBytes";
        let default_limit =
            u64::try_from(ResourceLimits::default().max_encoded_state_bytes).unwrap();
        let mut engine = fixture(field, default_limit);
        let before = atomic_audit(&engine);
        let current_encoded = u64::try_from(engine.encoded_state().unwrap().len()).unwrap();

        engine.resource_limits.max_encoded_state_bytes = usize::try_from(current_encoded).unwrap();
        reset_root_window_lowering_counts_for_test();
        let probe_error = apply(&mut engine, request_id).unwrap_err();
        assert_eq!(
            take_root_window_lowering_counts_for_test(),
            probe_counts,
            "{field} probe"
        );
        assert_eq!(atomic_audit(&engine), before, "{field} probe");
        assert_eq!(probe_error.code, "DOCUMENT_LIMIT_EXCEEDED");
        assert_eq!(probe_error.details, Some(json!({ "field": field })));
        assert_eq!(probe_error.limit, Some(current_encoded));
        let exact = probe_error
            .actual
            .expect("encoded-state rejection must report the exact instance size");
        assert!(exact > current_encoded);
        let one_under = exact
            .checked_sub(1)
            .expect("encoded state must consume at least one byte");

        engine.resource_limits.max_encoded_state_bytes = usize::try_from(one_under).unwrap();
        reset_root_window_lowering_counts_for_test();
        let one_under_error = apply(&mut engine, request_id).unwrap_err();
        assert_eq!(
            take_root_window_lowering_counts_for_test(),
            rejected_counts,
            "{field} one-under"
        );
        assert_eq!(atomic_audit(&engine), before, "{field} one-under");
        assert_eq!(one_under_error.code, "DOCUMENT_LIMIT_EXCEEDED");
        assert_eq!(one_under_error.details, Some(json!({ "field": field })));
        assert_eq!(one_under_error.limit, Some(one_under));
        assert_eq!(one_under_error.actual, Some(exact));

        engine.resource_limits.max_encoded_state_bytes = usize::try_from(exact).unwrap();
        reset_root_window_lowering_counts_for_test();
        let result = apply(&mut engine, request_id).unwrap();
        assert_eq!(
            take_root_window_lowering_counts_for_test(),
            success_counts,
            "{field} exact"
        );
        assert!(engine.encoded_state().unwrap().len() <= usize::try_from(exact).unwrap());
        (engine, result)
    }

    let request_id = 70_036_021;
    // The mutation entry point refreshes the ResourceLimits-bound lookup
    // seed before compilation, so the prepared root window remains valid.
    let (prepared, prepared_result) = exercise_max_encoded_state_boundary(
        request_id,
        prepared_apply,
        (0, 0, 1, 0, 1, 0),
        (0, 0, 1, 0, 1, 0),
        (0, 0, 1, 0, 1, 1),
    );
    let (generic, generic_result) = exercise_max_encoded_state_boundary(
        request_id,
        public_eager_apply,
        (1, 1, 0, 0, 1, 0),
        (1, 1, 0, 0, 1, 0),
        (1, 1, 0, 0, 2, 0),
    );
    assert_eq!(
        prepared_result, generic_result,
        "maxEncodedStateBytes exact"
    );
    assert_eq!(prepared.document_json(), generic.document_json());
    assert_eq!(prepared.document_html(), generic.document_html());
    assert_eq!(prepared.resolved_selection(), generic.resolved_selection());
    assert_eq!(prepared.stored_marks(), generic.stored_marks());
    assert_eq!(prepared.can_undo(), generic.can_undo());
    assert_eq!(prepared.can_redo(), generic.can_redo());
}

#[test]
fn prepared_wrap_is_atomic_at_every_recoverable_failpoint() {
    use crate::yrs_engine::compiler::{set_atomic_failpoint_for_test, AtomicFailpoint};
    use crate::yrs_engine::mutation::{
        reset_root_window_lowering_counts_for_test, take_root_window_lowering_counts_for_test,
    };

    let failpoints = [
        AtomicFailpoint::EnvelopeAdmission,
        AtomicFailpoint::SemanticCompilation,
        AtomicFailpoint::MutationPreflight,
        AtomicFailpoint::FinalPreflight,
        AtomicFailpoint::EncodedAdmission,
        AtomicFailpoint::CanonicalOutputAdmission,
        AtomicFailpoint::RevisionAdmission,
        AtomicFailpoint::DurableMetadataAdmission,
    ];
    for (index, failpoint) in failpoints.into_iter().enumerate() {
        let mut engine = transaction_engine();
        let before = atomic_audit(&engine);
        let seed_before = engine
            .derived_state
            .as_ref()
            .unwrap()
            .mutation_lookup_seed
            .clone();
        assert!(seed_before.is_ready_for_test());
        reset_root_window_lowering_counts_for_test();
        set_atomic_failpoint_for_test(Some(failpoint));

        let error = engine
            .apply_command(
                70_036_100 + index as u64,
                TypedCommand::WrapInList {
                    list_type: "bulletList".into(),
                    item_type: "listItem".into(),
                },
            )
            .unwrap_err();

        set_atomic_failpoint_for_test(None);
        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED", "{failpoint:?}");
        assert_eq!(
            error.details,
            Some(json!({ "failpoint": failpoint.field_name() })),
            "{failpoint:?}"
        );
        assert_eq!(
            take_root_window_lowering_counts_for_test().5,
            0,
            "{failpoint:?}"
        );
        assert!(Arc::ptr_eq(
            &seed_before,
            &engine.derived_state.as_ref().unwrap().mutation_lookup_seed
        ));
        assert!(engine
            .derived_state
            .as_ref()
            .unwrap()
            .mutation_lookup_seed
            .is_ready_for_test());
        assert_eq!(atomic_audit(&engine), before, "{failpoint:?}");
    }
}

#[test]
fn prepared_toggle_mark_is_atomic_at_every_recoverable_failpoint() {
    use crate::yrs_engine::compiler::{set_atomic_failpoint_for_test, AtomicFailpoint};
    use crate::yrs_engine::mutation::{
        reset_localized_lookup_counts_for_test, reset_range_format_lowering_counts_for_test,
        take_localized_lookup_counts_for_test, take_range_format_lowering_counts_for_test,
    };

    let failpoints = [
        AtomicFailpoint::EnvelopeAdmission,
        AtomicFailpoint::SemanticCompilation,
        AtomicFailpoint::MutationPreflight,
        AtomicFailpoint::FinalPreflight,
        AtomicFailpoint::EncodedAdmission,
        AtomicFailpoint::CanonicalOutputAdmission,
        AtomicFailpoint::RevisionAdmission,
        AtomicFailpoint::DurableMetadataAdmission,
    ];
    for (index, failpoint) in failpoints.into_iter().enumerate() {
        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        select_text(&mut engine, 70_036_200 + index as u64, 0, 3);
        hydrate_import_for_compile_test(&mut engine);
        let before = atomic_audit(&engine);
        let seed_before = engine
            .derived_state
            .as_ref()
            .unwrap()
            .mutation_lookup_seed
            .clone();
        reset_localized_lookup_counts_for_test();
        reset_range_format_lowering_counts_for_test();
        set_atomic_failpoint_for_test(Some(failpoint));

        let error = engine
            .apply_command(
                70_036_300 + index as u64,
                TypedCommand::ToggleMark {
                    mark_type: "bold".into(),
                },
            )
            .unwrap_err();

        set_atomic_failpoint_for_test(None);
        let lookup_counts = take_localized_lookup_counts_for_test();
        let range_counts = take_range_format_lowering_counts_for_test();
        let expected_range_counts = if matches!(
            failpoint,
            AtomicFailpoint::EnvelopeAdmission | AtomicFailpoint::SemanticCompilation
        ) {
            (0, 0, 0, 0)
        } else {
            (0, 0, 1, 0)
        };
        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED", "{failpoint:?}");
        assert_eq!(
            error.details,
            Some(json!({ "failpoint": failpoint.field_name() })),
            "{failpoint:?}"
        );
        assert_eq!(range_counts, expected_range_counts, "{failpoint:?}");
        assert_eq!(lookup_counts, (0, 0, 0), "{failpoint:?}");
        assert!(Arc::ptr_eq(
            &seed_before,
            &engine.derived_state.as_ref().unwrap().mutation_lookup_seed
        ));
        assert_eq!(atomic_audit(&engine), before, "{failpoint:?}");
    }
}

#[test]
fn prepared_wrap_matches_the_public_planned_transaction_path() {
    let mut prepared = transaction_engine();
    let mut generic = transaction_engine();
    let command = TypedCommand::WrapInList {
        list_type: "bulletList".into(),
        item_type: "listItem".into(),
    };

    let prepared_result = prepared
        .apply_command(70_037, command.clone())
        .unwrap()
        .unwrap();
    let CommandPlan::Transaction(transaction) = generic.plan_command(70_037, command).unwrap()
    else {
        panic!("public wrap planning must produce a transaction")
    };
    let generic_result = generic
        .apply_typed_transaction_with_result(transaction)
        .unwrap();

    assert_eq!(prepared_result, generic_result);
    assert_eq!(prepared.document_json(), generic.document_json());
    assert_eq!(prepared.document_html(), generic.document_html());
    assert_eq!(prepared.resolved_selection(), generic.resolved_selection());
    assert_eq!(prepared.stored_marks(), generic.stored_marks());
    assert_eq!(prepared.can_undo(), generic.can_undo());
    assert_eq!(prepared.can_redo(), generic.can_redo());

    assert_eq!(
        prepared.undo_with_result(70_038).unwrap(),
        generic.undo_with_result(70_038).unwrap()
    );
    assert_eq!(prepared.document_json(), generic.document_json());
    assert_eq!(
        prepared.redo_with_result(70_039).unwrap(),
        generic.redo_with_result(70_039).unwrap()
    );
    assert_eq!(prepared.document_json(), generic.document_json());
}

#[test]
fn derived_state_node_count_refreshes_and_empty_results_use_equivalent_commands() {
    let mut engine = transaction_engine();
    let initial = engine.derived_state.as_ref().unwrap();
    assert_eq!(
        initial.document_node_count,
        crate::editor_state::document_node_count(initial.document.root())
    );

    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"one"}]},{"type":"paragraph","content":[{"type":"text","text":"two"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let refreshed = engine.derived_state.as_ref().unwrap();
    assert_eq!(refreshed.document_revision, engine.revision());
    assert_eq!(
        refreshed.document_node_count,
        crate::editor_state::document_node_count(refreshed.document.root())
    );

    let transaction = TypedTransaction {
        request_id: 991,
        base_document_revision: engine.revision(),
        origin: TransactionOrigin::LocalApi,
        operations: Vec::new(),
        selection_intent: SelectionIntent::Set(SelectionInput::Text {
            anchor: RevisionedPosition {
                offset: 1,
                kind: EditorOffsetKind::Scalar,
                affinity: Affinity::Before,
            },
            head: RevisionedPosition {
                offset: 1,
                kind: EditorOffsetKind::Scalar,
                affinity: Affinity::Before,
            },
        }),
        history_policy: HistoryPolicy::Skip,
    };
    let result = engine
        .apply_typed_transaction_with_result(transaction)
        .unwrap();
    let state = engine.derived_state.as_ref().unwrap();
    let selection = state.legacy_selection();
    assert_eq!(
        result.active_state.commands,
        crate::editor_state::command_applicability(
            &state.document,
            &engine.schema,
            &selection,
            &engine.resource_limits,
        )
    );
}
