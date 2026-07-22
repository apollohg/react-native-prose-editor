//! Task 7: bounded pre-commit collaboration outbox.
//!
//! Direct coverage of `CollaborationOutbox` reservation/install/take
//! semantics plus the session-level attachment contract: only attached
//! sessions own an outbox, reservations count against capacity before any
//! irreversible write, installs are infallible, and remote updates are never
//! echoed into the outbox.

use crate::collaboration_runtime::outbox::{
    set_reservation_allocation_failure_for_test, CollaborationOutbox, OutboxReservationError,
};
use crate::native_bridge_test_support::{self as bridge, BridgeTestOutcome, SessionOptions};

const PLAIN_DOC: &str =
    r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"seed"}]}]}"#;

fn input_envelope(request_id: u64, revision: u64, text: &str) -> String {
    serde_json::json!({
        "version": 1,
        "requestId": request_id.to_string(),
        "baseDocumentRevision": revision.to_string(),
        "text": text,
    })
    .to_string()
}

#[test]
fn reserve_install_take_flow_is_fifo_with_exact_accounting() {
    let mut outbox = CollaborationOutbox::with_ceilings(4, 1024);
    assert!(!outbox.has_pending_document_updates());
    assert_eq!(outbox.pending_document_update_count(), 0);
    assert_eq!(outbox.pending_document_update_bytes(), 0);

    let first = outbox.reserve_document_update(11, 10).unwrap();
    assert_eq!(outbox.reserved_messages(), 1);
    assert_eq!(outbox.reserved_bytes(), 10);
    outbox.install(first, vec![1, 2, 3]);
    assert_eq!(outbox.reserved_messages(), 0);
    assert_eq!(outbox.reserved_bytes(), 0);
    assert_eq!(outbox.pending_document_update_count(), 1);
    // Install charges the actual captured length, not the reserved bound.
    assert_eq!(outbox.pending_document_update_bytes(), 3);

    let second = outbox.reserve_document_update(12, 8).unwrap();
    outbox.install(second, vec![9; 8]);
    assert!(outbox.has_pending_document_updates());
    assert_eq!(outbox.pending_document_update_count(), 2);
    assert_eq!(outbox.pending_document_update_bytes(), 11);

    let taken = outbox.take_next().unwrap();
    assert_eq!(taken.request_id, 11);
    assert_eq!(taken.update_v1, vec![1, 2, 3]);
    assert_eq!(outbox.pending_document_update_bytes(), 8);
    let taken = outbox.take_next().unwrap();
    assert_eq!(taken.request_id, 12);
    assert_eq!(taken.update_v1, vec![9; 8]);
    assert!(outbox.take_next().is_none());
    assert!(!outbox.has_pending_document_updates());
    assert_eq!(outbox.pending_document_update_bytes(), 0);
}

#[test]
fn dropping_an_uninstalled_reservation_releases_its_capacity() {
    let mut outbox = CollaborationOutbox::with_ceilings(1, 16);
    {
        let reservation = outbox.reserve_document_update(21, 16).unwrap();
        assert_eq!(outbox.reserved_messages(), 1);
        assert_eq!(outbox.reserved_bytes(), 16);
        drop(reservation);
    }
    assert_eq!(outbox.reserved_messages(), 0);
    assert_eq!(outbox.reserved_bytes(), 0);
    // The released capacity is immediately reusable.
    let reservation = outbox.reserve_document_update(22, 16).unwrap();
    outbox.install(reservation, vec![0; 16]);
    assert_eq!(outbox.pending_document_update_bytes(), 16);
}

#[test]
fn exact_count_is_admitted_and_one_over_count_rejects() {
    let mut outbox = CollaborationOutbox::with_ceilings(2, 1024);
    let first = outbox.reserve_document_update(31, 4).unwrap();
    outbox.install(first, vec![1; 4]);
    // Exactly at the ceiling: pending 1 + this reservation = 2 <= 2.
    let second = outbox.reserve_document_update(32, 4).unwrap();
    outbox.install(second, vec![2; 4]);
    let error = outbox.reserve_document_update(33, 4).unwrap_err();
    assert_eq!(
        error,
        OutboxReservationError::Saturated {
            field: "maxPendingOutboxMessages",
            limit: 2,
            actual: 3,
        }
    );
    // The failed reservation left the accounting untouched.
    assert_eq!(outbox.pending_document_update_count(), 2);
    assert_eq!(outbox.reserved_messages(), 0);
    assert_eq!(outbox.reserved_bytes(), 0);
}

#[test]
fn exact_bytes_are_admitted_and_one_over_bytes_reject() {
    let mut outbox = CollaborationOutbox::with_ceilings(8, 10);
    let reservation = outbox.reserve_document_update(41, 6).unwrap();
    outbox.install(reservation, vec![1; 6]);
    // Exactly at the byte ceiling: pending 6 + bound 4 = 10 <= 10.
    let reservation = outbox.reserve_document_update(42, 4).unwrap();
    outbox.install(reservation, vec![2; 4]);
    let error = outbox.reserve_document_update(43, 1).unwrap_err();
    assert_eq!(
        error,
        OutboxReservationError::Saturated {
            field: "maxPendingOutboxBytes",
            limit: 10,
            actual: 11,
        }
    );
    assert_eq!(outbox.pending_document_update_bytes(), 10);
}

#[test]
fn live_reservations_count_against_capacity_before_any_install() {
    let mut outbox = CollaborationOutbox::with_ceilings(2, 100);
    let held_message = outbox.reserve_document_update(51, 10).unwrap();
    let held_bytes = outbox.reserve_document_update(52, 85).unwrap();
    // Count: 0 pending + 2 reserved + 1 = 3 > 2.
    let error = outbox.reserve_document_update(53, 1).unwrap_err();
    assert_eq!(
        error,
        OutboxReservationError::Saturated {
            field: "maxPendingOutboxMessages",
            limit: 2,
            actual: 3,
        }
    );
    drop(held_message);
    // Bytes: 0 pending + 85 reserved + 20 = 105 > 100.
    let error = outbox.reserve_document_update(54, 20).unwrap_err();
    assert_eq!(
        error,
        OutboxReservationError::Saturated {
            field: "maxPendingOutboxBytes",
            limit: 100,
            actual: 105,
        }
    );
    drop(held_bytes);
    assert_eq!(outbox.reserved_messages(), 0);
    assert_eq!(outbox.reserved_bytes(), 0);
}

#[test]
fn allocation_failure_injection_rejects_the_reservation_atomically() {
    let mut outbox = CollaborationOutbox::with_ceilings(4, 1024);
    set_reservation_allocation_failure_for_test(true);
    let error = outbox.reserve_document_update(61, 8).unwrap_err();
    set_reservation_allocation_failure_for_test(false);
    assert_eq!(error, OutboxReservationError::Allocation);
    assert_eq!(outbox.pending_document_update_count(), 0);
    assert_eq!(outbox.reserved_messages(), 0);
    assert_eq!(outbox.reserved_bytes(), 0);
    // Recovery: the same reservation succeeds once the injection clears.
    let reservation = outbox.reserve_document_update(62, 8).unwrap();
    outbox.install(reservation, vec![7; 8]);
    assert_eq!(outbox.pending_document_update_bytes(), 8);
}

#[test]
fn protocol_reply_reservations_share_ceilings_and_release_on_drop() {
    let mut outbox = CollaborationOutbox::with_ceilings(3, 100);
    let replies = outbox.reserve_protocol_replies(2, 40).unwrap();
    assert_eq!(outbox.reserved_messages(), 2);
    assert_eq!(outbox.reserved_bytes(), 40);
    // Document updates compete with reserved protocol replies for capacity.
    let error = outbox.reserve_document_update(71, 61).unwrap_err();
    assert_eq!(
        error,
        OutboxReservationError::Saturated {
            field: "maxPendingOutboxBytes",
            limit: 100,
            actual: 101,
        }
    );
    let document = outbox.reserve_document_update(72, 60).unwrap();
    let error = outbox.reserve_protocol_replies(1, 1).unwrap_err();
    assert_eq!(
        error,
        OutboxReservationError::Saturated {
            field: "maxPendingOutboxMessages",
            limit: 3,
            actual: 4,
        }
    );
    drop(replies);
    assert_eq!(outbox.reserved_messages(), 1);
    assert_eq!(outbox.reserved_bytes(), 60);
    outbox.install(document, vec![0; 60]);
    assert_eq!(outbox.reserved_messages(), 0);
    assert_eq!(outbox.pending_document_update_bytes(), 60);
}

#[test]
fn protocol_reply_allocation_failure_injection_is_atomic() {
    let mut outbox = CollaborationOutbox::with_ceilings(3, 100);
    set_reservation_allocation_failure_for_test(true);
    let error = outbox.reserve_protocol_replies(1, 10).unwrap_err();
    set_reservation_allocation_failure_for_test(false);
    assert_eq!(error, OutboxReservationError::Allocation);
    assert_eq!(outbox.reserved_messages(), 0);
    assert_eq!(outbox.reserved_bytes(), 0);
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "captured Update-v1 exceeds its reserved outbox bound")]
fn installing_beyond_the_reserved_bound_panics_in_debug_builds() {
    let mut outbox = CollaborationOutbox::with_ceilings(4, 1024);
    let reservation = outbox.reserve_document_update(81, 2).unwrap();
    outbox.install(reservation, vec![0; 3]);
}

#[test]
fn attached_session_enqueues_local_edits_and_detached_session_has_no_outbox() {
    let attached = bridge::create_session(SessionOptions {
        initial_json: Some(PLAIN_DOC.into()),
        attach_runtime: true,
        ..SessionOptions::default()
    })
    .unwrap();
    let detached = bridge::create_session(SessionOptions {
        initial_json: Some(PLAIN_DOC.into()),
        attach_runtime: false,
        ..SessionOptions::default()
    })
    .unwrap();

    // Detached/local-only sessions own no outbox by construction.
    assert_eq!(bridge::outbox_pending(detached).unwrap(), None);

    let revision = bridge::session_audit(attached).unwrap().document_revision;
    let outcome = bridge::submit_input(attached, &input_envelope(101, revision, "x")).unwrap();
    assert!(matches!(
        outcome,
        BridgeTestOutcome::Transaction { changed: true, .. }
    ));
    let (count, bytes) = bridge::outbox_pending(attached).unwrap().unwrap();
    assert_eq!(count, 1);
    assert!(bytes > 0);
    let bound = bridge::last_reserved_upper_bound(attached)
        .unwrap()
        .unwrap();
    let (request_id, update) = bridge::take_next_update(attached).unwrap().unwrap();
    assert_eq!(request_id, 101);
    assert_eq!(update.len(), bytes);
    assert!(update.len() <= bound);

    // The same edit on the detached session behaves identically and still
    // exposes no outbox.
    let revision = bridge::session_audit(detached).unwrap().document_revision;
    let outcome = bridge::submit_input(detached, &input_envelope(102, revision, "x")).unwrap();
    assert!(matches!(
        outcome,
        BridgeTestOutcome::Transaction { changed: true, .. }
    ));
    assert_eq!(bridge::outbox_pending(detached).unwrap(), None);

    let attached_audit = bridge::session_audit(attached).unwrap();
    let detached_audit = bridge::session_audit(detached).unwrap();
    assert_eq!(attached_audit.document_json, detached_audit.document_json);
    assert_eq!(attached_audit.document_html, detached_audit.document_html);

    bridge::destroy_session(attached);
    bridge::destroy_session(detached);
}

/// Item 5 of the behavioral contract: a session without an attached
/// collaboration runtime edits with pre-Task-7 semantics. The bridge-driven
/// session and a directly driven engine produce the same document, selection,
/// history availability, and revision progression; encoded states differ only
/// by the random client identity and decode to the same document.
#[test]
fn detached_session_editing_matches_direct_engine_behavior() {
    use crate::boundary::ResourceLimits;
    use crate::tiptap_schema;
    use crate::yrs_engine::{
        EditingLimits, InitializationMode, TransactionOrigin, TypedCommand, YrsDocumentEngine,
        YrsEngineConfig,
    };

    let id = bridge::create_session(SessionOptions {
        initial_json: Some(PLAIN_DOC.into()),
        attach_runtime: false,
        ..SessionOptions::default()
    })
    .unwrap();

    let mut engine = YrsDocumentEngine::new(YrsEngineConfig {
        schema: tiptap_schema(),
        fragment_name: "prosemirror".into(),
        initialization_mode: InitializationMode::LocalEmpty,
        resource_limits: ResourceLimits::default(),
        editing_limits: EditingLimits::default(),
        max_length: None,
        scope: None,
    })
    .unwrap();
    engine
        .import_json(PLAIN_DOC, TransactionOrigin::DocumentImport)
        .unwrap();

    // Same trace on both: type once, toggle a command, undo, redo. The
    // engine twin uses the identical pre-Task-7 lowering the bridge froze:
    // one planner-lowered typed transaction with the local-input origin.
    let revision = bridge::session_audit(id).unwrap().document_revision;
    bridge::submit_input(id, &input_envelope(201, revision, "zz")).unwrap();
    match engine
        .plan_command(201, TypedCommand::InsertText { text: "zz".into() })
        .unwrap()
    {
        crate::yrs_engine::CommandPlan::Transaction(mut transaction)
        | crate::yrs_engine::CommandPlan::SelectionOnly(mut transaction) => {
            transaction.origin = TransactionOrigin::LocalInput;
            engine.apply_typed_transaction(transaction).unwrap();
        }
        plan => panic!("typing must lower to a transaction: {plan:?}"),
    }
    let revision = bridge::session_audit(id).unwrap().document_revision;
    bridge::submit_command(
        id,
        &serde_json::json!({
            "version": 1,
            "requestId": "202",
            "baseDocumentRevision": revision.to_string(),
            "command": { "type": "toggleBlockquote" },
        })
        .to_string(),
    )
    .unwrap();
    engine
        .apply_command(202, TypedCommand::ToggleBlockquote)
        .unwrap();
    // Undo/redo is deliberately not part of this cross-engine trace: the
    // pre-existing, separately tracked undo-chain defect makes pops through
    // previously grouped structural changes client-id-dependent, so two
    // engines with random client identities cannot be compared through a
    // pop. Bridge undo/redo behavior is pinned by the deterministic
    // same-session convergence and saturation suites instead.
    assert_eq!(
        bridge::session_audit(id).unwrap().can_undo,
        engine.can_undo()
    );

    let audit = bridge::session_audit(id).unwrap();
    assert_eq!(audit.document_json, engine.document_json());
    assert_eq!(audit.document_html, engine.document_html());
    assert_eq!(audit.document_revision, engine.revision());
    assert_eq!(audit.state_revision, engine.state_revision());
    assert_eq!(audit.can_undo, engine.can_undo());
    assert_eq!(audit.can_redo, engine.can_redo());
    assert_eq!(
        audit.selection,
        engine.resolved_selection().map(|s| format!("{s:?}")),
    );
    assert_eq!(audit.outbox_pending_updates, None);
    assert_eq!(audit.outbox_pending_bytes, None);

    // Encoded states decode to the same document on a content-free replica
    // (an AwaitRemote store carries no locally seeded bootstrap content).
    let mut replica = YrsDocumentEngine::new(YrsEngineConfig {
        schema: tiptap_schema(),
        fragment_name: "prosemirror".into(),
        initialization_mode: InitializationMode::AwaitRemote,
        resource_limits: ResourceLimits::default(),
        editing_limits: EditingLimits::default(),
        max_length: None,
        scope: Some(crate::yrs_engine::DocumentScope {
            document_id: "outbox-equivalence".into(),
            lineage_id: "outbox-equivalence-lineage".into(),
        }),
    })
    .unwrap();
    replica
        .apply_remote_update_v1(205, &audit.encoded_state.clone().unwrap())
        .unwrap();
    assert_eq!(replica.document_json(), engine.document_json());

    bridge::destroy_session(id);
}

/// No echo: remote updates admitted through the engine never produce an
/// outbox entry; local edits on the same attached session still do.
#[test]
fn remote_updates_are_never_echoed_into_the_outbox() {
    use crate::boundary::ResourceLimits;
    use crate::tiptap_schema;
    use crate::yrs_engine::{
        EditingLimits, InitializationMode, TransactionOrigin, YrsDocumentEngine, YrsEngineConfig,
    };

    let id = bridge::create_session(SessionOptions {
        attach_runtime: true,
        ..SessionOptions::default()
    })
    .unwrap();

    let mut source = YrsDocumentEngine::new(YrsEngineConfig {
        schema: tiptap_schema(),
        fragment_name: "prosemirror".into(),
        initialization_mode: InitializationMode::LocalEmpty,
        resource_limits: ResourceLimits::default(),
        editing_limits: EditingLimits::default(),
        max_length: None,
        scope: None,
    })
    .unwrap();
    source
        .import_json(PLAIN_DOC, TransactionOrigin::DocumentImport)
        .unwrap();
    let update = source.encoded_state().unwrap();

    let changed = bridge::apply_remote_update(id, 301, &update).unwrap();
    assert!(changed);
    assert_eq!(bridge::outbox_pending(id).unwrap(), Some((0, 0)));
    assert_eq!(bridge::last_reserved_upper_bound(id).unwrap(), None);

    let revision = bridge::session_audit(id).unwrap().document_revision;
    bridge::submit_input(id, &input_envelope(302, revision, "y")).unwrap();
    let (count, bytes) = bridge::outbox_pending(id).unwrap().unwrap();
    assert_eq!(count, 1);
    assert!(bytes > 0);

    bridge::destroy_session(id);
}
