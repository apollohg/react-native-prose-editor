use super::*;

#[test]
fn from_limits_uses_the_configured_outbox_ceilings() {
    let limits = CollaborationLimits {
        max_pending_outbox_messages: 3,
        max_pending_outbox_bytes: 32,
        ..CollaborationLimits::default()
    };
    let mut outbox = CollaborationOutbox::from_limits(&limits);
    for request in 0..3 {
        let reservation = outbox.reserve_document_update(request, 10).unwrap();
        outbox.install(reservation, vec![0; 10]);
    }
    assert_eq!(
        outbox.reserve_document_update(4, 1).unwrap_err(),
        OutboxReservationError::Saturated {
            field: OUTBOX_MESSAGES_FIELD,
            limit: 3,
            actual: 4,
        },
    );
    assert_eq!(outbox.pending_document_update_bytes(), 30);
}

#[test]
fn install_charges_actual_length_and_acknowledged_lease_restores_it() {
    let mut outbox = CollaborationOutbox::with_ceilings(2, 64);
    let reservation = outbox.reserve_document_update(7, 40).unwrap();
    assert_eq!(outbox.reserved_bytes(), 40);
    outbox.install(reservation, vec![1; 5]);
    assert_eq!(outbox.reserved_bytes(), 0);
    assert_eq!(outbox.pending_document_update_bytes(), 5);
    let lease = outbox.lease_next().unwrap().unwrap();
    assert_eq!(
        outbox.pending_document_update_request_id_for_leased_front(),
        Some(7)
    );
    assert!(matches!(
        lease.payload,
        OutboundLeasePayload::DocumentUpdate(ref update) if update.len() == 5
    ));
    outbox.ack_lease(lease.lease_id).unwrap();
    assert_eq!(outbox.pending_document_update_bytes(), 0);
    assert!(outbox.lease_next().unwrap().is_none());
}

#[test]
fn dropping_reservations_releases_both_reservation_kinds() {
    let mut outbox = CollaborationOutbox::with_ceilings(4, 64);
    let document = outbox.reserve_document_update(1, 16).unwrap();
    let replies = outbox.reserve_protocol_replies(2, 20).unwrap();
    assert_eq!(replies.reply_count(), 2);
    assert_eq!(replies.upper_bound_bytes(), 20);
    assert_eq!(outbox.reserved_messages(), 3);
    assert_eq!(outbox.reserved_bytes(), 36);
    drop(document);
    drop(replies);
    assert_eq!(outbox.reserved_messages(), 0);
    assert_eq!(outbox.reserved_bytes(), 0);
}

#[test]
fn protocol_reply_install_and_acknowledged_lease_keep_separate_accounting() {
    let mut outbox = CollaborationOutbox::with_ceilings(4, 64);
    let reservation = outbox.reserve_protocol_replies(2, 20).unwrap();
    assert_eq!(outbox.reserved_messages(), 2);
    assert_eq!(outbox.reserved_bytes(), 20);

    outbox.install_protocol_replies(reservation, 9, vec![vec![1; 6], vec![2; 4]]);
    // Consumed reservation releases exactly once; pending accounting
    // charges actual framed lengths, separate from document entries.
    assert_eq!(outbox.reserved_messages(), 0);
    assert_eq!(outbox.reserved_bytes(), 0);
    assert_eq!(outbox.pending_protocol_reply_count(), 2);
    assert_eq!(outbox.pending_protocol_reply_bytes(), 10);
    assert_eq!(outbox.pending_document_update_count(), 0);
    assert_eq!(outbox.pending_document_update_bytes(), 0);

    let first = outbox.lease_next().unwrap().unwrap();
    assert!(matches!(
        first.payload,
        OutboundLeasePayload::ProtocolReply(ref reply) if reply == &vec![1; 6]
    ));
    outbox.ack_lease(first.lease_id).unwrap();
    assert_eq!(outbox.pending_protocol_reply_bytes(), 4);
    let second = outbox.lease_next().unwrap().unwrap();
    assert!(matches!(
        second.payload,
        OutboundLeasePayload::ProtocolReply(ref reply) if reply == &vec![2; 4]
    ));
    outbox.ack_lease(second.lease_id).unwrap();
    assert_eq!(outbox.pending_protocol_reply_count(), 0);
    assert_eq!(outbox.pending_protocol_reply_bytes(), 0);
    assert!(outbox.lease_next().unwrap().is_none());
}

#[test]
fn coalesced_awareness_broadcast_follows_the_document_update_it_references() {
    let mut outbox = CollaborationOutbox::with_ceilings(5, 64);

    let first_document = outbox.reserve_document_update(1, 2).unwrap();
    outbox.install(first_document, vec![1, 2]);
    let first_awareness = outbox.reserve_awareness_broadcast(2).unwrap();
    outbox.install_awareness_broadcast(first_awareness, 1, vec![11, 12]);

    let second_document = outbox.reserve_document_update(2, 2).unwrap();
    outbox.install(second_document, vec![3, 4]);
    let second_awareness = outbox.reserve_awareness_broadcast(2).unwrap();
    outbox.install_awareness_broadcast(second_awareness, 2, vec![13, 14]);

    let sync_reply = outbox.reserve_protocol_replies(1, 2).unwrap();
    outbox.install_protocol_replies(sync_reply, 3, vec![vec![21, 22]]);

    let expected = [
        OutboundLeasePayload::ProtocolReply(vec![21, 22]),
        OutboundLeasePayload::DocumentUpdate(vec![1, 2]),
        OutboundLeasePayload::DocumentUpdate(vec![3, 4]),
        OutboundLeasePayload::ProtocolReply(vec![13, 14]),
    ];
    for expected_payload in expected {
        let lease = outbox.lease_next().unwrap().unwrap();
        assert_eq!(lease.payload, expected_payload);
        outbox.ack_lease(lease.lease_id).unwrap();
    }
    assert!(outbox.lease_next().unwrap().is_none());
}

#[test]
fn pending_protocol_replies_share_the_configured_outbox_ceilings() {
    let mut outbox = CollaborationOutbox::with_ceilings(2, 16);
    let reservation = outbox.reserve_protocol_replies(1, 10).unwrap();
    outbox.install_protocol_replies(reservation, 3, vec![vec![7; 10]]);

    // Message-count ceiling counts the pending protocol entry.
    let reservation = outbox.reserve_document_update(4, 1).unwrap();
    assert_eq!(
        outbox.reserve_protocol_replies(1, 1).unwrap_err(),
        OutboxReservationError::Saturated {
            field: OUTBOX_MESSAGES_FIELD,
            limit: 2,
            actual: 3,
        },
    );
    drop(reservation);

    // Byte ceiling counts the pending protocol bytes.
    assert_eq!(
        outbox.reserve_document_update(5, 7).unwrap_err(),
        OutboxReservationError::Saturated {
            field: OUTBOX_BYTES_FIELD,
            limit: 16,
            actual: 17,
        },
    );
    outbox.reserve_document_update(5, 6).unwrap();
}

#[test]
fn clear_protocol_replies_drops_only_transport_scoped_entries() {
    let mut outbox = CollaborationOutbox::with_ceilings(4, 64);
    let document = outbox.reserve_document_update(1, 8).unwrap();
    outbox.install(document, vec![1; 8]);
    let replies = outbox.reserve_protocol_replies(2, 12).unwrap();
    outbox.install_protocol_replies(replies, 2, vec![vec![2; 7], vec![3; 5]]);
    assert_eq!(outbox.pending_protocol_reply_count(), 2);
    assert_eq!(outbox.pending_protocol_reply_bytes(), 12);

    outbox.clear_protocol_replies();

    assert_eq!(outbox.pending_protocol_reply_count(), 0);
    assert_eq!(outbox.pending_protocol_reply_bytes(), 0);
    // Document entries and their accounting are never touched.
    assert_eq!(outbox.pending_document_update_count(), 1);
    assert_eq!(outbox.pending_document_update_bytes(), 8);
    assert!(outbox.has_pending_document_updates());
    let document_lease = outbox.lease_next().unwrap().unwrap();
    assert!(matches!(
        document_lease.payload,
        OutboundLeasePayload::DocumentUpdate(ref update) if update == &vec![1; 8]
    ));
    outbox.ack_lease(document_lease.lease_id).unwrap();
    // Clearing an already empty queue is a no-op.
    outbox.clear_protocol_replies();
    assert_eq!(outbox.pending_protocol_reply_count(), 0);
}

#[test]
fn allocation_failpoint_rejects_both_reservation_kinds_atomically() {
    let mut outbox = CollaborationOutbox::with_ceilings(4, 64);
    set_reservation_allocation_failure_for_test(true);
    assert_eq!(
        outbox.reserve_document_update(1, 8).unwrap_err(),
        OutboxReservationError::Allocation,
    );
    assert_eq!(
        outbox.reserve_protocol_replies(1, 8).unwrap_err(),
        OutboxReservationError::Allocation,
    );
    set_reservation_allocation_failure_for_test(false);
    assert_eq!(outbox.reserved_messages(), 0);
    assert_eq!(outbox.reserved_bytes(), 0);
    assert_eq!(outbox.last_reserved_upper_bound_for_test(), None);
}

#[test]
fn lease_id_exhaustion_never_wraps() {
    let mut outbox = CollaborationOutbox::with_ceilings(2, 16);
    let first = outbox.reserve_document_update(1, 1).unwrap();
    outbox.install(first, vec![1]);
    let second = outbox.reserve_document_update(2, 1).unwrap();
    outbox.install(second, vec![2]);
    outbox.next_lease_id = Some(u64::MAX);

    let final_lease = outbox.lease_next().unwrap().unwrap();
    assert_eq!(final_lease.lease_id, OutboundLeaseId(u64::MAX));
    outbox.ack_lease(final_lease.lease_id).unwrap();
    assert_eq!(outbox.next_lease_id, None);
    assert_eq!(outbox.pending_document_update_count(), 1);
    assert_eq!(outbox.pending_document_update_bytes(), 1);
    assert_eq!(
        outbox.lease_next(),
        Err(OutboundLeaseError::LeaseIdExhausted)
    );
    assert_eq!(outbox.pending_document_update_count(), 1);
    assert_eq!(outbox.pending_document_update_bytes(), 1);
}

#[test]
#[should_panic(expected = "active document lease accounting underflow")]
fn ack_lease_rejects_document_accounting_underflow() {
    let mut outbox = CollaborationOutbox::with_ceilings(1, 16);
    let reservation = outbox.reserve_document_update(1, 2).unwrap();
    outbox.install(reservation, vec![1, 2]);
    let lease = outbox.lease_next().unwrap().unwrap();
    outbox.pending_bytes = 0;

    outbox.ack_lease(lease.lease_id).unwrap();
}

#[test]
#[should_panic(expected = "active protocol lease accounting underflow")]
fn ack_lease_rejects_protocol_accounting_underflow() {
    let mut outbox = CollaborationOutbox::with_ceilings(1, 16);
    let reservation = outbox.reserve_protocol_replies(1, 2).unwrap();
    outbox.install_protocol_replies(reservation, 1, vec![vec![1, 2]]);
    let lease = outbox.lease_next().unwrap().unwrap();
    outbox.pending_protocol_bytes = 0;

    outbox.ack_lease(lease.lease_id).unwrap();
}
