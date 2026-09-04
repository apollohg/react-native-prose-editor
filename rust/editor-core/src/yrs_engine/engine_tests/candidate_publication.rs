use super::*;

fn task5_changed_remote_fixture() -> (YrsDocumentEngine, Vec<u8>) {
    let target = import_document_with_unavailable_lookup_seed();
    let base = target.encoded_state().unwrap();
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
    source.apply_remote_update_v1(65_228, &base).unwrap();
    source
        .apply_command(65_229, TypedCommand::InsertText { text: "r".into() })
        .unwrap()
        .unwrap();
    let target_vector = target.doc.transact().state_vector();
    let delta = source
        .doc
        .transact()
        .encode_state_as_update_v1(&target_vector);
    (target, delta)
}

fn task5_candidate_publication_fixture() -> (
    YrsDocumentEngine,
    Doc,
    crate::model::Document,
    crate::yrs_engine::canonical::CanonicalArtifact,
    u64,
    u64,
) {
    let (engine, delta) = task5_changed_remote_fixture();
    let current_encoded = engine.encoded_state().unwrap();
    let candidate_doc =
        super::fresh_utf16_doc_excluding(&engine.durable_client_ids, engine.client_id());
    {
        let mut txn = candidate_doc.transact_mut();
        txn.apply_update(Update::decode_v1(&current_encoded).unwrap())
            .unwrap();
        txn.apply_update(Update::decode_v1(&delta).unwrap())
            .unwrap();
    }
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
    (
        engine,
        candidate_doc,
        candidate_document,
        candidate_artifact,
        next_revision,
        next_epoch,
    )
}

#[allow(clippy::too_many_arguments)]
fn prepare_history_candidate_capability_for_test<T: ReadTxn>(
    request_id: u64,
    txn: &T,
    fragment: &yrs::types::xml::XmlFragmentRef,
    schema: &crate::schema::Schema,
    source_document: &crate::model::Document,
    canonical_artifact: &crate::yrs_engine::canonical::CanonicalArtifact,
    resource_limits: &ResourceLimits,
    editing_limits: &crate::yrs_engine::EditingLimits,
    max_length: Option<u32>,
    schema_fingerprint: &str,
    yrs_state_epoch: u64,
    document_revision: u64,
) -> crate::yrs_engine::derived_state::HistoryMutationLookupCapability {
    let (json, admission) =
        crate::yrs_engine::derived_state::prepare_history_candidate_read_for_test(
            request_id,
            txn,
            fragment,
            schema,
            source_document,
            canonical_artifact,
            resource_limits,
            editing_limits,
            max_length,
            schema_fingerprint,
            yrs_state_epoch,
            document_revision,
        )
        .unwrap()
        .into_parts();
    assert_eq!(&json, canonical_artifact.value());
    admission
        .expect("exact candidate read must create one consuming admission")
        .mint_capability_for_test(request_id, txn, fragment)
        .unwrap()
}

#[allow(clippy::too_many_arguments)]
fn publish_history_candidate_seed_for_test<T: ReadTxn>(
    capability: crate::yrs_engine::derived_state::HistoryMutationLookupCapability,
    request_id: u64,
    txn: &T,
    fragment: &yrs::types::xml::XmlFragmentRef,
    schema: &crate::schema::Schema,
    source_document: &crate::model::Document,
    canonical_artifact: &crate::yrs_engine::canonical::CanonicalArtifact,
    resource_limits: &ResourceLimits,
    editing_limits: &crate::yrs_engine::EditingLimits,
    max_length: Option<u32>,
    schema_fingerprint: &str,
    yrs_state_epoch: u64,
    document_revision: u64,
) -> crate::yrs_engine::OperationResult<Arc<crate::yrs_engine::mutation::MutationLookupSeed>> {
    capability.prepare_candidate_publication(
        request_id,
        txn,
        fragment,
        schema,
        source_document,
        canonical_artifact,
        resource_limits,
        editing_limits,
        max_length,
        schema_fingerprint,
        yrs_state_epoch,
        document_revision,
    )
}

#[test]
fn candidate_seed_publication_is_ready_and_bound_only_to_its_candidate_store() {
    use crate::yrs_engine::observability::{
        reset_prepared_admission_counts_for_test, take_prepared_admission_counts_for_test,
    };

    let (engine, delta) = task5_changed_remote_fixture();
    let installed = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
    let current_encoded = engine.encoded_state().unwrap();
    let candidate_doc =
        super::fresh_utf16_doc_excluding(&engine.durable_client_ids, engine.client_id());
    {
        let mut txn = candidate_doc.transact_mut();
        txn.apply_update(Update::decode_v1(&current_encoded).unwrap())
            .unwrap();
        txn.apply_update(Update::decode_v1(&delta).unwrap())
            .unwrap();
    }
    let (candidate_document, candidate_artifact, next_revision, next_epoch) = {
        let txn = candidate_doc.transact();
        let fragment = txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();
        let json =
            crate::yrs_engine::YrsDocumentCodec::new(&engine.schema, &engine.resource_limits)
                .read_json(&fragment, &txn)
                .unwrap();
        let document =
            from_prosemirror_json(&json, &engine.schema, UnknownTypeMode::Preserve).unwrap();
        DocumentValidator::validate(&document, &engine.schema, &engine.resource_limits).unwrap();
        let artifact = engine.canonical_schema.derive(&document).unwrap();
        (
            document,
            artifact,
            engine.revision.checked_add(1).unwrap(),
            engine.yrs_state_epoch.checked_add(1).unwrap(),
        )
    };

    reset_prepared_admission_counts_for_test();
    let candidate_seed = {
        let txn = candidate_doc.transact();
        let fragment = txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();
        prepare_history_candidate_capability_for_test(
            65_233,
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
            65_233,
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
    let counts = take_prepared_admission_counts_for_test();

    let candidate_txn = candidate_doc.transact();
    let candidate_fragment = candidate_txn
        .get_xml_fragment(engine.fragment_name.as_str())
        .unwrap();
    assert!(candidate_seed.is_ready_for_test());
    assert!(candidate_seed.matches_canonical_artifact(&candidate_artifact));
    assert!(candidate_seed.matches(
        &candidate_txn,
        &candidate_fragment,
        &candidate_document,
        &engine.resource_limits,
        &engine.editing_limits,
        engine.max_length,
        &engine.schema_fingerprint,
        next_epoch,
        next_revision,
    ));
    let live_txn = engine.doc.transact();
    let live_fragment = live_txn
        .get_xml_fragment(engine.fragment_name.as_str())
        .unwrap();
    assert!(!candidate_seed.matches(
        &live_txn,
        &live_fragment,
        &candidate_document,
        &engine.resource_limits,
        &engine.editing_limits,
        engine.max_length,
        &engine.schema_fingerprint,
        next_epoch,
        next_revision,
    ));
    assert!(Arc::ptr_eq(
        &installed,
        &engine.derived_state.as_ref().unwrap().mutation_lookup_seed
    ));
    assert!(installed.is_unavailable_for_test());
    assert_eq!(counts.staged_seed_preparations, 1);
    assert_eq!(counts.installed_base_seed_publications, 0);
}

#[test]
fn consumed_history_capability_cannot_be_replayed_through_a_general_seed_clone() {
    use crate::yrs_engine::mutation::{
        set_lookup_seed_hydration_failpoint_for_test, LookupSeedHydrationFailpoint,
    };
    use crate::yrs_engine::observability::{
        reset_prepared_admission_counts_for_test, take_prepared_admission_counts_for_test,
    };

    let (engine, candidate_doc, candidate_document, candidate_artifact, next_revision, next_epoch) =
        task5_candidate_publication_fixture();
    let before = atomic_audit(&engine);
    let installed = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
    let txn = candidate_doc.transact();
    let fragment = txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();
    let capability = prepare_history_candidate_capability_for_test(
        65_244,
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
    let general_seed = capability
        .into_unavailable_seed_for_test(65_244)
        .expect("consuming conversion must publish one unavailable general seed");

    reset_prepared_admission_counts_for_test();
    set_lookup_seed_hydration_failpoint_for_test(Some(
        LookupSeedHydrationFailpoint::BindingPublication,
    ));
    let result = general_seed.as_ref().clone().prepare_candidate_publication(
        65_245,
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

    let error = result.expect_err("a general seed clone must not retain the one-shot seal");
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
fn history_capability_rejects_request_relabeling_before_publication_probes() {
    use crate::yrs_engine::mutation::{
        set_lookup_seed_hydration_failpoint_for_test, LookupSeedHydrationFailpoint,
    };
    use crate::yrs_engine::observability::{
        reset_prepared_admission_counts_for_test, take_prepared_admission_counts_for_test,
    };

    for (publish_candidate, failpoint) in [
        (true, LookupSeedHydrationFailpoint::BindingPublication),
        (false, LookupSeedHydrationFailpoint::SeedPublication),
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
        let capability = prepare_history_candidate_capability_for_test(
            65_246,
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
        let result = if publish_candidate {
            capability.prepare_candidate_publication(
                65_247,
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
        } else {
            capability.into_unavailable_seed_for_test(65_247)
        };
        set_lookup_seed_hydration_failpoint_for_test(None);

        let error = result.expect_err("a one-shot history request must not be relabeled");
        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
        assert_eq!(error.request_id, 65_247);
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
fn history_candidate_seed_publication_rejects_contradictory_claims_before_failpoints() {
    use crate::schema::presets::prosemirror_schema;
    use crate::yrs_engine::mutation::{
        set_lookup_seed_hydration_failpoint_for_test, LookupSeedHydrationFailpoint,
    };
    use crate::yrs_engine::observability::{
        reset_prepared_admission_counts_for_test, take_prepared_admission_counts_for_test,
    };

    #[derive(Clone, Copy, Debug)]
    enum Case {
        Document,
        CanonicalArtifact,
        CanonicalIdentity,
        Schema,
        SchemaFingerprint,
        ResourceLimits,
        EditingLimits,
        MaxLength,
        Store,
        Revision,
        Epoch,
        Fragment,
    }

    for case in [
        Case::Document,
        Case::CanonicalArtifact,
        Case::CanonicalIdentity,
        Case::Schema,
        Case::SchemaFingerprint,
        Case::ResourceLimits,
        Case::EditingLimits,
        Case::MaxLength,
        Case::Store,
        Case::Revision,
        Case::Epoch,
        Case::Fragment,
    ] {
        for failpoint in [
            LookupSeedHydrationFailpoint::BindingPublication,
            LookupSeedHydrationFailpoint::SeedPublication,
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
            let installed =
                Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
            let txn = candidate_doc.transact();
            let candidate_fragment = txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();
            let unavailable = prepare_history_candidate_capability_for_test(
                65_236,
                &txn,
                &candidate_fragment,
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
            drop(txn);
            let wrong_fragment = candidate_doc.get_or_insert_xml_fragment("foreign");
            let txn = candidate_doc.transact();
            let candidate_fragment = txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();
            let wrong_document = engine.derived_state.as_ref().unwrap().document.clone();
            let wrong_artifact = engine
                .derived_state
                .as_ref()
                .unwrap()
                .canonical_artifact
                .clone();
            let fresh_same_content_artifact =
                engine.canonical_schema.derive(&candidate_document).unwrap();
            let wrong_schema = prosemirror_schema();
            let mut wrong_resource_limits = engine.resource_limits.clone();
            wrong_resource_limits.max_input_bytes =
                wrong_resource_limits.max_input_bytes.saturating_add(1);
            let mut wrong_editing_limits = engine.editing_limits.clone();
            wrong_editing_limits.max_operations_per_transaction = wrong_editing_limits
                .max_operations_per_transaction
                .saturating_add(1);
            let wrong_max_length = match engine.max_length {
                Some(_) => None,
                None => Some(u32::MAX),
            };
            let foreign_doc =
                super::fresh_utf16_doc_excluding(&engine.durable_client_ids, engine.client_id());
            let foreign_store_fragment =
                foreign_doc.get_or_insert_xml_fragment(engine.fragment_name.as_str());
            let foreign_txn = foreign_doc.transact();
            let source_document = if matches!(case, Case::Document) {
                &wrong_document
            } else {
                &candidate_document
            };
            let canonical_artifact = match case {
                Case::CanonicalArtifact => &wrong_artifact,
                Case::CanonicalIdentity => &fresh_same_content_artifact,
                _ => &candidate_artifact,
            };
            let schema = if matches!(case, Case::Schema) {
                &wrong_schema
            } else {
                &engine.schema
            };
            let resource_limits = if matches!(case, Case::ResourceLimits) {
                &wrong_resource_limits
            } else {
                &engine.resource_limits
            };
            let editing_limits = if matches!(case, Case::EditingLimits) {
                &wrong_editing_limits
            } else {
                &engine.editing_limits
            };
            let max_length = if matches!(case, Case::MaxLength) {
                wrong_max_length
            } else {
                engine.max_length
            };
            let schema_fingerprint = if matches!(case, Case::SchemaFingerprint) {
                "contradictory-schema-fingerprint"
            } else {
                engine.schema_fingerprint.as_str()
            };
            let revision = if matches!(case, Case::Revision) {
                next_revision.saturating_add(1)
            } else {
                next_revision
            };
            let epoch = if matches!(case, Case::Epoch) {
                next_epoch.saturating_add(1)
            } else {
                next_epoch
            };
            let fragment = if matches!(case, Case::Fragment) {
                &wrong_fragment
            } else if matches!(case, Case::Store) {
                &foreign_store_fragment
            } else {
                &candidate_fragment
            };
            reset_prepared_admission_counts_for_test();
            set_lookup_seed_hydration_failpoint_for_test(Some(failpoint));
            let publish_txn = if matches!(case, Case::Store) {
                &foreign_txn
            } else {
                &txn
            };
            let error = publish_history_candidate_seed_for_test(
                unavailable,
                65_236,
                publish_txn,
                fragment,
                schema,
                source_document,
                canonical_artifact,
                resource_limits,
                editing_limits,
                max_length,
                schema_fingerprint,
                epoch,
                revision,
            )
            .expect_err("contradictory history candidate claims must reject before probes");
            set_lookup_seed_hydration_failpoint_for_test(None);
            assert_eq!(
                error.code, "ENGINE_INVARIANT_FAILED",
                "{case:?}/{failpoint:?}"
            );
            let counts = take_prepared_admission_counts_for_test();
            assert_eq!(counts.staged_seed_preparations, 0, "{case:?}/{failpoint:?}");
            assert_eq!(
                counts.installed_base_seed_publications, 0,
                "{case:?}/{failpoint:?}"
            );
            assert_eq!(atomic_audit(&engine), before, "{case:?}/{failpoint:?}");
            assert!(Arc::ptr_eq(
                &installed,
                &engine.derived_state.as_ref().unwrap().mutation_lookup_seed
            ));
        }
    }
}

#[test]
fn history_candidate_seed_publication_rejects_same_store_deletion_after_mint() {
    use crate::yrs_engine::mutation::{
        set_lookup_seed_hydration_failpoint_for_test, LookupSeedHydrationFailpoint,
    };
    use crate::yrs_engine::observability::{
        reset_prepared_admission_counts_for_test, take_prepared_admission_counts_for_test,
    };
    use yrs::types::Text;

    for failpoint in [
        LookupSeedHydrationFailpoint::BindingPublication,
        LookupSeedHydrationFailpoint::SeedPublication,
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
        let (unavailable, text) = {
            let txn = candidate_doc.transact();
            let fragment = txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();
            let unavailable = prepare_history_candidate_capability_for_test(
                65_237,
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
            let XmlOut::Element(paragraph) = fragment.get(&txn, 0).unwrap() else {
                panic!("candidate paragraph missing")
            };
            let XmlOut::Text(text) = paragraph.get(&txn, 0).unwrap() else {
                panic!("candidate text missing")
            };
            (unavailable, text)
        };
        {
            let mut txn = candidate_doc.transact_mut();
            text.remove_range(&mut txn, 0, 1);
        }
        let txn = candidate_doc.transact();
        let fragment = txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();
        reset_prepared_admission_counts_for_test();
        set_lookup_seed_hydration_failpoint_for_test(Some(failpoint));
        let error = publish_history_candidate_seed_for_test(
            unavailable,
            65_237,
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
        .expect_err("same-store deletion after mint must reject before publication probes");
        set_lookup_seed_hydration_failpoint_for_test(None);
        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED", "{failpoint:?}");
        let counts = take_prepared_admission_counts_for_test();
        assert_eq!(counts.staged_seed_preparations, 0, "{failpoint:?}");
        assert_eq!(counts.installed_base_seed_publications, 0, "{failpoint:?}");
        assert_eq!(atomic_audit(&engine), before, "{failpoint:?}");
        assert!(Arc::ptr_eq(
            &installed,
            &engine.derived_state.as_ref().unwrap().mutation_lookup_seed
        ));
    }
}

#[test]
fn history_candidate_read_rejects_a_self_consistent_document_from_another_store_before_probes() {
    use crate::yrs_engine::mutation::{
        set_lookup_seed_hydration_failpoint_for_test, LookupSeedHydrationFailpoint,
    };
    use crate::yrs_engine::observability::{
        reset_prepared_admission_counts_for_test, take_prepared_admission_counts_for_test,
    };

    let (
        engine,
        candidate_doc,
        _candidate_document,
        _candidate_artifact,
        next_revision,
        next_epoch,
    ) = task5_candidate_publication_fixture();
    let before = atomic_audit(&engine);
    let installed = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
    let foreign_state = engine.derived_state.as_ref().unwrap();
    let txn = candidate_doc.transact();
    let fragment = txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();

    reset_prepared_admission_counts_for_test();
    set_lookup_seed_hydration_failpoint_for_test(Some(
        LookupSeedHydrationFailpoint::BindingPublication,
    ));
    let result = crate::yrs_engine::derived_state::prepare_history_candidate_read_for_test(
        65_238,
        &txn,
        &fragment,
        &engine.schema,
        &foreign_state.document,
        &foreign_state.canonical_artifact,
        &engine.resource_limits,
        &engine.editing_limits,
        engine.max_length,
        &engine.schema_fingerprint,
        next_epoch,
        next_revision,
    );
    set_lookup_seed_hydration_failpoint_for_test(None);

    let (_json, admission) = result
        .expect("exact codec read remains available for generic history fallback")
        .into_parts();
    assert!(
        admission.is_none(),
        "a self-consistent document/artifact from another store must not mint history proof"
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

#[test]
fn revision_overflow_rejects_before_candidate_swap() {
    let mut engine =
        crate::yrs_engine::YrsDocumentEngine::new(crate::yrs_engine::YrsEngineConfig {
            schema: tiptap_schema(),
            fragment_name: "prosemirror".into(),
            initialization_mode: crate::yrs_engine::InitializationMode::LocalEmpty,
            resource_limits: ResourceLimits::default(),
            editing_limits: crate::yrs_engine::EditingLimits::default(),
            max_length: None,
            scope: None,
        })
        .unwrap();
    engine.revision = u64::MAX;
    engine.derived_state.as_mut().unwrap().document_revision = u64::MAX;
    let before_client = engine.client_id();
    let before_json = engine.document_json();
    let before_state = engine.encoded_state().unwrap();

    let error = engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"changed"}]}]}"#,
            crate::yrs_engine::TransactionOrigin::DocumentImport,
        )
        .unwrap_err();

    assert_eq!(error.code, "REVISION_OVERFLOW");
    assert_eq!(engine.revision(), u64::MAX);
    assert_eq!(engine.client_id(), before_client);
    assert_eq!(engine.document_json(), before_json);
    assert_eq!(engine.encoded_state().unwrap(), before_state);
}

#[test]
fn candidate_state_revision_and_epoch_overflow_reject_before_swap() {
    for field in ["stateRevision", "yrsStateEpoch"] {
        let mut engine = transaction_engine();
        if field == "stateRevision" {
            engine.state_revision = u64::MAX;
            engine.derived_state.as_mut().unwrap().state_revision = u64::MAX;
        } else {
            engine.yrs_state_epoch = u64::MAX;
        }
        let before = atomic_audit(&engine);

        let error = engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"changed"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap_err();

        assert_eq!(error.code, "REVISION_OVERFLOW", "{field}");
        assert_eq!(error.details, Some(json!({ "field": field })), "{field}");
        assert_eq!(atomic_audit(&engine), before, "{field}");
    }
}

#[test]
fn identical_selection_is_no_op_even_when_state_revision_is_max() {
    let mut engine = transaction_engine();
    engine.state_revision = u64::MAX;
    if let Some(state) = &mut engine.derived_state {
        state.state_revision = u64::MAX;
    }
    let before = atomic_audit(&engine);
    let transaction = TypedTransaction {
        request_id: 90_001,
        base_document_revision: engine.revision(),
        origin: TransactionOrigin::LocalApi,
        operations: vec![],
        selection_intent: SelectionIntent::Set(crate::yrs_engine::SelectionInput::Text {
            anchor: RevisionedPosition {
                offset: 0,
                kind: EditorOffsetKind::Scalar,
                affinity: Affinity::Before,
            },
            head: RevisionedPosition {
                offset: 0,
                kind: EditorOffsetKind::Scalar,
                affinity: Affinity::Before,
            },
        }),
        history_policy: HistoryPolicy::Skip,
    };
    let commit = engine.apply_typed_transaction(transaction).unwrap();
    assert!(!commit.changed);
    assert_eq!(atomic_audit(&engine), before);
}

#[test]
fn snapshot_export_envelope_budget_has_exact_and_over_boundaries_without_mutation() {
    let mut engine =
        crate::yrs_engine::YrsDocumentEngine::new(crate::yrs_engine::YrsEngineConfig {
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
        })
        .unwrap();
    let state = engine.encoded_state().unwrap();
    let metadata_bytes =
        "doc".len() + "lineage".len() + "prosemirror".len() + engine.schema_fingerprint().len();
    engine.resource_limits.max_input_bytes = metadata_bytes;
    engine.resource_limits.max_encoded_state_bytes = state.len();
    assert!(engine.export_snapshot().is_ok());

    let before_revision = engine.revision();
    let before_client = engine.client_id();
    let before_json = engine.document_json();
    engine.resource_limits.max_input_bytes = metadata_bytes - 1;
    let error = engine.export_snapshot().unwrap_err();

    assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
    assert_eq!(
        error.details,
        Some(serde_json::json!({"phase": "snapshotExport"}))
    );
    assert_eq!(engine.revision(), before_revision);
    assert_eq!(engine.client_id(), before_client);
    assert_eq!(engine.document_json(), before_json);
    assert_eq!(engine.encoded_state().unwrap(), state);
}

#[test]
fn typed_transaction_rejects_every_revision_or_epoch_overflow_before_mutation() {
    for field in ["documentRevision", "stateRevision", "yrsStateEpoch"] {
        let mut engine = transaction_engine();
        match field {
            "documentRevision" => {
                engine.revision = u64::MAX;
                engine.derived_state.as_mut().unwrap().document_revision = u64::MAX;
            }
            "stateRevision" => {
                engine.state_revision = u64::MAX;
                engine.derived_state.as_mut().unwrap().state_revision = u64::MAX;
            }
            "yrsStateEpoch" => engine.yrs_state_epoch = u64::MAX,
            _ => unreachable!(),
        }
        let transaction = insert_transaction(&engine, 71);
        let before = atomic_audit(&engine);

        let error = engine.apply_typed_transaction(transaction).unwrap_err();

        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED", "{field}");
        assert_eq!(error.details, Some(json!({ "field": field })), "{field}");
        assert_eq!(atomic_audit(&engine), before, "{field}");
    }
}

#[test]
fn compiled_transaction_epoch_is_checked_before_yrs_metadata_revalidation() {
    for changed in [true, false] {
        let mut engine = transaction_engine();
        let transaction = if changed {
            insert_transaction(&engine, 72)
        } else {
            TypedTransaction {
                request_id: 72,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalApi,
                operations: vec![],
                selection_intent: SelectionIntent::Preserve,
                history_policy: HistoryPolicy::Skip,
            }
        };
        let compiled = engine.compile_typed_transaction(transaction).unwrap();
        engine.yrs_state_epoch += 1;
        let before = atomic_audit(&engine);

        let error = engine
            .apply_compiled_transaction(compiled, false)
            .unwrap_err();

        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED", "changed={changed}");
        assert!(error.message.contains("stale"), "changed={changed}");
        assert_eq!(atomic_audit(&engine), before, "changed={changed}");
    }
}

#[test]
fn compiled_transaction_state_revision_is_checked_before_result_or_no_op_work() {
    for changed in [true, false] {
        let mut engine = transaction_engine();
        let transaction = if changed {
            insert_transaction(&engine, 72_001)
        } else {
            TypedTransaction {
                request_id: 72_001,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalApi,
                operations: vec![],
                selection_intent: SelectionIntent::Preserve,
                history_policy: HistoryPolicy::Skip,
            }
        };
        let compiled = engine.compile_typed_transaction(transaction).unwrap();
        let seed = engine
            .derived_state
            .as_ref()
            .unwrap()
            .mutation_lookup_seed
            .clone();
        engine
            .apply_typed_transaction(TypedTransaction {
                request_id: 72_002,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalApi,
                operations: vec![],
                selection_intent: SelectionIntent::Set(SelectionInput::All),
                history_policy: HistoryPolicy::Skip,
            })
            .unwrap();
        assert!(Arc::ptr_eq(
            &seed,
            &engine.derived_state.as_ref().unwrap().mutation_lookup_seed
        ));
        let before = atomic_audit(&engine);

        let error = engine
            .apply_compiled_transaction(compiled, true)
            .unwrap_err();

        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED", "changed={changed}");
        assert!(error.message.contains("stale"), "changed={changed}");
        assert_eq!(atomic_audit(&engine), before, "changed={changed}");
    }
}

#[test]
fn projected_encoded_ceiling_accepts_exact_and_rejects_one_under_without_new_clock() {
    let mut exact = transaction_engine();
    let exact_transaction = insert_transaction(&exact, 73);
    let exact_compiled = exact
        .compile_typed_transaction(exact_transaction.clone())
        .unwrap();
    let exact_limit = exact
        .encoded_state()
        .unwrap()
        .len()
        .checked_add(exact_compiled.encoded_growth_bound)
        .unwrap();
    exact.resource_limits.max_encoded_state_bytes = exact_limit;

    let commit = exact.apply_typed_transaction(exact_transaction).unwrap();

    assert!(commit.changed);
    assert!(exact.encoded_state().unwrap().len() <= exact_limit);

    let mut one_under = transaction_engine();
    let rejected_transaction = insert_transaction(&one_under, 74);
    let rejected_compiled = one_under
        .compile_typed_transaction(rejected_transaction.clone())
        .unwrap();
    let rejected_limit = one_under
        .encoded_state()
        .unwrap()
        .len()
        .checked_add(rejected_compiled.encoded_growth_bound)
        .unwrap()
        - 1;
    one_under.resource_limits.max_encoded_state_bytes = rejected_limit;
    let before = atomic_audit(&one_under);

    let error = one_under
        .apply_typed_transaction(rejected_transaction)
        .unwrap_err();

    assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
    assert_eq!(
        error.details,
        Some(json!({ "field": "maxEncodedStateBytes" }))
    );
    assert_eq!(error.limit, Some(rejected_limit as u64));
    assert_eq!(error.actual, Some((rejected_limit + 1) as u64));
    assert_eq!(atomic_audit(&one_under), before);
}

#[test]
fn canonical_cache_output_accepts_exact_rejects_one_under_and_reuses_empty_noop_cache() {
    let expected = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "x" }]
        }]
    });
    let exact_bytes = serde_json::to_vec(&expected).unwrap().len();

    let mut exact = transaction_engine();
    exact.editing_limits.max_derived_output_bytes = exact_bytes;
    let transaction = insert_transaction(&exact, 77);
    exact.apply_typed_transaction(transaction).unwrap();
    assert_eq!(exact.document_json(), Some(expected));

    let mut one_under = transaction_engine();
    one_under.editing_limits.max_derived_output_bytes = exact_bytes - 1;
    let transaction = insert_transaction(&one_under, 78);
    let before = atomic_audit(&one_under);
    let error = one_under.apply_typed_transaction(transaction).unwrap_err();
    assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
    assert_eq!(error.limit, Some((exact_bytes - 1) as u64));
    assert_eq!(error.actual, Some(exact_bytes as u64));
    assert_eq!(atomic_audit(&one_under), before);

    let mut empty_noop = transaction_engine();
    empty_noop.editing_limits.max_derived_output_bytes = 1;
    let transaction = TypedTransaction {
        request_id: 79,
        base_document_revision: empty_noop.revision(),
        origin: TransactionOrigin::LocalApi,
        operations: vec![],
        selection_intent: SelectionIntent::Preserve,
        history_policy: HistoryPolicy::Skip,
    };
    let before = atomic_audit(&empty_noop);
    let commit = empty_noop.apply_typed_transaction(transaction).unwrap();
    assert!(!commit.changed);
    assert_eq!(atomic_audit(&empty_noop), before);
}

#[test]
fn local_empty_initialization_enforces_the_exact_canonical_output_ceiling() {
    let schema = tiptap_schema();
    let document = schema.default_document().unwrap();
    let value = crate::serialize::to_prosemirror_json(&document, &schema);
    let exact = serde_json::to_vec(&value).unwrap().len();
    let config = |limit| YrsEngineConfig {
        schema: tiptap_schema(),
        fragment_name: "prosemirror".into(),
        initialization_mode: crate::yrs_engine::InitializationMode::LocalEmpty,
        resource_limits: ResourceLimits::default(),
        editing_limits: crate::yrs_engine::EditingLimits {
            max_derived_output_bytes: limit,
            ..crate::yrs_engine::EditingLimits::default()
        },
        max_length: None,
        scope: None,
    };

    assert_eq!(
        YrsDocumentEngine::new(config(exact))
            .unwrap()
            .document_json(),
        Some(value)
    );
    let error = YrsDocumentEngine::new(config(exact - 1)).err().unwrap();
    assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
    assert_eq!(error.limit, Some(exact - 1));
    assert_eq!(error.actual, Some(exact));
    assert_eq!(
        error.details,
        Some(json!({ "field": "maxDerivedOutputBytes" }))
    );
}

#[test]
fn json_and_html_import_enforce_output_before_any_live_state_change() {
    use crate::yrs_engine::canonical::{
        reset_canonical_artifact_counts_for_test, take_canonical_artifact_counts_for_test,
    };

    let expected = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{"type": "text", "text": "x"}]
        }]
    });
    let exact = serde_json::to_vec(&expected).unwrap().len();
    for (is_html, input) in [
        (false, serde_json::to_string(&expected).unwrap()),
        (true, "<p>x</p>".to_string()),
    ] {
        let mut accepted = transaction_engine();
        accepted.editing_limits.max_derived_output_bytes = exact;
        reset_canonical_artifact_counts_for_test();
        let commit = if is_html {
            accepted.import_html(
                &input,
                &FromHtmlOptions::default(),
                TransactionOrigin::DocumentImport,
            )
        } else {
            accepted.import_json(&input, TransactionOrigin::DocumentImport)
        }
        .unwrap();
        assert!(commit.changed);
        assert_eq!(accepted.document_json(), Some(expected.clone()));
        assert_eq!(
            take_canonical_artifact_counts_for_test(),
            (1, usize::from(is_html))
        );

        let mut rejected = transaction_engine();
        rejected.editing_limits.max_derived_output_bytes = exact - 1;
        rejected.revision = u64::MAX;
        rejected.state_revision = u64::MAX;
        rejected.yrs_state_epoch = u64::MAX;
        rejected.derived_state.as_mut().unwrap().document_revision = u64::MAX;
        rejected.derived_state.as_mut().unwrap().state_revision = u64::MAX;
        let before = atomic_audit(&rejected);
        let artifact_before = rejected
            .derived_state
            .as_ref()
            .unwrap()
            .canonical_artifact
            .clone();
        reset_canonical_artifact_counts_for_test();
        let error = if is_html {
            rejected.import_html(
                &input,
                &FromHtmlOptions::default(),
                TransactionOrigin::DocumentImport,
            )
        } else {
            rejected.import_json(&input, TransactionOrigin::DocumentImport)
        }
        .unwrap_err();
        assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED", "is_html={is_html}");
        assert_eq!(error.limit, Some(exact - 1));
        assert_eq!(error.actual, Some(exact));
        assert_eq!(
            error.details,
            Some(json!({ "field": "maxDerivedOutputBytes" }))
        );
        assert_eq!(atomic_audit(&rejected), before);
        assert!(
            artifact_before.ptr_eq(&rejected.derived_state.as_ref().unwrap().canonical_artifact)
        );
        assert_eq!(take_canonical_artifact_counts_for_test(), (1, 1));
    }
}

#[test]
fn changed_snapshot_restore_enforces_output_before_revisions_history_or_swap() {
    use crate::yrs_engine::canonical::{
        reset_canonical_artifact_counts_for_test, take_canonical_artifact_counts_for_test,
    };

    let mut source = transaction_engine();
    source
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"snapshot"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let snapshot = source.export_snapshot().unwrap();
    let exact = serde_json::to_vec(&source.document_json().unwrap())
        .unwrap()
        .len();

    let mut accepted = transaction_engine();
    accepted.editing_limits.max_derived_output_bytes = exact;
    reset_canonical_artifact_counts_for_test();
    assert!(accepted.restore_snapshot(&snapshot).unwrap().changed);
    assert_eq!(take_canonical_artifact_counts_for_test(), (1, 2));
    accepted.editing_limits.max_derived_output_bytes = 1;
    reset_canonical_artifact_counts_for_test();
    assert!(!accepted.restore_snapshot(&snapshot).unwrap().changed);
    assert_eq!(take_canonical_artifact_counts_for_test(), (0, 0));

    let mut rejected = transaction_engine();
    rejected.editing_limits.max_derived_output_bytes = exact - 1;
    rejected.revision = u64::MAX;
    rejected.state_revision = u64::MAX;
    rejected.yrs_state_epoch = u64::MAX;
    rejected.derived_state.as_mut().unwrap().document_revision = u64::MAX;
    rejected.derived_state.as_mut().unwrap().state_revision = u64::MAX;
    let before = atomic_audit(&rejected);
    let artifact_before = rejected
        .derived_state
        .as_ref()
        .unwrap()
        .canonical_artifact
        .clone();
    reset_canonical_artifact_counts_for_test();
    let error = rejected.restore_snapshot(&snapshot).unwrap_err();
    assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
    assert_eq!(error.limit, Some(exact - 1));
    assert_eq!(error.actual, Some(exact));
    assert_eq!(
        error.details,
        Some(json!({ "field": "maxDerivedOutputBytes" }))
    );
    assert_eq!(atomic_audit(&rejected), before);
    assert!(artifact_before.ptr_eq(&rejected.derived_state.as_ref().unwrap().canonical_artifact));
    assert_eq!(take_canonical_artifact_counts_for_test(), (1, 1));
}

#[test]
fn typed_commit_installs_local_client_origin_and_candidate_revisions() {
    let mut source = transaction_engine();
    let imported = source
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"seed"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    assert!(imported.changed);
    assert_eq!(
        (
            source.revision,
            source.state_revision,
            source.yrs_state_epoch
        ),
        (1, 1, 1)
    );
    let snapshot = source.export_snapshot().unwrap();
    let mut target = transaction_engine();
    target.restore_snapshot(&snapshot).unwrap();
    let local_client = target.client_id();
    assert!(!target.durable_client_ids.contains(&local_client));
    assert_eq!(
        (
            target.revision,
            target.state_revision,
            target.yrs_state_epoch
        ),
        (1, 1, 1)
    );

    let transaction = insert_transaction(&target, 75);
    let commit = target.apply_typed_transaction(transaction).unwrap();

    assert!(commit.changed);
    assert!(target.durable_client_ids.contains(&local_client));
    assert_eq!(
        target.last_committed_origin,
        Some(TransactionOrigin::LocalApi)
    );
    assert_eq!(
        (
            target.revision,
            target.state_revision,
            target.yrs_state_epoch
        ),
        (2, 2, 2)
    );

    let unchanged = target.document_json().unwrap();
    let commit = target
        .import_json(
            &serde_json::to_string(&unchanged).unwrap(),
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    assert!(!commit.changed);
    assert_eq!(
        (
            target.revision,
            target.state_revision,
            target.yrs_state_epoch
        ),
        (2, 2, 2)
    );
}

#[test]
fn restored_deletion_only_commit_does_not_claim_an_unauthored_local_client() {
    let mut source = transaction_engine();
    source
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"seed"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let snapshot = source.export_snapshot().unwrap();
    let mut target = transaction_engine();
    target.restore_snapshot(&snapshot).unwrap();
    let local_client = target.client_id();
    assert!(!target.durable_client_ids.contains(&local_client));
    let from = RevisionedPosition {
        offset: 0,
        kind: EditorOffsetKind::Scalar,
        affinity: Affinity::After,
    };
    let to = RevisionedPosition { offset: 1, ..from };
    let transaction = TypedTransaction {
        request_id: 80,
        base_document_revision: target.revision(),
        origin: TransactionOrigin::LocalApi,
        operations: vec![TypedOperation::DeleteRange {
            range: RevisionedRange { from, to },
        }],
        selection_intent: SelectionIntent::Preserve,
        history_policy: HistoryPolicy::Skip,
    };

    let compiled = target
        .compile_typed_transaction(transaction.clone())
        .unwrap();
    assert_eq!(compiled.authored_clock_units, 0);
    target.apply_typed_transaction(transaction).unwrap();

    assert_prepared_candidate_state_vector_exact(&target);
    assert!(!target.durable_client_ids.contains(&local_client));
    let durable_clients = Update::decode_v1(&target.encoded_state().unwrap())
        .unwrap()
        .state_vector();
    assert!(durable_clients.get(&ClientID::new(local_client)) == 0);
}

#[test]
fn restored_format_only_commit_records_its_authored_local_clock() {
    let mut source = transaction_engine();
    source
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"seed"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let snapshot = source.export_snapshot().unwrap();
    let mut target = transaction_engine();
    target.restore_snapshot(&snapshot).unwrap();
    let local_client = target.client_id();
    let from = RevisionedPosition {
        offset: 0,
        kind: EditorOffsetKind::Scalar,
        affinity: Affinity::After,
    };
    let to = RevisionedPosition { offset: 1, ..from };
    let transaction = TypedTransaction {
        request_id: 81,
        base_document_revision: target.revision(),
        origin: TransactionOrigin::LocalApi,
        operations: vec![TypedOperation::AddMark {
            range: RevisionedRange { from, to },
            mark: Mark::new("bold".into(), HashMap::new()),
        }],
        selection_intent: SelectionIntent::Preserve,
        history_policy: HistoryPolicy::Skip,
    };

    let compiled = target
        .compile_typed_transaction(transaction.clone())
        .unwrap();
    assert!(compiled.authored_clock_units > 0);
    target.apply_typed_transaction(transaction).unwrap();

    assert_prepared_candidate_state_vector_exact(&target);
    assert!(target.durable_client_ids.contains(&local_client));
    let durable_clients = Update::decode_v1(&target.encoded_state().unwrap())
        .unwrap()
        .state_vector();
    assert!(durable_clients.get(&ClientID::new(local_client)) > 0);
}
