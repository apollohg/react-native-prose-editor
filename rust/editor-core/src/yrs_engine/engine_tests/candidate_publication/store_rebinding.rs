#[test]
fn authoritative_store_rebind_rejects_a_foreign_candidate_store() {
    let (engine, delta) = task5_changed_remote_fixture();
    let current_encoded = engine.encoded_state().unwrap();
    let build_candidate = || {
        let doc = super::fresh_utf16_doc_excluding(&engine.durable_client_ids, engine.client_id());
        {
            let mut txn = doc.transact_mut();
            txn.apply_update(Update::decode_v1(&current_encoded).unwrap())
                .unwrap();
            txn.apply_update(Update::decode_v1(&delta).unwrap())
                .unwrap();
        }
        doc
    };
    let candidate_doc = build_candidate();
    let foreign_candidate_doc = build_candidate();
    let (candidate_document, candidate_artifact) = {
        let txn = candidate_doc.transact();
        let fragment = txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();
        let json =
            crate::yrs_engine::YrsDocumentCodec::new(&engine.schema, &engine.resource_limits)
                .read_json(&fragment, &txn)
                .unwrap();
        let document =
            from_prosemirror_json(&json, &engine.schema, UnknownTypeMode::Preserve).unwrap();
        let artifact = engine.canonical_schema.derive(&document).unwrap();
        (document, artifact)
    };
    let next_revision = engine.revision.checked_add(1).unwrap();
    let next_epoch = engine.yrs_state_epoch.checked_add(1).unwrap();
    let candidate_seed = {
        let txn = candidate_doc.transact();
        let fragment = txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();
        prepare_history_candidate_capability_for_test(
            65_234,
            &txn,
            &fragment,
            &engine.schema,
            &candidate_document,
            &candidate_artifact,
            &engine.resource_limits,
            &engine.editing_limits,
            engine.max_length,
            &engine.schema_fingerprint,
            next_epoch,
            next_revision,
        )
        .prepare_candidate_publication(
            65_234,
            &txn,
            &fragment,
            &engine.schema,
            &candidate_document,
            &candidate_artifact,
            &engine.resource_limits,
            &engine.editing_limits,
            engine.max_length,
            &engine.schema_fingerprint,
            next_epoch,
            next_revision,
        )
        .unwrap()
    };
    let foreign_txn = foreign_candidate_doc.transact();
    let foreign_fragment = foreign_txn
        .get_xml_fragment(engine.fragment_name.as_str())
        .unwrap();
    let live_txn = engine.doc.transact();
    let live_fragment = live_txn
        .get_xml_fragment(engine.fragment_name.as_str())
        .unwrap();

    let error = candidate_seed
        .prepare_authoritative_store_rebind(
            65_235,
            &foreign_txn,
            &foreign_fragment,
            &candidate_document,
            &candidate_artifact,
            &engine.resource_limits,
            &engine.editing_limits,
            engine.max_length,
            &engine.schema_fingerprint,
            next_epoch,
            next_revision,
            &live_txn,
            &live_fragment,
        )
        .expect_err("a foreign candidate store must not be relabeled as live authority");
    assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
}

#[test]
fn authoritative_store_rebind_rejects_a_foreign_live_fragment_before_probes() {
    use crate::yrs_engine::mutation::{
        set_lookup_seed_hydration_failpoint_for_test, LookupSeedHydrationFailpoint,
    };
    use crate::yrs_engine::observability::{
        reset_prepared_admission_counts_for_test, take_prepared_admission_counts_for_test,
    };

    let (engine, candidate_doc, candidate_document, candidate_artifact, next_revision, next_epoch) =
        task5_candidate_publication_fixture();
    let candidate_seed = {
        let txn = candidate_doc.transact();
        let fragment = txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();
        prepare_history_candidate_capability_for_test(
            65_239,
            &txn,
            &fragment,
            &engine.schema,
            &candidate_document,
            &candidate_artifact,
            &engine.resource_limits,
            &engine.editing_limits,
            engine.max_length,
            &engine.schema_fingerprint,
            next_epoch,
            next_revision,
        )
        .prepare_candidate_publication(
            65_239,
            &txn,
            &fragment,
            &engine.schema,
            &candidate_document,
            &candidate_artifact,
            &engine.resource_limits,
            &engine.editing_limits,
            engine.max_length,
            &engine.schema_fingerprint,
            next_epoch,
            next_revision,
        )
        .unwrap()
    };
    let foreign_live_fragment = engine.doc.get_or_insert_xml_fragment("foreign-live");
    let before = atomic_audit(&engine);
    let installed = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
    let candidate_txn = candidate_doc.transact();
    let candidate_fragment = candidate_txn
        .get_xml_fragment(engine.fragment_name.as_str())
        .unwrap();
    let live_txn = engine.doc.transact();

    reset_prepared_admission_counts_for_test();
    set_lookup_seed_hydration_failpoint_for_test(Some(
        LookupSeedHydrationFailpoint::BindingPublication,
    ));
    let result = candidate_seed.prepare_authoritative_store_rebind(
        65_240,
        &candidate_txn,
        &candidate_fragment,
        &candidate_document,
        &candidate_artifact,
        &engine.resource_limits,
        &engine.editing_limits,
        engine.max_length,
        &engine.schema_fingerprint,
        next_epoch,
        next_revision,
        &live_txn,
        &foreign_live_fragment,
    );
    set_lookup_seed_hydration_failpoint_for_test(None);

    let error = result.expect_err("a foreign live fragment must reject before publication");
    assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
    let counts = take_prepared_admission_counts_for_test();
    assert_eq!(counts.staged_seed_preparations, 0);
    assert_eq!(counts.installed_base_seed_publications, 0);
    assert_eq!(atomic_audit(&engine), before);
    assert!(Arc::ptr_eq(
        &installed,
        &engine.derived_state.as_ref().unwrap().mutation_lookup_seed
    ));
}

#[test]
fn matching_history_seed_publications_reach_all_four_exact_failpoint_stages() {
    use crate::yrs_engine::mutation::{
        set_lookup_seed_hydration_failpoint_for_test, LookupSeedHydrationFailpoint,
    };
    use crate::yrs_engine::observability::{
        reset_prepared_admission_counts_for_test, take_prepared_admission_counts_for_test,
    };

    for (failpoint, expected_stage) in [
        (
            LookupSeedHydrationFailpoint::BindingPublication,
            "candidateBindingPublication",
        ),
        (
            LookupSeedHydrationFailpoint::SeedPublication,
            "candidateSeedPublication",
        ),
    ] {
        let (
            engine,
            candidate_doc,
            candidate_document,
            candidate_artifact,
            next_revision,
            next_epoch,
        ) = task5_candidate_publication_fixture();
        let before = atomic_audit(&engine);
        let installed = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
        let txn = candidate_doc.transact();
        let fragment = txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();
        let unavailable = prepare_history_candidate_capability_for_test(
            65_241,
            &txn,
            &fragment,
            &engine.schema,
            &candidate_document,
            &candidate_artifact,
            &engine.resource_limits,
            &engine.editing_limits,
            engine.max_length,
            &engine.schema_fingerprint,
            next_epoch,
            next_revision,
        );
        reset_prepared_admission_counts_for_test();
        set_lookup_seed_hydration_failpoint_for_test(Some(failpoint));
        let result = unavailable.prepare_candidate_publication(
            65_241,
            &txn,
            &fragment,
            &engine.schema,
            &candidate_document,
            &candidate_artifact,
            &engine.resource_limits,
            &engine.editing_limits,
            engine.max_length,
            &engine.schema_fingerprint,
            next_epoch,
            next_revision,
        );
        set_lookup_seed_hydration_failpoint_for_test(None);
        let error = result.expect_err("matching candidate must reach armed publication stage");
        assert_eq!(error.code, "OPERATION_RESOURCE_EXHAUSTED");
        assert_eq!(error.request_id, 65_241);
        assert_eq!(
            error.message.as_ref(),
            format!("mutation lookup seed allocation failed during {expected_stage}")
        );
        assert_eq!(
            error.details,
            Some(json!({ "field": "mutationLookupSeed" }))
        );
        let counts = take_prepared_admission_counts_for_test();
        assert_eq!(counts.staged_seed_preparations, 0);
        assert_eq!(counts.installed_base_seed_publications, 0);
        assert_eq!(atomic_audit(&engine), before);
        assert!(Arc::ptr_eq(
            &installed,
            &engine.derived_state.as_ref().unwrap().mutation_lookup_seed
        ));
    }

    let (engine, candidate_doc, candidate_document, candidate_artifact, next_revision, next_epoch) =
        task5_candidate_publication_fixture();
    let candidate_seed = {
        let txn = candidate_doc.transact();
        let fragment = txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();
        prepare_history_candidate_capability_for_test(
            65_242,
            &txn,
            &fragment,
            &engine.schema,
            &candidate_document,
            &candidate_artifact,
            &engine.resource_limits,
            &engine.editing_limits,
            engine.max_length,
            &engine.schema_fingerprint,
            next_epoch,
            next_revision,
        )
        .prepare_candidate_publication(
            65_242,
            &txn,
            &fragment,
            &engine.schema,
            &candidate_document,
            &candidate_artifact,
            &engine.resource_limits,
            &engine.editing_limits,
            engine.max_length,
            &engine.schema_fingerprint,
            next_epoch,
            next_revision,
        )
        .unwrap()
    };
    let before = atomic_audit(&engine);
    let installed = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
    let candidate_txn = candidate_doc.transact();
    let candidate_fragment = candidate_txn
        .get_xml_fragment(engine.fragment_name.as_str())
        .unwrap();
    let live_txn = engine.doc.transact();
    let live_fragment = live_txn
        .get_xml_fragment(engine.fragment_name.as_str())
        .unwrap();
    for (failpoint, expected_stage) in [
        (
            LookupSeedHydrationFailpoint::BindingPublication,
            "authoritativeStoreBindingPublication",
        ),
        (
            LookupSeedHydrationFailpoint::SeedPublication,
            "authoritativeStoreSeedPublication",
        ),
    ] {
        reset_prepared_admission_counts_for_test();
        set_lookup_seed_hydration_failpoint_for_test(Some(failpoint));
        let result = candidate_seed.prepare_authoritative_store_rebind(
            65_243,
            &candidate_txn,
            &candidate_fragment,
            &candidate_document,
            &candidate_artifact,
            &engine.resource_limits,
            &engine.editing_limits,
            engine.max_length,
            &engine.schema_fingerprint,
            next_epoch,
            next_revision,
            &live_txn,
            &live_fragment,
        );
        set_lookup_seed_hydration_failpoint_for_test(None);
        let error = result.expect_err("matching rebind must reach armed publication stage");
        assert_eq!(error.code, "OPERATION_RESOURCE_EXHAUSTED");
        assert_eq!(error.request_id, 65_243);
        assert_eq!(
            error.message.as_ref(),
            format!("mutation lookup seed allocation failed during {expected_stage}")
        );
        assert_eq!(
            error.details,
            Some(json!({ "field": "mutationLookupSeed" }))
        );
        let counts = take_prepared_admission_counts_for_test();
        assert_eq!(counts.staged_seed_preparations, 0);
        assert_eq!(counts.installed_base_seed_publications, 0);
        assert_eq!(atomic_audit(&engine), before);
        assert!(Arc::ptr_eq(
            &installed,
            &engine.derived_state.as_ref().unwrap().mutation_lookup_seed
        ));
    }
}

#[test]
fn changed_remote_candidate_installs_only_its_candidate_owned_seed() {
    use crate::yrs_engine::observability::{
        reset_prepared_admission_counts_for_test, take_prepared_admission_counts_for_test,
    };

    let (mut engine, delta) = task5_changed_remote_fixture();
    let unavailable = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
    let unchanged = engine.encoded_state().unwrap();
    reset_prepared_admission_counts_for_test();
    assert!(
        !engine
            .apply_remote_update_v1(65_230, &unchanged)
            .unwrap()
            .changed
    );
    let unchanged_counts = take_prepared_admission_counts_for_test();
    assert_eq!(unchanged_counts.staged_seed_preparations, 0);
    assert_eq!(unchanged_counts.installed_base_seed_publications, 0);
    assert!(Arc::ptr_eq(
        &unavailable,
        &engine.derived_state.as_ref().unwrap().mutation_lookup_seed
    ));

    reset_prepared_admission_counts_for_test();
    assert!(
        engine
            .apply_remote_update_v1(65_231, &delta)
            .unwrap()
            .changed
    );
    let changed_counts = take_prepared_admission_counts_for_test();
    assert_eq!(changed_counts.staged_seed_preparations, 1);
    assert_eq!(changed_counts.installed_base_seed_publications, 0);
    assert!(!Arc::ptr_eq(
        &unavailable,
        &engine.derived_state.as_ref().unwrap().mutation_lookup_seed
    ));
    assert!(engine
        .derived_state
        .as_ref()
        .unwrap()
        .mutation_lookup_seed
        .is_ready_for_test());
}

#[test]
fn remote_live_store_rebind_allocation_failure_is_prewrite_and_atomic() {
    use crate::yrs_engine::mutation::{
        set_lookup_seed_hydration_failpoint_for_test, LookupSeedHydrationFailpoint,
    };
    use crate::yrs_engine::observability::{
        reset_prepared_admission_counts_for_test, take_prepared_admission_counts_for_test,
    };

    let (mut engine, delta) = task5_changed_remote_fixture();
    let before = atomic_audit(&engine);
    let quarantine_before = engine.quarantined_remote_update.clone();
    let ledger_before = engine.history.replay_ledger_allocation_audit_for_test();
    let unavailable = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
    reset_prepared_admission_counts_for_test();
    set_lookup_seed_hydration_failpoint_for_test(Some(
        LookupSeedHydrationFailpoint::SeedPublication,
    ));
    let result = engine.apply_remote_update_v1(65_232, &delta);
    set_lookup_seed_hydration_failpoint_for_test(None);
    let error = result.expect_err("live-store rebind allocation failure must reject");
    assert_eq!(error.code, "OPERATION_RESOURCE_EXHAUSTED");
    assert_eq!(
        error.message.as_ref(),
        "mutation lookup seed allocation failed during authoritativeStoreSeedPublication"
    );
    assert_eq!(
        error.details,
        Some(json!({ "field": "mutationLookupSeed" }))
    );
    let counts = take_prepared_admission_counts_for_test();
    assert_eq!(counts.staged_seed_preparations, 0);
    assert_eq!(counts.installed_base_seed_publications, 0);
    assert_eq!(atomic_audit(&engine), before);
    assert_eq!(engine.quarantined_remote_update, quarantine_before);
    assert_eq!(
        engine.history.replay_ledger_allocation_audit_for_test(),
        ledger_before
    );
    assert!(Arc::ptr_eq(
        &unavailable,
        &engine.derived_state.as_ref().unwrap().mutation_lookup_seed
    ));
}
