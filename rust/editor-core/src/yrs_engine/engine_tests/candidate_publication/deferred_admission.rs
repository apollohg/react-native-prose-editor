#[test]
fn deferred_finalization_reuses_saved_evidence_without_revalidation() {
    use crate::yrs_engine::observability::{
        reset_full_pass_counts_for_test, reset_prepared_admission_counts_for_test,
        take_full_pass_counts_for_test, take_prepared_admission_counts_for_test,
    };

    reset_prepared_admission_counts_for_test();
    let (engine, deferred, mut context, transaction, expected_document) =
        deferred_finalization_fixture();
    engine.prepare_mutation_identity(&mut context).unwrap();
    reset_full_pass_counts_for_test();
    let prepared = engine
        .finalize_deferred_for_test(deferred, &context, &transaction, &expected_document)
        .unwrap();
    assert!(prepared.admits_expected_document(&expected_document));
    let passes = take_full_pass_counts_for_test();
    let admission = take_prepared_admission_counts_for_test();
    assert_eq!(passes.planner_simulations, 0);
    assert_eq!(passes.document_validations, 0);
    assert_eq!(passes.render_limit_tree_scans, 0);
    assert_eq!(passes.render_identity_scans, 0);
    assert_eq!(admission.deferred_capsules_created, 1);
    assert_eq!(admission.deferred_capsules_finalized, 1);
}

#[test]
fn deferred_capsule_tamper_cases_reject_before_write() {
    for case in
        crate::yrs_engine::prepared_admission::DeferredCommandAdmission::tamper_cases_for_test()
    {
        let (engine, deferred, mut context, transaction, expected_document) =
            deferred_tamper_fixture(case);
        engine.prepare_mutation_identity(&mut context).unwrap();
        let before = atomic_audit(&engine);
        let error = engine
            .finalize_deferred_for_test(deferred, &context, &transaction, &expected_document)
            .expect_err(&format!("tampered deferred capsule must reject: {case}"));
        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED", "{case}");
        assert_eq!(atomic_audit(&engine), before, "{case}");
    }
}

#[test]
fn deferred_same_summary_evidence_replacements_reject_without_identity_scans() {
    use crate::yrs_engine::observability::{
        reset_full_pass_counts_for_test, take_full_pass_counts_for_test,
    };

    for case in ["position", "render"] {
        let (engine, mut deferred, mut context, transaction, expected_document) =
            deferred_finalization_fixture();
        deferred.tamper_same_summary_evidence_for_test(case);
        engine.prepare_mutation_identity(&mut context).unwrap();
        let before = atomic_audit(&engine);
        reset_full_pass_counts_for_test();

        let error = engine
            .finalize_deferred_for_test(deferred, &context, &transaction, &expected_document)
            .unwrap_err();

        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED", "{case}");
        assert_eq!(atomic_audit(&engine), before, "{case}");
        let passes = take_full_pass_counts_for_test();
        assert_eq!(passes.position_map_clones, 0, "{case}");
        assert_eq!(passes.render_limit_tree_scans, 0, "{case}");
        assert_eq!(passes.render_identity_scans, 0, "{case}");
    }
}

#[test]
fn deferred_shape_rejects_matching_transaction_position_tamper() {
    let (engine, mut deferred, mut context, mut transaction, expected_document) =
        deferred_finalization_fixture();
    deferred.tamper_matching_transaction_position_for_test(&mut transaction);
    engine.prepare_mutation_identity(&mut context).unwrap();
    let before = atomic_audit(&engine);

    let error = engine
        .finalize_deferred_for_test(deferred, &context, &transaction, &expected_document)
        .unwrap_err();

    assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
    assert_eq!(atomic_audit(&engine), before);
}

#[test]
fn deferred_finalization_preserves_warmed_candidate_scalar_caches() {
    use crate::yrs_engine::observability::{
        reset_full_pass_counts_for_test, take_full_pass_counts_for_test,
    };

    let (engine, deferred, mut context, transaction, expected_document) =
        deferred_finalization_fixture();
    let (expected_len, expected_sha256) = deferred.warm_candidate_caches_for_test();
    engine.prepare_mutation_identity(&mut context).unwrap();
    reset_full_pass_counts_for_test();

    let prepared = engine
        .finalize_deferred_for_test(deferred, &context, &transaction, &expected_document)
        .unwrap();

    assert_eq!(prepared.canonical_artifact().serialized_len(), expected_len);
    assert_eq!(prepared.canonical_artifact().sha256(), expected_sha256);
    let passes = take_full_pass_counts_for_test();
    assert_eq!(passes.canonical_serializations, 0);
    assert_eq!(passes.canonical_hashes, 0);
}

#[test]
fn deferred_finalization_rejects_mismatched_prefilled_candidate_caches() {
    use crate::yrs_engine::observability::{
        reset_full_pass_counts_for_test, take_full_pass_counts_for_test,
    };

    for case in ["length", "sha256"] {
        let (engine, mut deferred, mut context, transaction, expected_document) =
            deferred_finalization_fixture();
        let _ = deferred.warm_candidate_caches_for_test();
        deferred.tamper_candidate_cache_for_test(case);
        engine.prepare_mutation_identity(&mut context).unwrap();
        let before = atomic_audit(&engine);
        reset_full_pass_counts_for_test();

        let error = engine
            .finalize_deferred_for_test(deferred, &context, &transaction, &expected_document)
            .unwrap_err();

        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED", "{case}");
        assert_eq!(atomic_audit(&engine), before, "{case}");
        let passes = take_full_pass_counts_for_test();
        assert_eq!(passes.canonical_serializations, 0, "{case}");
        assert_eq!(passes.canonical_hashes, 0, "{case}");
    }
}

#[test]
fn imported_commands_plan_not_applicable_and_stored_marks_before_hydration() {
    use crate::yrs_engine::mutation::{
        set_lookup_seed_hydration_failpoint_for_test, LookupSeedHydrationFailpoint,
    };
    use crate::yrs_engine::observability::{
        reset_prepared_admission_counts_for_test, take_prepared_admission_counts_for_test,
    };

    let mut not_applicable = import_document_with_unavailable_lookup_seed();
    reset_prepared_admission_counts_for_test();
    set_lookup_seed_hydration_failpoint_for_test(Some(
        LookupSeedHydrationFailpoint::InitialReservation,
    ));
    let result = not_applicable
        .apply_command(65_130, TypedCommand::ToggleTaskItemChecked)
        .unwrap();
    set_lookup_seed_hydration_failpoint_for_test(None);
    assert!(result.is_none());
    let not_applicable_counts = take_prepared_admission_counts_for_test();
    assert_eq!(not_applicable_counts.staged_seed_preparations, 0);
    assert_eq!(not_applicable_counts.installed_base_seed_publications, 0);
    assert!(not_applicable
        .derived_state
        .as_ref()
        .unwrap()
        .mutation_lookup_seed
        .is_unavailable_for_test());

    let mut stored_mark = import_document_with_unavailable_lookup_seed();
    reset_prepared_admission_counts_for_test();
    set_lookup_seed_hydration_failpoint_for_test(Some(
        LookupSeedHydrationFailpoint::InitialReservation,
    ));
    let result = stored_mark
        .apply_command(
            65_131,
            TypedCommand::ToggleMark {
                mark_type: "bold".into(),
            },
        )
        .unwrap();
    set_lookup_seed_hydration_failpoint_for_test(None);
    assert!(result.is_some());
    let stored_mark_counts = take_prepared_admission_counts_for_test();
    assert_eq!(stored_mark_counts.staged_seed_preparations, 0);
    assert_eq!(stored_mark_counts.installed_base_seed_publications, 0);
    assert_eq!(
        stored_mark
            .stored_marks()
            .unwrap()
            .iter()
            .map(Mark::mark_type)
            .collect::<Vec<_>>(),
        vec!["bold"]
    );
    assert!(stored_mark
        .derived_state
        .as_ref()
        .unwrap()
        .mutation_lookup_seed
        .is_unavailable_for_test());
}

#[test]
fn immediate_import_local_input_local_api_and_structural_routes_hydrate_real_consumers() {
    let mut local_input = import_document_with_unavailable_lookup_seed();
    let mut transaction = insert_transaction(&local_input, 65_140);
    transaction.origin = TransactionOrigin::LocalInput;
    local_input.apply_typed_transaction(transaction).unwrap();
    assert!(local_input
        .derived_state
        .as_ref()
        .unwrap()
        .mutation_lookup_seed
        .is_ready_for_test());

    let mut local_api = import_document_with_unavailable_lookup_seed();
    local_api
        .apply_typed_transaction(insert_transaction(&local_api, 65_141))
        .unwrap();
    assert!(local_api
        .derived_state
        .as_ref()
        .unwrap()
        .mutation_lookup_seed
        .is_ready_for_test());

    let mut structural = import_document_with_unavailable_lookup_seed();
    crate::yrs_engine::mutation::reset_localized_lookup_counts_for_test();
    structural
        .apply_command(
            65_142,
            TypedCommand::WrapInList {
                list_type: "bulletList".into(),
                item_type: "listItem".into(),
            },
        )
        .unwrap()
        .expect("paragraph should wrap in a bullet list");
    assert_eq!(
        crate::yrs_engine::mutation::take_localized_lookup_counts_for_test(),
        (0, 0, 0),
        "the structural command must consume the staged seed without a live rebuild"
    );
    assert_eq!(
        structural.document_json().unwrap()["content"][0]["type"],
        "bulletList"
    );
}

#[test]
fn immediate_import_noop_remote_candidate_does_not_hydrate_live_seed() {
    use crate::yrs_engine::mutation::{
        set_lookup_seed_hydration_failpoint_for_test, LookupSeedHydrationFailpoint,
    };

    let mut engine = import_document_with_unavailable_lookup_seed();
    let update = engine.encoded_state().unwrap();
    let unavailable = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
    set_lookup_seed_hydration_failpoint_for_test(Some(
        LookupSeedHydrationFailpoint::InitialReservation,
    ));

    let commit = engine.apply_remote_update_v1(65_143, &update).unwrap();

    set_lookup_seed_hydration_failpoint_for_test(None);
    assert!(!commit.changed);
    assert!(Arc::ptr_eq(
        &unavailable,
        &engine.derived_state.as_ref().unwrap().mutation_lookup_seed
    ));
    assert!(engine
        .derived_state
        .as_ref()
        .unwrap()
        .mutation_lookup_seed
        .is_unavailable_for_test());

    let mut source = YrsDocumentEngine::new(YrsEngineConfig {
        schema: tiptap_schema(),
        fragment_name: "prosemirror".into(),
        initialization_mode: crate::yrs_engine::InitializationMode::AwaitRemote,
        resource_limits: ResourceLimits::default(),
        editing_limits: crate::yrs_engine::EditingLimits::default(),
        max_length: None,
        scope: None,
    })
    .unwrap();
    source.apply_remote_update_v1(65_144, &update).unwrap();
    source
        .apply_command(65_145, TypedCommand::InsertText { text: "r".into() })
        .unwrap()
        .unwrap();
    let target_vector = engine.doc.transact().state_vector();
    let delta = source
        .doc
        .transact()
        .encode_state_as_update_v1(&target_vector);

    let commit = engine.apply_remote_update_v1(65_146, &delta).unwrap();

    assert!(commit.changed);
    assert!(engine
        .derived_state
        .as_ref()
        .unwrap()
        .mutation_lookup_seed
        .is_ready_for_test());
}

#[test]
fn prepare_mutation_context_does_not_publish_the_installed_seed() {
    let engine = import_document_with_unavailable_lookup_seed();
    let before = atomic_audit(&engine);
    let installed = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
    crate::yrs_engine::observability::reset_prepared_admission_counts_for_test();
    let context = engine.prepare_mutation_lookup_seed(65_210).unwrap();
    assert!(context.lookup_seed().is_ready_for_test());
    assert!(Arc::ptr_eq(
        &installed,
        &engine.derived_state.as_ref().unwrap().mutation_lookup_seed,
    ));
    assert_eq!(atomic_audit(&engine), before);
    let counts = crate::yrs_engine::observability::take_prepared_admission_counts_for_test();
    assert_eq!(counts.staged_seed_preparations, 1);
    assert_eq!(counts.installed_base_seed_publications, 0);
}

#[test]
fn prepared_mutation_identity_is_lazy_and_does_not_mutate_installed_caches() {
    let engine = import_document_with_unavailable_lookup_seed();
    let mut context = engine.prepare_mutation_lookup_seed(65_211).unwrap();
    crate::yrs_engine::observability::reset_prepared_admission_counts_for_test();
    assert!(context.materialized_identity().is_none());
    engine.prepare_mutation_identity(&mut context).unwrap();
    assert!(context.materialized_identity().is_some());
    assert_eq!(
        crate::yrs_engine::observability::take_prepared_admission_counts_for_test()
            .staged_identity_materializations,
        1,
    );
    assert!(!engine
        .derived_state
        .as_ref()
        .unwrap()
        .validation_certificate
        .canonical_fingerprint_materialized_for_test());
    assert!(!engine
        .derived_state
        .as_ref()
        .unwrap()
        .localized_text_index
        .as_ref()
        .unwrap()
        .canonical_fingerprint_materialized_for_test());
}

#[test]
fn prepared_mutation_authority_rejects_request_mismatch_atomically() {
    let engine = import_document_with_unavailable_lookup_seed();
    let before = atomic_audit(&engine);
    let installed = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
    crate::yrs_engine::observability::reset_prepared_admission_counts_for_test();
    let context = engine.prepare_mutation_lookup_seed(65_212).unwrap();
    let state = engine.derived_state.as_ref().unwrap();
    let txn = engine.doc.transact();
    let fragment = txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();

    let error = match context.authority(
        crate::yrs_engine::prepared_admission::LiveMutationAuthorityContext {
            request_id: 65_213,
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
    ) {
        Ok(_) => panic!("a prepared context must not authorize another request"),
        Err(error) => error,
    };
    assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
    assert_eq!(error.request_id, 65_212);

    {
        let authority = context
            .authority(
                crate::yrs_engine::prepared_admission::LiveMutationAuthorityContext {
                    request_id: 65_212,
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
        assert!(authority.lookup_seed().is_ready_for_test());
    }
    drop(txn);

    assert!(Arc::ptr_eq(
        &installed,
        &engine.derived_state.as_ref().unwrap().mutation_lookup_seed,
    ));
    assert_eq!(atomic_audit(&engine), before);
    let counts = crate::yrs_engine::observability::take_prepared_admission_counts_for_test();
    assert_eq!(counts.staged_seed_preparations, 1);
    assert_eq!(counts.installed_base_seed_publications, 0);
}

#[test]
fn lookup_seed_rejects_same_value_stale_canonical_artifact_identity() {
    let mut engine = import_document_with_unavailable_lookup_seed();
    engine.ensure_mutation_lookup_seed(65_108).unwrap();
    let state = engine.derived_state.as_ref().unwrap();
    let stale_seed = Arc::clone(&state.mutation_lookup_seed);
    assert!(stale_seed.matches_canonical_artifact(&state.canonical_artifact));

    let replacement = state
        .canonical_artifact
        .schema_context()
        .derive(&state.document)
        .unwrap();
    assert!(!replacement.ptr_eq(&state.canonical_artifact));
    engine.derived_state.as_mut().unwrap().canonical_artifact = replacement;
    assert!(!stale_seed
        .matches_canonical_artifact(&engine.derived_state.as_ref().unwrap().canonical_artifact));

    crate::yrs_engine::mutation::reset_localized_lookup_counts_for_test();
    engine.ensure_mutation_lookup_seed(65_109).unwrap();
    assert_eq!(
        crate::yrs_engine::mutation::take_localized_lookup_counts_for_test().0,
        1
    );
    let state = engine.derived_state.as_ref().unwrap();
    assert!(state
        .mutation_lookup_seed
        .matches_canonical_artifact(&state.canonical_artifact));
}

#[test]
fn unavailable_lookup_hydration_failure_is_atomic() {
    let mut engine = import_document_with_unavailable_lookup_seed();
    engine.fragment_name = "missing-after-import".into();
    let before = atomic_audit(&engine);
    let unavailable = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);

    let error = engine
        .apply_command(65_108, TypedCommand::InsertText { text: "x".into() })
        .unwrap_err();

    assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
    assert!(Arc::ptr_eq(
        &unavailable,
        &engine.derived_state.as_ref().unwrap().mutation_lookup_seed
    ));
    assert_eq!(atomic_audit(&engine), before);
}

#[test]
fn unavailable_lookup_allocation_failpoints_are_resource_errors_and_atomic() {
    use crate::yrs_engine::mutation::{
        set_lookup_seed_hydration_failpoint_for_test, LookupSeedHydrationFailpoint,
    };

    for (index, failpoint) in [
        LookupSeedHydrationFailpoint::InitialReservation,
        LookupSeedHydrationFailpoint::MapGrowth,
        LookupSeedHydrationFailpoint::MapPublication,
        LookupSeedHydrationFailpoint::BindingPublication,
        LookupSeedHydrationFailpoint::SeedPublication,
    ]
    .into_iter()
    .enumerate()
    {
        let mut engine = import_document_with_unavailable_lookup_seed();
        assert!(engine.prepared_candidate_cache.take().is_some());
        let before = atomic_audit(&engine);
        let unavailable = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
        set_lookup_seed_hydration_failpoint_for_test(Some(failpoint));

        let error = engine
            .apply_command(
                65_120 + index as u64,
                TypedCommand::InsertText { text: "x".into() },
            )
            .unwrap_err();

        set_lookup_seed_hydration_failpoint_for_test(None);
        assert_eq!(error.code, "OPERATION_RESOURCE_EXHAUSTED", "{failpoint:?}");
        assert_eq!(
            error.details,
            Some(json!({ "field": "mutationLookupSeed" })),
            "{failpoint:?}"
        );
        assert!(
            Arc::ptr_eq(
                &unavailable,
                &engine.derived_state.as_ref().unwrap().mutation_lookup_seed
            ),
            "{failpoint:?}"
        );
        assert_eq!(atomic_audit(&engine), before, "{failpoint:?}");
    }
}

#[test]
fn lookup_seed_hydration_does_not_reserve_growth_with_spare_capacity() {
    use crate::yrs_engine::mutation::{
        reset_lookup_seed_map_growth_attempts_for_test,
        take_lookup_seed_map_growth_attempts_for_test,
    };

    let mut engine = import_document_with_unavailable_lookup_seed();
    assert!(engine.prepared_candidate_cache.take().is_some());
    reset_lookup_seed_map_growth_attempts_for_test();
    engine
        .apply_command(65_126, TypedCommand::InsertText { text: "x".into() })
        .unwrap()
        .unwrap();

    assert_eq!(take_lookup_seed_map_growth_attempts_for_test(), 0);
}

#[test]
fn engine_commands_reuse_the_proven_schema_context_without_recomputing_it() {
    use crate::yrs_engine::canonical::{
        reset_canonical_schema_context_count_for_test, take_canonical_schema_context_count_for_test,
    };

    let mut engine = transaction_engine();
    reset_canonical_schema_context_count_for_test();
    engine
        .apply_command(65_001, TypedCommand::InsertText { text: "x".into() })
        .unwrap();

    assert_eq!(take_canonical_schema_context_count_for_test(), 0);
}

#[test]
fn collision_excluding_candidate_selection_retries_live_and_durable_ids() {
    let durable = HashSet::from([7_u64]);
    let mut ids = [5_u64, 7_u64, 11_u64].into_iter();
    let selected = fresh_utf16_doc_excluding_with(&durable, 5, || {
        Doc::with_options(Options {
            client_id: ClientID::new(ids.next().unwrap()),
            offset_kind: OffsetKind::Utf16,
            ..Options::default()
        })
    });

    assert_eq!(selected.client_id().get(), 11);
}

#[test]
fn restored_and_local_candidates_cache_all_relevant_durable_clients() {
    let config = || crate::yrs_engine::YrsEngineConfig {
        schema: tiptap_schema(),
        fragment_name: "prosemirror".into(),
        initialization_mode: crate::yrs_engine::InitializationMode::LocalEmpty,
        resource_limits: ResourceLimits::default(),
        editing_limits: crate::yrs_engine::EditingLimits::default(),
        max_length: None,
        scope: Some(crate::yrs_engine::DocumentScope {
            document_id: "doc".into(),
            lineage_id: "lineage".into(),
        }),
    };
    let source = crate::yrs_engine::YrsDocumentEngine::new(config()).unwrap();
    let snapshot = source.export_snapshot().unwrap();
    let expected = Update::decode_v1(&snapshot.encoded_state)
        .unwrap()
        .state_vector()
        .iter()
        .map(|(client, _)| client.get())
        .collect::<HashSet<_>>();
    let mut target = crate::yrs_engine::YrsDocumentEngine::new(config()).unwrap();

    target.restore_snapshot(&snapshot).unwrap();
    assert_eq!(target.durable_client_ids, expected);

    target
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"local"}]}]}"#,
            crate::yrs_engine::TransactionOrigin::DocumentImport,
        )
        .unwrap();
    assert_eq!(
        target.durable_client_ids,
        HashSet::from([target.client_id()])
    );
}
