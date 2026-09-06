#[test]
fn bounded_dependencies_stay_quarantined_inside_the_engine() {
    let (prefix, delta_b) = dependent_room_updates();
    let (id, snapshot) = create_ready_room();
    let generation = synchronize_ready_room(id, &snapshot);
    let before_json = session_audit(id).unwrap().document_json.unwrap();
    assert_eq!(remote_dependency_accounting(id).unwrap(), (0, 0));

    // The dependency-pending update stays quarantined inside the engine; the
    // runtime holds only byte/work accounting, never a second payload copy.
    let outcome = receive_message(id, 331, generation, &update_frame(delta_b.clone())).unwrap();
    assert!(outcome.close.is_none(), "{outcome:?}");
    assert!(!outcome.remote_commit_applied, "{outcome:?}");
    let (retained_bytes, retained_work) = remote_dependency_accounting(id).unwrap();
    assert_eq!(retained_bytes, delta_b.len());
    assert_eq!(retained_work, delta_b.len() as u64);
    assert_eq!(
        session_audit(id).unwrap().document_json.unwrap(),
        before_json
    );
    assert_eq!(transport_state(id).unwrap(), TransportState::Synchronized);

    // Completing the dependency converges and clears the accounting.
    let outcome = receive_message(id, 332, generation, &update_frame(prefix)).unwrap();
    assert!(outcome.close.is_none(), "{outcome:?}");
    assert!(outcome.remote_commit_applied, "{outcome:?}");
    assert_eq!(remote_dependency_accounting(id).unwrap(), (0, 0));
    let json = session_audit(id)
        .unwrap()
        .document_json
        .unwrap()
        .to_string();
    assert!(json.contains("ab"), "{json}");

    destroy_session(id);
}

#[test]
fn first_one_over_dependency_update_is_rejected_before_commit() {
    let (_prefix, incoming) = dependent_room_updates();
    let (id, snapshot) = create_ready_room();
    let generation = synchronize_ready_room(id, &snapshot);
    let before_session = session_audit(id).unwrap();
    let before_engine = bridge::session_audit(id).unwrap();
    let before_dependencies = remote_dependency_accounting(id).unwrap();
    assert_eq!(before_dependencies, (0, 0));

    set_collaboration_limit_for_test(id, "maxPendingDependencyUpdateBytes", incoming.len() - 1)
        .unwrap();
    let outcome = receive_message(id, 333, generation, &update_frame(incoming)).unwrap();
    let close = outcome
        .close
        .as_ref()
        .expect("the first one-over dependency candidate must close");
    assert_eq!(
        close.disposition,
        CloseDisposition::Incompatible,
        "{close:?}"
    );
    assert_eq!(
        close.error.code, TRANSPORT_DEPENDENCY_LIMIT_EXCEEDED,
        "{close:?}"
    );
    assert_eq!(
        close.error.details.as_ref().unwrap()["field"],
        "maxPendingDependencyUpdateBytes",
    );
    assert!(!outcome.remote_commit_applied, "{outcome:?}");

    let mut expected_session = before_session;
    expected_session.transport_state = TransportState::Incompatible;
    assert_eq!(session_audit(id).unwrap(), expected_session);
    assert_eq!(
        bridge::session_audit(id).unwrap(),
        before_engine,
        "canonical JSON, encoded state, revisions, history, selection, and outbox must be unchanged",
    );
    assert_eq!(
        remote_dependency_accounting(id).unwrap(),
        before_dependencies,
        "the refused candidate must not rewrite quarantine or charge work",
    );
    destroy_session(id);
}

#[test]
fn recovery_update_is_judged_by_drained_candidate_state() {
    let (recovery, incoming) = dependent_room_updates();
    let (id, snapshot) = create_ready_room();
    let generation = synchronize_ready_room(id, &snapshot);

    let outcome = receive_message(id, 334, generation, &update_frame(incoming.clone())).unwrap();
    assert!(outcome.close.is_none(), "{outcome:?}");
    assert_eq!(
        remote_dependency_accounting(id).unwrap(),
        (incoming.len(), incoming.len() as u64),
    );

    // The candidate drains the quarantine, so neither retained bytes nor
    // accumulated pending work may be charged for this recovery update.
    set_collaboration_limit_for_test(id, "maxPendingDependencyUpdateBytes", 0).unwrap();
    set_collaboration_limit_for_test(id, "maxPendingDependencyUpdateWork", 0).unwrap();
    let outcome = receive_message(id, 335, generation, &update_frame(recovery)).unwrap();
    assert!(outcome.close.is_none(), "{outcome:?}");
    assert!(outcome.remote_commit_applied, "{outcome:?}");
    assert_eq!(remote_dependency_accounting(id).unwrap(), (0, 0));
    let json = session_audit(id)
        .unwrap()
        .document_json
        .unwrap()
        .to_string();
    assert!(json.contains("ab"), "{json}");
    destroy_session(id);
}

#[test]
fn rejected_dependency_work_never_ratchets_or_rewrites_quarantine() {
    let (recovery, delta_b, delta_c) = dependent_room_update_chain();
    let (id, snapshot) = create_ready_room();
    let generation = synchronize_ready_room(id, &snapshot);

    let outcome = receive_message(id, 336, generation, &update_frame(delta_b.clone())).unwrap();
    assert!(outcome.close.is_none(), "{outcome:?}");
    let before_dependencies = remote_dependency_accounting(id).unwrap();
    assert_eq!(before_dependencies, (delta_b.len(), delta_b.len() as u64));
    set_collaboration_limit_for_test(id, "maxPendingDependencyUpdateWork", delta_b.len()).unwrap();
    let before_session = session_audit(id).unwrap();
    let before_engine = bridge::session_audit(id).unwrap();

    let outcome = receive_message(id, 337, generation, &update_frame(delta_c)).unwrap();
    let close = outcome
        .close
        .as_ref()
        .expect("one-over candidate work must close");
    assert_eq!(
        close.disposition,
        CloseDisposition::Incompatible,
        "{close:?}"
    );
    assert_eq!(
        close.error.code, TRANSPORT_DEPENDENCY_LIMIT_EXCEEDED,
        "{close:?}"
    );
    assert_eq!(
        close.error.details.as_ref().unwrap()["field"],
        "maxPendingDependencyUpdateWork",
    );
    assert!(!outcome.remote_commit_applied, "{outcome:?}");

    let mut expected_session = before_session;
    expected_session.transport_state = TransportState::Incompatible;
    assert_eq!(session_audit(id).unwrap(), expected_session);
    assert_eq!(bridge::session_audit(id).unwrap(), before_engine);
    assert_eq!(
        remote_dependency_accounting(id).unwrap(),
        before_dependencies,
        "the refusal must preserve retained bytes and accumulated work",
    );

    // Probe the preserved quarantine directly: recovery may publish `b`,
    // but the refused `c` must not have been installed into the candidate.
    let slot = crate::registry::get_session(id).expect("session must remain registered");
    slot.with_alive(|session| {
        let prepared = session
            .engine
            .prepare_remote_update_v1(338, &recovery)
            .unwrap();
        assert!(!prepared.has_pending_dependencies());
        session
            .engine
            .commit_prepared_remote_update(prepared)
            .unwrap();
        let json = session.engine.document_json().unwrap().to_string();
        assert!(json.contains("ab"), "{json}");
        assert!(!json.contains("abc"), "{json}");
    })
    .unwrap();
    destroy_session(id);
}

#[test]
fn dependency_byte_and_work_ceilings_close_as_incompatible() {
    let (_prefix, delta_b) = dependent_room_updates();
    let quarantined_len = delta_b.len();

    // Byte ceiling: exact retained size passes.
    let (id, snapshot) = create_ready_room();
    let generation = synchronize_ready_room(id, &snapshot);
    set_collaboration_limit_for_test(id, "maxPendingDependencyUpdateBytes", quarantined_len)
        .unwrap();
    let outcome = receive_message(id, 341, generation, &update_frame(delta_b.clone())).unwrap();
    assert!(outcome.close.is_none(), "{outcome:?}");
    destroy_session(id);

    // Byte ceiling: one under the retained size closes as incompatible.
    let (id, snapshot) = create_ready_room();
    let generation = synchronize_ready_room(id, &snapshot);
    set_collaboration_limit_for_test(id, "maxPendingDependencyUpdateBytes", quarantined_len - 1)
        .unwrap();
    let outcome = receive_message(id, 342, generation, &update_frame(delta_b.clone())).unwrap();
    let close = outcome
        .close
        .as_ref()
        .expect("dependency byte overflow must close");
    assert_eq!(
        close.disposition,
        CloseDisposition::Incompatible,
        "{close:?}"
    );
    assert_eq!(
        close.error.code, TRANSPORT_DEPENDENCY_LIMIT_EXCEEDED,
        "{close:?}"
    );
    assert_eq!(
        close.error.details.as_ref().unwrap()["field"],
        "maxPendingDependencyUpdateBytes",
    );
    assert_eq!(transport_state(id).unwrap(), TransportState::Incompatible);
    destroy_session(id);

    // Work ceiling: work accumulates across quarantined admissions even when
    // the merged retained bytes do not grow, and closes one over the limit.
    let (id, snapshot) = create_ready_room();
    let generation = synchronize_ready_room(id, &snapshot);
    set_collaboration_limit_for_test(
        id,
        "maxPendingDependencyUpdateWork",
        2 * quarantined_len - 1,
    )
    .unwrap();
    let outcome = receive_message(id, 343, generation, &update_frame(delta_b.clone())).unwrap();
    assert!(outcome.close.is_none(), "{outcome:?}");
    let outcome = receive_message(id, 344, generation, &update_frame(delta_b.clone())).unwrap();
    let close = outcome
        .close
        .as_ref()
        .expect("dependency work overflow must close");
    assert_eq!(
        close.disposition,
        CloseDisposition::Incompatible,
        "{close:?}"
    );
    assert_eq!(
        close.error.code, TRANSPORT_DEPENDENCY_LIMIT_EXCEEDED,
        "{close:?}"
    );
    assert_eq!(
        close.error.details.as_ref().unwrap()["field"],
        "maxPendingDependencyUpdateWork",
    );
    destroy_session(id);

    // Work ceiling: the exact accumulated work passes.
    let (id, snapshot) = create_ready_room();
    let generation = synchronize_ready_room(id, &snapshot);
    set_collaboration_limit_for_test(id, "maxPendingDependencyUpdateWork", 2 * quarantined_len)
        .unwrap();
    receive_message(id, 345, generation, &update_frame(delta_b.clone())).unwrap();
    let outcome = receive_message(id, 346, generation, &update_frame(delta_b)).unwrap();
    assert!(outcome.close.is_none(), "{outcome:?}");
    destroy_session(id);
}

#[test]
fn permanently_inadmissible_remote_state_preserves_the_engine_audit() {
    let mut foreign = YrsDocumentEngine::new(YrsEngineConfig {
        schema: incompatible_blockquote_schema(),
        fragment_name: FRAGMENT_NAME.into(),
        initialization_mode: InitializationMode::LocalEmpty,
        resource_limits: ResourceLimits::default(),
        editing_limits: EditingLimits::default(),
        max_length: None,
        scope: None,
    })
    .unwrap();
    foreign
        .import_json(
            &serde_json::json!({"type":"doc","content":[{"type":"blockquote","content":[{"type":"text","text":"invalid in target"}]}]}).to_string(),
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let invalid_update = foreign.encoded_state().unwrap();

    let (id, snapshot) = create_ready_room();
    let generation = synchronize_ready_room(id, &snapshot);
    let before = session_audit(id).unwrap();
    let before_engine = bridge::session_audit(id).unwrap();

    let outcome = receive_message(id, 351, generation, &update_frame(invalid_update)).unwrap();
    let close = outcome
        .close
        .expect("schema-invalid remote state must close the generation");
    assert_eq!(
        close.disposition,
        CloseDisposition::Incompatible,
        "{close:?}"
    );
    assert_eq!(close.error.code, TRANSPORT_REMOTE_INADMISSIBLE, "{close:?}");
    let details = close.error.details.as_ref().unwrap();
    assert_eq!(details["cause"]["code"], "DOCUMENT_INVALID", "{details}");

    let mut expected = before.clone();
    expected.transport_state = TransportState::Incompatible;
    assert_eq!(session_audit(id).unwrap(), expected);
    assert_eq!(bridge::session_audit(id).unwrap(), before_engine);

    destroy_session(id);
}

#[test]
fn local_operation_errors_do_not_disconnect_a_synchronized_transport() {
    let (id, snapshot) = create_ready_room();
    let generation = synchronize_ready_room(id, &snapshot);

    // A failing local edit (stale base revision) is an operation error and
    // must leave the healthy transport untouched.
    let stale_envelope = serde_json::json!({
        "version": 1,
        "requestId": "361",
        "baseDocumentRevision": "999999",
        "text": "stale local edit",
    })
    .to_string();
    let error = bridge::submit_input(id, &stale_envelope).unwrap_err();
    assert_eq!(error.code, "REVISION_MISMATCH", "{error:?}");
    assert_eq!(transport_state(id).unwrap(), TransportState::Synchronized);

    // The generation stays live: the next frame is still accepted.
    let outcome =
        receive_message(id, 362, generation, &update_frame(NOOP_UPDATE_V1.to_vec())).unwrap();
    assert!(outcome.close.is_none(), "{outcome:?}");
    assert_eq!(transport_state(id).unwrap(), TransportState::Synchronized);

    destroy_session(id);
}

#[test]
fn remote_commits_are_never_echoed_and_local_edits_still_enqueue_once() {
    let (id, snapshot) = create_ready_room();
    let generation = synchronize_ready_room(id, &snapshot);
    assert_eq!(bridge::outbox_pending(id).unwrap().unwrap(), (0, 0));

    // A committed remote update produces no outbox document entry.
    let server = RawPeer::from_snapshot(&snapshot);
    server.push_text(" no echo");
    let outcome = receive_message(
        id,
        371,
        generation,
        &update_frame(server.diff_for(&session_state_vector_bytes(id))),
    )
    .unwrap();
    assert!(outcome.remote_commit_applied, "{outcome:?}");
    assert_eq!(
        bridge::outbox_pending(id).unwrap().unwrap(),
        (0, 0),
        "remote commits must never be echoed as document updates",
    );
    assert!(lease_outbound(id, 373, generation).unwrap().is_none());

    // A local edit while Synchronized still enqueues exactly one bounded
    // document update, and the two paths coexist.
    local_edit(id, 372, "local while synchronized");
    let (pending_count, pending_bytes) = bridge::outbox_pending(id).unwrap().unwrap();
    assert_eq!(pending_count, 1);
    assert!(pending_bytes > 0);
    let lease = bridge::lease_next_update(id).unwrap().unwrap();
    let lease_id = lease.lease_id;
    assert_eq!(lease.request_id, 372);
    let update = lease.update_v1;

    // The captured update converges the independent peer.
    server.apply(&update);
    assert_eq!(server.state_vector(), session_state_vector(id));
    bridge::ack_leased_update(id, lease_id).unwrap();
    assert_eq!(bridge::outbox_pending(id).unwrap().unwrap(), (0, 0));

    destroy_session(id);
}

#[test]
fn update_exchange_converges_with_an_independent_raw_peer() {
    let (id, snapshot) = create_ready_room();
    let generation = synchronize_ready_room(id, &snapshot);
    let server = RawPeer::from_snapshot(&snapshot);

    // Their edit -> our runtime.
    server.push_text(" from peer");
    let outcome = receive_message(
        id,
        381,
        generation,
        &update_frame(server.diff_for(&session_state_vector_bytes(id))),
    )
    .unwrap();
    assert!(outcome.remote_commit_applied, "{outcome:?}");

    // Our edit -> their doc.
    local_edit(id, 382, " from us");
    let lease = bridge::lease_next_update(id).unwrap().unwrap();
    let lease_id = lease.lease_id;
    let update = lease.update_v1;
    server.apply(&update);
    bridge::ack_leased_update(id, lease_id).unwrap();

    // Both directions converge to state-vector equality.
    assert_eq!(server.state_vector(), session_state_vector(id));
    let json = session_audit(id)
        .unwrap()
        .document_json
        .unwrap()
        .to_string();
    assert!(json.contains("from peer"), "{json}");

    destroy_session(id);
}

// Dependency-quarantine byte/work ceilings are charged from the prepared
// post-state before commit can mutate the live quarantine.

/// `(prefix, delta_b, delta_c)`: both deltas depend on content only present
/// in earlier states, so a receiver holding the seed must quarantine each
/// until the prefix (and for `delta_c`, also `delta_b`) arrives.
fn dependent_room_update_chain() -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let mut source = source_engine();
    source
        .apply_command(101, TypedCommand::InsertText { text: "a".into() })
        .unwrap();
    let after_a = source.encoded_state().unwrap();
    source
        .apply_command(102, TypedCommand::InsertText { text: "b".into() })
        .unwrap();
    let after_b = source.encoded_state().unwrap();
    source
        .apply_command(103, TypedCommand::InsertText { text: "c".into() })
        .unwrap();
    let after_c = source.encoded_state().unwrap();
    let after_a_sv = encode_state_vector_from_update_v1(&after_a).unwrap();
    let after_b_sv = encode_state_vector_from_update_v1(&after_b).unwrap();
    let delta_b = diff_updates_v1(&after_b, &after_a_sv).unwrap();
    let delta_c = diff_updates_v1(&after_c, &after_b_sv).unwrap();
    (after_a, delta_b, delta_c)
}

#[test]
fn prepared_remote_update_drop_is_observationally_pure() {
    let (_, delta_b, _) = dependent_room_update_chain();
    let (id, snapshot) = create_ready_room();
    synchronize_ready_room(id, &snapshot);

    let before_audit = session_audit(id).unwrap();
    let before_dependencies = remote_dependency_accounting(id).unwrap();
    let before_encoded = bridge::session_audit(id).unwrap().encoded_state;

    let slot = crate::registry::get_session(id).expect("session must remain registered");
    slot.with_alive(|session| {
        let prepared = session
            .engine
            .prepare_remote_update_v1(360, &delta_b)
            .unwrap();
        assert_eq!(prepared.retained_dependency_bytes(), delta_b.len());
        assert!(prepared.has_pending_dependencies());
        drop(prepared);
    })
    .unwrap();

    assert_eq!(session_audit(id).unwrap(), before_audit);
    assert_eq!(
        remote_dependency_accounting(id).unwrap(),
        before_dependencies
    );
    assert_eq!(
        bridge::session_audit(id).unwrap().encoded_state,
        before_encoded
    );
    destroy_session(id);
}

#[test]
fn dependency_byte_ceiling_refuses_before_any_quarantine_mutation() {
    let (prefix, delta_b, delta_c) = dependent_room_update_chain();
    let retained = delta_b.len();

    // One over: the exact merged candidate crosses the retained-byte
    // ceiling. The refusal must leave the quarantine byte-identical and the
    // work counter untouched — candidate admission, not a post-commit
    // apology.
    let (id, snapshot) = create_ready_room();
    let generation = synchronize_ready_room(id, &snapshot);
    let outcome = receive_message(id, 361, generation, &update_frame(delta_b.clone())).unwrap();
    assert!(outcome.close.is_none(), "{outcome:?}");
    assert_eq!(
        remote_dependency_accounting(id).unwrap(),
        (retained, retained as u64),
    );
    set_collaboration_limit_for_test(id, "maxPendingDependencyUpdateBytes", retained).unwrap();
    let before = session_audit(id).unwrap();
    let before_engine = bridge::session_audit(id).unwrap();
    let expected_retained = Update::merge_updates(vec![
        Update::decode_v1(&delta_b).unwrap(),
        Update::decode_v1(&delta_c).unwrap(),
    ])
    .encode_v1()
    .len();
    let slot = crate::registry::get_session(id).expect("session must remain registered");
    slot.with_alive(|session| {
        let before_bytes = session.engine.pending_remote_dependency_bytes();
        let prepared = session
            .engine
            .prepare_remote_update_v1(362, &delta_c)
            .unwrap();
        assert_eq!(prepared.retained_dependency_bytes(), expected_retained);
        assert!(prepared.has_pending_dependencies());
        drop(prepared);
        assert_eq!(
            session.engine.pending_remote_dependency_bytes(),
            before_bytes
        );
    })
    .unwrap();

    let outcome = receive_message(id, 362, generation, &update_frame(delta_c.clone())).unwrap();
    let close = outcome
        .close
        .as_ref()
        .expect("one over the candidate byte ceiling must close");
    assert_eq!(
        close.disposition,
        CloseDisposition::Incompatible,
        "{close:?}"
    );
    assert_eq!(
        close.error.code, TRANSPORT_DEPENDENCY_LIMIT_EXCEEDED,
        "{close:?}"
    );
    assert_eq!(
        close.error.details.as_ref().unwrap()["field"],
        "maxPendingDependencyUpdateBytes",
    );
    // Zero mutation: retained bytes and charged work are exactly the
    // pre-refusal figures, and the full audits match (the deliberate
    // generation close aside).
    assert_eq!(
        remote_dependency_accounting(id).unwrap(),
        (retained, retained as u64),
        "the refused payload was never retained or charged",
    );
    let mut expected = before.clone();
    expected.transport_state = TransportState::Incompatible;
    assert_eq!(session_audit(id).unwrap(), expected);
    assert_eq!(bridge::session_audit(id).unwrap(), before_engine);
    destroy_session(id);

    // Exact: the prepared candidate at the ceiling admits.
    let (id, snapshot) = create_ready_room();
    let generation = synchronize_ready_room(id, &snapshot);
    receive_message(id, 363, generation, &update_frame(delta_b.clone())).unwrap();
    set_collaboration_limit_for_test(id, "maxPendingDependencyUpdateBytes", expected_retained)
        .unwrap();
    let outcome = receive_message(id, 364, generation, &update_frame(delta_c.clone())).unwrap();
    assert!(outcome.close.is_none(), "{outcome:?}");
    assert_eq!(
        remote_dependency_accounting(id).unwrap().0,
        expected_retained,
        "the exact admitted candidate stays pending",
    );
    destroy_session(id);

    // After pruning (the dependency completes and the quarantine drains),
    // the identical update succeeds — the refusal above was purely the
    // retained candidate charge, never the payload's content.
    let (id, snapshot) = create_ready_room();
    let generation = synchronize_ready_room(id, &snapshot);
    receive_message(id, 365, generation, &update_frame(delta_b.clone())).unwrap();
    let slot = crate::registry::get_session(id).expect("session must remain registered");
    slot.with_alive(|session| {
        let before_bytes = session.engine.pending_remote_dependency_bytes();
        let prepared = session
            .engine
            .prepare_remote_update_v1(366, &prefix)
            .unwrap();
        assert_eq!(prepared.retained_dependency_bytes(), 0);
        assert!(!prepared.has_pending_dependencies());
        drop(prepared);
        assert_eq!(
            session.engine.pending_remote_dependency_bytes(),
            before_bytes
        );
    })
    .unwrap();
    let outcome = receive_message(id, 366, generation, &update_frame(prefix)).unwrap();
    assert!(outcome.close.is_none(), "{outcome:?}");
    assert_eq!(remote_dependency_accounting(id).unwrap(), (0, 0));
    let outcome = receive_message(id, 367, generation, &update_frame(delta_c)).unwrap();
    assert!(outcome.close.is_none(), "{outcome:?}");
    assert!(outcome.remote_commit_applied, "{outcome:?}");
    assert_eq!(remote_dependency_accounting(id).unwrap(), (0, 0));
    let json = session_audit(id)
        .unwrap()
        .document_json
        .unwrap()
        .to_string();
    assert!(json.contains("abc"), "{json}");
    destroy_session(id);
}

#[test]
fn dependency_work_ceiling_refusal_never_ratchets_the_counter() {
    let (_prefix, delta_b, delta_c) = dependent_room_update_chain();
    let retained = delta_b.len();

    // One over the prepared candidate's work ceiling: the refusal must not
    // ratchet the counter (the review's permanent-ratchet defect).
    let (id, snapshot) = create_ready_room();
    let generation = synchronize_ready_room(id, &snapshot);
    receive_message(id, 371, generation, &update_frame(delta_b.clone())).unwrap();
    assert_eq!(
        remote_dependency_accounting(id).unwrap(),
        (retained, retained as u64),
    );
    set_collaboration_limit_for_test(id, "maxPendingDependencyUpdateWork", retained).unwrap();
    let outcome = receive_message(id, 372, generation, &update_frame(delta_c.clone())).unwrap();
    let close = outcome
        .close
        .as_ref()
        .expect("one over the candidate work ceiling must close");
    assert_eq!(
        close.disposition,
        CloseDisposition::Incompatible,
        "{close:?}"
    );
    assert_eq!(
        close.error.code, TRANSPORT_DEPENDENCY_LIMIT_EXCEEDED,
        "{close:?}"
    );
    assert_eq!(
        close.error.details.as_ref().unwrap()["field"],
        "maxPendingDependencyUpdateWork",
    );
    assert_eq!(
        remote_dependency_accounting(id).unwrap(),
        (retained, retained as u64),
        "the refused admission never ratcheted the work counter",
    );
    destroy_session(id);

    // Exact: the accumulated work at the ceiling admits.
    let (id, snapshot) = create_ready_room();
    let generation = synchronize_ready_room(id, &snapshot);
    receive_message(id, 373, generation, &update_frame(delta_b.clone())).unwrap();
    let exact_work = retained + delta_c.len();
    set_collaboration_limit_for_test(id, "maxPendingDependencyUpdateWork", exact_work).unwrap();
    let outcome = receive_message(id, 374, generation, &update_frame(delta_c)).unwrap();
    assert!(outcome.close.is_none(), "{outcome:?}");
    assert_eq!(
        remote_dependency_accounting(id).unwrap().1,
        exact_work as u64,
    );
    destroy_session(id);
}
