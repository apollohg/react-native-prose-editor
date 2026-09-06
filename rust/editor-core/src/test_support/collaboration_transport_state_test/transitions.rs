#[test]
fn generations_increment_exactly_once_per_accepted_connect() {
    let id = create_ready_room();

    let first = drive_generation(id, 101).unwrap();
    // A drive while an attempt is live reports no generation and does not
    // consume one.
    let parked = collaboration_drive(id, 102, next_legacy_test_now_millis()).unwrap();
    assert_eq!(parked.generation_to_open, None);
    assert_eq!(
        close_socket(id, 103, first, CloseDisposition::Retryable).unwrap(),
        TransportState::Disconnected,
    );

    let second = drive_generation(id, 104).unwrap();
    assert_eq!(
        second,
        first + 1,
        "an accepted connect increments the generation exactly once",
    );
    transport_disconnect(id, 105).unwrap();
    transport_detach(id, 105).unwrap();
    transport_reattach(id, 105).unwrap();

    let third = drive_generation(id, 106).unwrap();
    assert_eq!(
        third,
        second + 1,
        "parked drives must never burn a generation"
    );

    destroy_session(id);
}

#[test]
fn stale_generations_can_neither_advance_nor_regress_state() {
    let id = create_ready_room();
    bridge::attach_runtime(id).unwrap();

    // Open and retire a first generation, then start a second attempt.
    let first = drive_generation(id, 111).unwrap();
    close_socket(id, 112, first, CloseDisposition::Retryable).unwrap();
    let second = drive_generation(id, 113).unwrap();
    let audit = session_audit(id).unwrap();
    assert_eq!(audit.transport_state, TransportState::Connecting);

    // The retired generation is refused for every callback, observably, and
    // both the state and the live generation stay untouched.
    for (request_id, result) in [
        (114, open_socket_and_ack_step1(id, 114, first)),
        (
            115,
            close_socket(id, 115, first, CloseDisposition::Retryable).map(|_| ()),
        ),
        (
            116,
            close_socket(id, 116, first, CloseDisposition::Incompatible).map(|_| ()),
        ),
        (117, mark_synchronized_for_test(id, 117, first)),
    ] {
        let error = result.expect_err("stale generation must be refused");
        assert_eq!(error.domain, "transport", "{error:?}");
        assert_eq!(error.code, TRANSPORT_STALE_GENERATION, "{error:?}");
        assert_eq!(error.request_id, Some(request_id), "{error:?}");
        assert_eq!(session_audit(id).unwrap(), audit);
    }

    // The live generation still works: the attempt was not poisoned.
    open_socket_and_ack_step1(id, 118, second).unwrap();
    assert_eq!(transport_state(id).unwrap(), TransportState::Handshaking);

    // Local disconnect retires the live generation: its late close callback
    // is stale and cannot resurrect or regress the transport.
    transport_disconnect(id, 119).unwrap();
    let error = close_socket(id, 120, second, CloseDisposition::Incompatible).unwrap_err();
    assert_eq!(error.code, TRANSPORT_STALE_GENERATION, "{error:?}");
    assert_eq!(transport_state(id).unwrap(), TransportState::Disconnected);
    // A lifecycle reset makes a fresh Rust drive eligible after a local disconnect.
    transport_detach(id, 121).unwrap();
    transport_reattach(id, 121).unwrap();
    drive_generation(id, 121).unwrap();

    destroy_session(id);
}

#[test]
fn incompatible_ignores_repeated_drives_until_detach_and_reattach() {
    let id = create_ready_room();
    bridge::attach_runtime(id).unwrap();

    let generation = drive_generation(id, 131).unwrap();
    open_socket_and_ack_step1(id, 132, generation).unwrap();
    assert_eq!(
        close_socket(id, 133, generation, CloseDisposition::Incompatible).unwrap(),
        TransportState::Incompatible,
    );

    // Repeated native calls to Rust's drive cannot force a reconnect out of
    // deterministic incompatibility.
    for request_id in [134, 135, 136] {
        let parked = collaboration_drive(id, request_id, next_legacy_test_now_millis()).unwrap();
        assert_eq!(parked.transport_state, TransportState::Incompatible);
        assert_eq!(parked.generation_to_open, None);
        assert_eq!(parked.next_deadline_millis, None);
    }

    // The only escape hatch is the explicit detach/reattach cycle.
    transport_detach(id, 137).unwrap();
    assert_eq!(transport_state(id).unwrap(), TransportState::Detached);
    transport_reattach(id, 138).unwrap();
    assert_eq!(transport_state(id).unwrap(), TransportState::Disconnected);
    let next = drive_generation(id, 139).unwrap();
    assert_eq!(next, generation + 1);

    destroy_session(id);
}

#[test]
fn replacement_policy_gate_follows_real_transport_transitions() {
    let id = create_ready_room();
    bridge::attach_runtime(id).unwrap();

    let assert_connected_refusal = |request_id: u64, json: &str| {
        let error =
            write_json(id, request_id, json, ReplacementHistory::UndoableBoundary).unwrap_err();
        assert_eq!(error.domain, "lifecycle", "{error:?}");
        assert_eq!(
            error.code, "WHOLE_DOCUMENT_REPLACEMENT_CONNECTED",
            "{error:?}"
        );
        assert_eq!(error.request_id, Some(request_id), "{error:?}");
    };

    // Disconnected: allowed.
    assert!(
        write_json(
            id,
            141,
            JSON_REPLACEMENT_A,
            ReplacementHistory::UndoableBoundary
        )
        .unwrap()
        .changed
    );

    // Connecting, Handshaking, Synchronized via real transitions: refused.
    let generation = drive_generation(id, 142).unwrap();
    assert_connected_refusal(143, JSON_REPLACEMENT_B);
    open_socket_and_ack_step1(id, 144, generation).unwrap();
    assert_connected_refusal(145, JSON_REPLACEMENT_B);
    mark_synchronized_for_test(id, 146, generation).unwrap();
    assert_connected_refusal(147, JSON_REPLACEMENT_B);

    // Remote close back to Disconnected: allowed again.
    assert_eq!(
        close_socket(id, 148, generation, CloseDisposition::Retryable).unwrap(),
        TransportState::Disconnected,
    );
    assert!(
        write_json(
            id,
            149,
            JSON_REPLACEMENT_B,
            ReplacementHistory::UndoableBoundary
        )
        .unwrap()
        .changed
    );

    // Local disconnect also reopens the gate.
    let generation = drive_generation(id, 150).unwrap();
    assert_connected_refusal(151, JSON_REPLACEMENT_A);
    transport_disconnect(id, 152).unwrap();
    assert!(
        write_json(
            id,
            153,
            JSON_REPLACEMENT_A,
            ReplacementHistory::UndoableBoundary
        )
        .unwrap()
        .changed
    );
    let _ = generation;

    // Incompatible is an allowed replacement row of the matrix.
    transport_detach(id, 154).unwrap();
    transport_reattach(id, 154).unwrap();
    let generation = drive_generation(id, 154).unwrap();
    close_socket(id, 155, generation, CloseDisposition::Incompatible).unwrap();
    assert!(
        write_json(
            id,
            156,
            JSON_REPLACEMENT_B,
            ReplacementHistory::UndoableBoundary
        )
        .unwrap()
        .changed
    );

    destroy_session(id);
}

#[test]
fn transport_teardown_never_drops_pending_outbox_entries() {
    let id = create_ready_room();
    bridge::attach_runtime(id).unwrap();

    // Enqueue one real captured local edit.
    let revision = bridge::session_audit(id).unwrap().document_revision;
    let envelope = serde_json::json!({
        "version": 1,
        "requestId": "161",
        "baseDocumentRevision": revision.to_string(),
        "text": "offline edit",
    })
    .to_string();
    bridge::submit_input(id, &envelope).unwrap();
    let pending = bridge::outbox_pending(id).unwrap().unwrap();
    assert_eq!(
        pending.0, 1,
        "the edit must be captured before transport work"
    );

    // Close (retryable), close (incompatible), disconnect, and detach all
    // leave the pending offline entry in place for post-reconnect delivery.
    let generation = drive_generation(id, 162).unwrap();
    close_socket(id, 163, generation, CloseDisposition::Retryable).unwrap();
    assert_eq!(bridge::outbox_pending(id).unwrap().unwrap(), pending);

    let generation = drive_generation(id, 164).unwrap();
    close_socket(id, 165, generation, CloseDisposition::Incompatible).unwrap();
    assert_eq!(bridge::outbox_pending(id).unwrap().unwrap(), pending);

    transport_detach(id, 166).unwrap();
    assert_eq!(bridge::outbox_pending(id).unwrap().unwrap(), pending);

    transport_reattach(id, 167).unwrap();
    let generation = drive_generation(id, 168).unwrap();
    open_socket_and_ack_step1(id, 169, generation).unwrap();
    transport_disconnect(id, 170).unwrap();
    assert_eq!(bridge::outbox_pending(id).unwrap().unwrap(), pending);

    // The entry is still deliverable.
    let lease = bridge::lease_next_update(id).unwrap().unwrap();
    assert_eq!(lease.request_id, 161);
    assert!(!lease.update_v1.is_empty());
    bridge::ack_leased_update(id, lease.lease_id).unwrap();

    bridge::destroy_session(id);
}

/// Extension of the teardown matrix rows above: the same transitions
/// that preserve the pending outbox entry clear the transport-scoped
/// awareness peers while the desired local awareness survives every
/// one of them.
#[test]
fn transport_teardown_clears_awareness_peers_and_retains_desired_awareness() {
    use crate::session_initialization_test_support::{
        awareness_peers, desired_awareness, receive_message,
        set_desired_awareness_for_test as set_desired_awareness,
    };
    use yrs::updates::encoder::Encode as _;

    let id = create_ready_room();
    bridge::attach_runtime(id).unwrap();

    // One pending offline edit proves outbox survival stays untouched by
    // the awareness lifecycle.
    let revision = bridge::session_audit(id).unwrap().document_revision;
    let envelope = serde_json::json!({
        "version": 1,
        "requestId": "171",
        "baseDocumentRevision": revision.to_string(),
        "text": "offline edit",
    })
    .to_string();
    bridge::submit_input(id, &envelope).unwrap();
    let pending = bridge::outbox_pending(id).unwrap().unwrap();

    let desired = serde_json::json!({ "name": "survivor" });
    set_desired_awareness(id, 172, &desired.to_string()).unwrap();

    let awareness_frame = |client: u64, clock: u32| -> Vec<u8> {
        let mut clients = std::collections::HashMap::new();
        clients.insert(
            yrs::ClientID::new(client),
            yrs::sync::awareness::AwarenessUpdateEntry {
                clock,
                json: r#"{"name":"transient"}"#.into(),
            },
        );
        yrs::sync::Message::Awareness(yrs::sync::awareness::AwarenessUpdate { clients }).encode_v1()
    };
    let remote_peer_count = |id: u64| {
        awareness_peers(id)
            .unwrap()
            .iter()
            .filter(|peer| !peer.is_local)
            .count()
    };

    // Retryable close.
    let generation = drive_generation(id, 173).unwrap();
    open_socket_and_ack_step1(id, 174, generation).unwrap();
    receive_message(id, 175, generation, &awareness_frame(9_501, 1)).unwrap();
    assert_eq!(remote_peer_count(id), 1);
    close_socket(id, 176, generation, CloseDisposition::Retryable).unwrap();
    assert_eq!(remote_peer_count(id), 0, "retryable close clears peers");
    assert_eq!(desired_awareness(id).unwrap(), Some(desired.clone()));
    assert_eq!(bridge::outbox_pending(id).unwrap().unwrap(), pending);

    // Incompatible close.
    let generation = drive_generation(id, 177).unwrap();
    open_socket_and_ack_step1(id, 178, generation).unwrap();
    receive_message(id, 179, generation, &awareness_frame(9_502, 1)).unwrap();
    assert_eq!(remote_peer_count(id), 1);
    close_socket(id, 180, generation, CloseDisposition::Incompatible).unwrap();
    assert_eq!(remote_peer_count(id), 0, "incompatible close clears peers");
    assert_eq!(desired_awareness(id).unwrap(), Some(desired.clone()));

    // Detach/reattach escape hatch, then a local disconnect.
    transport_detach(id, 181).unwrap();
    transport_reattach(id, 182).unwrap();
    let generation = drive_generation(id, 183).unwrap();
    open_socket_and_ack_step1(id, 184, generation).unwrap();
    receive_message(id, 185, generation, &awareness_frame(9_503, 1)).unwrap();
    assert_eq!(remote_peer_count(id), 1);
    transport_disconnect(id, 186).unwrap();
    assert_eq!(remote_peer_count(id), 0, "local disconnect clears peers");
    assert_eq!(desired_awareness(id).unwrap(), Some(desired));
    assert_eq!(bridge::outbox_pending(id).unwrap().unwrap(), pending);

    bridge::destroy_session(id);
}

#[test]
fn destroy_wins_over_transport_transitions_from_pre_acquired_handles() {
    let id = create_ready_room();
    let holder_handle = transport_handle(id).expect("live session must yield a handle");
    let caller_handle = transport_handle(id).expect("live session must yield a handle");

    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let holder = thread::spawn(move || holder_handle.hold_session_lock(entered_tx, release_rx));
    entered_rx
        .recv_timeout(WAIT_TIMEOUT)
        .expect("holder must enter while alive and take the session lock");

    let destroyer = thread::spawn(move || destroy_session(id));
    // Destroy removes the registry entry (after flipping to Destroying)
    // before it blocks on the held session lock.
    let deadline = Instant::now() + WAIT_TIMEOUT;
    while transport_handle(id).is_some() {
        assert!(
            Instant::now() < deadline,
            "destroy never removed the session from the registry"
        );
        thread::yield_now();
    }

    // Every transport verb on the pre-acquired handle is refused with the
    // frozen lifecycle code while destroy is in flight, without touching
    // transport state.
    assert_handle_refusals(&caller_handle, "ENGINE_DESTROYING");

    release_tx
        .send(())
        .expect("holder must still be waiting for release");
    holder
        .join()
        .expect("holder must not panic")
        .expect("a call linearized before destroy must finish normally");
    destroyer.join().expect("destroy must not panic");

    // After destroy completes the same pre-acquired handle stays refused,
    // for every verb.
    assert_handle_refusals(&caller_handle, "ENGINE_DESTROYED");
    assert!(
        transport_handle(id).is_none(),
        "no new handle may be acquired after destroy"
    );
}

/// All seven transport verbs on a pre-acquired handle must refuse with the
/// given frozen lifecycle code.
fn assert_handle_refusals(handle: &TransportHandle, expected_code: &str) {
    let refusals: [(&str, Result<(), TestError>); 7] = [
        (
            "collaborationDrive",
            handle
                .collaboration_drive(ACTION_REQUEST_ID, next_legacy_test_now_millis())
                .map(|_| ()),
        ),
        (
            "collaborationSocketOpen",
            handle
                .collaboration_socket_open(
                    ACTION_REQUEST_ID,
                    FABRICATED_GENERATION,
                    next_legacy_test_now_millis(),
                )
                .map(|_| ()),
        ),
        (
            "collaborationSocketClose",
            handle
                .collaboration_socket_close(
                    ACTION_REQUEST_ID,
                    FABRICATED_GENERATION,
                    CloseDisposition::Retryable,
                    next_legacy_test_now_millis(),
                )
                .map(|_| ()),
        ),
        ("disconnect", handle.disconnect(ACTION_REQUEST_ID)),
        ("detach", handle.detach(ACTION_REQUEST_ID)),
        ("reattach", handle.reattach(ACTION_REQUEST_ID)),
        (
            "markSynchronized",
            handle.mark_synchronized(ACTION_REQUEST_ID, FABRICATED_GENERATION),
        ),
    ];
    for (op, result) in refusals {
        let error = result.expect_err(&format!("{op}: must refuse with {expected_code}"));
        assert_eq!(error.domain, "lifecycle", "{op}: {error:?}");
        assert_eq!(error.code, expected_code, "{op}: {error:?}");
    }
}

#[test]
fn await_remote_rooms_connect_without_document_promotion() {
    let id = create_await_remote_room();
    bridge::attach_runtime(id).unwrap();
    assert_eq!(
        crate::session_initialization_test_support::document_state(id).unwrap(),
        DocumentState::AwaitRemote
    );

    // The full connect/handshake/synchronize path is transport-only: the
    // AwaitRemote -> RoomReady promotion belongs to Task 9's Step 2 handling.
    let generation = drive_generation(id, 181).unwrap();
    open_socket_and_ack_step1(id, 182, generation).unwrap();
    assert_eq!(
        transport_state(id).unwrap(),
        TransportState::Handshaking,
        "socket open must never mean Synchronized",
    );
    mark_synchronized_guard(id, generation);

    destroy_session(id);
}

/// `mark_synchronized` is generation-checked even on `AwaitRemote` rooms and
/// never promotes the document state.
fn mark_synchronized_guard(id: u64, generation: u64) {
    let error = mark_synchronized_for_test(id, 183, FABRICATED_GENERATION).unwrap_err();
    assert_eq!(error.code, TRANSPORT_STALE_GENERATION, "{error:?}");
    mark_synchronized_for_test(id, 184, generation).unwrap();
    assert_eq!(transport_state(id).unwrap(), TransportState::Synchronized);
    assert_eq!(
        crate::session_initialization_test_support::document_state(id).unwrap(),
        DocumentState::AwaitRemote,
        "transport synchronization must not promote the document state",
    );
}

/// A complete y-sync no-op update message (`[MSG_SYNC, MSG_SYNC_UPDATE,
/// buf(len=2, [0, 0])]`): the standard fixture framing of the canonical
/// empty Update-v1.
const NOOP_SYNC_UPDATE_MESSAGE: [u8; 5] = [0, 2, 2, 0, 0];

/// `receive_message` obeys the same generation discipline as every other
/// callback — stale generations and non-frame-accepting states refuse
/// before any decode work, leaving the full audit untouched.
#[test]
fn receive_message_is_generation_gated_across_all_live_transport_states() {
    use crate::session_initialization_test_support::receive_message;

    for state in LIVE_TRANSPORT_STATES {
        let id = create_ready_room();
        bridge::attach_runtime(id).unwrap();
        let latest = drive_transport(id, Doc::RoomReady, state);
        let current = latest.unwrap_or(FABRICATED_GENERATION);
        let before = session_audit(id).unwrap();
        let before_engine = bridge::session_audit(id).unwrap();
        let label = format!("receiveMessage in {state:?}");

        match state {
            TransportState::Handshaking | TransportState::Synchronized => {
                let outcome =
                    receive_message(id, ACTION_REQUEST_ID, current, &NOOP_SYNC_UPDATE_MESSAGE)
                        .unwrap_or_else(|error| panic!("{label}: expected acceptance: {error:?}"));
                assert!(outcome.close.is_none(), "{label}: {outcome:?}");
                assert!(!outcome.remote_commit_applied, "{label}: {outcome:?}");
                assert_eq!(transport_state(id).unwrap(), state, "{label}");
                assert_eq!(
                    bridge::session_audit(id).unwrap(),
                    before_engine,
                    "{label}: a no-op frame changes nothing",
                );
            }
            TransportState::Connecting => {
                let error =
                    receive_message(id, ACTION_REQUEST_ID, current, &NOOP_SYNC_UPDATE_MESSAGE)
                        .expect_err(&format!("{label}: must refuse"));
                assert_eq!(error.domain, "transport", "{label}: {error:?}");
                assert_eq!(
                    error.code, TRANSPORT_INVALID_TRANSITION,
                    "{label}: {error:?}"
                );
                assert_eq!(session_audit(id).unwrap(), before, "{label}");
            }
            TransportState::Detached
            | TransportState::Disconnected
            | TransportState::Incompatible => {
                let error =
                    receive_message(id, ACTION_REQUEST_ID, current, &NOOP_SYNC_UPDATE_MESSAGE)
                        .expect_err(&format!("{label}: must refuse"));
                assert_eq!(error.domain, "transport", "{label}: {error:?}");
                assert_eq!(error.code, TRANSPORT_STALE_GENERATION, "{label}: {error:?}");
                assert_eq!(session_audit(id).unwrap(), before, "{label}");
            }
        }
        // A stale generation refuses identically in every state, even the
        // frame-accepting ones.
        let error = receive_message(
            id,
            ACTION_REQUEST_ID,
            FABRICATED_GENERATION,
            &NOOP_SYNC_UPDATE_MESSAGE,
        )
        .expect_err(&format!("{label}: fabricated generation must refuse"));
        assert_eq!(error.code, TRANSPORT_STALE_GENERATION, "{label}: {error:?}");
        assert_eq!(
            bridge::session_audit(id).unwrap(),
            before_engine,
            "{label}: refusals leave the engine/outbox audit untouched",
        );
        destroy_session(id);
    }
}

/// Destroyed sessions refuse `receive_message` with the frozen
/// lifecycle codes, exactly like every other transport verb.
#[test]
fn receive_message_refuses_destroyed_sessions_with_lifecycle_codes() {
    use crate::session_initialization_test_support::receive_message;

    let id = create_ready_room();
    bridge::attach_runtime(id).unwrap();
    let generation = drive_generation(id, 191).unwrap();
    open_socket_and_ack_step1(id, 192, generation).unwrap();
    destroy_session(id);

    let error = receive_message(id, 193, generation, &NOOP_SYNC_UPDATE_MESSAGE)
        .expect_err("destroyed sessions must refuse frames");
    assert_eq!(error.domain, "lifecycle", "{error:?}");
    assert_eq!(error.code, "ENGINE_DESTROYED", "{error:?}");
}
