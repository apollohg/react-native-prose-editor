#[test]
fn frame_count_ceiling_has_exact_and_one_over_behavior() {
    let (id, snapshot) = create_ready_room();
    let generation = synchronize_ready_room(id, &snapshot);
    set_collaboration_limit_for_test(id, "maxFramesPerMessage", 3).unwrap();

    let noop = || update_frame(NOOP_UPDATE_V1.to_vec());
    let exact = concat_frames(&[noop(), noop(), noop()]);
    let outcome = receive_message(id, 291, generation, &exact).unwrap();
    assert!(outcome.close.is_none(), "{outcome:?}");
    assert_eq!(outcome.frames_decoded, 3, "{outcome:?}");

    let over = concat_frames(&[noop(), noop(), noop(), noop()]);
    let outcome = receive_message(id, 292, generation, &over).unwrap();
    let close = outcome
        .close
        .as_ref()
        .expect("frame-count overflow must close");
    assert_eq!(
        close.disposition,
        CloseDisposition::Incompatible,
        "{close:?}"
    );
    assert_eq!(
        close.error.code, TRANSPORT_FRAME_LIMIT_EXCEEDED,
        "{close:?}"
    );
    let details = close.error.details.as_ref().unwrap();
    assert_eq!(details["field"], "maxFramesPerMessage", "{details}");
    assert_eq!(details["limit"], 3, "{details}");
    assert_eq!(transport_state(id).unwrap(), TransportState::Incompatible);

    destroy_session(id);
}

#[test]
fn frame_bytes_ceiling_has_exact_and_one_over_behavior() {
    let (id, snapshot) = create_ready_room();
    let generation = synchronize_ready_room(id, &snapshot);

    let message = update_frame(NOOP_UPDATE_V1.to_vec());
    set_collaboration_limit_for_test(id, "maxFrameBytes", message.len()).unwrap();
    let outcome = receive_message(id, 301, generation, &message).unwrap();
    assert!(outcome.close.is_none(), "{outcome:?}");

    set_collaboration_limit_for_test(id, "maxFrameBytes", message.len() - 1).unwrap();
    let outcome = receive_message(id, 302, generation, &message).unwrap();
    let close = outcome
        .close
        .as_ref()
        .expect("frame-bytes overflow must close");
    assert_eq!(
        close.disposition,
        CloseDisposition::Incompatible,
        "{close:?}"
    );
    assert_eq!(
        close.error.code, TRANSPORT_FRAME_LIMIT_EXCEEDED,
        "{close:?}"
    );
    let details = close.error.details.as_ref().unwrap();
    assert_eq!(details["field"], "maxFrameBytes", "{details}");
    assert_eq!(details["limit"], message.len() - 1, "{details}");
    assert_eq!(details["actual"], message.len(), "{details}");
    assert_eq!(transport_state(id).unwrap(), TransportState::Incompatible);

    destroy_session(id);
}

#[test]
fn reply_aggregate_ceiling_has_exact_and_one_over_behavior() {
    let (id, snapshot) = create_ready_room();
    let generation = synchronize_ready_room(id, &snapshot);
    let step1 = step1_frame(&StateVector::default().encode_v1());

    // Measure the deterministic reply size first.
    let outcome = receive_message(id, 311, generation, &step1).unwrap();
    assert!(outcome.close.is_none(), "{outcome:?}");
    let reply_bytes = outcome.reply_bytes_enqueued;
    assert!(reply_bytes > 0, "{outcome:?}");
    let reply = lease_outbound(id, 314, generation).unwrap().unwrap();
    ack_outbound(id, 315, generation, reply.lease_id).unwrap();

    set_collaboration_limit_for_test(id, "maxAggregateResponseBytes", reply_bytes).unwrap();
    let outcome = receive_message(id, 312, generation, &step1).unwrap();
    assert!(outcome.close.is_none(), "{outcome:?}");
    assert_eq!(outcome.reply_bytes_enqueued, reply_bytes, "{outcome:?}");
    let reply = lease_outbound(id, 316, generation).unwrap().unwrap();
    ack_outbound(id, 317, generation, reply.lease_id).unwrap();

    set_collaboration_limit_for_test(id, "maxAggregateResponseBytes", reply_bytes - 1).unwrap();
    let before_engine = bridge::session_audit(id).unwrap();
    let outcome = receive_message(id, 313, generation, &step1).unwrap();
    let close = outcome
        .close
        .as_ref()
        .expect("reply-aggregate overflow must close");
    assert_eq!(
        close.disposition,
        CloseDisposition::Incompatible,
        "{close:?}"
    );
    assert_eq!(
        close.error.code, TRANSPORT_REPLY_LIMIT_EXCEEDED,
        "{close:?}"
    );
    assert_eq!(outcome.replies_enqueued, 0, "{outcome:?}");
    let error = lease_outbound(id, 318, generation).unwrap_err();
    assert_eq!(error.domain, "transport", "{error:?}");
    assert_eq!(error.code, TRANSPORT_STALE_GENERATION, "{error:?}");
    assert_eq!(bridge::session_audit(id).unwrap(), before_engine);

    destroy_session(id);
}

#[test]
fn reply_reservation_failures_close_before_any_engine_commit() {
    let coupled_message = |id: u64| {
        let server = RawPeer::from_snapshot(&snapshot_source());
        server.push_text(" coupled");
        concat_frames(&[
            step1_frame(&StateVector::default().encode_v1()),
            update_frame(server.diff_for(&session_state_vector_bytes(id))),
        ])
    };

    // Saturation of the SHARED outbox ceilings: pending offline document
    // updates drain over time, so retry can change the result — the close
    // is retryable, never Incompatible.
    let (id, snapshot) = create_ready_room();
    let generation = synchronize_ready_room(id, &snapshot);
    bridge::set_outbox_ceilings(id, 0, 0).unwrap();
    let before_engine = bridge::session_audit(id).unwrap();
    let outcome = receive_message(id, 321, generation, &coupled_message(id)).unwrap();
    let close = outcome
        .close
        .as_ref()
        .expect("saturated reply reservation must close");
    assert_eq!(close.disposition, CloseDisposition::Retryable, "{close:?}");
    assert_eq!(
        close.error.code, TRANSPORT_REPLY_LIMIT_EXCEEDED,
        "{close:?}"
    );
    assert!(!outcome.remote_commit_applied, "{outcome:?}");
    assert_eq!(
        bridge::session_audit(id).unwrap(),
        before_engine,
        "the coupled update must not commit when its reply cannot be reserved",
    );
    assert_eq!(transport_state(id).unwrap(), TransportState::Disconnected);
    destroy_session(id);

    // Recoverable allocation failure: retryable close, same atomicity.
    let (id, snapshot) = create_ready_room();
    let generation = synchronize_ready_room(id, &snapshot);
    let before_engine = bridge::session_audit(id).unwrap();
    bridge::set_outbox_allocation_failure(true);
    let outcome = receive_message(id, 322, generation, &coupled_message(id)).unwrap();
    bridge::set_outbox_allocation_failure(false);
    let close = outcome
        .close
        .as_ref()
        .expect("allocation failure must close");
    assert_eq!(close.disposition, CloseDisposition::Retryable, "{close:?}");
    assert_eq!(close.error.code, TRANSPORT_RESOURCE_EXHAUSTED, "{close:?}");
    assert!(!outcome.remote_commit_applied, "{outcome:?}");
    assert_eq!(bridge::session_audit(id).unwrap(), before_engine);
    assert_eq!(transport_state(id).unwrap(), TransportState::Disconnected);
    destroy_session(id);
}

/// The retryable-saturation contract end to end: an outbox filled by
/// offline document edits refuses a reply reservation with a retryable
/// close, the queue drains through the normal transport pickup, and the
/// SAME session reconnects and completes the handshake — no `Incompatible`
/// wedge, no detach/reattach required.
#[test]
fn outbox_saturation_drains_and_the_transport_reconnects_without_a_wedge() {
    let (id, snapshot) = create_ready_room();
    // Two slots let the production socket-open queue and ACK its Sync Step
    // 1 alongside the offline edit. Before the peer's Step 1 arrives, put
    // the ceiling back at one so its reply reservation still saturates.
    bridge::set_outbox_ceilings(id, 2, 10 * 1024 * 1024).unwrap();
    local_edit(id, 401, "offline edit");
    let (pending_count, _) = bridge::outbox_pending(id).unwrap().unwrap();
    assert_eq!(pending_count, 1, "the offline edit must occupy one slot");

    // Reconnect: the peer's Step 1 cannot reserve its reply — retryable.
    let generation = handshake(id);
    bridge::set_outbox_ceilings(id, 1, 10 * 1024 * 1024).unwrap();
    let step1 = step1_frame(&StateVector::default().encode_v1());
    let outcome = receive_message(id, 402, generation, &step1).unwrap();
    let close = outcome
        .close
        .as_ref()
        .expect("a full shared outbox must refuse the reply");
    assert_eq!(close.disposition, CloseDisposition::Retryable, "{close:?}");
    assert_eq!(
        close.error.code, TRANSPORT_REPLY_LIMIT_EXCEEDED,
        "{close:?}"
    );
    assert_eq!(transport_state(id).unwrap(), TransportState::Disconnected);

    // The pending offline edit drains through the normal pickup seam.
    let lease = bridge::lease_next_update(id).unwrap().unwrap();
    assert_eq!(lease.request_id, 401);
    bridge::ack_leased_update(id, lease.lease_id).unwrap();
    assert_eq!(bridge::outbox_pending(id).unwrap().unwrap(), (0, 0));

    // The same session's due Rust drive reconnects (not wedged in
    // Incompatible) and completes the full handshake.
    let generation = synchronize_ready_room(id, &snapshot);
    let outcome = receive_message(id, 403, generation, &step1).unwrap();
    assert!(outcome.close.is_none(), "{outcome:?}");
    assert_eq!(outcome.replies_enqueued, 1, "{outcome:?}");
    assert_eq!(transport_state(id).unwrap(), TransportState::Synchronized);

    destroy_session(id);
}
