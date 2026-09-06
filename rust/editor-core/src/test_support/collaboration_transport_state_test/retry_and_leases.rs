#[test]
fn drive_starts_initial_generation_and_owns_exponential_retry_deadlines() {
    let id = create_ready_room_with_runtime();
    let mut now_millis = 0;
    let mut generation = collaboration_drive(id, 20_001, now_millis)
        .unwrap()
        .generation_to_open
        .expect("the initial attached drive opens generation one immediately");
    assert_eq!(generation, 1);

    for delay in [500, 1_000, 2_000, 4_000, 8_000, 16_000, 30_000, 30_000] {
        let deadline = now_millis + delay;
        let closed = collaboration_socket_close(
            id,
            20_002,
            generation,
            CloseDisposition::Retryable,
            now_millis,
        )
        .unwrap();
        assert_eq!(closed.transport_state, TransportState::Disconnected);
        assert_eq!(closed.generation_to_open, None);
        assert_eq!(closed.next_deadline_millis, Some(deadline));

        let before = collaboration_drive(id, 20_003, deadline - 1).unwrap();
        assert_eq!(before.transport_state, TransportState::Disconnected);
        assert_eq!(before.generation_to_open, None);
        assert_eq!(before.next_deadline_millis, Some(deadline));

        let due = collaboration_drive(id, 20_004, deadline).unwrap();
        assert_eq!(due.transport_state, TransportState::Connecting);
        assert_eq!(due.next_deadline_millis, None);
        generation = due
            .generation_to_open
            .expect("only Rust's due drive may issue a retry generation");
        now_millis = deadline;
    }
    assert_eq!(generation, 9, "the 30-second retry delay remains capped");
    destroy_session(id);
}

#[test]
fn retry_deadline_overflow_parks_without_busy_loop_or_outbox_loss_until_reattach() {
    let id = create_ready_room_with_runtime();
    let revision = bridge::session_audit(id).unwrap().document_revision;
    bridge::submit_input(
        id,
        &serde_json::json!({
            "version": 1,
            "requestId": "20",
            "baseDocumentRevision": revision.to_string(),
            "text": " retained",
        })
        .to_string(),
    )
    .unwrap();
    let pending = bridge::outbox_pending(id).unwrap().unwrap();

    let generation = collaboration_drive(id, 20_050, 0)
        .unwrap()
        .generation_to_open
        .unwrap();
    collaboration_socket_open(id, 20_051, generation, 0).unwrap();
    let step1 = lease_outbound(id, 20_052, generation).unwrap().unwrap();
    ack_outbound(id, 20_053, generation, step1.lease_id).unwrap();
    let retained_document = lease_outbound(id, 20_054, generation).unwrap().unwrap();

    let closed = collaboration_socket_close(
        id,
        20_055,
        generation,
        CloseDisposition::Retryable,
        u64::MAX,
    )
    .unwrap();
    assert_eq!(closed.transport_state, TransportState::Disconnected);
    assert_eq!(closed.generation_to_open, None);
    assert_eq!(closed.next_deadline_millis, None);
    assert_eq!(bridge::outbox_pending(id).unwrap().unwrap(), pending);

    for request_id in [20_056, 20_057] {
        let parked = collaboration_drive(id, request_id, u64::MAX).unwrap();
        assert_eq!(parked.transport_state, TransportState::Disconnected);
        assert_eq!(parked.generation_to_open, None);
        assert_eq!(parked.next_deadline_millis, None);
        assert_eq!(bridge::outbox_pending(id).unwrap().unwrap(), pending);
    }

    transport_detach(id, 20_058).unwrap();
    transport_reattach(id, 20_059).unwrap();
    let resumed_generation = collaboration_drive(id, 20_060, u64::MAX)
        .unwrap()
        .generation_to_open
        .expect("reattach resets an exhausted retry schedule for a fresh drive");
    collaboration_socket_open(id, 20_061, resumed_generation, u64::MAX).unwrap();
    let resumed_step1 = lease_outbound(id, 20_062, resumed_generation)
        .unwrap()
        .unwrap();
    ack_outbound(id, 20_063, resumed_generation, resumed_step1.lease_id).unwrap();
    let resumed_document = lease_outbound(id, 20_064, resumed_generation)
        .unwrap()
        .unwrap();
    assert_eq!(resumed_document.frame, retained_document.frame);
    ack_outbound(id, 20_065, resumed_generation, resumed_document.lease_id).unwrap();
    assert_eq!(bridge::outbox_pending(id).unwrap().unwrap(), (0, 0));
    destroy_session(id);
}

#[test]
fn close_detach_and_generation_retirement_release_without_consuming_a_lease() {
    let id = create_ready_room_with_runtime();
    let revision = bridge::session_audit(id).unwrap().document_revision;
    bridge::submit_input(
        id,
        &serde_json::json!({
            "version": 1,
            "requestId": "20",
            "baseDocumentRevision": revision.to_string(),
            "text": " retained",
        })
        .to_string(),
    )
    .unwrap();
    let pending_before = bridge::outbox_pending(id).unwrap().unwrap();
    assert_eq!(pending_before.0, 1);

    let first_generation = collaboration_drive(id, 20_100, 0)
        .unwrap()
        .generation_to_open
        .unwrap();
    collaboration_socket_open(id, 20_101, first_generation, 0).unwrap();
    let step1 = lease_outbound(id, 20_102, first_generation)
        .unwrap()
        .unwrap();
    ack_outbound(id, 20_103, first_generation, step1.lease_id).unwrap();
    let first_document = lease_outbound(id, 20_104, first_generation)
        .unwrap()
        .unwrap();

    let closed = collaboration_socket_close(
        id,
        20_105,
        first_generation,
        CloseDisposition::Retryable,
        10,
    )
    .unwrap();
    assert_eq!(closed.next_deadline_millis, Some(510));
    assert_eq!(bridge::outbox_pending(id).unwrap().unwrap(), pending_before);

    let second_generation = collaboration_drive(id, 20_106, 510)
        .unwrap()
        .generation_to_open
        .unwrap();
    collaboration_socket_open(id, 20_107, second_generation, 510).unwrap();
    let second_step1 = lease_outbound(id, 20_108, second_generation)
        .unwrap()
        .unwrap();
    ack_outbound(id, 20_109, second_generation, second_step1.lease_id).unwrap();
    let second_document = lease_outbound(id, 20_110, second_generation)
        .unwrap()
        .unwrap();
    assert_ne!(second_document.lease_id, first_document.lease_id);
    assert_eq!(second_document.frame, first_document.frame);

    transport_detach(id, 20_111).unwrap();
    assert_eq!(bridge::outbox_pending(id).unwrap().unwrap(), pending_before);
    transport_reattach(id, 20_112).unwrap();
    let third_generation = collaboration_drive(id, 20_113, 511)
        .unwrap()
        .generation_to_open
        .unwrap();
    collaboration_socket_open(id, 20_114, third_generation, 511).unwrap();
    let third_step1 = lease_outbound(id, 20_115, third_generation)
        .unwrap()
        .unwrap();
    ack_outbound(id, 20_116, third_generation, third_step1.lease_id).unwrap();
    let third_document = lease_outbound(id, 20_117, third_generation)
        .unwrap()
        .unwrap();
    assert_ne!(third_document.lease_id, second_document.lease_id);
    assert_eq!(third_document.frame, first_document.frame);
    assert_eq!(bridge::outbox_pending(id).unwrap().unwrap(), pending_before);

    ack_outbound(id, 20_118, third_generation, third_document.lease_id).unwrap();
    assert_eq!(bridge::outbox_pending(id).unwrap().unwrap(), (0, 0));
    destroy_session(id);
}

#[test]
fn stale_ack_nack_and_socket_callbacks_are_observationally_pure() {
    let id = create_ready_room_with_runtime();
    let generation = collaboration_drive(id, 20_200, 0)
        .unwrap()
        .generation_to_open
        .unwrap();
    collaboration_socket_open(id, 20_201, generation, 0).unwrap();
    let retained = lease_outbound(id, 20_202, generation).unwrap().unwrap();
    let before_state = transport_state(id).unwrap();

    for error in [
        ack_outbound(id, 20_203, FABRICATED_GENERATION, retained.lease_id).unwrap_err(),
        nack_outbound(id, 20_204, FABRICATED_GENERATION, retained.lease_id).unwrap_err(),
        collaboration_socket_open(id, 20_205, FABRICATED_GENERATION, 1).unwrap_err(),
        collaboration_socket_close(
            id,
            20_206,
            FABRICATED_GENERATION,
            CloseDisposition::Retryable,
            1,
        )
        .unwrap_err(),
        collaboration_receive(id, 20_207, FABRICATED_GENERATION, &[0xff], 1).unwrap_err(),
    ] {
        assert_eq!(error.code, TRANSPORT_STALE_GENERATION, "{error:?}");
    }

    assert_eq!(transport_state(id).unwrap(), before_state);
    let re_leased = lease_outbound(id, 20_208, generation).unwrap().unwrap();
    assert_eq!(re_leased, retained);
    nack_outbound(id, 20_209, generation, retained.lease_id).unwrap();
    destroy_session(id);
}

#[test]
fn synchronization_resets_retry_and_incompatible_has_no_deadline() {
    let id = create_ready_room_with_runtime();
    let initial = collaboration_drive(id, 20_300, 0)
        .unwrap()
        .generation_to_open
        .unwrap();
    let first_close =
        collaboration_socket_close(id, 20_301, initial, CloseDisposition::Retryable, 0).unwrap();
    assert_eq!(first_close.next_deadline_millis, Some(500));

    let synchronized_generation = collaboration_drive(id, 20_302, 500)
        .unwrap()
        .generation_to_open
        .unwrap();
    collaboration_socket_open(id, 20_303, synchronized_generation, 500).unwrap();
    let synchronized = collaboration_receive(
        id,
        20_304,
        synchronized_generation,
        &Message::Sync(SyncMessage::SyncStep2(vec![0, 0])).encode_v1(),
        500,
    )
    .unwrap();
    assert_eq!(synchronized.transport_state, TransportState::Synchronized);

    let reset = collaboration_socket_close(
        id,
        20_305,
        synchronized_generation,
        CloseDisposition::Retryable,
        500,
    )
    .unwrap();
    assert_eq!(reset.next_deadline_millis, Some(1_000));

    let incompatible_generation = collaboration_drive(id, 20_306, 1_000)
        .unwrap()
        .generation_to_open
        .unwrap();
    let incompatible = collaboration_socket_close(
        id,
        20_307,
        incompatible_generation,
        CloseDisposition::Incompatible,
        1_000,
    )
    .unwrap();
    assert_eq!(incompatible.transport_state, TransportState::Incompatible);
    assert_eq!(incompatible.next_deadline_millis, None);
    let parked = collaboration_drive(id, 20_308, 1_000_000).unwrap();
    assert_eq!(parked.transport_state, TransportState::Incompatible);
    assert_eq!(parked.generation_to_open, None);
    assert_eq!(parked.next_deadline_millis, None);
    destroy_session(id);
}
