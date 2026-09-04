use super::*;

#[test]
fn remote_history_admission_failure_retains_dependency_quarantine_for_retry() {
    use crate::yrs_engine::compiler::{set_atomic_failpoint_for_test, AtomicFailpoint};
    use yrs::{diff_updates_v1, encode_state_vector_from_update_v1};

    let mut source = transaction_engine();
    let base = source.encoded_state().unwrap();
    source
        .apply_command(200, TypedCommand::InsertText { text: "a".into() })
        .unwrap();
    let after_a = source.encoded_state().unwrap();
    source
        .apply_command(201, TypedCommand::InsertText { text: "b".into() })
        .unwrap();
    let after_b = source.encoded_state().unwrap();
    let base_sv = encode_state_vector_from_update_v1(&base).unwrap();
    let after_a_sv = encode_state_vector_from_update_v1(&after_a).unwrap();
    let delta_a = diff_updates_v1(&after_a, &base_sv).unwrap();
    let delta_b = diff_updates_v1(&after_b, &after_a_sv).unwrap();

    let mut target = YrsDocumentEngine::new(YrsEngineConfig {
        schema: tiptap_schema(),
        fragment_name: "prosemirror".into(),
        initialization_mode: crate::yrs_engine::InitializationMode::AwaitRemote,
        resource_limits: ResourceLimits::default(),
        editing_limits: crate::yrs_engine::EditingLimits::default(),
        max_length: None,
        scope: Some(crate::yrs_engine::DocumentScope {
            document_id: "doc".into(),
            lineage_id: "lineage".into(),
        }),
    })
    .unwrap();
    assert!(
        !target
            .apply_remote_update_v1(202, &delta_b)
            .unwrap()
            .changed
    );
    assert!(
        !target
            .apply_remote_update_v1(203, &delta_a)
            .unwrap()
            .changed
    );
    let before = atomic_audit(&target);

    set_atomic_failpoint_for_test(Some(AtomicFailpoint::RemoteHistoryAdmission));
    let error = target.apply_remote_update_v1(204, &base).unwrap_err();
    set_atomic_failpoint_for_test(None);

    assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
    assert_eq!(
        error.details,
        Some(json!({ "failpoint": "remoteHistoryAdmission" }))
    );
    assert_eq!(atomic_audit(&target), before);
    let retry = target.apply_remote_update_v1(205, &base).unwrap();
    assert!(retry.changed);
    assert_eq!(target.document().unwrap().root().text_content(), "ab");
    assert_eq!(target.encoded_state().unwrap(), after_b);
}

/// Task 9 classification seam: the read-only preflight accepts exactly
/// what the prepare pipeline's ingress admission accepts, rejects
/// malformed encodings with the same structured errors, and never
/// touches engine state.
#[test]
fn preflight_remote_update_v1_classifies_encoding_without_engine_effects() {
    let mut source = transaction_engine();
    source
        .apply_command(210, TypedCommand::InsertText { text: "pf".into() })
        .unwrap();
    let valid = source.encoded_state().unwrap();
    let engine = transaction_engine();
    let before = atomic_audit(&engine);

    engine.preflight_remote_update_v1(211, &valid).unwrap();
    engine.preflight_remote_update_v1(212, &[0, 0]).unwrap();

    let error = engine
        .preflight_remote_update_v1(213, &[0xff, 0xff, 0xff])
        .unwrap_err();
    assert_eq!(error.code, "DOCUMENT_INVALID");
    assert_eq!(error.request_id, 213);

    let mut truncated = valid.clone();
    truncated.truncate(valid.len() / 2);
    assert!(engine.preflight_remote_update_v1(214, &truncated).is_err());

    assert_eq!(atomic_audit(&engine), before);
}

/// Task 9 accounting seam: the engine reports its retained
/// dependency-quarantine bytes (the exact pending payload length) and
/// returns to zero once the dependency completes.
#[test]
fn pending_remote_dependency_bytes_tracks_the_quarantine_lifecycle() {
    use yrs::{diff_updates_v1, encode_state_vector_from_update_v1};

    let mut source = transaction_engine();
    let base = source.encoded_state().unwrap();
    source
        .apply_command(220, TypedCommand::InsertText { text: "q".into() })
        .unwrap();
    let after = source.encoded_state().unwrap();
    let base_sv = encode_state_vector_from_update_v1(&base).unwrap();
    let delta = diff_updates_v1(&after, &base_sv).unwrap();

    let mut target = transaction_engine();
    assert_eq!(target.pending_remote_dependency_bytes(), 0);

    // transaction_engine() starts from a different lineage than
    // `source`, so the delta's dependencies are missing and quarantine.
    assert!(!target.apply_remote_update_v1(221, &delta).unwrap().changed);
    assert_eq!(target.pending_remote_dependency_bytes(), delta.len());

    assert!(target.apply_remote_update_v1(222, &base).unwrap().changed);
    assert_eq!(target.pending_remote_dependency_bytes(), 0);
    assert_eq!(target.document().unwrap().root().text_content(), "q");
}

#[test]
fn state_only_boundary_reservation_failure_is_fully_atomic() {
    use crate::yrs_engine::history::set_boundary_reservation_failure_for_test;

    let mut engine = transaction_engine();
    let before = atomic_audit(&engine);
    set_boundary_reservation_failure_for_test(true);

    let error = engine
        .apply_command(
            90,
            crate::yrs_engine::TypedCommand::ToggleMark {
                mark_type: "bold".into(),
            },
        )
        .unwrap_err();

    set_boundary_reservation_failure_for_test(false);
    assert_eq!(error.code, "OPERATION_RESOURCE_EXHAUSTED");
    assert_eq!(atomic_audit(&engine), before);
}

/// Task 16B: the quarantined remote-update reservation is a demonstrated
/// fallible allocation seam and keeps OPERATION_RESOURCE_EXHAUSTED.
#[test]
fn quarantined_remote_update_reservation_failure_keeps_resource_exhausted() {
    use yrs::{diff_updates_v1, encode_state_vector_from_update_v1};

    let mut source = transaction_engine();
    let base = source.encoded_state().unwrap();
    source
        .apply_command(220, TypedCommand::InsertText { text: "q".into() })
        .unwrap();
    let after = source.encoded_state().unwrap();
    let base_sv = encode_state_vector_from_update_v1(&base).unwrap();
    let delta = diff_updates_v1(&after, &base_sv).unwrap();

    let mut target = transaction_engine();
    let before = atomic_audit(&target);
    super::set_quarantined_update_reservation_failure_for_test(true);
    let error = target.apply_remote_update_v1(221, &delta).unwrap_err();
    super::set_quarantined_update_reservation_failure_for_test(false);
    assert_eq!(error.code, "OPERATION_RESOURCE_EXHAUSTED");
    assert_eq!(error.details, Some(json!({ "field": "remoteUpdate" })));
    assert_eq!(atomic_audit(&target), before);
    // Recovery: the identical update quarantines once allocation recovers.
    assert!(!target.apply_remote_update_v1(221, &delta).unwrap().changed);
}

/// Task 16B: the outbound staging-copy allocation seam keeps
/// OPERATION_RESOURCE_EXHAUSTED.
#[test]
fn outbound_staging_copy_allocation_failure_keeps_resource_exhausted() {
    let limits = crate::session::CollaborationLimits::default();
    let mut outbox = crate::collaboration_runtime::CollaborationOutbox::from_limits(&limits);
    let mut sink = OutboundUpdateSink::attached(&mut outbox);
    super::set_outbound_staging_copy_failure_for_test(true);
    let error = sink.reserve_and_stage(41, 4, &[1, 2, 3]).unwrap_err();
    super::set_outbound_staging_copy_failure_for_test(false);
    assert_eq!(error.code, "OPERATION_RESOURCE_EXHAUSTED");
    assert_eq!(
        error.details,
        Some(json!({ "field": "pendingOutboxUpdateBytes" }))
    );
    sink.reserve_and_stage(41, 4, &[1, 2, 3]).unwrap();
}

/// Task 6 fix round 1: exact/one-over coverage of the shared
/// `maxEncodedStateBytes` gate used by the remote pipeline and the sealed
/// state-vector/diff encoders. The state-vector *output* branch is
/// unreachable through any consistent engine (the full encoded state is
/// strictly larger than its state vector and is bounded by the same
/// ceiling on every admission path), so the gate is proven here at the
/// boundary instead.
#[test]
fn max_encoded_state_gate_admits_exact_and_rejects_one_over() {
    assert!(super::admit_max_encoded_state_len(90_001, 64, 64).is_ok());
    assert!(super::admit_max_encoded_state_len(90_002, 0, 0).is_ok());

    let error = super::admit_max_encoded_state_len(90_003, 65, 64).unwrap_err();
    assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
    assert_eq!(error.request_id, 90_003);
    assert_eq!(error.limit, Some(64));
    assert_eq!(error.actual, Some(65));
    assert_eq!(
        error.details.as_ref().unwrap()["field"],
        "maxEncodedStateBytes"
    );
}

/// Task 6 same-doc binding proof: the codec's sole `Awareness` wraps the
/// live authoritative `Doc` handle (documents edits are visible through
/// it, the client identity matches), and the binding follows every store
/// swap (undo/redo candidate installation and import).
#[test]
fn awareness_codec_owns_an_awareness_bound_to_the_live_doc() {
    use yrs::GetString;

    fn bound_fragment_text(engine: &YrsDocumentEngine) -> String {
        let codec = engine.awareness.as_ref().expect("codec stays bound");
        let doc = codec.doc_for_test();
        assert!(
            Doc::ptr_eq(doc, &engine.doc),
            "awareness must wrap the live authoritative doc handle"
        );
        assert_eq!(doc.client_id().get(), engine.client_id());
        let txn = doc.transact();
        txn.get_xml_fragment("prosemirror")
            .expect("live doc retains the document fragment")
            .get_string(&txn)
    }

    let mut engine =
        transaction_engine_with_editing_limits(crate::yrs_engine::EditingLimits::default());
    engine.awareness();

    engine
        .apply_command(
            1,
            TypedCommand::InsertText {
                text: "bound".into(),
            },
        )
        .unwrap()
        .expect("insert applies");
    assert!(bound_fragment_text(&engine).contains("bound"));

    engine.undo(2).unwrap().expect("undo applies");
    assert!(!bound_fragment_text(&engine).contains("bound"));

    engine
        .import_json(
            &json!({"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"imported"}]}]})
                .to_string(),
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    assert!(bound_fragment_text(&engine).contains("imported"));
}
