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

include!("candidate_publication/store_rebinding.rs");

include!("candidate_publication/deferred_admission.rs");

include!("candidate_publication/revision_and_output.rs");
