// Deterministic renewal and expiry clocks

#[test]
fn tick_renews_local_awareness_at_exactly_the_renewal_interval() {
    let (id, snapshot) = create_ready_room();
    let generation = synchronize_ready_room(id, &snapshot);
    drive_awareness(id, 501, 0).unwrap();
    set_desired_awareness(id, 502, &json!({ "name": "renewer" }).to_string()).unwrap();
    let published_clock = local_peer(id).unwrap().clock;
    drain_protocol_replies(id, generation);

    // One millisecond before the interval: no renewal, deadline reported.
    let outcome = drive_awareness(id, 503, AWARENESS_RENEWAL_INTERVAL_MILLIS - 1).unwrap();
    assert!(!outcome.renewed_local, "{outcome:?}");
    assert_eq!(
        outcome.next_deadline_millis,
        Some(AWARENESS_RENEWAL_INTERVAL_MILLIS),
        "{outcome:?}"
    );
    assert_eq!(drain_protocol_replies(id, generation).len(), 0);
    assert_eq!(local_peer(id).unwrap().clock, published_clock);

    // Exactly at the interval: the local state renews with a fresh clock and
    // the renewed frame is enqueued for broadcast.
    let outcome = drive_awareness(id, 504, AWARENESS_RENEWAL_INTERVAL_MILLIS).unwrap();
    assert!(outcome.renewed_local, "{outcome:?}");
    assert_eq!(
        outcome.next_deadline_millis,
        Some(AWARENESS_RENEWAL_INTERVAL_MILLIS * 2),
        "{outcome:?}"
    );
    let renewed_clock = local_peer(id).unwrap().clock;
    assert!(
        renewed_clock > published_clock,
        "renewal must publish a fresh clock: {renewed_clock} vs {published_clock}",
    );
    let replies = drain_protocol_replies(id, generation);
    assert_eq!(replies.len(), 1, "{replies:?}");
    let reply = decode_awareness_reply(&replies[0]);
    let entry = reply.clients.values().next().unwrap();
    assert_eq!(entry.clock, renewed_clock);
    destroy_session(id);
}

#[test]
fn refused_broadcast_keeps_the_publish_clock_and_a_tick_heals_it_after_drain() {
    let (id, snapshot) = create_ready_room();
    let generation = synchronize_ready_room(id, &snapshot);
    drive_awareness(id, 541, 0).unwrap();

    // Baseline: one successful publish; its clock is the last successful one.
    let desired = json!({ "name": "pre-fill" });
    set_desired_awareness(id, 542, &desired.to_string()).unwrap();
    let local = local_peer(id).unwrap();
    let local_client = ClientID::new(local.client_id);
    let published_clock = local.clock;
    let mut raw = Awareness::new(Doc::new());
    for reply in drain_protocol_replies(id, generation) {
        raw.apply_update(decode_awareness_reply(&reply)).unwrap();
    }
    assert_eq!(raw.meta(local_client).unwrap().0, published_clock);

    // Saturate the shared outbox the wedge way: a single slot, filled by a
    // real local document edit through the bridge.
    bridge::set_outbox_ceilings(id, 1, 10 * 1024 * 1024).unwrap();
    let revision = bridge::session_audit(id).unwrap().document_revision;
    bridge::submit_selection(
        id,
        &json!({
            "version": 1,
            "requestId": "543",
            "baseDocumentRevision": revision.to_string(),
            "selection": {
                "type": "text",
                "anchor": { "offset": 0, "kind": "scalar" },
                "head": { "offset": 0, "kind": "scalar" },
            },
        })
        .to_string(),
    )
    .unwrap();
    let revision = bridge::session_audit(id).unwrap().document_revision;
    bridge::submit_input(
        id,
        &json!({
            "version": 1,
            "requestId": "544",
            "baseDocumentRevision": revision.to_string(),
            "text": "fill",
        })
        .to_string(),
    )
    .unwrap();
    assert_eq!(
        bridge::outbox_pending(id).unwrap().unwrap().0,
        1,
        "the local edit must fill the only shared slot",
    );

    // (a)+(b): the valid desired-state change is retained, but its broadcast
    // reservation is refused retryably WITHOUT closing the generation.
    drive_awareness(id, 545, 5_000).unwrap();
    let changed = json!({ "name": "retained through refusal" });
    let error = set_desired_awareness(id, 546, &changed.to_string()).unwrap_err();
    assert_eq!(error.code, TRANSPORT_REPLY_LIMIT_EXCEEDED, "{error:?}");
    assert_eq!(
        transport_state(id).unwrap(),
        TransportState::Synchronized,
        "a broadcast refusal must not close the generation",
    );
    assert_eq!(desired_awareness(id).unwrap(), Some(changed.clone()));
    assert_eq!(
        local_peer(id).unwrap().state,
        changed,
        "the retained state stays visible in the local projection",
    );

    // No partial install: the failed attempt left zero entries anywhere.
    assert_eq!(pending_protocol_replies(id).unwrap(), Some((0, 0)));
    assert_eq!(
        bridge::outbox_pending(id).unwrap().unwrap().0,
        1,
        "only the fill edit may pend",
    );

    // (c): the publish clock did not advance — the renewal deadline stays
    // anchored at the last successful broadcast (t=0), not at the refusal
    // (t=5_000), and the boundary tick still finds renewal due: it attempts
    // the broadcast and is refused the same retryable way.
    let outcome = drive_awareness(id, 547, AWARENESS_RENEWAL_INTERVAL_MILLIS - 1).unwrap();
    assert!(!outcome.renewed_local, "{outcome:?}");
    assert_eq!(
        outcome.next_deadline_millis,
        Some(AWARENESS_RENEWAL_INTERVAL_MILLIS),
        "a refused publish must not push the renewal deadline out: {outcome:?}",
    );
    let error = drive_awareness(id, 548, AWARENESS_RENEWAL_INTERVAL_MILLIS).unwrap_err();
    assert_eq!(error.code, TRANSPORT_REPLY_LIMIT_EXCEEDED, "{error:?}");
    assert_eq!(transport_state(id).unwrap(), TransportState::Synchronized);
    assert_eq!(pending_protocol_replies(id).unwrap(), Some((0, 0)));

    // (d): drain through the normal pickup seam, then a retried tick heals
    // the broadcast end-to-end with a fresh clock past the last successful
    // publish — the raw peer sees the retained state again.
    let lease = bridge::lease_next_update(id).unwrap().unwrap();
    assert_eq!(lease.request_id, 544);
    bridge::ack_leased_update(id, lease.lease_id).unwrap();
    assert_eq!(bridge::outbox_pending(id).unwrap().unwrap(), (0, 0));
    let outcome = drive_awareness(id, 549, AWARENESS_RENEWAL_INTERVAL_MILLIS).unwrap();
    assert!(outcome.renewed_local, "{outcome:?}");
    assert_eq!(
        outcome.next_deadline_millis,
        Some(AWARENESS_RENEWAL_INTERVAL_MILLIS * 2),
        "{outcome:?}",
    );
    let replies = drain_protocol_replies(id, generation);
    assert_eq!(replies.len(), 1, "{replies:?}");
    for reply in replies {
        raw.apply_update(decode_awareness_reply(&reply)).unwrap();
    }
    let (healed_clock, _) = raw.meta(local_client).unwrap();
    assert!(
        healed_clock > published_clock,
        "the healed broadcast clock {healed_clock} must exceed the last successful publish {published_clock}",
    );
    assert_eq!(raw.state::<Value>(local_client), Some(changed.clone()));

    // The generation survived the whole refusal→heal arc: a query on it is
    // answered without a close, with the healed local entry included.
    let outcome = receive_message(id, 550, generation, &query_awareness_message()).unwrap();
    assert!(outcome.close.is_none(), "{outcome:?}");
    let replies = drain_protocol_replies(id, generation);
    assert_eq!(replies.len(), 1, "{replies:?}");
    let reply = decode_awareness_reply(&replies[0]);
    let entry = &reply.clients[&local_client];
    assert_eq!(entry.clock, healed_clock);
    assert_eq!(
        serde_json::from_str::<Value>(entry.json.as_ref()).unwrap(),
        changed,
    );
    destroy_session(id);
}

#[test]
fn directive_deadline_is_minimum_of_retry_renewal_and_peer_expiry() {
    let (id, snapshot) = create_ready_room();
    let generation = collaboration_drive(id, 20_400, 0)
        .unwrap()
        .generation_to_open
        .expect("the initial drive issues the current generation");
    collaboration_socket_open(id, 20_401, generation, 0).unwrap();
    let server = raw_doc_from_snapshot(&snapshot);
    let synchronized = collaboration_receive(
        id,
        20_402,
        generation,
        &Message::Sync(yrs::sync::SyncMessage::SyncStep2(
            server
                .transact()
                .encode_state_as_update_v1(&yrs::StateVector::default()),
        ))
        .encode_v1(),
        0,
    )
    .unwrap();
    assert_eq!(synchronized.transport_state, TransportState::Synchronized);

    set_desired_awareness(id, 20_403, &json!({ "name": "deadline local" }).to_string()).unwrap();
    collaboration_receive(
        id,
        20_404,
        generation,
        &awareness_message(&[(6_901, 1, r#"{"name":"deadline peer"}"#)]),
        0,
    )
    .unwrap();

    let renewal_wins = collaboration_drive(id, 20_405, 0).unwrap();
    assert_eq!(renewal_wins.next_deadline_millis, Some(15_000));
    assert!(
        renewal_wins.next_deadline_millis < Some(AWARENESS_EXPIRY_MILLIS),
        "the directive chooses the earlier local renewal over peer expiry"
    );

    let tied = collaboration_drive(id, 20_406, AWARENESS_RENEWAL_INTERVAL_MILLIS).unwrap();
    assert!(tied.renewed_local, "the due drive renews local awareness");
    assert_eq!(tied.next_deadline_millis, Some(AWARENESS_EXPIRY_MILLIS));

    let retry_wins = collaboration_socket_close(
        id,
        20_407,
        generation,
        CloseDisposition::Retryable,
        AWARENESS_RENEWAL_INTERVAL_MILLIS,
    )
    .unwrap();
    assert_eq!(retry_wins.transport_state, TransportState::Disconnected);
    assert_eq!(retry_wins.next_deadline_millis, Some(15_500));
    destroy_session(id);
}

#[test]
fn tick_never_renews_without_a_synchronized_transport_or_desired_state() {
    // No desired state: nothing renews, no deadline is requested.
    let (id, snapshot) = create_ready_room();
    let _generation = synchronize_ready_room(id, &snapshot);
    drive_awareness(id, 511, 0).unwrap();
    let outcome = drive_awareness(id, 512, AWARENESS_RENEWAL_INTERVAL_MILLIS).unwrap();
    assert!(!outcome.renewed_local, "{outcome:?}");
    assert_eq!(outcome.next_deadline_millis, None, "{outcome:?}");
    destroy_session(id);

    // Desired state but a closed transport: the state is retained, but
    // nothing broadcasts while disconnected.
    let (id, snapshot) = create_ready_room();
    let generation = synchronize_ready_room(id, &snapshot);
    set_desired_awareness(id, 513, &json!({ "name": "offline" }).to_string()).unwrap();
    drain_protocol_replies(id, generation);
    collaboration_socket_close(id, 514, generation, CloseDisposition::Retryable, 0).unwrap();
    let outcome = drive_awareness(id, 515, AWARENESS_RENEWAL_INTERVAL_MILLIS * 3).unwrap();
    assert!(!outcome.renewed_local, "{outcome:?}");
    assert_eq!(outcome.next_deadline_millis, None, "{outcome:?}");
    assert_eq!(pending_protocol_replies(id).unwrap(), Some((0, 0)));
    assert_eq!(
        desired_awareness(id).unwrap(),
        Some(json!({ "name": "offline" })),
    );
    destroy_session(id);
}

#[test]
fn tick_expires_remote_peers_at_exactly_the_expiry_deadline() {
    let (id, snapshot) = create_ready_room();
    let generation = synchronize_ready_room(id, &snapshot);
    drive_awareness(id, 521, 0).unwrap();
    receive_message(
        id,
        522,
        generation,
        &awareness_message(&[(6_801, 1, r#"{"name":"mortal"}"#)]),
    )
    .unwrap();

    // One millisecond before the deadline: the peer survives.
    let outcome = drive_awareness(id, 523, AWARENESS_EXPIRY_MILLIS - 1).unwrap();
    assert!(outcome.expired_peers.is_empty(), "{outcome:?}");
    assert_eq!(
        outcome.next_deadline_millis,
        Some(AWARENESS_EXPIRY_MILLIS),
        "{outcome:?}"
    );
    assert_eq!(remote_peers(id).len(), 1);

    // Exactly at the deadline: the peer expires, leaves peers(), and leaves
    // the complete query answer.
    let outcome = drive_awareness(id, 524, AWARENESS_EXPIRY_MILLIS).unwrap();
    assert_eq!(outcome.expired_peers, vec![6_801], "{outcome:?}");
    assert!(remote_peers(id).is_empty());
    receive_message(id, 525, generation, &query_awareness_message()).unwrap();
    let replies = drain_protocol_replies(id, generation);
    let reply = decode_awareness_reply(&replies[0]);
    assert!(
        !reply.clients.contains_key(&ClientID::new(6_801)),
        "expired peers leave the query answer: {reply:?}",
    );

    // Expiry is a standard removal: only a strictly newer clock reappears.
    receive_message(
        id,
        526,
        generation,
        &awareness_message(&[(6_801, 2, r#"{"name":"stale echo"}"#)]),
    )
    .unwrap();
    assert!(
        remote_peers(id).is_empty(),
        "an equal-clock echo of an expired peer stays invisible",
    );
    receive_message(
        id,
        527,
        generation,
        &awareness_message(&[(6_801, 3, r#"{"name":"returned"}"#)]),
    )
    .unwrap();
    assert_eq!(remote_peers(id).len(), 1);
    destroy_session(id);
}

#[test]
fn renewed_announcements_refresh_the_peer_expiry_deadline() {
    let (id, snapshot) = create_ready_room();
    let generation = synchronize_ready_room(id, &snapshot);
    drive_awareness(id, 531, 0).unwrap();
    receive_message(
        id,
        532,
        generation,
        &awareness_message(&[(6_901, 1, r#"{"name":"heartbeat"}"#)]),
    )
    .unwrap();

    // A renewed announcement (fresh clock) at t=20s pushes the deadline out.
    drive_awareness(id, 533, 20_000).unwrap();
    receive_message(
        id,
        534,
        generation,
        &awareness_message(&[(6_901, 2, r#"{"name":"heartbeat"}"#)]),
    )
    .unwrap();
    let outcome = drive_awareness(id, 535, AWARENESS_EXPIRY_MILLIS).unwrap();
    assert!(
        outcome.expired_peers.is_empty(),
        "activity at 20s keeps the peer alive at 30s: {outcome:?}",
    );
    assert_eq!(
        outcome.next_deadline_millis,
        Some(20_000 + AWARENESS_EXPIRY_MILLIS),
        "{outcome:?}"
    );
    let outcome = drive_awareness(id, 536, 20_000 + AWARENESS_EXPIRY_MILLIS).unwrap();
    assert_eq!(outcome.expired_peers, vec![6_901], "{outcome:?}");
    destroy_session(id);
}
