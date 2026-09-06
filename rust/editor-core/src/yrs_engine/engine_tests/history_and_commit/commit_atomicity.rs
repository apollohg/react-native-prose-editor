#[test]
fn cached_render_preparation_failure_is_atomic_before_durable_write() {
    let mut engine = transaction_engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let before = atomic_audit(&engine);
    crate::render::incremental::set_cached_render_error_for_test(Some(
        crate::render::incremental::CachedRenderError::AllocationFailed,
    ));
    let error = engine
        .apply_typed_transaction(insert_transaction(&engine, 109))
        .unwrap_err();
    crate::render::incremental::set_cached_render_error_for_test(None);

    assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
    assert_eq!(atomic_audit(&engine), before);
}

fn apply_render_update_for_test(
    mut old_blocks: Vec<Vec<crate::render::RenderElement>>,
    update: crate::yrs_engine::RenderUpdate,
) -> Vec<Vec<crate::render::RenderElement>> {
    match update {
        crate::yrs_engine::RenderUpdate::None => old_blocks,
        crate::yrs_engine::RenderUpdate::Full(blocks) => blocks,
        crate::yrs_engine::RenderUpdate::Patch(patch) => {
            old_blocks.splice(
                patch.start_index..patch.start_index + patch.delete_count,
                patch.blocks,
            );
            old_blocks
        }
    }
}

#[test]
fn direct_command_admission_error_is_not_replanned_as_structure() {
    let mut engine = transaction_engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"target"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    select_text(&mut engine, 104, 6, 0);
    engine.resource_limits.max_input_bytes = 0;
    let before = atomic_audit(&engine);

    let error = engine
        .plan_command(105, TypedCommand::ReplaceSelectionText { text: "x".into() })
        .unwrap_err();

    assert_eq!(error.code, "OPERATION_LIMIT_EXCEEDED");
    assert_eq!(error.details, Some(json!({ "field": "maxInputBytes" })));
    assert_eq!(atomic_audit(&engine), before);
}

#[test]
fn every_recoverable_atomic_stage_failpoint_is_pre_open_and_read_only() {
    use crate::yrs_engine::compiler::{set_atomic_failpoint_for_test, AtomicFailpoint};

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
    for failpoint in failpoints {
        let mut engine = transaction_engine();
        let transaction = insert_transaction(&engine, 76);
        let before = atomic_audit(&engine);
        let canonical_before = engine
            .derived_state
            .as_ref()
            .unwrap()
            .canonical_artifact
            .clone();
        set_atomic_failpoint_for_test(Some(failpoint));

        let error = engine.apply_typed_transaction(transaction).unwrap_err();

        set_atomic_failpoint_for_test(None);
        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED", "{failpoint:?}");
        assert_eq!(
            error.details,
            Some(json!({ "failpoint": failpoint.field_name() })),
            "{failpoint:?}"
        );
        assert_eq!(atomic_audit(&engine), before, "{failpoint:?}");
        assert!(canonical_before.ptr_eq(&engine.derived_state.as_ref().unwrap().canonical_artifact));
    }
}

#[test]
fn compiled_history_failure_does_not_publish_candidate_active_state_lifecycle() {
    use crate::yrs_engine::derived_state::{
        reset_active_state_cache_counts_for_test, take_active_state_cache_counts_for_test,
    };

    for pending_install in [true, false] {
        let request_id = if pending_install { 760_010 } else { 760_020 };
        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        let point = RevisionedPosition {
            offset: 1,
            kind: EditorOffsetKind::Scalar,
            affinity: Affinity::After,
        };
        engine
            .apply_typed_transaction(TypedTransaction {
                request_id,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalApi,
                operations: Vec::new(),
                selection_intent: SelectionIntent::Set(SelectionInput::Text {
                    anchor: point,
                    head: point,
                }),
                history_policy: HistoryPolicy::Skip,
            })
            .unwrap();
        engine
            .apply_command(
                request_id + 1,
                TypedCommand::InsertText { text: "x".into() },
            )
            .unwrap()
            .unwrap();
        let live_certificate = engine
            .derived_state
            .as_ref()
            .unwrap()
            .active_state_cache_for_test()
            .expect("fixture must retain a live active-state certificate");
        let preparation = std::cell::RefCell::new(None);
        let CommandPlan::Transaction(transaction) = engine
            .plan_command_internal(
                request_id + 2,
                TypedCommand::InsertText { text: "y".into() },
                Some(&preparation),
            )
            .unwrap()
        else {
            panic!("insert command must prepare a transaction")
        };
        let mut compiled = engine
            .compile_prepared_typed_transaction(transaction, preparation.into_inner().unwrap())
            .unwrap();
        assert!(compiled.prepared_active_state_transition.is_some());
        if !pending_install {
            compiled.prepared_active_state_transition = None;
        }
        let before = atomic_audit(&engine);
        reset_active_state_cache_counts_for_test();
        set_compiled_commit_stage_failpoint_for_test(Some(
            CompiledCommitPreparationStage::HistorySnapshotConstruction,
        ));

        let error = engine
            .apply_compiled_transaction(compiled, true)
            .expect_err("late snapshot construction must reject the prepared candidate");

        set_compiled_commit_stage_failpoint_for_test(None);
        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED", "{pending_install}");
        assert!(
            error.message.contains("historySnapshotConstruction"),
            "{pending_install}"
        );
        let counts = take_active_state_cache_counts_for_test();
        assert_eq!(counts.5, 0, "pending install={pending_install}");
        assert_eq!(counts.6, 0, "pending install={pending_install}");
        assert_eq!(atomic_audit(&engine), before, "{pending_install}");
        assert!(Arc::ptr_eq(
            &live_certificate,
            &engine
                .derived_state
                .as_ref()
                .unwrap()
                .active_state_cache_for_test()
                .unwrap(),
        ));
    }
}

#[test]
fn compiled_recorded_history_admission_preserves_live_replay_allocation_on_later_failure() {
    let mut engine = transaction_engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    engine.history.compact_replay_event_capacity_for_test();
    let before = atomic_audit(&engine);
    let ledger_before = engine.history.replay_ledger_allocation_audit_for_test();
    let mut transaction = insert_transaction(&engine, 760_030);
    transaction.history_policy = HistoryPolicy::Auto;
    set_compiled_commit_stage_failpoint_for_test(Some(
        CompiledCommitPreparationStage::HistoryUpdateEncoding,
    ));

    let error = engine
        .apply_typed_transaction(transaction)
        .expect_err("candidate update encoding must fail after recorded admission");

    set_compiled_commit_stage_failpoint_for_test(None);
    assert!(error.message.contains("historyUpdateEncoding"));
    assert_eq!(atomic_audit(&engine), before);
    assert_eq!(
        engine.history.replay_ledger_allocation_audit_for_test(),
        ledger_before
    );
}

#[test]
fn compiled_excluded_history_admission_preserves_live_replay_allocation_on_later_failure() {
    let mut engine = transaction_engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    engine.history.compact_replay_event_capacity_for_test();
    let before = atomic_audit(&engine);
    let ledger_before = engine.history.replay_ledger_allocation_audit_for_test();
    let transaction = insert_transaction(&engine, 760_040);
    set_compiled_commit_stage_failpoint_for_test(Some(
        CompiledCommitPreparationStage::HistoryUpdateEncoding,
    ));

    let error = engine
        .apply_typed_transaction(transaction)
        .expect_err("candidate update encoding must fail after excluded admission");

    set_compiled_commit_stage_failpoint_for_test(None);
    assert!(error.message.contains("historyUpdateEncoding"));
    assert_eq!(atomic_audit(&engine), before);
    assert_eq!(
        engine.history.replay_ledger_allocation_audit_for_test(),
        ledger_before
    );
}

#[test]
fn compiled_history_admission_error_precedes_candidate_preparation_failure() {
    use crate::yrs_engine::history::set_replay_update_allocation_failure_for_test;

    let mut engine = transaction_engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let mut transaction = insert_transaction(&engine, 760_020);
    transaction.history_policy = HistoryPolicy::Auto;
    let compiled = engine.compile_typed_transaction(transaction).unwrap();
    let before = atomic_audit(&engine);
    set_replay_update_allocation_failure_for_test(true);
    set_compiled_commit_stage_failpoint_for_test(Some(
        CompiledCommitPreparationStage::HistoryUpdateEncoding,
    ));

    let error = engine
        .apply_compiled_transaction(compiled, true)
        .expect_err("history admission must win error precedence");

    set_replay_update_allocation_failure_for_test(false);
    set_compiled_commit_stage_failpoint_for_test(None);
    assert_eq!(error.code, "OPERATION_RESOURCE_EXHAUSTED");
    assert_eq!(error.details, Some(json!({ "field": "historyReplay" })));
    assert_eq!(atomic_audit(&engine), before);

    let mut lookup_first = transaction_engine();
    lookup_first
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    lookup_first.ensure_mutation_lookup_seed(760_021).unwrap();
    let mut transaction = insert_transaction(&lookup_first, 760_022);
    transaction.history_policy = HistoryPolicy::Auto;
    let compiled = lookup_first.compile_typed_transaction(transaction).unwrap();
    let before = atomic_audit(&lookup_first);
    set_replay_update_allocation_failure_for_test(true);
    set_compiled_commit_stage_failpoint_for_test(Some(
        CompiledCommitPreparationStage::LookupTransition,
    ));

    let error = lookup_first
        .apply_compiled_transaction(compiled, true)
        .expect_err("baseline lookup failure must retain precedence over history admission");

    set_replay_update_allocation_failure_for_test(false);
    set_compiled_commit_stage_failpoint_for_test(None);
    assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
    assert!(error.message.contains("lookupTransition"));
    assert_eq!(atomic_audit(&lookup_first), before);
}

#[test]
fn compiled_first_structural_mutation_supports_an_empty_configured_root() {
    let schema = crate::schema::Schema::from_json(&json!({
        "nodes": [
            { "name": "doc", "content": "block*", "role": "doc" },
            { "name": "paragraph", "content": "inline*", "group": "block", "role": "textBlock", "htmlTag": "p" },
            { "name": "text", "group": "inline", "role": "text" }
        ],
        "marks": []
    }))
    .unwrap();
    let mut engine = YrsDocumentEngine::new(YrsEngineConfig {
        schema,
        fragment_name: "empty-root".into(),
        initialization_mode: crate::yrs_engine::InitializationMode::LocalEmpty,
        resource_limits: ResourceLimits::default(),
        editing_limits: crate::yrs_engine::EditingLimits::default(),
        max_length: None,
        scope: Some(crate::yrs_engine::DocumentScope {
            document_id: "empty-root-doc".into(),
            lineage_id: "empty-root-lineage".into(),
        }),
    })
    .unwrap();
    let initial_json = engine.document_json().unwrap();
    let initial_encoded = engine.encoded_state().unwrap();
    let initial_revision = engine.revision();
    let initial_state_revision = engine.state_revision();
    let initial_selection = engine.resolved_selection().cloned();
    let initial_history = engine.history.replay_audit_for_test();
    let result = engine
        .apply_typed_transaction_with_result(TypedTransaction {
            request_id: 760_030,
            base_document_revision: initial_revision,
            origin: TransactionOrigin::LocalInput,
            operations: vec![TypedOperation::InsertNode {
                at: RevisionedPosition {
                    offset: 0,
                    kind: EditorOffsetKind::Scalar,
                    affinity: Affinity::After,
                },
                node: crate::model::Node::element(
                    "paragraph".into(),
                    HashMap::new(),
                    crate::model::Fragment::empty(),
                ),
            }],
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Boundary,
        })
        .unwrap();

    let changed_json = engine.document_json().unwrap();
    assert_ne!(engine.encoded_state().unwrap(), initial_encoded);
    assert_eq!(changed_json["type"], "doc");
    assert_eq!(changed_json["content"][0]["type"], "paragraph");
    assert_eq!(engine.revision(), initial_revision + 1);
    assert_eq!(engine.state_revision(), initial_state_revision + 1);
    assert_eq!(result.document_revision, engine.revision());
    assert_eq!(result.state_revision, engine.state_revision());
    assert_eq!(engine.resolved_selection(), Some(&result.selection));
    assert_eq!(result.history_state.can_undo, engine.can_undo());
    assert_eq!(result.history_state.can_redo, engine.can_redo());
    assert!(engine.can_undo());
    assert_ne!(engine.history.replay_audit_for_test(), initial_history);

    let undo = engine
        .undo(760_031)
        .unwrap()
        .expect("insert must be undoable");
    assert!(undo.changed);
    assert_eq!(engine.document_json().unwrap(), initial_json);
    assert_eq!(engine.resolved_selection(), initial_selection.as_ref());
    assert!(!engine.can_undo());
    assert!(engine.can_redo());
}

#[test]
fn compiled_excluded_rebase_rolls_baseline_and_appends_the_event() {
    let mut engine = transaction_engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let before_encoded = engine.encoded_state().unwrap();
    engine.history.force_rebase_before_next_event_for_test();
    let transaction = insert_transaction(&engine, 760_040);

    engine.apply_typed_transaction(transaction).unwrap();

    let (rebase, baseline, event_count, last_is_excluded) =
        engine.history.compiled_excluded_rebase_audit_for_test();
    assert!(!rebase);
    assert_eq!(baseline, before_encoded);
    assert_eq!(event_count, 1);
    assert!(last_is_excluded);
}

#[test]
fn compiled_commit_guard_rejects_every_preparation_stage_after_durable_open() {
    let stages = [
        CompiledCommitPreparationStage::AllocationProbe,
        CompiledCommitPreparationStage::OperationPreparation,
        CompiledCommitPreparationStage::DocumentValidation,
        CompiledCommitPreparationStage::LookupTransition,
        CompiledCommitPreparationStage::HistoryReservation,
        CompiledCommitPreparationStage::HistoryUpdateEncoding,
        CompiledCommitPreparationStage::SelectionFinalization,
        CompiledCommitPreparationStage::DerivedStateBuild,
        CompiledCommitPreparationStage::HistorySnapshotConstruction,
    ];
    for stage in stages {
        set_compiled_commit_stage_failpoint_for_test(None);
        mark_compiled_commit_durable_write_for_test();
        let error = check_compiled_commit_preparation_stage_for_test(760_050, stage)
            .expect_err("every guarded preparation stage must reject after durable open");
        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED", "{stage:?}");
        assert!(error.message.contains("postwrite"), "{stage:?}");
    }
    set_compiled_commit_stage_failpoint_for_test(None);
}

#[test]
fn compiled_commit_prepares_all_recoverable_work_before_durable_write() {
    let stages = [
        CompiledCommitPreparationStage::AllocationProbe,
        CompiledCommitPreparationStage::OperationPreparation,
        CompiledCommitPreparationStage::DocumentValidation,
        CompiledCommitPreparationStage::LookupTransition,
        CompiledCommitPreparationStage::HistoryReservation,
        CompiledCommitPreparationStage::HistoryUpdateEncoding,
        CompiledCommitPreparationStage::SelectionFinalization,
        CompiledCommitPreparationStage::DerivedStateBuild,
        CompiledCommitPreparationStage::HistorySnapshotConstruction,
    ];
    for (index, stage) in stages.into_iter().enumerate() {
        let mut engine = transaction_engine();
        let request_id = 760_100 + index as u64;
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        engine.ensure_mutation_lookup_seed(request_id).unwrap();
        let mut transaction = insert_transaction(&engine, request_id);
        transaction.history_policy = HistoryPolicy::Auto;
        let before = atomic_audit(&engine);
        let seed_before = engine
            .derived_state
            .as_ref()
            .expect("ready fixture has derived state")
            .mutation_lookup_seed
            .clone();
        set_compiled_commit_stage_failpoint_for_test(Some(stage));

        let error = engine
            .apply_typed_transaction(transaction)
            .expect_err("every recoverable compiled-commit stage must be injectable");

        set_compiled_commit_stage_failpoint_for_test(None);
        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED", "{stage:?}");
        assert_eq!(atomic_audit(&engine), before, "{stage:?}");
        assert!(Arc::ptr_eq(
            &seed_before,
            &engine
                .derived_state
                .as_ref()
                .expect("failed commit retains derived state")
                .mutation_lookup_seed,
        ));
    }
}

#[test]
fn localized_seed_promotion_is_not_installed_before_any_recoverable_failpoint() {
    use crate::yrs_engine::compiler::{set_atomic_failpoint_for_test, AtomicFailpoint};
    use crate::yrs_engine::mutation::{
        reset_localized_lookup_counts_for_test, take_localized_lookup_counts_for_test,
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
    for failpoint in failpoints {
        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"a😀b"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        let transaction = insert_transaction(&engine, 76_001);
        let before = atomic_audit(&engine);
        reset_localized_lookup_counts_for_test();
        set_atomic_failpoint_for_test(Some(failpoint));

        let error = engine.apply_typed_transaction(transaction).unwrap_err();

        set_atomic_failpoint_for_test(None);
        let (_, _, promotions) = take_localized_lookup_counts_for_test();
        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED", "{failpoint:?}");
        assert_eq!(promotions, 0, "{failpoint:?}");
        assert_eq!(atomic_audit(&engine), before, "{failpoint:?}");
    }
}
