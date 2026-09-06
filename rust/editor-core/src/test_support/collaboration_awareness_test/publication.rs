// Desired local awareness: ownership, validation, lifecycle

#[test]
fn set_desired_awareness_publishes_immediately_and_bounds_the_state_at_set_time() {
    let (id, snapshot) = create_ready_room();
    let generation = synchronize_ready_room(id, &snapshot);

    let desired = json!({ "name": "author", "color": "#204060" });
    set_desired_awareness(id, 401, &desired.to_string()).unwrap();
    assert_eq!(desired_awareness(id).unwrap(), Some(desired.clone()));
    let local = local_peer(id).expect("published local state projects");
    assert_eq!(local.state, desired);

    // The synchronized transport broadcasts the state immediately; a raw
    // peer applying the frame sees exactly the desired state.
    let replies = drain_protocol_replies(id, generation);
    assert_eq!(replies.len(), 1, "{replies:?}");
    let mut raw = Awareness::new(Doc::new());
    raw.apply_update(decode_awareness_reply(&replies[0]))
        .unwrap();
    assert_eq!(
        raw.state::<Value>(ClientID::new(local.client_id)),
        Some(desired.clone()),
    );

    // Malformed JSON rejects without touching the retained state.
    let error = set_desired_awareness(id, 402, "{not json").unwrap_err();
    assert_eq!(error.code, "AWARENESS_STATE_INVALID", "{error:?}");
    assert_eq!(desired_awareness(id).unwrap(), Some(desired.clone()));

    // Per-peer byte bound applies at set time: exact passes, one over
    // rejects atomically (previous desired state and clock retained).
    let exact = json!({ "pad": "y".repeat(40) });
    let exact_len = exact.to_string().len();
    set_collaboration_limit_for_test(id, "maxAwarenessPeerBytes", exact_len).unwrap();
    set_desired_awareness(id, 403, &exact.to_string()).unwrap();
    assert_eq!(desired_awareness(id).unwrap(), Some(exact.clone()));
    let clock_after_exact = local_peer(id).unwrap().clock;

    let over = json!({ "pad": "y".repeat(41) });
    let error = set_desired_awareness(id, 404, &over.to_string()).unwrap_err();
    assert_eq!(error.code, "INPUT_LIMIT_EXCEEDED", "{error:?}");
    assert_eq!(desired_awareness(id).unwrap(), Some(exact));
    assert_eq!(
        local_peer(id).unwrap().clock,
        clock_after_exact,
        "atomic rejection must not bump the published clock",
    );
    destroy_session(id);
}

#[test]
fn clear_desired_awareness_broadcasts_a_tombstone_and_clears_the_projection() {
    let (id, snapshot) = create_ready_room();
    let generation = synchronize_ready_room(id, &snapshot);
    let desired = json!({ "name": "leaving" });
    set_desired_awareness(id, 411, &desired.to_string()).unwrap();
    let local = local_peer(id).unwrap();
    let replies = drain_protocol_replies(id, generation);
    let mut raw = Awareness::new(Doc::new());
    raw.apply_update(decode_awareness_reply(&replies[0]))
        .unwrap();
    assert_eq!(
        raw.state::<Value>(ClientID::new(local.client_id)),
        Some(desired)
    );

    clear_desired_awareness(id, 412).unwrap();
    assert_eq!(desired_awareness(id).unwrap(), None);
    assert!(
        local_peer(id).is_none(),
        "cleared state leaves the projection"
    );

    // The broadcast tombstone removes us from the raw peer's view.
    let replies = drain_protocol_replies(id, generation);
    assert_eq!(replies.len(), 1, "{replies:?}");
    raw.apply_update(decode_awareness_reply(&replies[0]))
        .unwrap();
    assert_eq!(raw.state::<Value>(ClientID::new(local.client_id)), None);

    // Clearing twice stays an idempotent no-op with nothing to broadcast.
    clear_desired_awareness(id, 413).unwrap();
    assert_eq!(drain_protocol_replies(id, generation).len(), 0);
    destroy_session(id);
}

#[test]
fn withdrawal_review_fix_saturated_clear_is_one_shot_and_tick_heals_after_drain() {
    let (id, snapshot) = create_ready_room();
    let generation = synchronize_ready_room(id, &snapshot);
    drive_awareness(id, 414, 0).unwrap();
    let desired = json!({ "name": "saturated withdrawal" });
    set_desired_awareness(id, 415, &desired.to_string()).unwrap();
    let local = local_peer(id).unwrap();
    let local_client = ClientID::new(local.client_id);
    let mut raw = Awareness::new(Doc::new());
    for reply in drain_protocol_replies(id, generation) {
        raw.apply_update(decode_awareness_reply(&reply)).unwrap();
    }
    let published_clock = raw.meta(local_client).unwrap().0;

    // Fill the sole shared outbox slot with a real awareness-query reply.
    bridge::set_outbox_ceilings(id, 1, 10 * 1024 * 1024).unwrap();
    let outcome = receive_message(id, 416, generation, &query_awareness_message()).unwrap();
    assert!(outcome.close.is_none(), "{outcome:?}");
    assert_eq!(outcome.replies_enqueued, 1, "{outcome:?}");

    let error = clear_desired_awareness(id, 417).unwrap_err();
    assert_eq!(error.code, TRANSPORT_REPLY_LIMIT_EXCEEDED, "{error:?}");
    assert_eq!(transport_state(id).unwrap(), TransportState::Synchronized);
    assert_eq!(desired_awareness(id).unwrap(), None);
    assert!(local_peer(id).is_none(), "withdrawal is locally immediate");

    // Repeating clear cannot recreate a tombstone or advance its clock.
    clear_desired_awareness(id, 418).unwrap();
    let before_retry = drive_awareness(id, 419, AWARENESS_RENEWAL_INTERVAL_MILLIS - 1).unwrap();
    assert!(!before_retry.outbound_changed, "{before_retry:?}");
    assert_eq!(
        before_retry.next_deadline_millis,
        Some(AWARENESS_RENEWAL_INTERVAL_MILLIS),
        "{before_retry:?}",
    );

    // Draining the blocker lets the deterministic boundary tick enqueue the
    // exact already-clocked tombstone.
    let blocker = lease_outbound(id, 419, generation)
        .unwrap()
        .expect("the query reply must remain retained until explicit ACK");
    ack_outbound(id, 419, generation, blocker.lease_id).unwrap();
    let healed = drive_awareness(id, 420, AWARENESS_RENEWAL_INTERVAL_MILLIS).unwrap();
    assert!(!healed.renewed_local, "{healed:?}");
    assert!(healed.outbound_changed, "{healed:?}");
    assert_eq!(healed.next_deadline_millis, None, "{healed:?}");
    let replies = drain_protocol_replies(id, generation);
    assert_eq!(replies.len(), 1, "{replies:?}");
    let tombstone = decode_awareness_reply(&replies[0]);
    assert_eq!(tombstone.clients[&local_client].clock, published_clock + 1);
    raw.apply_update(tombstone).unwrap();
    assert_eq!(raw.state::<Value>(local_client), None);
    assert_eq!(raw.meta(local_client).unwrap().0, published_clock + 1);
    destroy_session(id);
}

#[test]
fn withdrawal_review_fix_allocation_retry_stays_pending_without_closing_generation() {
    let (id, snapshot) = create_ready_room();
    let generation = synchronize_ready_room(id, &snapshot);
    drive_awareness(id, 421, 0).unwrap();
    set_desired_awareness(
        id,
        422,
        &json!({ "name": "allocation withdrawal" }).to_string(),
    )
    .unwrap();
    let local = local_peer(id).unwrap();
    let local_client = ClientID::new(local.client_id);
    let mut raw = Awareness::new(Doc::new());
    for reply in drain_protocol_replies(id, generation) {
        raw.apply_update(decode_awareness_reply(&reply)).unwrap();
    }
    let published_clock = raw.meta(local_client).unwrap().0;

    bridge::set_outbox_allocation_failure(true);
    let clear_result = clear_desired_awareness(id, 423);
    bridge::set_outbox_allocation_failure(false);
    let error = clear_result.unwrap_err();
    assert_eq!(error.code, TRANSPORT_RESOURCE_EXHAUSTED, "{error:?}");
    assert_eq!(transport_state(id).unwrap(), TransportState::Synchronized);
    assert_eq!(desired_awareness(id).unwrap(), None);
    assert!(local_peer(id).is_none());

    bridge::set_outbox_allocation_failure(true);
    let retry_result = drive_awareness(id, 424, AWARENESS_RENEWAL_INTERVAL_MILLIS);
    bridge::set_outbox_allocation_failure(false);
    let error = retry_result.unwrap_err();
    assert_eq!(error.code, TRANSPORT_RESOURCE_EXHAUSTED, "{error:?}");
    assert_eq!(transport_state(id).unwrap(), TransportState::Synchronized);
    assert_eq!(drain_protocol_replies(id, generation).len(), 0);

    // Retry failure preserves the frame but moves the next attempt out by
    // one interval, so scheduling cannot spin at the failed timestamp.
    let before_retry = drive_awareness(id, 425, AWARENESS_RENEWAL_INTERVAL_MILLIS * 2 - 1).unwrap();
    assert!(!before_retry.outbound_changed, "{before_retry:?}");
    assert_eq!(
        before_retry.next_deadline_millis,
        Some(AWARENESS_RENEWAL_INTERVAL_MILLIS * 2),
        "{before_retry:?}",
    );
    let healed = drive_awareness(id, 426, AWARENESS_RENEWAL_INTERVAL_MILLIS * 2).unwrap();
    assert!(healed.outbound_changed, "{healed:?}");
    let replies = drain_protocol_replies(id, generation);
    assert_eq!(replies.len(), 1, "{replies:?}");
    let tombstone = decode_awareness_reply(&replies[0]);
    assert_eq!(tombstone.clients[&local_client].clock, published_clock + 1);
    raw.apply_update(tombstone).unwrap();
    assert_eq!(raw.state::<Value>(local_client), None);
    destroy_session(id);
}

#[test]
fn withdrawal_review_fix_pending_tombstone_survives_close_detach_and_reconnect() {
    type LifecycleAction = fn(u64, u64);
    let scenarios: [(&str, LifecycleAction); 2] = [
        ("retryable close", |id, generation| {
            collaboration_socket_close(id, 427, generation, CloseDisposition::Retryable, 0)
                .unwrap();
        }),
        ("detach and reattach", |id, _generation| {
            transport_detach(id, 428).unwrap();
            transport_reattach(id, 429).unwrap();
        }),
    ];

    for (label, lifecycle_action) in scenarios {
        let (id, snapshot) = create_ready_room();
        let generation = synchronize_ready_room(id, &snapshot);
        drive_awareness(id, 430, 0).unwrap();
        set_desired_awareness(id, 431, &json!({ "name": label }).to_string()).unwrap();
        let local = local_peer(id).unwrap();
        let local_client = ClientID::new(local.client_id);
        let mut raw = Awareness::new(Doc::new());
        for reply in drain_protocol_replies(id, generation) {
            raw.apply_update(decode_awareness_reply(&reply)).unwrap();
        }
        let published_clock = raw.meta(local_client).unwrap().0;

        bridge::set_outbox_ceilings(id, 1, 10 * 1024 * 1024).unwrap();
        receive_message(id, 432, generation, &query_awareness_message()).unwrap();
        let error = clear_desired_awareness(id, 433).unwrap_err();
        assert_eq!(
            error.code, TRANSPORT_REPLY_LIMIT_EXCEEDED,
            "{label}: {error:?}"
        );
        // A full queue cannot reserve the new socket's Step 1. Preserve the
        // earlier saturation assertion, then make room for its actual
        // production-shaped reconnect sequence.
        // Two protocol frames plus the protected document slot.
        bridge::set_outbox_ceilings(id, 3, 10 * 1024 * 1024).unwrap();
        lifecycle_action(id, generation);
        assert_eq!(desired_awareness(id).unwrap(), None, "{label}");
        assert!(local_peer(id).is_none(), "{label}");

        // The close releases any active lease but preserves the frame. The
        // reconnect drains that retained blocker before its newly queued
        // Step 1, then completes the normal Step 2 handshake.
        let new_generation =
            synchronize_ready_room_after_draining_retained_protocol_reply(id, &snapshot);
        assert_eq!(
            drain_protocol_replies(id, new_generation).len(),
            0,
            "{label}"
        );
        let before_retry = drive_awareness(id, 434, AWARENESS_RENEWAL_INTERVAL_MILLIS - 1).unwrap();
        assert_eq!(
            before_retry.next_deadline_millis,
            Some(AWARENESS_RENEWAL_INTERVAL_MILLIS),
            "{label}: {before_retry:?}",
        );
        let healed = drive_awareness(id, 435, AWARENESS_RENEWAL_INTERVAL_MILLIS).unwrap();
        assert!(healed.outbound_changed, "{label}: {healed:?}");
        let replies = drain_protocol_replies(id, new_generation);
        assert_eq!(replies.len(), 1, "{label}: {replies:?}");
        let tombstone = decode_awareness_reply(&replies[0]);
        assert_eq!(
            tombstone.clients[&local_client].clock,
            published_clock + 1,
            "{label}",
        );
        raw.apply_update(tombstone).unwrap();
        assert_eq!(raw.state::<Value>(local_client), None, "{label}");
        destroy_session(id);
    }
}

#[test]
fn desired_awareness_survives_disconnect_and_republishes_with_a_newer_clock() {
    let (id, snapshot) = create_ready_room();
    let generation = synchronize_ready_room(id, &snapshot);
    let desired = json!({ "name": "resilient" });
    set_desired_awareness(id, 421, &desired.to_string()).unwrap();
    let local = local_peer(id).unwrap();
    let local_client = ClientID::new(local.client_id);

    // A raw peer observes our published state at its current clock.
    let mut raw = Awareness::new(Doc::new());
    for reply in drain_protocol_replies(id, generation) {
        raw.apply_update(decode_awareness_reply(&reply)).unwrap();
    }
    let (observed_clock, _) = raw.meta(local_client).unwrap();

    // The peer sees us disappear: standard removal bumps its tombstone
    // clock one past the observed clock.
    raw.remove_state(local_client);
    let (tombstone_clock, _) = raw.meta(local_client).unwrap();
    assert_eq!(tombstone_clock, observed_clock + 1);
    assert_eq!(raw.state::<Value>(local_client), None);

    // Generation close: desired local awareness survives, remote peers do
    // not (transport-scoped).
    collaboration_socket_close(id, 422, generation, CloseDisposition::Retryable, 0).unwrap();
    assert_eq!(desired_awareness(id).unwrap(), Some(desired.clone()));

    // Reconnect + handshake completion re-publishes with a fresh clock; the
    // peer that tombstoned us sees us again with a strictly newer clock.
    let generation = synchronize_ready_room(id, &snapshot);
    let replies = drain_protocol_replies(id, generation);
    assert!(
        !replies.is_empty(),
        "handshake completion must re-publish the desired awareness",
    );
    for reply in replies {
        raw.apply_update(decode_awareness_reply(&reply)).unwrap();
    }
    assert_eq!(
        raw.state::<Value>(local_client),
        Some(desired),
        "the re-publish must overcome the removal tombstone",
    );
    let (republished_clock, _) = raw.meta(local_client).unwrap();
    assert!(
        republished_clock > tombstone_clock,
        "re-published clock {republished_clock} must exceed the tombstone clock {tombstone_clock}",
    );
    destroy_session(id);
}

#[test]
fn reconnect_after_cleanup_and_undo_preserves_clock_or_requires_fresh_identity() {
    let limits = CollaborationLimits::default();

    for (initial_clock, expected_clock) in [(1, Some(3)), (u32::MAX - 1, None)] {
        let mut engine = source_engine();
        engine
            .apply_command(
                424,
                crate::yrs_engine::TypedCommand::InsertText {
                    text: "undoable reconnect".into(),
                },
            )
            .unwrap();
        let mut runtime = CollaborationRuntime::new(&limits);
        runtime
            .set_desired_awareness_for_test(
                425,
                r#"{"name":"reconnecting after undo"}"#,
                crate::collaboration_runtime::awareness::AwarenessContext {
                    engine: &mut engine,
                    transport_state: RuntimeTransportState::Synchronized,
                    limits: &limits,
                },
            )
            .unwrap();
        engine
            .awareness()
            .set_live_local_clock_for_test(initial_clock);
        runtime.clear_transport_peers(&mut engine);
        engine.undo(426).unwrap().expect("undo applies");

        match expected_clock {
            Some(expected_clock) => {
                let frame = runtime
                    .prepare_handshake_republish(&mut engine, &limits)
                    .unwrap()
                    .expect("desired awareness re-publishes");
                let update = decode_awareness_reply(&frame);
                let entry = &update.clients[&ClientID::new(engine.client_id())];
                assert_eq!(entry.clock, expected_clock);
                assert_ne!(entry.clock, 1, "same identity must not restart its clock");
            }
            None => {
                let error = runtime
                    .prepare_handshake_republish(&mut engine, &limits)
                    .unwrap_err();
                assert_eq!(error.code, "AWARENESS_CLOCK_EXHAUSTED", "{error:?}");
                assert_eq!(error.details.as_ref().unwrap()["retryable"], false);
                assert_eq!(runtime.outbox().pending_protocol_reply_count(), 1);
            }
        }
    }
}

#[test]
fn desired_awareness_republishes_past_two_reconnect_tombstones_with_inductive_clocks() {
    let (id, snapshot) = create_ready_room();
    let (mut generation, mut now_millis) = synchronize_ready_room_at(id, &snapshot, 0);
    let desired = json!({ "name": "twice resilient" });
    set_desired_awareness(id, 471, &desired.to_string()).unwrap();
    let local_client = ClientID::new(local_peer(id).unwrap().client_id);

    // A raw peer observes the initial publish: the induction base clock.
    let mut raw = Awareness::new(Doc::new());
    for reply in drain_protocol_replies(id, generation) {
        raw.apply_update(decode_awareness_reply(&reply)).unwrap();
    }
    let initial_clock = raw.meta(local_client).unwrap().0;
    let mut publish_clock = initial_clock;

    // Two close→connect cycles. Each close lands the peer's removal
    // tombstone exactly one past the last publish; each reconnect re-publish
    // strictly exceeds the peer's newest tombstone, so the publish clock
    // gains at least +2 per cycle by induction.
    for (cycle, close_request) in [(1, 472), (2, 473)] {
        raw.remove_state(local_client);
        let (tombstone_clock, _) = raw.meta(local_client).unwrap();
        assert_eq!(
            tombstone_clock,
            publish_clock + 1,
            "cycle {cycle}: the removal tombstone lands one past the last publish",
        );
        assert_eq!(raw.state::<Value>(local_client), None);

        collaboration_socket_close(
            id,
            close_request,
            generation,
            CloseDisposition::Retryable,
            now_millis,
        )
        .unwrap();
        assert_eq!(
            desired_awareness(id).unwrap(),
            Some(desired.clone()),
            "cycle {cycle}: desired local awareness survives the close",
        );

        (generation, now_millis) = synchronize_ready_room_at(id, &snapshot, now_millis);
        let replies = drain_protocol_replies(id, generation);
        assert!(
            !replies.is_empty(),
            "cycle {cycle}: handshake completion must re-publish the desired awareness",
        );
        for reply in replies {
            raw.apply_update(decode_awareness_reply(&reply)).unwrap();
        }
        assert_eq!(
            raw.state::<Value>(local_client),
            Some(desired.clone()),
            "cycle {cycle}: the local state is visible again",
        );
        let (republished_clock, _) = raw.meta(local_client).unwrap();
        assert!(
            republished_clock > tombstone_clock,
            "cycle {cycle}: re-publish clock {republished_clock} must exceed the newest tombstone {tombstone_clock} (+2 per cycle by induction)",
        );
        publish_clock = republished_clock;
    }
    assert!(
        publish_clock >= initial_clock + 4,
        "two cycles of +2 each: {publish_clock} vs base {initial_clock}",
    );
    let _ = generation;
    destroy_session(id);
}

#[test]
fn remote_peers_clear_on_every_generation_close_while_desired_awareness_survives() {
    /// One generation-closing transition under test.
    type CloseAction = fn(u64, u64);
    let desired = json!({ "name": "still here" });
    let scenarios: [(&str, CloseAction); 4] = [
        ("retryable close", |id, generation| {
            collaboration_socket_close(id, 431, generation, CloseDisposition::Retryable, 0)
                .unwrap();
        }),
        ("incompatible close", |id, generation| {
            collaboration_socket_close(id, 432, generation, CloseDisposition::Incompatible, 0)
                .unwrap();
        }),
        ("local disconnect", |id, _generation| {
            transport_disconnect(id, 433).unwrap();
        }),
        ("detach and reattach", |id, _generation| {
            transport_detach(id, 434).unwrap();
            transport_reattach(id, 435).unwrap();
        }),
    ];

    for (label, close_action) in scenarios {
        let (id, snapshot) = create_ready_room();
        let generation = synchronize_ready_room(id, &snapshot);
        set_desired_awareness(id, 436, &desired.to_string()).unwrap();
        receive_message(
            id,
            437,
            generation,
            &awareness_message(&[(6_501, 1, r#"{"name":"transient"}"#)]),
        )
        .unwrap();
        assert_eq!(remote_peers(id).len(), 1, "{label}");

        close_action(id, generation);
        assert!(
            remote_peers(id).is_empty(),
            "{label}: remote peers are transport-scoped",
        );
        assert_eq!(
            desired_awareness(id).unwrap(),
            Some(desired.clone()),
            "{label}: desired local awareness survives",
        );
        destroy_session(id);
    }
}

#[test]
fn receive_failure_closes_also_clear_transport_peers() {
    let (id, snapshot) = create_ready_room();
    let generation = synchronize_ready_room(id, &snapshot);
    let desired = json!({ "name": "keeper" });
    set_desired_awareness(id, 441, &desired.to_string()).unwrap();
    receive_message(
        id,
        442,
        generation,
        &awareness_message(&[(6_601, 1, r#"{"name":"doomed"}"#)]),
    )
    .unwrap();
    assert_eq!(remote_peers(id).len(), 1);

    let outcome = receive_message(id, 443, generation, &[0xff]).unwrap();
    assert!(outcome.close.is_some(), "{outcome:?}");
    assert!(
        remote_peers(id).is_empty(),
        "a protocol-failure close clears transport-scoped peers",
    );
    assert_eq!(desired_awareness(id).unwrap(), Some(desired));
    destroy_session(id);
}

// Tombstone semantics through the runtime path

#[test]
fn tombstoned_peers_stay_excluded_until_a_strictly_newer_clock_reappears() {
    let (id, snapshot) = create_ready_room();
    let generation = synchronize_ready_room(id, &snapshot);

    receive_message(
        id,
        451,
        generation,
        &awareness_message(&[(6_701, 1, r#"{"name":"flicker"}"#)]),
    )
    .unwrap();
    assert_eq!(remote_peers(id).len(), 1);

    // Removal tombstone: the peer leaves the public projection.
    receive_message(
        id,
        452,
        generation,
        &awareness_message(&[(6_701, 2, "null")]),
    )
    .unwrap();
    assert!(
        remote_peers(id).is_empty(),
        "tombstones are excluded from peers()"
    );

    // The tombstone clock is preserved internally: an equal-clock
    // re-announce stays invisible.
    receive_message(
        id,
        453,
        generation,
        &awareness_message(&[(6_701, 2, r#"{"name":"too old"}"#)]),
    )
    .unwrap();
    assert!(
        remote_peers(id).is_empty(),
        "an equal-clock re-announce must lose against the preserved tombstone clock",
    );

    // A strictly newer clock reappears.
    receive_message(
        id,
        454,
        generation,
        &awareness_message(&[(6_701, 3, r#"{"name":"reborn"}"#)]),
    )
    .unwrap();
    let peers = remote_peers(id);
    assert_eq!(peers.len(), 1, "{peers:?}");
    assert_eq!(peers[0].clock, 3);
    assert_eq!(peers[0].state, json!({ "name": "reborn" }));
    destroy_session(id);
}

#[test]
fn remote_removal_echo_of_the_local_state_does_not_take_clock_ownership() {
    let (id, snapshot) = create_ready_room();
    let generation = synchronize_ready_room(id, &snapshot);
    let desired = json!({ "name": "undeletable" });
    set_desired_awareness(id, 461, &desired.to_string()).unwrap();
    let local = local_peer(id).unwrap();

    // A remote tombstone targeting our own client is an accepted echo, but
    // it cannot remove the state or advance the locally owned clock.
    receive_message(
        id,
        462,
        generation,
        &awareness_message(&[(local.client_id, local.clock, "null")]),
    )
    .unwrap();
    let defended = local_peer(id).expect("the local state survives a remote removal");
    assert_eq!(defended.state, desired);
    assert_eq!(defended.clock, local.clock);
    assert_eq!(desired_awareness(id).unwrap(), Some(desired));
    destroy_session(id);
}

#[test]
fn remote_cannot_advance_the_local_client_clock() {
    let limits = CollaborationLimits::default();
    let mut engine = source_engine();
    let mut runtime = CollaborationRuntime::new(&limits);
    runtime
        .set_desired_awareness_for_test(
            463,
            r#"{"name":"locally owned"}"#,
            crate::collaboration_runtime::awareness::AwarenessContext {
                engine: &mut engine,
                transport_state: RuntimeTransportState::Synchronized,
                limits: &limits,
            },
        )
        .unwrap();
    let local = runtime
        .peers(&mut engine)
        .into_iter()
        .find(|peer| peer.is_local)
        .unwrap();
    runtime
        .apply_awareness_frame(
            &mut engine,
            &limits,
            &decode_awareness_reply(&awareness_message(&[(6_702, 7, r#"{"name":"peer"}"#)]))
                .encode_v1(),
        )
        .unwrap();
    let before_peers = runtime.peers(&mut engine);
    let before_reply_count = runtime.outbox().pending_protocol_reply_count();
    let before_reply_bytes = runtime.outbox().pending_protocol_reply_bytes();

    for clock in [local.clock, local.clock.saturating_sub(1)] {
        runtime
            .apply_awareness_frame(
                &mut engine,
                &limits,
                &decode_awareness_reply(&awareness_message(&[(
                    local.client_id,
                    clock,
                    r#"{"name":"remote echo"}"#,
                )]))
                .encode_v1(),
            )
            .unwrap();
        assert_eq!(runtime.peers(&mut engine), before_peers);
        assert_eq!(
            runtime.outbox().pending_protocol_reply_count(),
            before_reply_count,
        );
        assert_eq!(
            runtime.outbox().pending_protocol_reply_bytes(),
            before_reply_bytes,
        );
    }

    for clock in [local.clock + 1, u32::MAX] {
        let error = runtime
            .apply_awareness_frame(
                &mut engine,
                &limits,
                &decode_awareness_reply(&awareness_message(&[(
                    local.client_id,
                    clock,
                    r#"{"name":"remote takeover"}"#,
                )]))
                .encode_v1(),
            )
            .unwrap_err();
        assert_eq!(error.code, "INPUT_LIMIT_EXCEEDED", "{error:?}");
        assert_eq!(
            error.details.as_ref().unwrap()["field"],
            "awarenessClock",
            "{error:?}",
        );
        assert_eq!(runtime.peers(&mut engine), before_peers);
        assert_eq!(
            runtime.outbox().pending_protocol_reply_count(),
            before_reply_count,
        );
        assert_eq!(
            runtime.outbox().pending_protocol_reply_bytes(),
            before_reply_bytes,
        );
    }
}
