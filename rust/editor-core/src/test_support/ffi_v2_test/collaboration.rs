#[test]
fn create_room_attaches_the_collaboration_runtime() {
    // Room sessions own the runtime (bounded outbox) from creation so
    // offline edits queue from the first keystroke; local sessions do not.
    let snapshot = snapshot_source();
    let id = create_handle_with_state(
        room_config(Some(&snapshot)),
        Some(snapshot.encoded_state.clone()),
    );
    let pending = crate::native_bridge_test_support::outbox_pending(id.parse().unwrap())
        .expect("test seam must read the session");
    assert_eq!(pending, Some((0, 0)), "room sessions attach the runtime");
    destroy_handle(&id);

    let local = create_handle(json!({ "initialization": { "type": "localEmpty" } }));
    let pending = crate::native_bridge_test_support::outbox_pending(local.parse().unwrap())
        .expect("test seam must read the session");
    assert_eq!(pending, None, "local sessions own no outbox");
    destroy_handle(&local);
}

#[test]
fn collaboration_generation_flow_with_stale_and_disposition_refusals() {
    let snapshot = snapshot_source();
    let id = create_handle_with_state(
        room_config(Some(&snapshot)),
        Some(snapshot.encoded_state.clone()),
    );
    let server = RawPeer::from_snapshot(&snapshot);

    // Local-only editors remain detached; drive never creates a generation.
    let local = create_handle(json!({ "initialization": { "type": "localEmpty" } }));
    let detached = drive_v2(&local, 0);
    assert_eq!(detached["transportState"], "Detached", "{detached:?}");
    assert_eq!(detached["generationToOpen"], Value::Null, "{detached:?}");
    assert_eq!(detached["nextDeadlineMillis"], Value::Null, "{detached:?}");
    destroy_handle(&local);

    // Drive issues generation 1. A subsequent drive is observational only;
    // socket open queues Sync Step 1 for the retained lease path.
    let generation = drive_v2(&id, 0)["generationToOpen"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(generation, "1", "first issued generation");
    let stale_generation = (generation.parse::<u64>().unwrap() + 100).to_string();
    let waiting = drive_v2(&id, 0);
    assert_eq!(waiting["transportState"], "Connecting", "{waiting:?}");
    assert_eq!(waiting["generationToOpen"], Value::Null, "{waiting:?}");

    let error = err_json(&v2_collab::editor_v2_collaboration_socket_open(
        id.clone(),
        stale_generation.clone(),
        "0".into(),
    ));
    assert_error(&error, "transport", "TRANSPORT_STALE_GENERATION", None);
    let opened = open_v2(&id, &generation, 0);
    assert_eq!(opened["transportState"], "Handshaking", "{opened:?}");
    let step1 = lease_v2(&id, &generation);
    let our_sv = step1_state_vector(&step1.frame);
    assert!(!step1.frame.is_empty(), "step 1 bytes ride through a lease");
    ack_v2(&id, &generation, step1.lease_id);

    // receive on a stale generation refuses before any decode work.
    let error = err_json(&v2_collab::editor_v2_collaboration_receive(
        id.clone(),
        stale_generation.clone(),
        step2_frame(server.diff_for(&our_sv.encode_v1())),
        "0".into(),
    ));
    assert_error(&error, "transport", "TRANSPORT_STALE_GENERATION", None);

    // The real Step 2 completes the handshake.
    let outcome = receive_v2(
        &id,
        &generation,
        step2_frame(server.diff_for(&our_sv.encode_v1())),
        0,
    );
    assert_eq!(outcome["transportState"], "Synchronized", "{outcome:?}");
    assert_eq!(outcome["remoteCommitApplied"], false, "{outcome:?}");

    // The peer's Step 1 earns a retained Step 2 reply. ACK consumes exactly
    // that lease; a subsequent lease observes the explicit empty variant.
    let outcome = receive_v2(
        &id,
        &generation,
        step1_frame(&server.state_vector_bytes()),
        0,
    );
    assert_eq!(outcome["transportState"], "Synchronized", "{outcome:?}");
    let lease = lease_v2(&id, &generation);
    match Message::decode_v1(&lease.frame).expect("outbound frame must decode") {
        Message::Sync(SyncMessage::SyncStep2(update)) => server.apply(&update),
        other => panic!("expected a Sync Step 2 reply frame, got {other:?}"),
    }
    ack_v2(&id, &generation, lease.lease_id);
    assert_empty_lease_v2(&id, &generation);

    // Stale generation refuses the lease; the close retires the generation.
    let error = err_lease(&v2_collab::editor_v2_collaboration_lease_outbound(
        id.clone(),
        stale_generation.clone(),
    ));
    assert_error(&error, "transport", "TRANSPORT_STALE_GENERATION", None);
    let outcome = close_v2(&id, &generation, None, None, 0);
    assert_eq!(outcome["transportState"], "Disconnected", "{outcome:?}");
    assert_eq!(outcome["nextDeadlineMillis"], "500", "{outcome:?}");
    let error = err_lease(&v2_collab::editor_v2_collaboration_lease_outbound(
        id.clone(),
        generation.clone(),
    ));
    assert_error(&error, "transport", "TRANSPORT_STALE_GENERATION", None);

    // Reconnect issues the next monotonic generation; a policy-violation
    // close code parks the transport Incompatible and drive remains inert.
    let generation = drive_v2(&id, 500)["generationToOpen"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(generation, "2", "generations stay monotonic");
    let outcome = close_v2(
        &id,
        &generation,
        Some(1008),
        Some("policy violation".into()),
        500,
    );
    assert_eq!(outcome["transportState"], "Incompatible", "{outcome:?}");
    assert_eq!(outcome["nextDeadlineMillis"], Value::Null, "{outcome:?}");
    let inert = drive_v2(&id, 500);
    assert_eq!(inert["transportState"], "Incompatible", "{inert:?}");
    assert_eq!(inert["generationToOpen"], Value::Null, "{inert:?}");
    destroy_handle(&id);
}

#[test]
fn typed_awareness_intent_ffi_and_collaboration_binary_round_trip() {
    let snapshot = snapshot_source();
    let id = create_handle_with_state(
        room_config(Some(&snapshot)),
        Some(snapshot.encoded_state.clone()),
    );
    let server = RawPeer::from_snapshot(&snapshot);
    let generation = synchronize_v2(&id, &server);

    // A local edit rides one retained outbound lease; the raw peer applies
    // it and ACK consumes that exact frame.
    let outcome = ok_json(&v2::editor_v2_apply_input(
        id.clone(),
        input_envelope(901, revision_of(&id), " outbound"),
    ));
    assert_eq!(outcome["changed"], true, "{outcome:?}");
    let lease = lease_v2(&id, &generation);
    match Message::decode_v1(&lease.frame).expect("update frame must decode") {
        Message::Sync(SyncMessage::Update(update)) => server.apply(&update),
        other => panic!("expected a document update frame, got {other:?}"),
    }
    ack_v2(&id, &generation, lease.lease_id);
    assert!(
        server.fragment_string().contains("ffi seed")
            && server.fragment_string().contains(" outbound"),
        "{:?}",
        server.fragment_string(),
    );

    // Awareness takes exactly a typed intent and Rust publishes the
    // application state/focus beside its engine-owned cursor.
    ok_unit(&v2_collab::editor_v2_collaboration_set_awareness(
        id.clone(),
        json!({
            "state": { "name": "ffi peer" },
            "focused": true,
            "selection": { "type": "text", "anchor": 4, "head": 6 },
        })
        .to_string(),
    ));
    let peers = ok_json(&v2_collab::editor_v2_collaboration_peers(id.clone()));
    let local = peers["peers"]
        .as_array()
        .expect("peers array")
        .iter()
        .find(|peer| peer["isLocal"] == true)
        .expect("a local peer");
    assert_eq!(local["state"]["name"], json!("ffi peer"), "{local:?}");
    assert!(local["state"].get("state").is_none(), "{local:?}");
    assert_eq!(local["state"]["focused"], true, "{local:?}");
    assert_eq!(
        local["cursor"],
        json!({ "anchor": 4, "head": 6 }),
        "{local:?}"
    );
    assert!(
        local["clientId"]
            .as_str()
            .expect("clientId is a decimal string")
            .parse::<u64>()
            .is_ok(),
        "{local:?}",
    );
    let lease = lease_v2(&id, &generation);
    let mut raw_awareness = Awareness::new(Doc::new());
    match Message::decode_v1(&lease.frame).expect("awareness frame must decode") {
        Message::Awareness(update) => raw_awareness.apply_update(update).unwrap(),
        other => panic!("expected an awareness frame, got {other:?}"),
    }
    ack_v2(&id, &generation, lease.lease_id);
    assert_empty_lease_v2(&id, &generation);

    // An explicit null selection removes the engine-owned cursor while
    // retaining the application state and focus flag. (Omitting the key
    // instead would retain the cursor — see the awareness suite.)
    ok_unit(&v2_collab::editor_v2_collaboration_set_awareness(
        id.clone(),
        json!({ "state": { "name": "cursorless" }, "focused": false, "selection": Value::Null })
            .to_string(),
    ));
    let peers = ok_json(&v2_collab::editor_v2_collaboration_peers(id.clone()));
    let local = peers["peers"]
        .as_array()
        .expect("peers array")
        .iter()
        .find(|peer| peer["isLocal"] == true)
        .expect("a local peer");
    assert_eq!(local["state"]["name"], json!("cursorless"));
    assert!(local["state"].get("state").is_none(), "{local:?}");
    assert_eq!(local["state"]["focused"], false);
    assert_eq!(local["cursor"], Value::Null, "{local:?}");
    let lease = lease_v2(&id, &generation);
    match Message::decode_v1(&lease.frame).expect("cursorless awareness frame must decode") {
        Message::Awareness(update) => raw_awareness.apply_update(update).unwrap(),
        other => panic!("expected cursorless awareness frame, got {other:?}"),
    }
    ack_v2(&id, &generation, lease.lease_id);

    // "null" withdraws the desired state with a tombstone broadcast.
    ok_unit(&v2_collab::editor_v2_collaboration_set_awareness(
        id.clone(),
        "null".into(),
    ));
    let peers = ok_json(&v2_collab::editor_v2_collaboration_peers(id.clone()));
    assert_eq!(peers["peers"], json!([]), "{peers:?}");
    let lease = lease_v2(&id, &generation);
    match Message::decode_v1(&lease.frame).expect("tombstone frame must decode") {
        Message::Awareness(update) => raw_awareness.apply_update(update).unwrap(),
        other => panic!("expected an awareness tombstone frame, got {other:?}"),
    }
    ack_v2(&id, &generation, lease.lease_id);

    // Malformed awareness state is a structured error, never a panic.
    let result = v2_collab::editor_v2_collaboration_set_awareness(id.clone(), "{not json".into());
    assert!(result.value.is_none(), "{result:?}");
    assert_error(
        &result.error.expect("error"),
        "boundary",
        "AWARENESS_STATE_INVALID",
        None,
    );

    // Sessions without an attached runtime refuse runtime-shaped calls.
    let local = create_handle(json!({ "initialization": { "type": "localEmpty" } }));
    let error = err_json(&v2_collab::editor_v2_collaboration_peers(local.clone()));
    assert_error(&error, "boundary", "CONFIG_INVALID", None);
    destroy_handle(&local);
    destroy_handle(&id);
}

#[test]
fn awareness_selection_patch_ffi_has_a_closed_result_and_input_shape() {
    let snapshot = snapshot_source();
    let id = create_handle_with_state(
        room_config(Some(&snapshot)),
        Some(snapshot.encoded_state.clone()),
    );
    let server = RawPeer::from_snapshot(&snapshot);
    let generation = synchronize_v2(&id, &server);
    ok_unit(&v2_collab::editor_v2_collaboration_set_awareness(
        id.clone(),
        json!({ "state": { "name": "ffi peer" }, "focused": true }).to_string(),
    ));
    let lease = lease_v2(&id, &generation);
    ack_v2(&id, &generation, lease.lease_id);

    let outcome = ok_json(&v2_collab::editor_v2_collaboration_set_awareness_selection(
        id.clone(),
        json!({ "type": "text", "anchor": 4, "head": 6 }).to_string(),
    ));
    let keys: std::collections::BTreeSet<&str> = outcome
        .as_object()
        .expect("selection patch outcome is an object")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        ["outboundChanged"]
            .into_iter()
            .collect::<std::collections::BTreeSet<&str>>(),
        "selection patch result has exactly one key: {keys:?}",
    );
    assert_eq!(outcome, json!({ "outboundChanged": true }));
    let lease = lease_v2(&id, &generation);
    ack_v2(&id, &generation, lease.lease_id);

    for invalid in [
        "{not json".to_string(),
        json!({ "type": "text", "anchor": 4, "head": 6, "unknown": true }).to_string(),
    ] {
        let error = err_json(&v2_collab::editor_v2_collaboration_set_awareness_selection(
            id.clone(),
            invalid,
        ));
        assert_error(&error, "boundary", "AWARENESS_STATE_INVALID", None);
    }
    destroy_handle(&id);
}

#[test]
fn awareness_review_fix_raw_publication_is_test_only() {
    let session = concat!(
        include_str!("../../session.rs"),
        include_str!("../../session/collaboration.rs"),
    );
    let runtime = include_str!("../../collaboration_runtime/awareness.rs");
    let document_api = concat!(
        include_str!("../../document_api.rs"),
        include_str!("../../document_api/session_initialization_test_support.rs"),
    );

    assert!(
        !session.contains("pub(crate) fn set_desired_awareness("),
        "EditorSession must not expose a production raw awareness setter",
    );
    assert!(
        session.contains("#[cfg(test)]\n    pub(crate) fn set_desired_awareness_for_test("),
        "legacy raw-state fixtures require a cfg(test)-gated session seam",
    );
    assert!(
        !runtime.contains("pub(crate) fn set_desired_awareness("),
        "the runtime must not retain a generic raw publication method",
    );
    assert!(
        runtime.contains("#[cfg(test)]\n    pub(crate) fn set_desired_awareness_for_test("),
        "the runtime raw parser must be cfg(test)-gated",
    );
    assert!(
        !document_api.contains("pub fn set_desired_awareness("),
        "the document facade must not expose a generic raw awareness setter",
    );
    assert!(
        document_api.contains("pub fn set_desired_awareness_for_test("),
        "raw document-facade publication must be explicitly test-only",
    );
    assert!(
        document_api.contains(concat!(
            "#[cfg(test)]\n",
            "#[path = \"document_api/session_initialization_test_support.rs\"]\n",
            "pub mod session_initialization_test_support;",
        )),
        "the document facade raw helper must remain inside test-only support",
    );
}

#[test]
fn ffi_drive_reports_local_renewal_as_peer_change() {
    let snapshot = snapshot_source();
    let id = create_handle_with_state(
        room_config(Some(&snapshot)),
        Some(snapshot.encoded_state.clone()),
    );
    let generation = synchronize_v2(&id, &RawPeer::from_snapshot(&snapshot));

    for malformed in ["+1", "01", " 1", "1 ", "1e3"] {
        let error = err_json(&v2_collab::editor_v2_collaboration_drive(
            id.clone(),
            malformed.into(),
        ));
        assert_error(&error, "boundary", "CONFIG_INVALID", None);
    }

    ok_unit(&v2_collab::editor_v2_collaboration_set_awareness(
        id.clone(),
        json!({ "state": { "name": "tick local" }, "focused": false }).to_string(),
    ));
    let before = drive_v2(&id, 14_999);
    assert_eq!(
        before,
        json!({
            "transportState": "Synchronized",
            "generationToOpen": null,
            "nextDeadlineMillis": "15000",
            "remoteCommitApplied": false,
            "renewedLocal": false,
            "expiredPeers": [],
            "peersChanged": false,
        }),
        "{before:?}"
    );

    let at = drive_v2(&id, 15_000);
    assert_eq!(
        at,
        json!({
            "transportState": "Synchronized",
            "generationToOpen": null,
            "nextDeadlineMillis": "30000",
            "remoteCommitApplied": false,
            "renewedLocal": true,
            "expiredPeers": [],
            "peersChanged": true,
        }),
        "{at:?}"
    );
    let lease = lease_v2(&id, &generation);
    assert!(
        !lease.frame.is_empty(),
        "renewal enqueues an outbound awareness frame"
    );
    ack_v2(&id, &generation, lease.lease_id);
    destroy_handle(&id);
}

#[test]
fn collaboration_drive_rejects_regressing_time_without_corrupting_peer_expiry() {
    let snapshot = snapshot_source();
    let id = create_handle_with_state(
        room_config(Some(&snapshot)),
        Some(snapshot.encoded_state.clone()),
    );
    let generation = synchronize_v2(&id, &RawPeer::from_snapshot(&snapshot));

    drive_v2(&id, 10_000);
    let error = err_json(&v2_collab::editor_v2_collaboration_drive(
        id.clone(),
        "9999".into(),
    ));
    assert_error(&error, "transport", "AWARENESS_TIME_REGRESSION", None);
    assert_eq!(
        serde_json::from_str::<Value>(
            error
                .details_json
                .as_deref()
                .expect("regressing time errors carry clock context"),
        )
        .expect("error details are JSON"),
        json!({ "nowMillis": "9999", "lastNowMillis": "10000" }),
    );

    // The remote update must retain the last accepted drive time (10s), not
    // the rejected 9_999ms input, so expiry remains scheduled for 40s.
    let clients = [(
        yrs::ClientID::new(9_001),
        yrs::sync::awareness::AwarenessUpdateEntry {
            clock: 1,
            json: json!({ "name": "monotonic peer" }).to_string().into(),
        },
    )]
    .into_iter()
    .collect();
    let receive = receive_v2(
        &id,
        &generation,
        Message::Awareness(yrs::sync::awareness::AwarenessUpdate { clients }).encode_v1(),
        10_000,
    );
    assert_eq!(receive["transportState"], "Synchronized", "{receive:?}");

    let before = drive_v2(&id, 39_999);
    assert_eq!(before["expiredPeers"], json!([]), "{before:?}");
    assert_eq!(before["nextDeadlineMillis"], json!("40000"), "{before:?}");

    let at = drive_v2(&id, 40_000);
    assert_eq!(at["expiredPeers"], json!(["9001"]), "{at:?}");
    destroy_handle(&id);
}

#[test]
fn collaboration_drive_expires_remote_peers_with_decimal_ids() {
    // Yrs client IDs occupy the same 53-bit integer domain as Yjs numbers.
    // Use its maximum valid value so the FFI must preserve the exact decimal
    // spelling without constructing an out-of-domain ID that aliases in
    // release builds.
    const MAX_YRS_CLIENT_ID: u64 = 9_007_199_254_740_991;
    let snapshot = snapshot_source();
    let id = create_handle_with_state(
        room_config(Some(&snapshot)),
        Some(snapshot.encoded_state.clone()),
    );
    let generation = synchronize_v2(&id, &RawPeer::from_snapshot(&snapshot));
    let clients = [(
        yrs::ClientID::new(MAX_YRS_CLIENT_ID),
        yrs::sync::awareness::AwarenessUpdateEntry {
            clock: 1,
            json: json!({ "name": "expiring remote" }).to_string().into(),
        },
    )]
    .into_iter()
    .collect();
    let receive = receive_v2(
        &id,
        &generation,
        Message::Awareness(yrs::sync::awareness::AwarenessUpdate { clients }).encode_v1(),
        0,
    );
    assert_eq!(receive["transportState"], "Synchronized", "{receive:?}");

    let before = drive_v2(&id, 29_999);
    assert_eq!(before["expiredPeers"], json!([]), "{before:?}");
    assert_eq!(before["peersChanged"], false, "{before:?}");

    let at = drive_v2(&id, 30_000);
    assert_eq!(at["nextDeadlineMillis"], Value::Null, "{at:?}");
    assert_eq!(
        at["expiredPeers"],
        json!([MAX_YRS_CLIENT_ID.to_string()]),
        "{at:?}"
    );
    assert_eq!(at["peersChanged"], true, "{at:?}");
    assert_eq!(at["remoteCommitApplied"], false, "{at:?}");
    destroy_handle(&id);
}

#[test]
fn collaboration_task8_detach_and_reattach_are_idempotent_after_incompatible() {
    let snapshot = snapshot_source();
    let id = create_handle_with_state(
        room_config(Some(&snapshot)),
        Some(snapshot.encoded_state.clone()),
    );
    let first_generation = synchronize_v2(&id, &RawPeer::from_snapshot(&snapshot));
    let close = close_v2(
        &id,
        &first_generation,
        Some(1008),
        Some("policy violation".into()),
        0,
    );
    assert_eq!(close["transportState"], "Incompatible", "{close:?}");
    let inert = drive_v2(&id, 0);
    assert_eq!(inert["transportState"], "Incompatible", "{inert:?}");
    assert_eq!(inert["generationToOpen"], Value::Null, "{inert:?}");

    ok_unit(&v2_collab::editor_v2_collaboration_detach(id.clone()));
    assert_eq!(state_of(&id)["transportState"], "Detached");
    ok_unit(&v2_collab::editor_v2_collaboration_detach(id.clone()));
    assert_eq!(state_of(&id)["transportState"], "Detached");
    ok_unit(&v2_collab::editor_v2_collaboration_reattach(id.clone()));
    assert_eq!(state_of(&id)["transportState"], "Disconnected");
    ok_unit(&v2_collab::editor_v2_collaboration_reattach(id.clone()));
    assert_eq!(state_of(&id)["transportState"], "Disconnected");

    let next = drive_v2(&id, 0);
    assert_eq!(next["generationToOpen"], "2", "{next:?}");
    destroy_handle(&id);
}

#[test]
fn leased_outbound_drains_protocol_replies_before_document_updates() {
    let snapshot = snapshot_source();
    let id = create_handle_with_state(
        room_config(Some(&snapshot)),
        Some(snapshot.encoded_state.clone()),
    );
    let server = RawPeer::from_snapshot(&snapshot);
    let generation = synchronize_v2(&id, &server);
    assert_eq!(
        crate::session_initialization_test_support::pending_protocol_replies(id.parse().unwrap())
            .unwrap(),
        Some((0, 0)),
        "a freshly synchronized session has no protocol residue",
    );
    assert_eq!(
        crate::native_bridge_test_support::outbox_pending(id.parse().unwrap()).unwrap(),
        Some((0, 0)),
    );

    // Fill BOTH queues on the one live session: an awareness broadcast and
    // a Step 2 reply are transport-scoped protocol frames; the local edit
    // is a pending document update.
    ok_unit(&v2_collab::editor_v2_collaboration_set_awareness(
        id.clone(),
        json!({ "state": { "name": "ordering peer" }, "focused": false }).to_string(),
    ));
    let outcome = receive_v2(
        &id,
        &generation,
        step1_frame(&server.state_vector_bytes()),
        0,
    );
    assert_eq!(outcome["transportState"], "Synchronized", "{outcome:?}");
    let outcome = ok_json(&v2::editor_v2_apply_input(
        id.clone(),
        input_envelope(1101, revision_of(&id), " ordered"),
    ));
    assert_eq!(outcome["changed"], true, "{outcome:?}");

    // Non-vacuity: both queues are provably non-empty at pickup time.
    let (protocol_count, protocol_bytes) =
        crate::session_initialization_test_support::pending_protocol_replies(id.parse().unwrap())
            .unwrap()
            .expect("room sessions own a protocol queue");
    assert_eq!(protocol_count, 2, "awareness broadcast + Step 2 reply");
    assert!(protocol_bytes > 0);
    let (document_count, document_bytes) =
        crate::native_bridge_test_support::outbox_pending(id.parse().unwrap())
            .unwrap()
            .expect("room sessions own a document outbox");
    assert_eq!(document_count, 1, "the local edit is pending");
    assert!(document_bytes > 0);

    // Lease one frame per call: every frame decodes as a standard
    // yrs::sync::Message, every successful lease is ACKed, and every
    // protocol frame precedes every document frame.
    let mut kinds = Vec::new();
    loop {
        let result =
            v2_collab::editor_v2_collaboration_lease_outbound(id.clone(), generation.clone());
        if result.empty {
            assert!(
                result.value.is_none() && result.error.is_none(),
                "{result:?}"
            );
            break;
        }
        let lease = ok_lease(&result);
        let frame = lease.frame;
        let kind = match Message::decode_v1(&frame).expect("outbound frame must decode") {
            Message::Sync(SyncMessage::SyncStep2(update)) => {
                server.apply(&update);
                "protocol"
            }
            Message::Awareness(_) => "protocol",
            Message::Sync(SyncMessage::Update(update)) => {
                server.apply(&update);
                "document"
            }
            other => panic!("unexpected outbound frame: {other:?}"),
        };
        kinds.push(kind);
        ack_v2(&id, &generation, lease.lease_id);
    }
    assert_eq!(
        kinds,
        ["protocol", "protocol", "document"],
        "protocol replies drain before document updates",
    );
    assert!(
        server.fragment_string().contains(" ordered"),
        "the document frame carries the local edit: {:?}",
        server.fragment_string(),
    );

    // Both queues drain to exactly (0, 0).
    assert_eq!(
        crate::session_initialization_test_support::pending_protocol_replies(id.parse().unwrap())
            .unwrap(),
        Some((0, 0)),
    );
    assert_eq!(
        crate::native_bridge_test_support::outbox_pending(id.parse().unwrap()).unwrap(),
        Some((0, 0)),
    );

    destroy_handle(&id);
}
