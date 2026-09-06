/// Serialize a sticky cursor anchored at `utf16_index` of the seed text on a
/// raw doc sharing the session's lineage.
fn sticky_cursor_json(doc: &Doc, utf16_index: u32) -> Value {
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment(FRAGMENT_NAME).unwrap();
    let Some(XmlOut::Element(paragraph)) = fragment.get(&txn, 0) else {
        panic!("seed content must start with a paragraph");
    };
    let Some(XmlOut::Text(text)) = paragraph.get(&txn, 0) else {
        panic!("seed paragraph must start with a text node");
    };
    let branch = yrs::branch::BranchPtr::from(<yrs::types::xml::XmlTextRef as AsRef<
        yrs::branch::Branch,
    >>::as_ref(&text));
    let sticky = StickyIndex::at(&txn, branch, utf16_index, Assoc::After).unwrap();
    serde_json::to_value(&sticky).unwrap()
}

#[test]
fn cursor_projections_resolve_and_recompute_after_every_document_revision() {
    let (id, snapshot) = create_ready_room();
    let generation = synchronize_ready_room(id, &snapshot);
    let raw_doc = raw_doc_from_snapshot(&snapshot);

    // Peer cursor anchored after "awaren" (utf16 index 6 of the seed text):
    // text content starts at doc position 1, so the cursor resolves to 7.
    let cursor = sticky_cursor_json(&raw_doc, 6);
    let state = json!({ "name": "cursor peer", "cursor": { "anchor": cursor, "head": cursor } });
    receive_message(
        id,
        601,
        generation,
        &awareness_message(&[(7_101, 1, &state.to_string())]),
    )
    .unwrap();
    let peers = remote_peers(id);
    assert_eq!(peers.len(), 1, "{peers:?}");
    assert_eq!(peers[0].cursor, Some((7, 7)), "{peers:?}");

    // A local edit at the start of the text moves the resolved cursor
    // without any awareness re-receive.
    let revision = bridge::session_audit(id).unwrap().document_revision;
    bridge::submit_selection(
        id,
        &json!({
            "version": 1,
            "requestId": "602",
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
            "requestId": "603",
            "baseDocumentRevision": revision.to_string(),
            "text": "xx",
        })
        .to_string(),
    )
    .unwrap();
    let peers = remote_peers(id);
    assert_eq!(
        peers[0].cursor,
        Some((9, 9)),
        "a local edit before the cursor shifts the projection: {peers:?}",
    );

    // A remote update through receive_message moves it again.
    {
        let mut txn = raw_doc.transact_mut();
        let fragment = txn.get_xml_fragment(FRAGMENT_NAME).unwrap();
        let Some(XmlOut::Element(paragraph)) = fragment.get(&txn, 0) else {
            panic!("seed content must start with a paragraph");
        };
        let Some(XmlOut::Text(text)) = paragraph.get(&txn, 0) else {
            panic!("seed paragraph must start with a text node");
        };
        use yrs::Text as _;
        text.insert(&mut txn, 0, "yy");
    }
    let update = raw_doc.transact().encode_state_as_update_v1(
        &yrs::StateVector::decode_v1(&snapshot_state_vector(&snapshot)).unwrap(),
    );
    let outcome = receive_message(
        id,
        604,
        generation,
        &Message::Sync(yrs::sync::SyncMessage::Update(update)).encode_v1(),
    )
    .unwrap();
    assert!(outcome.close.is_none(), "{outcome:?}");
    let peers = remote_peers(id);
    assert_eq!(
        peers[0].cursor,
        Some((11, 11)),
        "a remote edit before the cursor shifts the projection: {peers:?}",
    );
    destroy_session(id);
}

#[test]
fn typed_awareness_intent_owns_sticky_cursors_and_survives_or_omits_them_on_restore() {
    let (id, snapshot) = create_ready_room();
    let generation = synchronize_ready_room(id, &snapshot);
    let intent = json!({
        "state": { "name": "local author", "color": "#204060" },
        "focused": true,
        "selection": { "type": "text", "anchor": 7, "head": 7 },
    });

    let result =
        v2_collaboration::editor_v2_collaboration_set_awareness(id.to_string(), intent.to_string());
    assert!(result.error.is_none(), "{result:?}");
    let local = local_peer(id).expect("intent publishes a local peer");
    assert_eq!(local.state["name"], intent["state"]["name"]);
    assert_eq!(local.state["color"], intent["state"]["color"]);
    assert!(local.state.get("state").is_none(), "{local:?}");
    assert_eq!(local.state["focused"], true);
    assert!(local.state["cursor"].is_object(), "{local:?}");
    assert_eq!(local.cursor, Some((7, 7)), "{local:?}");
    drain_protocol_replies(id, generation);

    // Every invalid caller payload rejects before awareness, clocks, the
    // outbox, peer projections, or the document can move.
    let peers_before = awareness_peers(id).unwrap();
    let audit_before = session_audit(id).unwrap();
    for invalid in [
        json!({
            "state": { "name": "bad" },
            "focused": true,
            "cursor": { "anchor": 1, "head": 1 },
        }),
        json!({
            "state": { "nested": [{ "cursor": "forbidden" }] },
            "focused": true,
        }),
        json!({
            "state": { "focused": "reserved by the runtime" },
            "focused": true,
        }),
        json!({ "state": { "name": "missing focus" } }),
        json!({
            "state": { "name": "missing head" },
            "focused": true,
            "selection": { "type": "text", "anchor": 7 },
        }),
        json!({
            "state": { "name": "outside document" },
            "focused": true,
            "selection": { "type": "text", "anchor": 999, "head": 999 },
        }),
    ] {
        let result = v2_collaboration::editor_v2_collaboration_set_awareness(
            id.to_string(),
            invalid.to_string(),
        );
        assert!(result.value.is_none(), "{invalid}: {result:?}");
        let error = result.error.expect("invalid intent is structured");
        assert_eq!(
            error.code, "AWARENESS_STATE_INVALID",
            "{invalid}: {error:?}"
        );
        assert_eq!(awareness_peers(id).unwrap(), peers_before, "{invalid}");
        assert_eq!(session_audit(id).unwrap(), audit_before, "{invalid}");
        assert_eq!(
            pending_protocol_replies(id).unwrap(),
            Some((0, 0)),
            "{invalid} must not enqueue an awareness update",
        );
    }

    // The stored sticky cursor follows a local edit without re-submitting
    // awareness, then resolves against a restored surviving document.
    let revision = bridge::session_audit(id).unwrap().document_revision;
    bridge::submit_selection(
        id,
        &json!({
            "version": 1,
            "requestId": "612",
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
            "requestId": "613",
            "baseDocumentRevision": revision.to_string(),
            "text": "xx",
        })
        .to_string(),
    )
    .unwrap();
    assert_eq!(local_peer(id).unwrap().cursor, Some((9, 9)));

    // Snapshot restore deliberately refuses pending document updates, so
    // deliver the local edit before moving to the disconnected restore row.
    let frame = v2_collaboration::editor_v2_collaboration_lease_outbound(
        id.to_string(),
        generation.to_string(),
    );
    assert!(frame.error.is_none(), "{frame:?}");
    assert!(!frame.empty, "{frame:?}");
    let frame = frame.value.expect("the local edit must retain a lease");
    assert!(!frame.frame.is_empty());
    let ack = v2_collaboration::editor_v2_collaboration_ack_outbound(
        id.to_string(),
        generation.to_string(),
        frame.lease_id,
    );
    assert!(ack.error.is_none(), "{ack:?}");

    collaboration_socket_close(id, 614, generation, CloseDisposition::Retryable, 0).unwrap();
    restore_snapshot(id, 615, &snapshot).unwrap();
    assert_eq!(local_peer(id).unwrap().cursor, Some((7, 7)));

    // A same-scope snapshot minted by a different Yrs client cannot resolve
    // the old sticky targets. The peer remains, but its cursor is omitted.
    let foreign_snapshot = snapshot_source();
    restore_snapshot(id, 616, &foreign_snapshot).unwrap();
    assert_eq!(local_peer(id).unwrap().cursor, None);
    destroy_session(id);
}

#[test]
fn omitting_the_intent_selection_retains_the_cursor_and_an_explicit_null_clears_it() {
    let (id, snapshot) = create_ready_room();
    let generation = synchronize_ready_room(id, &snapshot);

    // A first intent establishes the Rust-owned sticky cursor.
    let established = v2_collaboration::editor_v2_collaboration_set_awareness(
        id.to_string(),
        json!({
            "state": { "name": "local author" },
            "focused": true,
            "selection": { "type": "text", "anchor": 7, "head": 7 },
        })
        .to_string(),
    );
    assert!(established.error.is_none(), "{established:?}");
    let cursor_after_establish = desired_awareness(id).unwrap().unwrap()["cursor"].clone();
    assert!(
        cursor_after_establish.is_object(),
        "{cursor_after_establish}"
    );
    assert_eq!(local_peer(id).unwrap().cursor, Some((7, 7)));
    drain_protocol_replies(id, generation);

    // A focus-only intent omits `selection` entirely. The caller states no
    // document position, so the established sticky cursor is kept verbatim
    // rather than being dropped or re-resolved.
    let focus_only = v2_collaboration::editor_v2_collaboration_set_awareness(
        id.to_string(),
        json!({ "state": { "name": "local author" }, "focused": false }).to_string(),
    );
    assert!(focus_only.error.is_none(), "{focus_only:?}");
    let desired = desired_awareness(id).unwrap().unwrap();
    assert_eq!(desired["focused"], false);
    assert_eq!(
        desired["cursor"], cursor_after_establish,
        "an omitted selection must retain the exact sticky cursor: {desired}",
    );
    assert_eq!(local_peer(id).unwrap().cursor, Some((7, 7)));

    // The sticky cursor keeps following the document across focus-only
    // republishes: no caller-side position is ever restated.
    let revision = bridge::session_audit(id).unwrap().document_revision;
    bridge::submit_selection(
        id,
        &json!({
            "version": 1,
            "requestId": "9101",
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
            "requestId": "9102",
            "baseDocumentRevision": revision.to_string(),
            "text": "xx",
        })
        .to_string(),
    )
    .unwrap();
    let refocused = v2_collaboration::editor_v2_collaboration_set_awareness(
        id.to_string(),
        json!({ "state": { "name": "local author" }, "focused": true }).to_string(),
    );
    assert!(refocused.error.is_none(), "{refocused:?}");
    assert_eq!(
        local_peer(id).unwrap().cursor,
        Some((9, 9)),
        "the retained sticky cursor tracks the edit that shifted it",
    );

    // An explicit null is the only way to publish presence without a cursor.
    let cleared = v2_collaboration::editor_v2_collaboration_set_awareness(
        id.to_string(),
        json!({
            "state": { "name": "local author" },
            "focused": true,
            "selection": Value::Null,
        })
        .to_string(),
    );
    assert!(cleared.error.is_none(), "{cleared:?}");
    let desired = desired_awareness(id).unwrap().unwrap();
    assert!(
        desired.get("cursor").is_none(),
        "an explicit null selection publishes no cursor: {desired}",
    );
    assert_eq!(local_peer(id).unwrap().cursor, None);

    // Retaining an absent cursor stays absent rather than resurrecting one.
    let still_absent = v2_collaboration::editor_v2_collaboration_set_awareness(
        id.to_string(),
        json!({ "state": { "name": "local author" }, "focused": false }).to_string(),
    );
    assert!(still_absent.error.is_none(), "{still_absent:?}");
    assert!(desired_awareness(id)
        .unwrap()
        .unwrap()
        .get("cursor")
        .is_none());
    destroy_session(id);
}

#[test]
fn awareness_selection_patch_preserves_state_and_focus_and_queues_one_frame() {
    let (id, snapshot) = create_ready_room();
    let generation = synchronize_ready_room(id, &snapshot);
    let initial = v2_collaboration::editor_v2_collaboration_set_awareness(
        id.to_string(),
        json!({
            "state": {"user": {"name": "Ada"}, "custom": 7},
            "focused": true
        })
        .to_string(),
    );
    assert!(initial.error.is_none(), "{initial:?}");
    drain_protocol_replies(id, generation);

    let result = v2_collaboration::editor_v2_collaboration_set_awareness_selection(
        id.to_string(),
        json!({"type": "text", "anchor": 2, "head": 2}).to_string(),
    );
    assert!(result.error.is_none(), "{result:?}");
    let outcome: Value =
        serde_json::from_str(result.value.as_deref().expect("selection patch value")).unwrap();

    assert_eq!(outcome, json!({"outboundChanged": true}));
    let desired = desired_awareness(id).unwrap().unwrap();
    assert_eq!(desired["user"], json!({"name": "Ada"}));
    assert_eq!(desired["custom"], json!(7));
    assert!(desired.get("state").is_none(), "{desired}");
    assert_eq!(desired["focused"], true);
    assert!(desired.get("cursor").is_some());
    assert_eq!(drain_protocol_replies(id, generation).len(), 1);
    destroy_session(id);
}

#[test]
fn awareness_selection_patch_without_retained_awareness_is_a_noop() {
    let (id, snapshot) = create_ready_room();
    let generation = synchronize_ready_room(id, &snapshot);

    let result = v2_collaboration::editor_v2_collaboration_set_awareness_selection(
        id.to_string(),
        json!({"type": "text", "anchor": 2, "head": 2}).to_string(),
    );
    assert!(result.error.is_none(), "{result:?}");
    let outcome: Value =
        serde_json::from_str(result.value.as_deref().expect("selection patch value")).unwrap();

    assert_eq!(outcome, json!({"outboundChanged": false}));
    assert_eq!(desired_awareness(id).unwrap(), None);
    assert_eq!(drain_protocol_replies(id, generation).len(), 0);
    destroy_session(id);
}

#[test]
fn repeating_an_awareness_selection_patch_does_not_advance_the_local_clock() {
    let (id, snapshot) = create_ready_room();
    let generation = synchronize_ready_room(id, &snapshot);
    let initial = v2_collaboration::editor_v2_collaboration_set_awareness(
        id.to_string(),
        json!({"state": {"name": "Ada"}, "focused": true}).to_string(),
    );
    assert!(initial.error.is_none(), "{initial:?}");
    drain_protocol_replies(id, generation);

    let selection = json!({"type": "text", "anchor": 2, "head": 2}).to_string();
    let initial_patch = v2_collaboration::editor_v2_collaboration_set_awareness_selection(
        id.to_string(),
        selection.clone(),
    );
    assert!(initial_patch.error.is_none(), "{initial_patch:?}");
    assert_eq!(
        serde_json::from_str::<Value>(
            initial_patch
                .value
                .as_deref()
                .expect("selection patch value"),
        )
        .unwrap(),
        json!({"outboundChanged": true}),
    );
    let clock_after_initial_patch = local_peer(id).unwrap().clock;
    drain_protocol_replies(id, generation);

    let repeated = v2_collaboration::editor_v2_collaboration_set_awareness_selection(
        id.to_string(),
        selection,
    );
    assert!(repeated.error.is_none(), "{repeated:?}");
    assert_eq!(
        serde_json::from_str::<Value>(repeated.value.as_deref().expect("selection patch value"),)
            .unwrap(),
        json!({"outboundChanged": false}),
    );
    assert_eq!(local_peer(id).unwrap().clock, clock_after_initial_patch);
    assert_eq!(drain_protocol_replies(id, generation).len(), 0);
    destroy_session(id);
}

#[test]
fn awareness_selection_patch_updates_the_retained_cursor_while_disconnected() {
    let (id, snapshot) = create_ready_room();
    let generation = synchronize_ready_room(id, &snapshot);
    let initial = v2_collaboration::editor_v2_collaboration_set_awareness(
        id.to_string(),
        json!({"state": {"name": "Ada"}, "focused": true}).to_string(),
    );
    assert!(initial.error.is_none(), "{initial:?}");
    drain_protocol_replies(id, generation);
    transport_disconnect(id, 617).unwrap();

    let result = v2_collaboration::editor_v2_collaboration_set_awareness_selection(
        id.to_string(),
        json!({"type": "text", "anchor": 2, "head": 2}).to_string(),
    );
    assert!(result.error.is_none(), "{result:?}");
    let outcome: Value =
        serde_json::from_str(result.value.as_deref().expect("selection patch value")).unwrap();

    assert_eq!(outcome, json!({"outboundChanged": false}));
    assert!(desired_awareness(id)
        .unwrap()
        .unwrap()
        .get("cursor")
        .is_some());
    assert_eq!(pending_protocol_replies(id).unwrap(), Some((0, 0)));
    destroy_session(id);
}

#[test]
fn out_of_range_awareness_selection_patch_is_atomic() {
    let (id, snapshot) = create_ready_room();
    let generation = synchronize_ready_room(id, &snapshot);
    let initial = v2_collaboration::editor_v2_collaboration_set_awareness(
        id.to_string(),
        json!({"state": {"name": "Ada"}, "focused": true}).to_string(),
    );
    assert!(initial.error.is_none(), "{initial:?}");
    drain_protocol_replies(id, generation);
    let desired_before = desired_awareness(id).unwrap();
    let clock_before = local_peer(id).unwrap().clock;

    let result = v2_collaboration::editor_v2_collaboration_set_awareness_selection(
        id.to_string(),
        json!({"type": "text", "anchor": 999, "head": 999}).to_string(),
    );
    assert!(result.value.is_none(), "{result:?}");
    let error = result.error.expect("out-of-range selection must reject");
    assert_eq!(error.code, "AWARENESS_STATE_INVALID", "{error:?}");
    assert_eq!(desired_awareness(id).unwrap(), desired_before);
    assert_eq!(local_peer(id).unwrap().clock, clock_before);
    assert_eq!(drain_protocol_replies(id, generation).len(), 0);
    destroy_session(id);
}

#[test]
fn an_explicit_null_selection_clears_the_cursor_while_a_malformed_one_rejects_atomically() {
    let (id, snapshot) = create_ready_room();
    let generation = synchronize_ready_room(id, &snapshot);
    let accepted = json!({
        "state": { "name": "kept" },
        "focused": true,
        "selection": { "type": "text", "anchor": 7, "head": 7 },
    });
    let result = v2_collaboration::editor_v2_collaboration_set_awareness(
        id.to_string(),
        accepted.to_string(),
    );
    assert!(result.error.is_none(), "{result:?}");
    drain_protocol_replies(id, generation);

    // A malformed selection is still refused without touching any state.
    let peers_before = awareness_peers(id).unwrap();
    let desired_before = desired_awareness(id).unwrap();
    let audit_before = session_audit(id).unwrap();
    let result = v2_collaboration::editor_v2_collaboration_set_awareness(
        id.to_string(),
        json!({
            "state": { "name": "must not replace kept" },
            "focused": false,
            "selection": { "type": "node", "pos": 1 },
        })
        .to_string(),
    );
    assert!(result.value.is_none(), "{result:?}");
    let error = result.error.expect("a malformed selection must reject");
    assert_eq!(error.code, "AWARENESS_STATE_INVALID", "{error:?}");
    assert_eq!(awareness_peers(id).unwrap(), peers_before);
    assert_eq!(desired_awareness(id).unwrap(), desired_before);
    assert_eq!(session_audit(id).unwrap(), audit_before);
    assert_eq!(pending_protocol_replies(id).unwrap(), Some((0, 0)));

    // An explicit null is the caller's way to publish presence with no
    // cursor at all, distinct from omitting the key to retain one.
    let result = v2_collaboration::editor_v2_collaboration_set_awareness(
        id.to_string(),
        json!({
            "state": { "name": "cursorless" },
            "focused": false,
            "selection": Value::Null,
        })
        .to_string(),
    );
    assert!(result.error.is_none(), "{result:?}");
    let desired = desired_awareness(id).unwrap().unwrap();
    assert_eq!(desired["name"], json!("cursorless"));
    assert!(desired.get("state").is_none(), "{desired}");
    assert!(desired.get("cursor").is_none(), "{desired}");
    assert_eq!(local_peer(id).unwrap().cursor, None);
    destroy_session(id);
}

fn snapshot_state_vector(snapshot: &DocumentSnapshot) -> Vec<u8> {
    yrs::encode_state_vector_from_update_v1(&snapshot.encoded_state).unwrap()
}

#[test]
fn invalid_sticky_cursors_degrade_to_cursorless_peers_without_errors() {
    let (id, snapshot) = create_ready_room();
    let generation = synchronize_ready_room(id, &snapshot);

    // A structurally nonsensical cursor value.
    let garbage_state =
        json!({ "name": "garbage", "cursor": { "anchor": { "bogus": 1 }, "head": 2 } });
    // A well-formed sticky index minted by an unrelated document, so its
    // identifiers can never resolve against this room.
    let foreign_doc = Doc::new();
    {
        use yrs::WriteTxn as _;
        let mut txn = foreign_doc.transact_mut();
        let text = txn.get_or_insert_text("alien");
        use yrs::Text as _;
        text.insert(&mut txn, 0, "foreign content");
    }
    let foreign_sticky = {
        let txn = foreign_doc.transact();
        let text = txn.get_text("alien").unwrap();
        let branch = yrs::branch::BranchPtr::from(<yrs::types::text::TextRef as AsRef<
            yrs::branch::Branch,
        >>::as_ref(&text));
        serde_json::to_value(StickyIndex::at(&txn, branch, 3, Assoc::After).unwrap()).unwrap()
    };
    let unresolvable_state = json!({ "name": "foreign", "cursor": { "anchor": foreign_sticky, "head": foreign_sticky } });

    let outcome = receive_message(
        id,
        611,
        generation,
        &awareness_message(&[
            (7_201, 1, &garbage_state.to_string()),
            (7_202, 1, &unresolvable_state.to_string()),
        ]),
    )
    .unwrap();
    assert!(
        outcome.close.is_none(),
        "degraded cursors are not errors: {outcome:?}"
    );
    let mut peers = remote_peers(id);
    peers.sort_by_key(|peer| peer.client_id);
    assert_eq!(peers.len(), 2, "{peers:?}");
    assert_eq!(peers[0].client_id, 7_201);
    assert_eq!(peers[0].cursor, None, "{peers:?}");
    assert_eq!(
        peers[0].state, garbage_state,
        "the peer entry itself survives"
    );
    assert_eq!(peers[1].client_id, 7_202);
    assert_eq!(peers[1].cursor, None, "{peers:?}");
    destroy_session(id);
}

// ---------------------------------------------------------------------------:
// security-review findings C1/I1/I2 through the receive pipeline.

#[test]
fn max_clock_awareness_frames_close_as_incompatible_without_installing() {
    let (id, snapshot) = create_ready_room();
    let generation = synchronize_ready_room(id, &snapshot);
    let before = session_audit(id).unwrap();

    let outcome = receive_message(
        id,
        411,
        generation,
        &awareness_message(&[(6_660, u32::MAX, r#"{"name":"over"}"#)]),
    )
    .unwrap();
    let close = outcome
        .close
        .as_ref()
        .expect("a u32::MAX awareness clock must close the generation");
    assert_eq!(
        close.disposition,
        CloseDisposition::Incompatible,
        "{close:?}"
    );
    assert_eq!(
        close.error.code, TRANSPORT_AWARENESS_LIMIT_EXCEEDED,
        "{close:?}"
    );
    assert!(
        remote_peers(id).is_empty(),
        "the rejected frame installed nothing",
    );
    let mut expected = before.clone();
    expected.transport_state = TransportState::Incompatible;
    assert_eq!(session_audit(id).unwrap(), expected);
    destroy_session(id);
}

#[test]
fn high_clock_tombstones_for_unknown_clients_do_not_suppress_later_announces() {
    let (id, snapshot) = create_ready_room();
    let generation = synchronize_ready_room(id, &snapshot);

    // A removal tombstone for a never-seen client is a no-op, even at the
    // highest admissible clock: it must not squat the victim's clock space.
    let outcome = receive_message(
        id,
        421,
        generation,
        &awareness_message(&[(7_777, u32::MAX - 1, "null")]),
    )
    .unwrap();
    assert!(outcome.close.is_none(), "{outcome:?}");
    assert!(remote_peers(id).is_empty());

    let outcome = receive_message(
        id,
        422,
        generation,
        &awareness_message(&[(7_777, 1, r#"{"name":"victim"}"#)]),
    )
    .unwrap();
    assert!(outcome.close.is_none(), "{outcome:?}");
    let peers = remote_peers(id);
    assert_eq!(peers.len(), 1, "{peers:?}");
    assert_eq!(peers[0].client_id, 7_777);
    assert_eq!(peers[0].clock, 1);
    destroy_session(id);
}

#[test]
fn unknown_tombstone_storms_are_accepted_as_no_ops() {
    let (id, snapshot) = create_ready_room();
    let generation = synchronize_ready_room(id, &snapshot);
    let before = session_audit(id).unwrap();
    let before_engine = bridge::session_audit(id).unwrap();

    // A framed storm of removal tombstones for never-seen clients: accepted
    // (it is valid protocol), but it installs nothing, holds no deadlines,
    // and leaves every audit untouched.
    let storm: Vec<(u64, u32, &str)> = (0..32u64)
        .map(|index| (900_000 + index, 5_000 + index as u32, "null"))
        .collect();
    let outcome = receive_message(id, 431, generation, &awareness_message(&storm)).unwrap();
    assert!(outcome.close.is_none(), "{outcome:?}");
    assert!(remote_peers(id).is_empty());
    assert_eq!(session_audit(id).unwrap(), before);
    assert_eq!(bridge::session_audit(id).unwrap(), before_engine);

    // A query-awareness answer contains none of the storm clients.
    let outcome = receive_message(id, 432, generation, &query_awareness_message()).unwrap();
    assert!(outcome.close.is_none(), "{outcome:?}");
    let replies = drain_protocol_replies(id, generation);
    assert_eq!(replies.len(), 1, "{replies:?}");
    let reply = decode_awareness_reply(&replies[0]);
    assert!(
        reply.clients.is_empty(),
        "no storm client is ever replayed: {reply:?}",
    );

    // The storm stamped no activity deadlines: a tick far in the future
    // expires nothing.
    let outcome = drive_awareness(id, 433, 10_000_000).unwrap();
    assert!(outcome.expired_peers.is_empty(), "{outcome:?}");
    destroy_session(id);
}
