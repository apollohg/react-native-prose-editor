use super::*;

#[test]
fn utf16_doc_preserves_fresh_client_ids_and_uses_utf16_offsets() {
    let first = utf16_doc();
    let second = utf16_doc();

    assert_eq!(first.offset_kind(), OffsetKind::Utf16);
    assert_eq!(second.offset_kind(), OffsetKind::Utf16);
    assert_ne!(first.client_id(), second.client_id());
}

#[test]
fn validated_import_source_reuses_one_schema_ranked_canonical_result() {
    use crate::yrs_engine::canonical::{
        reset_canonical_artifact_counts_for_test, reset_canonical_schema_context_count_for_test,
        take_canonical_artifact_counts_for_test, take_canonical_schema_context_count_for_test,
    };

    let schema = tiptap_schema();
    let limits = ResourceLimits::default();
    let input = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{
                "type": "text",
                "text": "ordered",
                "marks": [{ "type": "bold" }, { "type": "italic" }]
            }]
        }]
    });
    let parsed = from_prosemirror_json(&input, &schema, UnknownTypeMode::Preserve).unwrap();
    let canonical_schema = crate::yrs_engine::canonical::CanonicalSchemaContext::new(&schema);
    let engine = transaction_engine();
    reset_canonical_artifact_counts_for_test();
    reset_canonical_schema_context_count_for_test();
    crate::yrs_engine::observability::reset_full_pass_counts_for_test();

    let input_len = serde_json::to_vec(&input).unwrap().len();
    let validated =
        ValidatedImportDocument::new(parsed, &schema, &canonical_schema, &limits, Some(input_len))
            .unwrap();
    let artifact = validated.canonical_artifact.clone();

    assert_eq!(
        validated.canonical_artifact.value(),
        &json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{
                    "type": "text",
                    "text": "ordered",
                    "marks": [{ "type": "bold" }, { "type": "italic" }]
                }]
            }]
        })
    );
    assert_eq!(
        validated.canonical_artifact.value(),
        &crate::serialize::to_prosemirror_json(&validated.document, &schema)
    );
    let candidate = engine
        .build_candidate_from_document(validated, TransactionOrigin::DocumentImport)
        .unwrap();
    let super::EngineDocumentState::Ready {
        canonical_artifact, ..
    } = candidate.state
    else {
        panic!("validated import candidate must be ready")
    };
    assert!(artifact.ptr_eq(&canonical_artifact));
    assert_eq!(take_canonical_artifact_counts_for_test(), (1, 0));
    assert_eq!(take_canonical_schema_context_count_for_test(), 0);
    let counts = crate::yrs_engine::observability::take_full_pass_counts_for_test();
    assert_eq!(counts.canonical_mark_nodes_visited, 3);
    assert_eq!(counts.canonical_identity_predicate_nodes_visited, 0);
}

#[test]
fn admitted_import_runs_one_validation_certificate_and_render_path() {
    use crate::yrs_engine::observability::{
        reset_full_pass_counts_for_test, take_full_pass_counts_for_test,
    };

    let mut engine = transaction_engine();
    reset_full_pass_counts_for_test();
    crate::render::incremental::reset_cached_render_counts_for_test();

    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"one"}]},{"type":"paragraph","content":[{"type":"text","text":"two"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();

    let passes = take_full_pass_counts_for_test();
    let render = crate::render::incremental::take_cached_render_counts_for_test();
    assert_eq!(passes.import_model_parses, 1);
    assert_eq!(passes.validated_evidence_constructions, 1);
    assert_eq!(passes.validation_certificate_constructions, 1);
    assert_eq!(passes.document_validations, 1);
    assert_eq!(passes.canonical_mark_validation_attempts, 1);
    assert_eq!(passes.canonical_mark_validation_completions, 1);
    assert_eq!(passes.canonical_projections, 1);
    assert_eq!(passes.canonical_serializations, 0);
    assert_eq!(passes.canonical_hashes, 0);
    assert_eq!(
        passes.render_limit_tree_scans, 0,
        "sealed validation evidence should replace the redundant render node/depth scan"
    );
    assert_eq!(
        render.0, 1,
        "the admitted import should build one render cache"
    );

    let artifact = &engine.derived_state.as_ref().unwrap().canonical_artifact;
    let _ = artifact.sha256();
    assert_eq!(take_full_pass_counts_for_test().canonical_hashes, 1);
    let _ = artifact.sha256();
    assert_eq!(take_full_pass_counts_for_test().canonical_hashes, 0);
}

#[test]
fn admitted_import_hydrates_before_seed_consumers_but_not_selection_only_state() {
    let mut typed_input = import_document_with_unavailable_lookup_seed();
    typed_input
        .apply_typed_transaction(insert_transaction(&typed_input, 65_100))
        .unwrap();
    assert!(typed_input
        .derived_state
        .as_ref()
        .unwrap()
        .mutation_lookup_seed
        .is_ready_for_test());

    let mut command = import_document_with_unavailable_lookup_seed();
    command
        .apply_command(65_101, TypedCommand::InsertText { text: "x".into() })
        .unwrap()
        .expect("default-selection command should apply without preparatory selection");
    assert!(command
        .derived_state
        .as_ref()
        .unwrap()
        .mutation_lookup_seed
        .is_ready_for_test());

    let mut selection = import_document_with_unavailable_lookup_seed();
    selection
        .apply_typed_transaction(TypedTransaction {
            request_id: 65_102,
            base_document_revision: selection.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: vec![],
            selection_intent: SelectionIntent::Set(SelectionInput::All),
            history_policy: HistoryPolicy::Skip,
        })
        .unwrap();
    assert!(selection
        .derived_state
        .as_ref()
        .unwrap()
        .mutation_lookup_seed
        .is_unavailable_for_test());

    let mut rich_local_api = import_document_with_unavailable_lookup_seed();
    rich_local_api
        .apply_typed_transaction_with_result(TypedTransaction {
            request_id: 65_103,
            base_document_revision: rich_local_api.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: vec![],
            selection_intent: SelectionIntent::Preserve,
            history_policy: HistoryPolicy::Skip,
        })
        .unwrap();
    assert!(rich_local_api
        .derived_state
        .as_ref()
        .unwrap()
        .mutation_lookup_seed
        .is_unavailable_for_test());

    let mut history = import_document_with_unavailable_lookup_seed();
    assert!(history.undo(65_104).unwrap().is_none());
    assert!(history
        .derived_state
        .as_ref()
        .unwrap()
        .mutation_lookup_seed
        .is_unavailable_for_test());
    history
        .apply_command(65_105, TypedCommand::InsertText { text: "x".into() })
        .unwrap()
        .unwrap();
    force_lookup_seed_unavailable(&mut history);
    let unavailable_before_undo =
        Arc::clone(&history.derived_state.as_ref().unwrap().mutation_lookup_seed);
    assert!(history.undo(65_106).unwrap().is_some());
    assert!(!Arc::ptr_eq(
        &unavailable_before_undo,
        &history.derived_state.as_ref().unwrap().mutation_lookup_seed
    ));
    let unavailable_before_redo =
        Arc::clone(&history.derived_state.as_ref().unwrap().mutation_lookup_seed);
    assert!(history.redo(65_107).unwrap().is_some());
    assert!(!Arc::ptr_eq(
        &unavailable_before_redo,
        &history.derived_state.as_ref().unwrap().mutation_lookup_seed
    ));
}

#[test]
fn deferred_insert_shape_and_output_bound_eligibility_is_exact() {
    let exact = deferred_insert_fixture(DeferredInsertCase::StrictInteriorEqualMarks);
    assert_eq!(
        exact.execution_admission_kind(),
        ExecutionAdmissionKind::Deferred
    );

    for case in [
        DeferredInsertCase::Empty,
        DeferredInsertCase::LeafBoundary,
        DeferredInsertCase::MarkMismatch,
        DeferredInsertCase::StructuralGrowth,
        DeferredInsertCase::UnavailableUpperBound,
        DeferredInsertCase::OverflowingUpperBound,
        DeferredInsertCase::OneOverOutputLimit,
    ] {
        assert_eq!(
            deferred_insert_fixture(case).execution_admission_kind(),
            ExecutionAdmissionKind::Eager,
            "{case:?}",
        );
    }
}

#[test]
fn eager_semantic_errors_precede_staged_hydration_failure() {
    use crate::yrs_engine::mutation::{
        set_lookup_seed_hydration_failpoint_for_test, LookupSeedHydrationFailpoint,
    };

    for case in eager_pre_admission_error_cases() {
        let mut engine = case.engine;
        let before = atomic_audit(&engine);
        set_lookup_seed_hydration_failpoint_for_test(Some(
            LookupSeedHydrationFailpoint::InitialReservation,
        ));
        let error = engine
            .apply_command(case.request_id, case.command)
            .unwrap_err();
        set_lookup_seed_hydration_failpoint_for_test(None);
        assert_eq!(error, case.expected_error, "{}", case.name);
        assert_eq!(atomic_audit(&engine), before, "{}", case.name);
    }
}

#[test]
fn first_imported_deferred_insert_uses_two_serializations_two_hashes_once() {
    use crate::yrs_engine::mutation::{
        reset_localized_lookup_counts_for_test, take_localized_lookup_counts_for_test,
    };
    use crate::yrs_engine::observability::{
        reset_full_pass_counts_for_test, reset_prepared_admission_counts_for_test,
        take_full_pass_counts_for_test, take_prepared_admission_counts_for_test,
    };

    let mut engine = import_document_with_unavailable_lookup_seed();
    select_text(&mut engine, 65_199, 2, 2);
    assert!(engine
        .derived_state
        .as_ref()
        .unwrap()
        .mutation_lookup_seed
        .is_unavailable_for_test());
    reset_full_pass_counts_for_test();
    reset_prepared_admission_counts_for_test();
    reset_localized_lookup_counts_for_test();

    engine
        .apply_command(65_200, TypedCommand::InsertText { text: "x".into() })
        .unwrap()
        .expect("strict-interior imported insert should apply");

    let passes = take_full_pass_counts_for_test();
    assert_eq!(passes.planner_simulations, 1);
    assert_eq!(passes.document_validations, 1);
    assert_eq!(passes.canonical_serializations, 2);
    assert_eq!(passes.canonical_hashes, 2);
    assert_eq!(take_localized_lookup_counts_for_test(), (0, 1, 1));
    let admission = take_prepared_admission_counts_for_test();
    assert_eq!(admission.staged_seed_preparations, 1);
    assert_eq!(admission.installed_base_seed_publications, 0);
}

#[test]
fn public_insert_uses_eager_admission_after_admissible_resource_limit_change() {
    use crate::yrs_engine::observability::{
        reset_full_pass_counts_for_test, reset_prepared_admission_counts_for_test,
        take_full_pass_counts_for_test, take_prepared_admission_counts_for_test,
    };

    let mut engine = import_document_with_unavailable_lookup_seed();
    select_text(&mut engine, 65_201, 2, 2);
    engine.resource_limits.max_input_bytes -= 1;
    let changed_limits = engine.resource_limits.clone();
    let mut preconfigured = transaction_engine();
    preconfigured.resource_limits = changed_limits;
    preconfigured
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    select_text(&mut preconfigured, 65_201, 2, 2);
    let command = TypedCommand::InsertText { text: "x".into() };
    let preparation = std::cell::RefCell::new(None);
    assert!(matches!(
        engine
            .plan_command_internal(65_202, command.clone(), Some(&preparation))
            .unwrap(),
        CommandPlan::Transaction(_)
    ));
    assert!(matches!(
        preparation.into_inner().unwrap().execution_admission,
        crate::yrs_engine::prepared_admission::ExecutionSemanticAdmission::Eager(_)
    ));
    let installed = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
    reset_full_pass_counts_for_test();
    reset_prepared_admission_counts_for_test();

    let result = engine.apply_command(65_202, command).unwrap().unwrap();
    let passes = take_full_pass_counts_for_test();
    let counts = take_prepared_admission_counts_for_test();
    let preconfigured_result = preconfigured
        .apply_command(65_202, TypedCommand::InsertText { text: "x".into() })
        .unwrap()
        .unwrap();

    assert!(result.changed);
    assert_eq!(passes.planner_simulations, 1);
    assert_eq!(passes.document_validations, 4);
    assert_eq!(result, preconfigured_result);
    assert_eq!(engine.document_json(), preconfigured.document_json());
    assert_eq!(engine.document_html(), preconfigured.document_html());
    assert_eq!(
        engine.resolved_selection(),
        preconfigured.resolved_selection()
    );
    assert!(!Arc::ptr_eq(
        &installed,
        &engine.derived_state.as_ref().unwrap().mutation_lookup_seed,
    ));
    assert!(engine
        .derived_state
        .as_ref()
        .unwrap()
        .mutation_lookup_seed
        .is_ready_for_test());
    assert_eq!(counts.staged_seed_preparations, 1);
    assert_eq!(counts.installed_base_seed_publications, 0);
}

#[test]
fn private_prepared_command_orchestrator_finalizes_deferred_admission_once() {
    use crate::yrs_engine::mutation::{
        reset_localized_lookup_counts_for_test, take_localized_lookup_counts_for_test,
    };
    use crate::yrs_engine::observability::{
        reset_full_pass_counts_for_test, reset_prepared_admission_counts_for_test,
        take_full_pass_counts_for_test, take_prepared_admission_counts_for_test,
    };
    use crate::yrs_engine::TransactionCommit;

    let mut engine = import_document_with_unavailable_lookup_seed();
    let mut public = import_document_with_unavailable_lookup_seed();
    select_text(&mut engine, 65_260, 2, 2);
    select_text(&mut public, 65_260, 2, 2);
    let installed = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
    let preparation = std::cell::RefCell::new(None);
    reset_full_pass_counts_for_test();
    reset_prepared_admission_counts_for_test();
    reset_localized_lookup_counts_for_test();

    let CommandPlan::Transaction(transaction) = engine
        .plan_command_internal(
            65_261,
            TypedCommand::InsertText { text: "x".into() },
            Some(&preparation),
        )
        .unwrap()
    else {
        panic!("strict-interior imported insert must produce a transaction")
    };
    let proof = preparation
        .into_inner()
        .expect("strict-interior imported insert must retain its exact proof");
    assert!(matches!(
        &proof.execution_admission,
        crate::yrs_engine::prepared_admission::ExecutionSemanticAdmission::Deferred(_)
    ));
    let (commit, result) = engine
        .apply_prepared_command_transaction(
            transaction,
            proof,
            true,
            &mut OutboundUpdateSink::detached(),
        )
        .unwrap();
    let result = result.expect("changed command must return a result");
    let authority_counts = take_compiled_commit_authority_counts_for_test();
    let passes = take_full_pass_counts_for_test();
    let admission = take_prepared_admission_counts_for_test();
    assert_eq!(passes.planner_simulations, 1);
    assert_eq!(passes.document_validations, 1);
    assert_eq!(passes.canonical_serializations, 2);
    assert_eq!(passes.canonical_hashes, 2);
    assert_eq!(take_localized_lookup_counts_for_test(), (0, 1, 1));
    assert_eq!(admission.staged_seed_preparations, 1);
    assert_eq!(admission.staged_identity_materializations, 1);
    assert_eq!(admission.installed_base_seed_publications, 0);
    assert_eq!(admission.deferred_capsules_created, 1);
    assert_eq!(admission.deferred_capsules_finalized, 1);
    assert_eq!(authority_counts, (1, 1));
    assert!(!Arc::ptr_eq(
        &installed,
        &engine.derived_state.as_ref().unwrap().mutation_lookup_seed
    ));
    assert!(engine
        .derived_state
        .as_ref()
        .unwrap()
        .mutation_lookup_seed
        .is_ready_for_test());
    assert_eq!(
        commit,
        TransactionCommit {
            request_id: result.request_id,
            changed: result.changed,
            document_revision: result.document_revision,
            state_revision: result.state_revision,
            origin: result.origin,
        }
    );

    let public_result = public
        .apply_command(65_261, TypedCommand::InsertText { text: "x".into() })
        .unwrap()
        .unwrap();
    assert_eq!(result, public_result);
    assert_eq!(engine.document_json(), public.document_json());
    assert_eq!(engine.resolved_selection(), public.resolved_selection());
    assert_eq!(engine.stored_marks(), public.stored_marks());
    assert_eq!(engine.can_undo(), public.can_undo());
    assert_eq!(engine.can_redo(), public.can_redo());
    let private_undo = engine.undo(65_262).unwrap().unwrap();
    let public_undo = public.undo(65_262).unwrap().unwrap();
    assert_eq!(private_undo, public_undo);
    assert_eq!(engine.document_json(), public.document_json());
    assert_eq!(engine.resolved_selection(), public.resolved_selection());
    assert_eq!(engine.stored_marks(), public.stored_marks());
    assert_eq!(engine.can_undo(), public.can_undo());
    assert_eq!(engine.can_redo(), public.can_redo());
}

#[test]
fn first_imported_prepared_insert_traverses_each_history_document_once() {
    use crate::model::{
        reset_history_snapshot_retained_bytes_traversals_for_test,
        take_history_snapshot_retained_bytes_traversals_for_test,
    };

    let mut engine = import_document_with_unavailable_lookup_seed();
    select_text(&mut engine, 65_263, 2, 2);
    reset_history_snapshot_retained_bytes_traversals_for_test();

    engine
        .apply_command(65_264, TypedCommand::InsertText { text: "x".into() })
        .unwrap()
        .expect("strict-interior imported insert must apply");

    assert_eq!(
        take_history_snapshot_retained_bytes_traversals_for_test(),
        2,
        "history admission must traverse the before and after source documents once each"
    );
}

#[test]
fn first_imported_prepared_insert_uses_localized_history_render_evidence() {
    use crate::render::incremental::{
        reset_cached_render_counts_for_test, reset_localized_render_transition_counts_for_test,
        take_cached_render_counts_for_test, take_localized_render_transition_counts_for_test,
    };
    use crate::yrs_engine::observability::{
        reset_full_pass_counts_for_test, take_full_pass_counts_for_test,
    };

    let mut engine = import_document_with_unavailable_lookup_seed();
    select_text(&mut engine, 65_265, 2, 2);
    reset_full_pass_counts_for_test();
    reset_cached_render_counts_for_test();
    reset_localized_render_transition_counts_for_test();

    engine
        .apply_command(65_266, TypedCommand::InsertText { text: "x".into() })
        .unwrap()
        .expect("strict-interior imported insert must apply");

    let passes = take_full_pass_counts_for_test();
    let localized = take_localized_render_transition_counts_for_test();
    assert_eq!((passes.render_limit_tree_scans, localized), (0, (1, 1, 0)));
    assert_eq!(
        (
            passes.position_map_clones,
            passes.position_map_compactions,
            passes.rendered_text_derivations,
        ),
        (1, 1, 0),
        "sealed strict-interior evidence must incrementally derive the candidate map and text",
    );
    assert_eq!(take_cached_render_counts_for_test(), (0, 1, 1, 0, 0));
}

#[test]
fn tampered_localized_history_render_evidence_falls_back_with_exact_results() {
    use crate::render::incremental::{
        reset_cached_render_counts_for_test, reset_localized_render_transition_counts_for_test,
        take_cached_render_counts_for_test, take_localized_render_transition_counts_for_test,
    };
    use crate::yrs_engine::observability::{
        reset_full_pass_counts_for_test, take_full_pass_counts_for_test,
    };
    use crate::yrs_engine::prepared_admission::{
        DeferredCommandAdmission, ExecutionSemanticAdmission,
    };

    for case in DeferredCommandAdmission::history_render_tamper_cases_for_test() {
        let mut actual = import_document_with_unavailable_lookup_seed();
        let mut expected = import_document_with_unavailable_lookup_seed();
        select_text(&mut actual, 65_267, 2, 2);
        select_text(&mut expected, 65_267, 2, 2);
        let command = TypedCommand::InsertText { text: "x".into() };
        let preparation = std::cell::RefCell::new(None);
        let CommandPlan::Transaction(transaction) = actual
            .plan_command_internal(65_268, command.clone(), Some(&preparation))
            .unwrap()
        else {
            panic!("strict-interior imported insert must produce a transaction")
        };
        let mut proof = preparation.into_inner().unwrap();
        let ExecutionSemanticAdmission::Deferred(deferred) = &mut proof.execution_admission else {
            panic!("strict-interior imported insert must retain deferred evidence")
        };
        deferred.tamper_history_render_for_test(case);
        reset_full_pass_counts_for_test();
        reset_cached_render_counts_for_test();
        reset_localized_render_transition_counts_for_test();

        let actual_result = actual
            .apply_prepared_command_transaction(
                transaction,
                proof,
                true,
                &mut OutboundUpdateSink::detached(),
            )
            .unwrap()
            .1
            .unwrap();
        let passes = take_full_pass_counts_for_test();
        let cached = take_cached_render_counts_for_test();
        let localized = take_localized_render_transition_counts_for_test();
        let expected_result = expected.apply_command(65_268, command).unwrap().unwrap();

        assert_eq!(actual_result, expected_result, "{case}");
        assert_eq!(actual.document_json(), expected.document_json(), "{case}");
        assert_eq!(
            actual.resolved_selection(),
            expected.resolved_selection(),
            "{case}"
        );
        assert_eq!(actual.can_undo(), expected.can_undo(), "{case}");
        assert_eq!(passes.render_limit_tree_scans, 1, "{case}");
        assert_eq!(cached, (0, 1, 1, 0, 0), "{case}");
        assert_eq!(localized, (1, 0, 1), "{case}");
    }
}

#[test]
fn localized_history_render_errors_fall_back_with_exact_results() {
    use crate::render::incremental::{
        reset_cached_render_counts_for_test, reset_localized_render_transition_counts_for_test,
        set_localized_render_failure_stage_for_test, take_cached_render_counts_for_test,
        take_localized_render_transition_counts_for_test, LocalizedRenderFailureStage,
    };
    use crate::yrs_engine::observability::{
        reset_full_pass_counts_for_test, take_full_pass_counts_for_test,
    };

    for stage in [
        LocalizedRenderFailureStage::Allocation,
        LocalizedRenderFailureStage::Resource,
        LocalizedRenderFailureStage::Position,
        LocalizedRenderFailureStage::Invariant,
    ] {
        let mut actual = import_document_with_unavailable_lookup_seed();
        let mut expected = import_document_with_unavailable_lookup_seed();
        let two_blocks = r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]},{"type":"paragraph","content":[{"type":"text","text":"suffix"}]}]}"#;
        actual
            .import_json(two_blocks, TransactionOrigin::DocumentImport)
            .unwrap();
        expected
            .import_json(two_blocks, TransactionOrigin::DocumentImport)
            .unwrap();
        select_text(&mut actual, 65_269, 2, 2);
        select_text(&mut expected, 65_269, 2, 2);
        reset_full_pass_counts_for_test();
        reset_cached_render_counts_for_test();
        reset_localized_render_transition_counts_for_test();
        set_localized_render_failure_stage_for_test(Some(stage));

        let actual_result = actual
            .apply_command(65_270, TypedCommand::InsertText { text: "x".into() })
            .unwrap()
            .unwrap();
        set_localized_render_failure_stage_for_test(None);
        let passes = take_full_pass_counts_for_test();
        let cached = take_cached_render_counts_for_test();
        let localized = take_localized_render_transition_counts_for_test();
        let expected_result = expected
            .apply_command(65_270, TypedCommand::InsertText { text: "x".into() })
            .unwrap()
            .unwrap();

        assert_eq!(actual_result, expected_result, "{stage:?}");
        assert_eq!(
            actual.document_json(),
            expected.document_json(),
            "{stage:?}"
        );
        assert_eq!(
            actual.resolved_selection(),
            expected.resolved_selection(),
            "{stage:?}"
        );
        assert_eq!(actual.can_undo(), expected.can_undo(), "{stage:?}");
        assert_eq!(passes.render_limit_tree_scans, 1, "{stage:?}");
        assert_eq!(cached, (0, 1, 1, 0, 0), "{stage:?}");
        assert_eq!(localized, (1, 0, 1), "{stage:?}");
    }
}

#[test]
fn private_prepared_eager_noninsert_uses_staged_context_without_identity() {
    use crate::yrs_engine::observability::{
        reset_prepared_admission_counts_for_test, take_prepared_admission_counts_for_test,
    };
    use crate::yrs_engine::TransactionCommit;

    let mut engine = import_document_with_unavailable_lookup_seed();
    let mut public = import_document_with_unavailable_lookup_seed();
    select_text(&mut engine, 65_263, 0, 2);
    select_text(&mut public, 65_263, 0, 2);
    let preparation = std::cell::RefCell::new(None);
    reset_prepared_admission_counts_for_test();
    let command = TypedCommand::ToggleMark {
        mark_type: "bold".into(),
    };
    let CommandPlan::Transaction(transaction) = engine
        .plan_command_internal(65_264, command.clone(), Some(&preparation))
        .unwrap()
    else {
        panic!("range mark command must produce a transaction")
    };
    let proof = preparation.into_inner().unwrap();
    assert!(matches!(
        &proof.execution_admission,
        crate::yrs_engine::prepared_admission::ExecutionSemanticAdmission::Eager(_)
    ));

    let (commit, result) = engine
        .apply_prepared_command_transaction(
            transaction,
            proof,
            true,
            &mut OutboundUpdateSink::detached(),
        )
        .unwrap();
    let result = result.unwrap();
    let admission = take_prepared_admission_counts_for_test();
    assert_eq!(admission.staged_seed_preparations, 1);
    assert_eq!(admission.staged_identity_materializations, 0);
    assert_eq!(admission.installed_base_seed_publications, 0);
    assert_eq!(
        commit,
        TransactionCommit {
            request_id: result.request_id,
            changed: result.changed,
            document_revision: result.document_revision,
            state_revision: result.state_revision,
            origin: result.origin,
        }
    );

    let public_result = public.apply_command(65_264, command).unwrap().unwrap();
    assert_eq!(result, public_result);
    assert_eq!(engine.document_json(), public.document_json());
    assert_eq!(engine.resolved_selection(), public.resolved_selection());
    assert_eq!(engine.stored_marks(), public.stored_marks());
    assert_eq!(engine.can_undo(), public.can_undo());
    assert_eq!(engine.can_redo(), public.can_redo());
}

#[test]
fn private_prepared_history_error_precedes_staged_hydration_failure() {
    use crate::yrs_engine::mutation::{
        set_lookup_seed_hydration_failpoint_for_test, LookupSeedHydrationFailpoint,
    };
    use crate::yrs_engine::observability::{
        reset_prepared_admission_counts_for_test, take_prepared_admission_counts_for_test,
    };

    let limits = crate::yrs_engine::EditingLimits {
        max_derived_output_bytes: 100,
        ..crate::yrs_engine::EditingLimits::default()
    };
    let mut engine = transaction_engine_with_editing_limits(limits);
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    select_text(&mut engine, 65_265, 2, 2);
    engine.derived_state.as_mut().unwrap().canonical_artifact = engine
        .derived_state
        .as_ref()
        .unwrap()
        .canonical_artifact
        .with_admission_upper_bound_for_test(usize::MAX);
    let expected_actual = super::history_metadata_bytes(engine.stored_marks(), "prosemirror") * 2;
    let preparation = std::cell::RefCell::new(None);
    let CommandPlan::Transaction(transaction) = engine
        .plan_command_internal(
            65_266,
            TypedCommand::InsertText { text: "x".into() },
            Some(&preparation),
        )
        .unwrap()
    else {
        panic!("insert command must produce a transaction")
    };
    let proof = preparation.into_inner().unwrap();
    let installed = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
    let before = atomic_audit(&engine);
    reset_prepared_admission_counts_for_test();
    set_lookup_seed_hydration_failpoint_for_test(Some(
        LookupSeedHydrationFailpoint::InitialReservation,
    ));
    let error = engine
        .apply_prepared_command_transaction(
            transaction,
            proof,
            true,
            &mut OutboundUpdateSink::detached(),
        )
        .unwrap_err();
    set_lookup_seed_hydration_failpoint_for_test(None);

    assert_eq!(
        error,
        crate::yrs_engine::OperationError::document_limit_exceeded(
            65_266,
            None,
            "maxDerivedOutputBytes",
            100,
            expected_actual as u64,
        )
    );
    assert_eq!(atomic_audit(&engine), before);
    assert!(Arc::ptr_eq(
        &installed,
        &engine.derived_state.as_ref().unwrap().mutation_lookup_seed
    ));
    let admission = take_prepared_admission_counts_for_test();
    assert_eq!(admission.staged_seed_preparations, 0);
    assert_eq!(admission.installed_base_seed_publications, 0);
}

#[test]
fn private_prepared_deferred_compiler_failure_is_prewrite_and_atomic() {
    use crate::yrs_engine::compiler::{set_atomic_failpoint_for_test, AtomicFailpoint};
    use crate::yrs_engine::observability::{
        reset_prepared_admission_counts_for_test, take_prepared_admission_counts_for_test,
    };

    let mut engine = import_document_with_unavailable_lookup_seed();
    select_text(&mut engine, 65_267, 2, 2);
    let preparation = std::cell::RefCell::new(None);
    let CommandPlan::Transaction(transaction) = engine
        .plan_command_internal(
            65_268,
            TypedCommand::InsertText { text: "x".into() },
            Some(&preparation),
        )
        .unwrap()
    else {
        panic!("strict-interior imported insert must produce a transaction")
    };
    let proof = preparation.into_inner().unwrap();
    assert!(matches!(
        &proof.execution_admission,
        crate::yrs_engine::prepared_admission::ExecutionSemanticAdmission::Deferred(_)
    ));
    let installed = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
    let before = atomic_audit(&engine);
    reset_prepared_admission_counts_for_test();
    set_atomic_failpoint_for_test(Some(AtomicFailpoint::SemanticCompilation));
    let error = engine
        .apply_prepared_command_transaction(
            transaction,
            proof,
            true,
            &mut OutboundUpdateSink::detached(),
        )
        .unwrap_err();
    set_atomic_failpoint_for_test(None);

    assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
    assert_eq!(atomic_audit(&engine), before);
    assert!(Arc::ptr_eq(
        &installed,
        &engine.derived_state.as_ref().unwrap().mutation_lookup_seed
    ));
    let admission = take_prepared_admission_counts_for_test();
    assert_eq!(admission.staged_seed_preparations, 1);
    assert_eq!(admission.staged_identity_materializations, 1);
    assert_eq!(admission.installed_base_seed_publications, 0);
    assert_eq!(admission.deferred_capsules_finalized, 1);
}

#[test]
fn eager_non_insert_first_mutations_do_not_materialize_base_identity() {
    use crate::yrs_engine::observability::{
        reset_full_pass_counts_for_test, reset_prepared_admission_counts_for_test,
        take_full_pass_counts_for_test, take_prepared_admission_counts_for_test,
    };

    let mut toggle = import_document_with_unavailable_lookup_seed();
    select_text(&mut toggle, 65_201, 0, 2);
    reset_full_pass_counts_for_test();
    reset_prepared_admission_counts_for_test();
    toggle
        .apply_command(
            65_202,
            TypedCommand::ToggleMark {
                mark_type: "bold".into(),
            },
        )
        .unwrap()
        .unwrap();
    let toggle_passes = take_full_pass_counts_for_test();
    let toggle_admission = take_prepared_admission_counts_for_test();
    assert_eq!(toggle_passes.canonical_serializations, 3);
    assert_eq!(toggle_passes.canonical_hashes, 2);
    assert_eq!(toggle_admission.staged_identity_materializations, 0);

    let mut wrap = import_document_with_unavailable_lookup_seed();
    reset_full_pass_counts_for_test();
    reset_prepared_admission_counts_for_test();
    wrap.apply_command(
        65_203,
        TypedCommand::WrapInList {
            list_type: "bulletList".into(),
            item_type: "listItem".into(),
        },
    )
    .unwrap()
    .unwrap();
    let wrap_passes = take_full_pass_counts_for_test();
    let wrap_admission = take_prepared_admission_counts_for_test();
    assert_eq!(wrap_passes.canonical_serializations, 3);
    assert_eq!(wrap_passes.canonical_hashes, 2);
    assert_eq!(wrap_admission.staged_identity_materializations, 0);

    let mut undo = import_document_with_unavailable_lookup_seed();
    undo.apply_command(65_204, TypedCommand::InsertText { text: "x".into() })
        .unwrap()
        .unwrap();
    force_lookup_seed_unavailable(&mut undo);
    reset_full_pass_counts_for_test();
    reset_prepared_admission_counts_for_test();
    undo.undo(65_205).unwrap().unwrap();
    let undo_passes = take_full_pass_counts_for_test();
    let undo_admission = take_prepared_admission_counts_for_test();
    assert_eq!(undo_passes.canonical_serializations, 0);
    assert_eq!(undo_passes.canonical_hashes, 0);
    assert_eq!(undo_admission.staged_identity_materializations, 0);

    let mut redo = import_document_with_unavailable_lookup_seed();
    redo.apply_command(65_206, TypedCommand::InsertText { text: "x".into() })
        .unwrap()
        .unwrap();
    redo.undo(65_207).unwrap().unwrap();
    force_lookup_seed_unavailable(&mut redo);
    reset_full_pass_counts_for_test();
    reset_prepared_admission_counts_for_test();
    redo.redo(65_208).unwrap().unwrap();
    let redo_passes = take_full_pass_counts_for_test();
    let redo_admission = take_prepared_admission_counts_for_test();
    assert_eq!(redo_passes.canonical_serializations, 0);
    assert_eq!(redo_passes.canonical_hashes, 0);
    assert_eq!(redo_admission.staged_identity_materializations, 0);
}

#[test]
fn staged_authority_supplies_every_unavailable_seed_consumer_without_installed_reads() {
    use crate::yrs_engine::observability::{
        reset_full_pass_counts_for_test, reset_prepared_admission_counts_for_test,
        take_full_pass_counts_for_test, take_prepared_admission_counts_for_test,
    };

    reset_prepared_admission_counts_for_test();
    let (engine, deferred, mut context, transaction, expected_document) =
        deferred_finalization_fixture();
    let installed = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
    engine.prepare_mutation_identity(&mut context).unwrap();
    reset_full_pass_counts_for_test();

    let prepared = engine
        .finalize_deferred_for_test(deferred, &context, &transaction, &expected_document)
        .unwrap();
    let state = engine.derived_state.as_ref().unwrap();
    let cached = state.compilation_view();
    let txn = engine.doc.transact();
    let fragment = txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();
    let authority = context
        .authority(
            crate::yrs_engine::prepared_admission::LiveMutationAuthorityContext {
                request_id: transaction.request_id,
                installed: state,
                txn: &txn,
                fragment: &fragment,
                fragment_name: &engine.fragment_name,
                schema_fingerprint: &engine.schema_fingerprint,
                resource_limits: &engine.resource_limits,
                editing_limits: &engine.editing_limits,
                max_length: engine.max_length,
                document_revision: engine.revision,
                state_revision: engine.state_revision,
                yrs_state_epoch: engine.yrs_state_epoch,
            },
        )
        .unwrap();
    assert!(installed.is_unavailable_for_test());
    assert!(authority.lookup_seed().is_ready_for_test());
    assert!(!Arc::ptr_eq(&installed, authority.lookup_seed()));

    let format_from = crate::yrs_engine::position::editor_offset_to_doc_pos(
        0,
        EditorOffsetKind::Scalar,
        &state.rendered_text,
        &state.position_map,
        &state.document,
    )
    .unwrap();
    let format_to = crate::yrs_engine::position::editor_offset_to_doc_pos(
        2,
        EditorOffsetKind::Scalar,
        &state.rendered_text,
        &state.position_map,
        &state.document,
    )
    .unwrap();
    let format_block = state
        .position_map
        .find_block_for_doc_pos(format_from)
        .and_then(|index| state.position_map.block(index))
        .unwrap();
    let format_locator = crate::yrs_engine::mutation::LocalizedFormatLocator::mint(
        &state.document,
        &format_block.node_path,
        format_from,
        format_to,
        authority.lookup_seed().as_ref(),
        &txn,
        &fragment,
        &engine.resource_limits,
        &engine.editing_limits,
        engine.max_length,
        &engine.schema_fingerprint,
        engine.yrs_state_epoch,
        engine.revision,
    )
    .expect("staged authority mints a current localized format locator");
    assert!(
        crate::yrs_engine::mutation::LocalizedFormatCompiler::try_new(
            transaction.request_id,
            &txn,
            &fragment,
            &engine.schema,
            usize::MAX,
            engine.resource_limits.max_input_bytes,
            0,
            format_locator,
            &engine.schema_fingerprint,
            engine.yrs_state_epoch,
            engine.revision,
        )
        .unwrap()
        .is_some()
    );

    let first_child = state
        .document
        .root()
        .content()
        .and_then(|content| content.child(0))
        .unwrap()
        .clone();
    let root_replacement = crate::yrs_engine::StructuralReplacement::new(
        Vec::new(),
        0,
        1,
        crate::model::Fragment::from(vec![first_child]),
        Selection::cursor(0),
    );
    let root_locator = crate::yrs_engine::mutation::LocalizedRootWindowLocator::mint(
        transaction.request_id,
        &state.document,
        &state.document,
        &root_replacement,
        authority.lookup_seed().as_ref(),
        &txn,
        &fragment,
        &engine.resource_limits,
        &engine.editing_limits,
        engine.max_length,
        &engine.schema_fingerprint,
        engine.yrs_state_epoch,
        engine.revision,
    )
    .unwrap()
    .expect("staged authority mints a current localized root-window locator");
    assert!(
        crate::yrs_engine::mutation::LocalizedRootWindowCompiler::try_new(
            transaction.request_id,
            &txn,
            &fragment,
            &engine.schema,
            usize::MAX,
            engine.resource_limits.max_input_bytes,
            0,
            root_locator,
        )
        .unwrap()
        .is_some()
    );

    let mut compiled =
        crate::yrs_engine::compiler::compile_prepared_transaction_with_yrs_and_stored_marks(
            crate::yrs_engine::compiler::CompilationContext {
                document: cached.document,
                selection: Some(cached.selection),
                schema: &engine.schema,
                resource_limits: &engine.resource_limits,
                editing_limits: &engine.editing_limits,
                document_revision: engine.revision,
                max_length: engine.max_length,
            },
            transaction.clone(),
            &txn,
            &fragment,
            crate::yrs_engine::compiler::StoredMarksCompilationContext {
                stored_marks: state.stored_marks.as_deref(),
                resolved_selection: &state.resolved_selection,
                relative_selection: &state.relative_selection,
            },
            crate::yrs_engine::compiler::PreparedSemanticContext {
                admission: &prepared,
                expected_preview: &expected_document,
                yrs_state_epoch: engine.yrs_state_epoch,
                state_revision: engine.state_revision,
                schema_fingerprint: &engine.schema_fingerprint,
            },
            crate::yrs_engine::compiler::EngineCompilationView {
                cached,
                authority: &authority,
                state_revision: engine.state_revision,
                schema_fingerprint: &engine.schema_fingerprint,
                yrs_state_epoch: engine.yrs_state_epoch,
            },
        )
        .unwrap();
    assert!(compiled.localized_semantic_used);
    assert!(compiled.localized_insert_admission.is_some());
    assert!(compiled.prepared_derived_evidence.is_some());
    assert!(compiled.mutation_lookup_transition.is_some());

    let admission = compiled.localized_insert_admission.as_ref().unwrap();
    let crate::yrs_engine::compiler::StoredMarksPlan::Set(stored_marks) =
        &compiled.stored_marks_plan
    else {
        panic!("localized compiler seals stored marks")
    };
    let active_transition = state
        .prepare_active_state_transition(
            transaction.request_id,
            &authority,
            admission,
            &compiled.preview,
            admission.operation_result_selection(),
            stored_marks.as_deref(),
            &engine.resource_limits,
            &engine.editing_limits,
            engine.max_length,
            engine.yrs_state_epoch,
        )
        .unwrap();
    let structural = admission.active_state_structural_seal();
    assert!(state
        .validate_active_state_transition(
            &authority,
            &active_transition,
            &structural,
            &compiled.preview,
            admission.operation_result_selection(),
            stored_marks.as_deref(),
            &engine.resource_limits,
            &engine.editing_limits,
            engine.max_length,
            engine.yrs_state_epoch,
        )
        .is_some());

    let selection_seal =
        crate::yrs_engine::compiler::PreparedSelectionMutationSeal::capture(&compiled)
            .expect("localized insert captures its prepared selection seal");
    assert!(selection_seal.matches(&compiled, &authority));

    let evidence = compiled.prepared_derived_evidence.take().unwrap();
    let derivations = compiled.preview_derivations.as_ref().unwrap();
    let render_transition = evidence
        .prepare_localized_render_transition(
            state,
            &compiled.preview,
            derivations,
            &compiled.affected_top_level_blocks,
            &engine.schema,
            &engine.schema_fingerprint,
            &engine.resource_limits,
            &engine.editing_limits,
            engine.max_length,
        )
        .expect("localized render proof remains current")
        .unwrap();
    let next_document_revision = engine.revision.checked_add(1).unwrap();
    let next_state_revision = engine.state_revision.checked_add(1).unwrap();
    let next_yrs_state_epoch = engine.yrs_state_epoch.checked_add(1).unwrap();
    assert!(evidence
        .finalize(
            &authority,
            &compiled.preview,
            compiled.canonical_artifact.as_ref().unwrap(),
            derivations,
            &render_transition.cache,
            &engine.resource_limits,
            &engine.editing_limits,
            engine.max_length,
            &engine.schema_fingerprint,
            next_document_revision,
            next_state_revision,
            next_yrs_state_epoch,
        )
        .is_some());

    let next_seed = engine
        .prepare_mutation_lookup_transition_with_authority(
            transaction.request_id,
            &authority,
            compiled.mutation_lookup_transition.as_ref().unwrap(),
            &txn,
            &fragment,
            &compiled.preview,
            compiled.canonical_artifact.as_ref().unwrap(),
            next_yrs_state_epoch,
            next_document_revision,
        )
        .unwrap();
    assert!(next_seed.is_ready_for_test());
    assert!(!Arc::ptr_eq(&installed, &next_seed));
    let installed_adapter =
        crate::yrs_engine::prepared_admission::InstalledDerivedStateAuthority::new(state);
    assert!(
        crate::yrs_engine::prepared_admission::DerivedStateAuthority::lookup_seed(
            &installed_adapter,
            transaction.request_id,
        )
        .is_err()
    );
    assert!(Arc::ptr_eq(
        &installed,
        &engine.derived_state.as_ref().unwrap().mutation_lookup_seed,
    ));
    let passes = take_full_pass_counts_for_test();
    assert_eq!(passes.document_validations, 0);
    let counts = take_prepared_admission_counts_for_test();
    assert_eq!(counts.staged_seed_preparations, 1);
    assert_eq!(counts.installed_base_seed_publications, 0);
}

#[test]
fn staged_authority_rejects_installed_substitution_and_live_seal_drift_before_transition() {
    use crate::yrs_engine::observability::{
        reset_prepared_admission_counts_for_test, take_prepared_admission_counts_for_test,
    };

    for case in [
        "request",
        "store",
        "fragment",
        "schema",
        "resource_limits",
        "editing_limits",
        "max_length",
        "document_revision",
        "state_revision",
        "epoch",
        "identity",
    ] {
        let mut engine = import_document_with_unavailable_lookup_seed();
        let installed = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
        reset_prepared_admission_counts_for_test();
        let mut context = engine.prepare_mutation_lookup_seed(65_250).unwrap();
        engine.prepare_mutation_identity(&mut context).unwrap();

        if case == "identity" {
            let state = engine.derived_state.as_mut().unwrap();
            state.canonical_artifact = state
                .canonical_artifact
                .schema_context()
                .derive(&state.document)
                .unwrap();
        }
        let before = atomic_audit(&engine);
        let state = engine.derived_state.as_ref().unwrap();
        let txn = engine.doc.transact();
        let fragment = txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();
        let foreign = transaction_engine();
        let foreign_txn = foreign.doc.transact();
        let foreign_fragment = foreign_txn
            .get_xml_fragment(foreign.fragment_name.as_str())
            .unwrap();
        let mut drifted_resources = engine.resource_limits.clone();
        drifted_resources.max_input_bytes = drifted_resources
            .max_input_bytes
            .checked_sub(1)
            .expect("fixture resource limit is positive");
        let mut drifted_editing = engine.editing_limits.clone();
        drifted_editing.max_operations_per_transaction = drifted_editing
            .max_operations_per_transaction
            .checked_sub(1)
            .expect("fixture editing limit is positive");
        let drifted_max_length = match engine.max_length {
            Some(_) => None,
            None => Some(1),
        };
        let drifted_document_revision = engine
            .revision
            .checked_add(1)
            .expect("fixture document revision can advance");
        let drifted_state_revision = engine
            .state_revision
            .checked_add(1)
            .expect("fixture state revision can advance");
        let drifted_schema = format!("{}!", engine.schema_fingerprint);

        let error = match case {
            "request" => context
                .authority(
                    crate::yrs_engine::prepared_admission::LiveMutationAuthorityContext {
                        request_id: 65_251,
                        installed: state,
                        txn: &txn,
                        fragment: &fragment,
                        fragment_name: &engine.fragment_name,
                        schema_fingerprint: &engine.schema_fingerprint,
                        resource_limits: &engine.resource_limits,
                        editing_limits: &engine.editing_limits,
                        max_length: engine.max_length,
                        document_revision: engine.revision,
                        state_revision: engine.state_revision,
                        yrs_state_epoch: engine.yrs_state_epoch,
                    },
                )
                .err(),
            "store" => context
                .authority(
                    crate::yrs_engine::prepared_admission::LiveMutationAuthorityContext {
                        request_id: 65_250,
                        installed: state,
                        txn: &foreign_txn,
                        fragment: &foreign_fragment,
                        fragment_name: &engine.fragment_name,
                        schema_fingerprint: &engine.schema_fingerprint,
                        resource_limits: &engine.resource_limits,
                        editing_limits: &engine.editing_limits,
                        max_length: engine.max_length,
                        document_revision: engine.revision,
                        state_revision: engine.state_revision,
                        yrs_state_epoch: engine.yrs_state_epoch,
                    },
                )
                .err(),
            "fragment" => context
                .authority(
                    crate::yrs_engine::prepared_admission::LiveMutationAuthorityContext {
                        request_id: 65_250,
                        installed: state,
                        txn: &txn,
                        fragment: &fragment,
                        fragment_name: "foreign-fragment",
                        schema_fingerprint: &engine.schema_fingerprint,
                        resource_limits: &engine.resource_limits,
                        editing_limits: &engine.editing_limits,
                        max_length: engine.max_length,
                        document_revision: engine.revision,
                        state_revision: engine.state_revision,
                        yrs_state_epoch: engine.yrs_state_epoch,
                    },
                )
                .err(),
            "schema" => context
                .authority(
                    crate::yrs_engine::prepared_admission::LiveMutationAuthorityContext {
                        request_id: 65_250,
                        installed: state,
                        txn: &txn,
                        fragment: &fragment,
                        fragment_name: &engine.fragment_name,
                        schema_fingerprint: &drifted_schema,
                        resource_limits: &engine.resource_limits,
                        editing_limits: &engine.editing_limits,
                        max_length: engine.max_length,
                        document_revision: engine.revision,
                        state_revision: engine.state_revision,
                        yrs_state_epoch: engine.yrs_state_epoch,
                    },
                )
                .err(),
            "resource_limits" => context
                .authority(
                    crate::yrs_engine::prepared_admission::LiveMutationAuthorityContext {
                        request_id: 65_250,
                        installed: state,
                        txn: &txn,
                        fragment: &fragment,
                        fragment_name: &engine.fragment_name,
                        schema_fingerprint: &engine.schema_fingerprint,
                        resource_limits: &drifted_resources,
                        editing_limits: &engine.editing_limits,
                        max_length: engine.max_length,
                        document_revision: engine.revision,
                        state_revision: engine.state_revision,
                        yrs_state_epoch: engine.yrs_state_epoch,
                    },
                )
                .err(),
            "editing_limits" => context
                .authority(
                    crate::yrs_engine::prepared_admission::LiveMutationAuthorityContext {
                        request_id: 65_250,
                        installed: state,
                        txn: &txn,
                        fragment: &fragment,
                        fragment_name: &engine.fragment_name,
                        schema_fingerprint: &engine.schema_fingerprint,
                        resource_limits: &engine.resource_limits,
                        editing_limits: &drifted_editing,
                        max_length: engine.max_length,
                        document_revision: engine.revision,
                        state_revision: engine.state_revision,
                        yrs_state_epoch: engine.yrs_state_epoch,
                    },
                )
                .err(),
            "max_length" => context
                .authority(
                    crate::yrs_engine::prepared_admission::LiveMutationAuthorityContext {
                        request_id: 65_250,
                        installed: state,
                        txn: &txn,
                        fragment: &fragment,
                        fragment_name: &engine.fragment_name,
                        schema_fingerprint: &engine.schema_fingerprint,
                        resource_limits: &engine.resource_limits,
                        editing_limits: &engine.editing_limits,
                        max_length: drifted_max_length,
                        document_revision: engine.revision,
                        state_revision: engine.state_revision,
                        yrs_state_epoch: engine.yrs_state_epoch,
                    },
                )
                .err(),
            "document_revision" => context
                .authority(
                    crate::yrs_engine::prepared_admission::LiveMutationAuthorityContext {
                        request_id: 65_250,
                        installed: state,
                        txn: &txn,
                        fragment: &fragment,
                        fragment_name: &engine.fragment_name,
                        schema_fingerprint: &engine.schema_fingerprint,
                        resource_limits: &engine.resource_limits,
                        editing_limits: &engine.editing_limits,
                        max_length: engine.max_length,
                        document_revision: drifted_document_revision,
                        state_revision: engine.state_revision,
                        yrs_state_epoch: engine.yrs_state_epoch,
                    },
                )
                .err(),
            "state_revision" => context
                .authority(
                    crate::yrs_engine::prepared_admission::LiveMutationAuthorityContext {
                        request_id: 65_250,
                        installed: state,
                        txn: &txn,
                        fragment: &fragment,
                        fragment_name: &engine.fragment_name,
                        schema_fingerprint: &engine.schema_fingerprint,
                        resource_limits: &engine.resource_limits,
                        editing_limits: &engine.editing_limits,
                        max_length: engine.max_length,
                        document_revision: engine.revision,
                        state_revision: drifted_state_revision,
                        yrs_state_epoch: engine.yrs_state_epoch,
                    },
                )
                .err(),
            "epoch" => context
                .authority(
                    crate::yrs_engine::prepared_admission::LiveMutationAuthorityContext {
                        request_id: 65_250,
                        installed: state,
                        txn: &txn,
                        fragment: &fragment,
                        fragment_name: &engine.fragment_name,
                        schema_fingerprint: &engine.schema_fingerprint,
                        resource_limits: &engine.resource_limits,
                        editing_limits: &engine.editing_limits,
                        max_length: engine.max_length,
                        document_revision: engine.revision,
                        state_revision: engine.state_revision,
                        yrs_state_epoch: engine.yrs_state_epoch.saturating_add(1),
                    },
                )
                .err(),
            "identity" => context
                .authority(
                    crate::yrs_engine::prepared_admission::LiveMutationAuthorityContext {
                        request_id: 65_250,
                        installed: state,
                        txn: &txn,
                        fragment: &fragment,
                        fragment_name: &engine.fragment_name,
                        schema_fingerprint: &engine.schema_fingerprint,
                        resource_limits: &engine.resource_limits,
                        editing_limits: &engine.editing_limits,
                        max_length: engine.max_length,
                        document_revision: engine.revision,
                        state_revision: engine.state_revision,
                        yrs_state_epoch: engine.yrs_state_epoch,
                    },
                )
                .err(),
            _ => unreachable!(),
        }
        .expect("drifted live context must not mint an authority");
        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED", "{case}");
        drop(foreign_txn);
        drop(txn);
        assert_eq!(atomic_audit(&engine), before, "{case}");
        assert!(Arc::ptr_eq(
            &installed,
            &engine.derived_state.as_ref().unwrap().mutation_lookup_seed,
        ));
        let counts = take_prepared_admission_counts_for_test();
        assert_eq!(counts.staged_seed_preparations, 1, "{case}");
        assert_eq!(counts.installed_base_seed_publications, 0, "{case}");
    }
}

#[test]
fn generic_typed_compilation_uses_staged_authority_without_publishing_base_seed() {
    use crate::yrs_engine::observability::{
        reset_prepared_admission_counts_for_test, take_prepared_admission_counts_for_test,
    };

    let mut engine = import_document_with_unavailable_lookup_seed();
    let mut public = import_document_with_unavailable_lookup_seed();
    let mut public_rich = import_document_with_unavailable_lookup_seed();
    let installed = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
    reset_prepared_admission_counts_for_test();
    let transaction = insert_transaction(&engine, 65_225);
    let (commit, result) = engine
        .apply_typed_transaction_with_staged_context(
            transaction,
            false,
            &mut OutboundUpdateSink::detached(),
        )
        .unwrap();
    assert!(result.is_none());
    let counts = take_prepared_admission_counts_for_test();
    let authority_counts = take_compiled_commit_authority_counts_for_test();
    assert_eq!(counts.staged_seed_preparations, 1);
    assert_eq!(counts.installed_base_seed_publications, 0);
    assert_eq!(authority_counts, (1, 1));
    assert!(!Arc::ptr_eq(
        &installed,
        &engine.derived_state.as_ref().unwrap().mutation_lookup_seed
    ));
    assert!(engine
        .derived_state
        .as_ref()
        .unwrap()
        .mutation_lookup_seed
        .is_ready_for_test());

    reset_prepared_admission_counts_for_test();
    let public_commit = public
        .apply_typed_transaction(insert_transaction(&public, 65_225))
        .unwrap();
    let public_counts = take_prepared_admission_counts_for_test();
    assert_eq!(public_counts.staged_seed_preparations, 1);
    assert_eq!(public_counts.installed_base_seed_publications, 0);
    assert!(public
        .derived_state
        .as_ref()
        .unwrap()
        .mutation_lookup_seed
        .is_ready_for_test());
    assert_eq!(commit, public_commit);
    assert_eq!(engine.document_json(), public.document_json());
    assert_eq!(engine.resolved_selection(), public.resolved_selection());
    assert_eq!(engine.stored_marks(), public.stored_marks());
    assert_eq!(engine.can_undo(), public.can_undo());
    assert_eq!(engine.can_redo(), public.can_redo());

    reset_prepared_admission_counts_for_test();
    let rich_result = public_rich
        .apply_typed_transaction_with_result(insert_transaction(&public_rich, 65_225))
        .unwrap();
    assert!(rich_result.changed);
    let rich_counts = take_prepared_admission_counts_for_test();
    assert_eq!(rich_counts.staged_seed_preparations, 1);
    assert_eq!(rich_counts.installed_base_seed_publications, 0);
    assert!(public_rich
        .derived_state
        .as_ref()
        .unwrap()
        .mutation_lookup_seed
        .is_ready_for_test());
}

#[test]
fn staged_generic_compiler_semantic_failure_is_prewrite_and_atomic() {
    use crate::yrs_engine::compiler::{set_atomic_failpoint_for_test, AtomicFailpoint};
    use crate::yrs_engine::observability::{
        reset_prepared_admission_counts_for_test, take_prepared_admission_counts_for_test,
    };

    let mut engine = import_document_with_unavailable_lookup_seed();
    let installed = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
    let before = atomic_audit(&engine);
    reset_prepared_admission_counts_for_test();
    set_atomic_failpoint_for_test(Some(AtomicFailpoint::SemanticCompilation));
    let transaction = insert_transaction(&engine, 65_226);
    let error = engine
        .apply_typed_transaction_with_staged_context(
            transaction,
            false,
            &mut OutboundUpdateSink::detached(),
        )
        .unwrap_err();
    set_atomic_failpoint_for_test(None);

    assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
    assert_eq!(atomic_audit(&engine), before);
    assert!(Arc::ptr_eq(
        &installed,
        &engine.derived_state.as_ref().unwrap().mutation_lookup_seed,
    ));
    let counts = take_prepared_admission_counts_for_test();
    assert_eq!(counts.staged_seed_preparations, 1);
    assert_eq!(counts.installed_base_seed_publications, 0);
}

#[test]
fn staged_generic_lookup_transition_failure_is_prewrite_and_atomic() {
    use crate::yrs_engine::observability::{
        reset_prepared_admission_counts_for_test, take_prepared_admission_counts_for_test,
    };

    let mut engine = import_document_with_unavailable_lookup_seed();
    let installed = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
    let before = atomic_audit(&engine);
    reset_prepared_admission_counts_for_test();
    set_compiled_commit_stage_failpoint_for_test(Some(
        CompiledCommitPreparationStage::LookupTransition,
    ));
    let transaction = insert_transaction(&engine, 65_227);
    let error = engine
        .apply_typed_transaction_with_staged_context(
            transaction,
            false,
            &mut OutboundUpdateSink::detached(),
        )
        .unwrap_err();
    set_compiled_commit_stage_failpoint_for_test(None);

    assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
    assert_eq!(atomic_audit(&engine), before);
    assert!(Arc::ptr_eq(
        &installed,
        &engine.derived_state.as_ref().unwrap().mutation_lookup_seed,
    ));
    let counts = take_prepared_admission_counts_for_test();
    assert_eq!(counts.staged_seed_preparations, 1);
    assert_eq!(counts.installed_base_seed_publications, 0);
}

#[test]
fn history_candidate_swap_prepares_ready_candidate_seed_without_compiled_transaction() {
    use crate::yrs_engine::compiler::{set_atomic_failpoint_for_test, AtomicFailpoint};
    use crate::yrs_engine::observability::{
        reset_prepared_admission_counts_for_test, take_prepared_admission_counts_for_test,
    };
    use crate::yrs_engine::TransactionCommit;

    let mut engine = import_document_with_unavailable_lookup_seed();
    let mut public = import_document_with_unavailable_lookup_seed();
    engine
        .apply_command(65_226, TypedCommand::InsertText { text: "x".into() })
        .unwrap()
        .unwrap();
    public
        .apply_command(65_226, TypedCommand::InsertText { text: "x".into() })
        .unwrap()
        .unwrap();
    force_lookup_seed_unavailable(&mut engine);
    force_lookup_seed_unavailable(&mut public);
    let installed = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
    reset_prepared_admission_counts_for_test();
    set_atomic_failpoint_for_test(Some(AtomicFailpoint::SemanticCompilation));
    let result = engine.apply_history_pop(65_227, true, true, &mut OutboundUpdateSink::detached());
    let compiler_failpoint = crate::yrs_engine::compiler::check_atomic_failpoint(
        65_227,
        AtomicFailpoint::SemanticCompilation,
    );
    set_atomic_failpoint_for_test(None);
    let (commit, result) = result.unwrap().unwrap();
    let result = result.unwrap();
    let compiler_error = compiler_failpoint.unwrap_err();
    assert_eq!(compiler_error.code, "ENGINE_INVARIANT_FAILED");
    let counts = take_prepared_admission_counts_for_test();
    assert_eq!(counts.staged_seed_preparations, 1);
    assert_eq!(counts.installed_base_seed_publications, 0);
    assert!(!Arc::ptr_eq(
        &installed,
        &engine.derived_state.as_ref().unwrap().mutation_lookup_seed
    ));
    assert!(engine
        .derived_state
        .as_ref()
        .unwrap()
        .mutation_lookup_seed
        .is_ready_for_test());
    let state = engine.derived_state.as_ref().unwrap();
    let txn = engine.doc.transact();
    let fragment = txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();
    assert!(state
        .mutation_lookup_seed
        .matches_canonical_artifact(&state.canonical_artifact));
    assert!(state.mutation_lookup_seed.matches(
        &txn,
        &fragment,
        &state.document,
        &engine.resource_limits,
        &engine.editing_limits,
        engine.max_length,
        &engine.schema_fingerprint,
        engine.yrs_state_epoch,
        engine.revision,
    ));
    drop(txn);
    assert_eq!(
        commit,
        TransactionCommit {
            request_id: result.request_id,
            changed: result.changed,
            document_revision: result.document_revision,
            state_revision: result.state_revision,
            origin: result.origin,
        }
    );

    let public_result = public.undo_with_result(65_227).unwrap().unwrap();
    assert_eq!(result, public_result);
    assert_eq!(engine.document_json(), public.document_json());
    assert_eq!(engine.resolved_selection(), public.resolved_selection());
    assert_eq!(engine.stored_marks(), public.stored_marks());
    assert_eq!(engine.can_undo(), public.can_undo());
    assert_eq!(engine.can_redo(), public.can_redo());
    assert_eq!(
        engine.history.replay_audit_for_test(),
        public.history.replay_audit_for_test()
    );
    assert_eq!(
        engine.history.retained_units(65_227).unwrap(),
        public.history.retained_units(65_227).unwrap()
    );
}

#[test]
fn history_candidate_publication_failures_are_pre_swap_and_atomic() {
    use crate::yrs_engine::mutation::{
        set_lookup_seed_hydration_failpoint_for_test, LookupSeedHydrationFailpoint,
    };
    use crate::yrs_engine::observability::{
        reset_prepared_admission_counts_for_test, take_prepared_admission_counts_for_test,
    };

    for (request_id, failpoint, stage) in [
        (
            65_228,
            LookupSeedHydrationFailpoint::CandidateBindingPublication,
            "candidateBindingPublication",
        ),
        (
            65_229,
            LookupSeedHydrationFailpoint::CandidateSeedPublication,
            "candidateSeedPublication",
        ),
    ] {
        let mut engine = import_document_with_unavailable_lookup_seed();
        engine
            .apply_command(
                request_id - 1,
                TypedCommand::InsertText { text: "x".into() },
            )
            .unwrap()
            .unwrap();
        force_lookup_seed_unavailable(&mut engine);
        let installed = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
        let before = atomic_audit(&engine);
        reset_prepared_admission_counts_for_test();
        set_lookup_seed_hydration_failpoint_for_test(Some(failpoint));
        let error = engine
            .apply_history_pop(request_id, true, true, &mut OutboundUpdateSink::detached())
            .unwrap_err();
        set_lookup_seed_hydration_failpoint_for_test(None);

        assert_eq!(error.code, "OPERATION_RESOURCE_EXHAUSTED", "{stage}");
        assert_eq!(
            error.message.as_ref(),
            format!("mutation lookup seed allocation failed during {stage}"),
            "{stage}"
        );
        assert_eq!(
            error.details,
            Some(json!({ "field": "mutationLookupSeed" })),
            "{stage}"
        );
        assert_eq!(atomic_audit(&engine), before, "{stage}");
        assert!(Arc::ptr_eq(
            &installed,
            &engine.derived_state.as_ref().unwrap().mutation_lookup_seed,
        ));
        let counts = take_prepared_admission_counts_for_test();
        assert_eq!(counts.staged_seed_preparations, 0, "{stage}");
        assert_eq!(counts.installed_base_seed_publications, 0, "{stage}");
    }
}
