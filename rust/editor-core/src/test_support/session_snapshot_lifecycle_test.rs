//! Document-scoped snapshot lifecycle policy at the session level.
//!
//! Covers connected read-only export, the restore policy gate (active and
//! `Incompatible` transports reject `SNAPSHOT_RESTORE_CONNECTED`, pending
//! local document updates reject `SNAPSHOT_OUTBOX_NOT_EMPTY` while pending
//! protocol/awareness replies never block), manifest validation before
//! decode, fresh client identity with preserved remote clocks, the
//! teardown-on-restore cleanup (protocol replies, dependency quarantine,
//! peers, generation state), desired-awareness retention with cursor
//! recomputation, complete atomic failure audits, and the `AwaitRemote`
//! promotion row of the design transition table.

use std::collections::HashMap;

use crate::boundary::ResourceLimits;
use crate::collaboration_runtime::state::TRANSPORT_STALE_GENERATION;
use crate::native_bridge_test_support as bridge;
use crate::session_initialization_test_support::{
    ack_outbound, awareness_peers, client_id, collaboration_drive, collaboration_socket_close,
    collaboration_socket_open, create_local_json, create_room_from_json, desired_awareness,
    destroy_session, document_state, encoded_state, export_snapshot, get_json, lease_outbound,
    mark_synchronized_for_test, pending_protocol_replies, receive_message,
    remote_dependency_accounting, render_state, restore_snapshot, session_audit,
    set_desired_awareness_for_test as set_desired_awareness, transport_detach,
    transport_disconnect, transport_state, AwarenessPeerInfo, CloseDisposition, DocumentState,
    RenderState, SessionAudit, TransportState,
};
use crate::tiptap_schema;
use crate::yrs_engine::{
    DocumentScope, DocumentSnapshot, EditingLimits, InitializationMode, TransactionOrigin,
    TypedCommand, YrsDocumentEngine, YrsEngineConfig, SNAPSHOT_FORMAT_VERSION,
};
use serde_json::json;
use yrs::sync::awareness::{AwarenessUpdate, AwarenessUpdateEntry};
use yrs::sync::{Message, SyncMessage};
use yrs::updates::decoder::Decode;
use yrs::updates::encoder::Encode;
use yrs::{
    diff_updates_v1, encode_state_vector_from_update_v1, Assoc, ClientID, Doc, ReadTxn,
    StateVector, StickyIndex, Transact, Update, WriteTxn, XmlFragment, XmlOut,
};

const DOCUMENT_ID: &str = "snapshot-room";
const LINEAGE_ID: &str = "snapshot-lineage";
const JSON_SEED: &str = r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"snapshot seed"}]}]}"#;
const FRAGMENT_NAME: &str = "prosemirror";
/// Standard empty Update-v1 (zero client blocks, empty delete set): the
/// canonical valid no-op update.
const NOOP_UPDATE_V1: [u8; 2] = [0, 0];

fn source_engine() -> YrsDocumentEngine {
    source_engine_with(JSON_SEED)
}

fn source_engine_with(json: &str) -> YrsDocumentEngine {
    let mut source = YrsDocumentEngine::new(YrsEngineConfig {
        schema: tiptap_schema(),
        fragment_name: FRAGMENT_NAME.into(),
        initialization_mode: InitializationMode::LocalEmpty,
        resource_limits: ResourceLimits::default(),
        editing_limits: EditingLimits::default(),
        max_length: None,
        scope: Some(DocumentScope {
            document_id: DOCUMENT_ID.into(),
            lineage_id: LINEAGE_ID.into(),
        }),
    })
    .unwrap();
    source
        .import_json(json, TransactionOrigin::DocumentImport)
        .unwrap();
    source
}

fn snapshot_source() -> DocumentSnapshot {
    source_engine().export_snapshot().unwrap()
}

/// One shared snapshot per room: the returned snapshot IS the session's
/// lineage, so peers built from it start state-vector identical.
fn create_ready_room() -> (u64, DocumentSnapshot) {
    let snapshot = snapshot_source();
    let config = serde_json::json!({
        "documentId": snapshot.document_id,
        "lineageId": snapshot.lineage_id,
        "snapshot": snapshot,
    });
    let id = create_room_from_json(&config.to_string()).unwrap();
    bridge::attach_runtime(id).unwrap();
    (id, snapshot)
}

fn create_await_remote_room() -> u64 {
    let config = serde_json::json!({
        "documentId": DOCUMENT_ID,
        "lineageId": LINEAGE_ID,
    });
    let id = create_room_from_json(&config.to_string()).unwrap();
    bridge::attach_runtime(id).unwrap();
    id
}

/// Issue a generation through the Rust drive. A retry is driven at its exact
/// returned deadline rather than bypassing the schedule.
fn drive_generation(id: u64, request_id: u64, now_millis: u64) -> (u64, u64) {
    let initial = collaboration_drive(id, request_id, now_millis).unwrap();
    match initial.generation_to_open {
        Some(generation) => (generation, now_millis),
        None => {
            let deadline = initial
                .next_deadline_millis
                .expect("a disconnected retry must expose its Rust deadline");
            let due = collaboration_drive(id, request_id, deadline).unwrap();
            (
                due.generation_to_open
                    .expect("the due Rust drive must issue a generation"),
                deadline,
            )
        }
    }
}

/// Open the socket through the production directive and drain its queued Sync
/// Step 1 for tests that need no protocol work before their own assertions.
fn open_socket_and_ack_step1(id: u64, request_id: u64, generation: u64, now_millis: u64) {
    collaboration_socket_open(id, request_id, generation, now_millis).unwrap();
    let step1 = lease_outbound(id, request_id, generation)
        .unwrap()
        .expect("socket open must queue Sync Step 1");
    ack_outbound(id, request_id, generation, step1.lease_id).unwrap();
}

/// Drive and open a live generation with its Sync Step 1 explicitly ACKed.
fn handshake(id: u64) -> u64 {
    let (generation, now_millis) = drive_generation(id, 9_000, 0);
    open_socket_and_ack_step1(id, 9_001, generation, now_millis);
    generation
}

/// Drive a ready room to `Synchronized` through a real no-op Step 2 frame.
fn synchronize_ready_room(id: u64) -> u64 {
    let generation = handshake(id);
    let outcome =
        receive_message(id, 9_002, generation, &step2_frame(NOOP_UPDATE_V1.to_vec())).unwrap();
    assert!(outcome.close.is_none(), "{outcome:?}");
    assert_eq!(transport_state(id).unwrap(), TransportState::Synchronized);
    generation
}

fn sync_frame(message: SyncMessage) -> Vec<u8> {
    Message::Sync(message).encode_v1()
}

fn step1_frame(state_vector: &[u8]) -> Vec<u8> {
    sync_frame(SyncMessage::SyncStep1(
        StateVector::decode_v1(state_vector).unwrap(),
    ))
}

fn step2_frame(update: Vec<u8>) -> Vec<u8> {
    sync_frame(SyncMessage::SyncStep2(update))
}

fn update_frame(update: Vec<u8>) -> Vec<u8> {
    sync_frame(SyncMessage::Update(update))
}

/// Standard framed awareness message from explicit `(client, clock, json)`
/// entries, byte-compatible with `yrs::sync::Message::Awareness`.
fn awareness_message(entries: &[(u64, u32, &str)]) -> Vec<u8> {
    let clients: HashMap<ClientID, AwarenessUpdateEntry> = entries
        .iter()
        .map(|(client_id, clock, json)| {
            (
                ClientID::new(*client_id),
                AwarenessUpdateEntry {
                    clock: *clock,
                    json: (*json).into(),
                },
            )
        })
        .collect();
    Message::Awareness(AwarenessUpdate { clients }).encode_v1()
}

fn remote_peers(id: u64) -> Vec<AwarenessPeerInfo> {
    awareness_peers(id)
        .unwrap()
        .into_iter()
        .filter(|peer| !peer.is_local)
        .collect()
}

fn local_peer(id: u64) -> Option<AwarenessPeerInfo> {
    awareness_peers(id)
        .unwrap()
        .into_iter()
        .find(|peer| peer.is_local)
}

/// Structural state vector of an encoded doc state: semantic comparison,
/// never byte equality of hash-map-ordered encodings.
fn state_vector_of(encoded: &[u8]) -> StateVector {
    StateVector::decode_v1(&encode_state_vector_from_update_v1(encoded).unwrap()).unwrap()
}

fn session_state_vector(id: u64) -> StateVector {
    state_vector_of(&encoded_state(id).unwrap())
}

/// `(complete_prefix, dependent_delta)`: the delta depends on content only
/// present in the full prefix state, so a receiver holding neither must
/// quarantine the delta until the prefix arrives.
fn dependent_room_updates() -> (Vec<u8>, Vec<u8>) {
    let mut source = source_engine();
    source
        .apply_command(101, TypedCommand::InsertText { text: "a".into() })
        .unwrap();
    let after_a = source.encoded_state().unwrap();
    source
        .apply_command(102, TypedCommand::InsertText { text: "b".into() })
        .unwrap();
    let after_b = source.encoded_state().unwrap();
    let after_a_sv = encode_state_vector_from_update_v1(&after_a).unwrap();
    let delta_b = diff_updates_v1(&after_b, &after_a_sv).unwrap();
    (after_a, delta_b)
}

/// One genuine local document edit through the bridge: reserves and installs
/// exactly one pending document update in the attached outbox.
fn local_edit(id: u64, request_id: u64, text: &str) {
    let revision = bridge::session_audit(id).unwrap().document_revision;
    let envelope = serde_json::json!({
        "version": 1,
        "requestId": request_id.to_string(),
        "baseDocumentRevision": revision.to_string(),
        "text": text,
    })
    .to_string();
    bridge::submit_input(id, &envelope).unwrap();
}

/// Drain the outbound queue in order until one document update has been
/// acknowledged. An awareness broadcast published while synchronized is
/// queued ahead of a later local edit and survives disconnect, so the
/// document update is not always the front.
fn ack_next_document_lease(id: u64) {
    loop {
        let kind = bridge::ack_next_outbound(id)
            .unwrap()
            .expect("the pending local update must be leased before it can be acknowledged");
        if kind == bridge::DrainedOutboundKind::DocumentUpdate {
            return;
        }
    }
}

/// The complete observable state a restore rejection must leave untouched:
/// session/engine audits, client identity, both outbox queues, dependency
/// accounting, awareness peers, and the desired local awareness state.
#[derive(Debug, PartialEq)]
struct FullAudit {
    session: SessionAudit,
    native: bridge::NativeSessionAudit,
    client_id: u64,
    protocol_replies: Option<(usize, usize)>,
    dependency_accounting: (usize, u64),
    peers: Vec<AwarenessPeerInfo>,
    desired: Option<serde_json::Value>,
}

fn full_audit(id: u64) -> FullAudit {
    FullAudit {
        session: session_audit(id).unwrap(),
        native: bridge::session_audit(id).unwrap(),
        client_id: client_id(id).unwrap(),
        protocol_replies: pending_protocol_replies(id).unwrap(),
        dependency_accounting: remote_dependency_accounting(id).unwrap(),
        peers: awareness_peers(id).unwrap(),
        desired: desired_awareness(id).unwrap(),
    }
}

/// Serialize a sticky cursor anchored at `utf16_index` of the seed text on a
/// raw doc sharing the session's lineage (the projection idiom).
fn sticky_cursor_json(doc: &Doc, utf16_index: u32) -> serde_json::Value {
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

fn raw_doc_from_snapshot(snapshot: &DocumentSnapshot) -> Doc {
    let doc = Doc::new();
    doc.transact_mut()
        .apply_update(Update::decode_v1(&snapshot.encoded_state).unwrap())
        .unwrap();
    doc
}

#[test]
fn export_is_read_only_allowed_while_connected_and_round_trips() {
    let (id, snapshot) = create_ready_room();
    let seed_json = get_json(id).unwrap();

    // Connecting -> Handshaking -> Synchronized through real transitions;
    // export stays read-only and allowed in every connected state.
    let (generation, now_millis) = drive_generation(id, 100, 0);
    for (request_id, expected_transport) in [
        (101, TransportState::Connecting),
        (102, TransportState::Handshaking),
        (103, TransportState::Synchronized),
    ] {
        match expected_transport {
            TransportState::Handshaking => {
                open_socket_and_ack_step1(id, 104, generation, now_millis);
            }
            TransportState::Synchronized => {
                let outcome =
                    receive_message(id, 105, generation, &step2_frame(NOOP_UPDATE_V1.to_vec()))
                        .unwrap();
                assert!(outcome.close.is_none(), "{outcome:?}");
            }
            _ => {}
        }
        assert_eq!(transport_state(id).unwrap(), expected_transport);
        let before = session_audit(id).unwrap();

        let exported = export_snapshot(id, request_id).unwrap();
        assert_eq!(exported, snapshot, "{expected_transport:?}");
        assert_eq!(exported.format_version, SNAPSHOT_FORMAT_VERSION);
        assert_eq!(exported.document_id, DOCUMENT_ID);
        assert_eq!(exported.lineage_id, LINEAGE_ID);
        assert_eq!(exported.fragment_name, FRAGMENT_NAME);
        assert_eq!(exported.schema_fingerprint, snapshot.schema_fingerprint);
        Update::decode_v1(&exported.encoded_state).unwrap();

        // Export is read-only: the complete session audit is unchanged.
        assert_eq!(session_audit(id).unwrap(), before, "{expected_transport:?}");
    }

    // The exported payload round-trips through a session-level restore into
    // an awaiting room of the same lineage.
    let exported = export_snapshot(id, 106).unwrap();
    let awaiting = create_await_remote_room();
    let outcome = restore_snapshot(awaiting, 107, &exported).unwrap();
    assert!(outcome.changed);
    assert_eq!(document_state(awaiting).unwrap(), DocumentState::RoomReady);
    assert_eq!(get_json(awaiting).unwrap(), seed_json);
    assert_eq!(
        state_vector_of(&exported.encoded_state),
        session_state_vector(awaiting),
    );

    destroy_session(awaiting);
    destroy_session(id);
}

#[test]
fn connected_transports_reject_restore_with_the_frozen_snapshot_code() {
    let (id, snapshot) = create_ready_room();
    let (generation, now_millis) = drive_generation(id, 200, 0);

    // Every row is reached through the real Task 8 transitions. `Incompatible`
    // is pinned to the same code by decision: restore is
    // `Detached`/`Disconnected`-only (design line 223), and `Incompatible`
    // changes only through an explicit detach/reattach (design line 124), so
    // restore must not smuggle a transport transition out of it.
    for (request_id, expected_transport) in [
        (201, TransportState::Connecting),
        (202, TransportState::Handshaking),
        (203, TransportState::Synchronized),
        (204, TransportState::Incompatible),
    ] {
        match expected_transport {
            TransportState::Handshaking => {
                open_socket_and_ack_step1(id, 205, generation, now_millis);
            }
            TransportState::Synchronized => {
                let outcome =
                    receive_message(id, 206, generation, &step2_frame(NOOP_UPDATE_V1.to_vec()))
                        .unwrap();
                assert!(outcome.close.is_none(), "{outcome:?}");
            }
            TransportState::Incompatible => {
                collaboration_socket_close(
                    id,
                    207,
                    generation,
                    CloseDisposition::Incompatible,
                    now_millis,
                )
                .unwrap();
            }
            _ => {}
        }
        assert_eq!(transport_state(id).unwrap(), expected_transport);
        let before = full_audit(id);

        let error = restore_snapshot(id, request_id, &snapshot).unwrap_err();
        assert_eq!(error.domain, "snapshot", "{expected_transport:?}");
        assert_eq!(
            error.code, "SNAPSHOT_RESTORE_CONNECTED",
            "{expected_transport:?}"
        );
        assert_eq!(error.request_id, Some(request_id), "{expected_transport:?}");

        assert_eq!(full_audit(id), before, "{expected_transport:?}");
    }

    destroy_session(id);
}

#[test]
fn pending_document_updates_reject_restore_until_drained() {
    let (id, snapshot) = create_ready_room();
    assert_eq!(transport_state(id).unwrap(), TransportState::Disconnected);

    local_edit(id, 300, " offline");
    let (pending, pending_bytes) = bridge::outbox_pending(id).unwrap().unwrap();
    assert_eq!(pending, 1);
    assert!(pending_bytes > 0);
    let before = full_audit(id);

    let error = restore_snapshot(id, 301, &snapshot).unwrap_err();
    assert_eq!(error.domain, "snapshot");
    assert_eq!(error.code, "SNAPSHOT_OUTBOX_NOT_EMPTY");
    assert_eq!(error.request_id, Some(301));
    assert_eq!(full_audit(id), before);

    // Draining the genuine pending edit (transport handoff) unblocks restore.
    ack_next_document_lease(id);
    assert_eq!(bridge::outbox_pending(id).unwrap().unwrap().0, 0);
    let outcome = restore_snapshot(id, 302, &snapshot).unwrap();
    assert!(
        outcome.changed,
        "the drained edit is discarded by the restore"
    );
    assert_eq!(
        get_json(id).unwrap(),
        serde_json::from_str::<serde_json::Value>(JSON_SEED).unwrap(),
    );

    destroy_session(id);
}

#[test]
fn protocol_and_awareness_frames_alone_do_not_block_restore_and_are_cleared() {
    let (id, snapshot) = create_ready_room();
    let generation = synchronize_ready_room(id);

    // A desired-state awareness broadcast plus a Step 1 reply: protocol
    // entries only, never document updates.
    let desired = json!({ "name": "local author" });
    set_desired_awareness(id, 400, &desired.to_string()).unwrap();
    let outcome = receive_message(
        id,
        401,
        generation,
        &step1_frame(&StateVector::default().encode_v1()),
    )
    .unwrap();
    assert!(outcome.close.is_none(), "{outcome:?}");
    assert_eq!(outcome.replies_enqueued, 1, "{outcome:?}");
    let (reply_count, reply_bytes) = pending_protocol_replies(id).unwrap().unwrap();
    assert_eq!(reply_count, 2, "awareness broadcast + Step 1 reply");
    assert!(reply_bytes > 0);
    assert_eq!(bridge::outbox_pending(id).unwrap().unwrap().0, 0);

    // Disconnecting does NOT drain the protocol queue: the replies are still
    // pending, so restore is provably what clears them.
    transport_disconnect(id, 402).unwrap();
    assert_eq!(transport_state(id).unwrap(), TransportState::Disconnected);
    assert_eq!(
        pending_protocol_replies(id).unwrap().unwrap().0,
        2,
        "protocol replies survive disconnect; only restore clears them",
    );

    let outcome = restore_snapshot(id, 403, &snapshot).unwrap();
    assert!(
        !outcome.changed,
        "no document mutation happened since creation"
    );
    assert_eq!(pending_protocol_replies(id).unwrap().unwrap(), (0, 0));
    assert_eq!(bridge::outbox_pending(id).unwrap().unwrap().0, 0);
    // Desired local awareness is retained across the restore.
    assert_eq!(desired_awareness(id).unwrap(), Some(desired));

    destroy_session(id);
}

#[test]
fn every_manifest_mismatch_rejects_before_decode_with_an_unchanged_audit() {
    let (id, snapshot) = create_ready_room();

    let cases: [(&str, &str); 5] = [
        ("formatVersion", "SNAPSHOT_VERSION_UNSUPPORTED"),
        ("documentId", "SNAPSHOT_SCOPE_MISMATCH"),
        ("lineageId", "SNAPSHOT_LINEAGE_MISMATCH"),
        ("fragmentName", "SNAPSHOT_FRAGMENT_MISMATCH"),
        ("schemaFingerprint", "SNAPSHOT_SCHEMA_MISMATCH"),
    ];
    for (request_id, (field, expected_code)) in cases.iter().enumerate() {
        let mut mismatched = snapshot.clone();
        match *field {
            "formatVersion" => mismatched.format_version += 1,
            "documentId" => mismatched.document_id = "other-document".into(),
            "lineageId" => mismatched.lineage_id = "other-lineage".into(),
            "fragmentName" => mismatched.fragment_name = "other-fragment".into(),
            "schemaFingerprint" => mismatched.schema_fingerprint = "other-schema".into(),
            _ => unreachable!(),
        }
        // Malformed state proves validation precedes any decode work.
        mismatched.encoded_state = vec![0xff, 0xff, 0xff];

        let before = full_audit(id);
        let request_id = request_id as u64 + 500;
        let error = restore_snapshot(id, request_id, &mismatched).unwrap_err();
        assert_eq!(error.domain, "snapshot", "{field}");
        assert_eq!(error.code, *expected_code, "{field}");
        assert_eq!(error.request_id, Some(request_id), "{field}");
        assert_eq!(full_audit(id), before, "{field}: no candidate was touched");
    }

    destroy_session(id);
}

#[test]
fn restore_installs_a_fresh_client_identity_and_preserves_remote_clocks() {
    let (id, snapshot) = create_ready_room();
    let writer_before = client_id(id).unwrap();

    // A genuine local edit (drained, so restore is allowed) gives the
    // pre-restore client a durable clock the restore must discard.
    local_edit(id, 600, " local");
    ack_next_document_lease(id);
    assert!(session_state_vector(id).get(&ClientID::new(writer_before)) > 0);

    let outcome = restore_snapshot(id, 601, &snapshot).unwrap();
    assert!(outcome.changed);
    let writer_after = client_id(id).unwrap();
    assert_ne!(
        writer_after, writer_before,
        "restore mints a fresh identity"
    );

    // A new local edit lands under the fresh identity while every remote
    // clock carried by the snapshot is preserved exactly.
    local_edit(id, 602, " again");
    let after = session_state_vector(id);
    assert!(
        after.get(&ClientID::new(writer_after)) > 0,
        "the fresh identity writes: {after:?}",
    );
    let snapshot_vector = state_vector_of(&snapshot.encoded_state);
    for (client, clock) in snapshot_vector.iter() {
        assert_eq!(
            after.get(client),
            *clock,
            "remote client {client:?} clock must be preserved",
        );
    }
    assert_eq!(
        after.get(&ClientID::new(writer_before)),
        0,
        "the pre-restore local clock is discarded with the prior store",
    );

    destroy_session(id);
}

#[test]
fn restore_clears_protocol_replies_quarantine_peers_and_generation_state() {
    let (prefix, delta_b) = dependent_room_updates();
    let (id, snapshot) = create_ready_room();
    let generation = synchronize_ready_room(id);

    // Quarantine a dependent update: the engine retains the payload, the
    // runtime retains only byte/work accounting.
    let outcome = receive_message(id, 700, generation, &update_frame(delta_b.clone())).unwrap();
    assert!(outcome.close.is_none(), "{outcome:?}");
    assert!(!outcome.remote_commit_applied, "{outcome:?}");
    assert_eq!(
        remote_dependency_accounting(id).unwrap(),
        (delta_b.len(), delta_b.len() as u64),
    );
    // One awareness peer and one pending Step 1 reply.
    receive_message(
        id,
        701,
        generation,
        &awareness_message(&[(4_242, 1, r#"{"name":"peer one"}"#)]),
    )
    .unwrap();
    assert_eq!(remote_peers(id).len(), 1);
    let outcome = receive_message(
        id,
        702,
        generation,
        &step1_frame(&StateVector::default().encode_v1()),
    )
    .unwrap();
    assert_eq!(outcome.replies_enqueued, 1, "{outcome:?}");
    assert_eq!(pending_protocol_replies(id).unwrap().unwrap().0, 1);

    // Disconnect clears the peers (Task 10) but neither the protocol queue
    // nor the quarantine: restore is provably what clears those.
    transport_disconnect(id, 703).unwrap();
    assert_eq!(remote_peers(id).len(), 0);
    assert_eq!(pending_protocol_replies(id).unwrap().unwrap().0, 1);
    assert_eq!(
        remote_dependency_accounting(id).unwrap(),
        (delta_b.len(), delta_b.len() as u64),
    );

    let outcome = restore_snapshot(id, 704, &snapshot).unwrap();
    assert!(!outcome.changed);

    // Per-item cleanup, each observable:
    // 1. prior-store protocol replies are gone;
    assert_eq!(pending_protocol_replies(id).unwrap().unwrap(), (0, 0));
    // 2. the engine-owned dependency quarantine is empty and the runtime's
    //    work accounting reset;
    assert_eq!(remote_dependency_accounting(id).unwrap(), (0, 0));
    // 3. no awareness peers survive the restore;
    assert!(awareness_peers(id).unwrap().is_empty());
    // 4. the transport settled to Disconnected and the pre-restore generation
    //    refuses every callback as stale;
    assert_eq!(transport_state(id).unwrap(), TransportState::Disconnected);
    let error = collaboration_socket_close(id, 705, generation, CloseDisposition::Retryable, 0)
        .unwrap_err();
    assert_eq!(error.code, TRANSPORT_STALE_GENERATION, "{error:?}");
    let error =
        receive_message(id, 706, generation, &step2_frame(NOOP_UPDATE_V1.to_vec())).unwrap_err();
    assert_eq!(error.code, TRANSPORT_STALE_GENERATION, "{error:?}");
    let error = mark_synchronized_for_test(id, 707, generation).unwrap_err();
    assert_eq!(error.code, TRANSPORT_STALE_GENERATION, "{error:?}");
    assert_eq!(transport_state(id).unwrap(), TransportState::Disconnected);

    // A fresh connection issues a strictly newer generation (never reissued)
    // and the cleared quarantine treats the same dependent update as
    // missing-dependencies anew: exactly one payload's worth of accounting,
    // then the prefix drains it and the document converges.
    let (next_generation, next_now_millis) = drive_generation(id, 708, 0);
    assert!(next_generation > generation);
    open_socket_and_ack_step1(id, 709, next_generation, next_now_millis);
    let outcome = receive_message(
        id,
        710,
        next_generation,
        &step2_frame(NOOP_UPDATE_V1.to_vec()),
    )
    .unwrap();
    assert!(outcome.close.is_none(), "{outcome:?}");
    assert_eq!(transport_state(id).unwrap(), TransportState::Synchronized);

    let outcome =
        receive_message(id, 711, next_generation, &update_frame(delta_b.clone())).unwrap();
    assert!(outcome.close.is_none(), "{outcome:?}");
    assert!(!outcome.remote_commit_applied, "{outcome:?}");
    assert_eq!(
        remote_dependency_accounting(id).unwrap(),
        (delta_b.len(), delta_b.len() as u64),
        "the same delta quarantines anew — no prior-store residue merged",
    );
    let outcome = receive_message(id, 712, next_generation, &update_frame(prefix)).unwrap();
    assert!(outcome.close.is_none(), "{outcome:?}");
    assert!(outcome.remote_commit_applied, "{outcome:?}");
    assert_eq!(remote_dependency_accounting(id).unwrap(), (0, 0));
    let json = get_json(id).unwrap().to_string();
    assert!(json.contains("ab"), "the dependent update drains: {json}");

    destroy_session(id);
}

#[test]
fn desired_awareness_survives_restore_and_its_cursor_is_recomputed() {
    let (id, snapshot) = create_ready_room();
    let _generation = synchronize_ready_room(id);

    // Desired state with a cursor anchored after "snapsh" (utf16 index 6 of
    // the seed text; text content starts at doc position 1, so it resolves
    // to 7).
    let raw_doc = raw_doc_from_snapshot(&snapshot);
    let cursor = sticky_cursor_json(&raw_doc, 6);
    let desired = json!({ "name": "author", "cursor": { "anchor": cursor, "head": cursor } });
    set_desired_awareness(id, 800, &desired.to_string()).unwrap();
    assert_eq!(local_peer(id).unwrap().cursor, Some((7, 7)));

    transport_disconnect(id, 801).unwrap();
    assert!(
        local_peer(id).is_none(),
        "disconnect tombstones the live local entry",
    );
    assert_eq!(desired_awareness(id).unwrap(), Some(desired.clone()));

    // Make the store differ from the snapshot, then drain so restore runs.
    local_edit(id, 802, " more");
    ack_next_document_lease(id);
    let outcome = restore_snapshot(id, 803, &snapshot).unwrap();
    assert!(outcome.changed);

    // The desired state survives and is live again under the fresh identity;
    // its cursor remaps against the restored store — the anchor items are in
    // the snapshot, so it resolves to the same position, never a stale one.
    assert_eq!(desired_awareness(id).unwrap(), Some(desired.clone()));
    let local = local_peer(id).expect("the store swap re-publishes the desired state");
    assert!(local.is_local);
    assert_eq!(local.state, desired);
    assert_eq!(local.cursor, Some((7, 7)), "{local:?}");

    // Restoring a store that does NOT contain the anchor items leaves the
    // cursor absent — never silently pointing at the wrong position.
    let other = source_engine_with(
        r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"entirely different body"}]}]}"#,
    )
    .export_snapshot()
    .unwrap();
    let outcome = restore_snapshot(id, 804, &other).unwrap();
    assert!(outcome.changed);
    assert_eq!(desired_awareness(id).unwrap(), Some(desired.clone()));
    let local = local_peer(id).expect("the desired state survives the second restore");
    assert_eq!(local.state, desired);
    assert_eq!(
        local.cursor, None,
        "an unresolvable prior-store cursor is absent, never wrong: {local:?}",
    );

    destroy_session(id);
}

#[test]
fn restore_failures_are_atomic_across_session_engine_outbox_and_runtime() {
    let (_prefix, delta_b) = dependent_room_updates();
    let (id, snapshot) = create_ready_room();
    let generation = synchronize_ready_room(id);

    // Seed restorable residue: quarantined dependency, one protocol reply.
    receive_message(id, 900, generation, &update_frame(delta_b.clone())).unwrap();
    let outcome = receive_message(
        id,
        901,
        generation,
        &step1_frame(&StateVector::default().encode_v1()),
    )
    .unwrap();
    assert_eq!(outcome.replies_enqueued, 1, "{outcome:?}");
    transport_disconnect(id, 902).unwrap();
    assert_eq!(
        remote_dependency_accounting(id).unwrap(),
        (delta_b.len(), delta_b.len() as u64),
    );
    assert_eq!(pending_protocol_replies(id).unwrap().unwrap().0, 1);

    // Failure class 1: malformed encoded state (decode-time failure).
    let mut malformed = snapshot.clone();
    malformed.encoded_state = vec![0xff, 0xff, 0xff];
    let before = full_audit(id);
    let error = restore_snapshot(id, 903, &malformed).unwrap_err();
    assert_eq!(error.domain, "snapshot");
    assert_eq!(error.code, "COLLABORATION_DECODE_FAILED");
    assert_eq!(error.request_id, Some(903));
    assert_eq!(full_audit(id), before, "decode failure is fully atomic");

    // Failure class 2: a genuine mid-restore failure — the encoded state
    // decodes as a valid Update-v1, then the candidate fails document
    // validation ("root text is not a block"). No synthetic failpoint exists
    // inside the restore candidate pipeline and none was needed: every
    // fallible stage is pre-commit candidate construction (manifest,
    // preflight, decode, candidate build, derived admission), each covered
    // here and in the manifest matrix with full-audit equality, while the
    // post-commit section (store swap, rebinding, runtime reset, transport
    // settle) is infallible by construction.
    let invalid_doc = Doc::new();
    {
        let mut txn = invalid_doc.transact_mut();
        let fragment = txn.get_or_insert_xml_fragment(FRAGMENT_NAME);
        use yrs::types::xml::XmlFragment as _;
        use yrs::types::xml::XmlTextPrelim;
        fragment.push_back(&mut txn, XmlTextPrelim::new("root text is not a block"));
    }
    let mut invalid = snapshot.clone();
    invalid.encoded_state = invalid_doc
        .transact()
        .encode_state_as_update_v1(&StateVector::default());
    let error = restore_snapshot(id, 904, &invalid).unwrap_err();
    assert_eq!(error.domain, "snapshot");
    assert_eq!(error.code, "DOCUMENT_INVALID");
    assert_eq!(error.request_id, Some(904));
    assert_eq!(
        full_audit(id),
        before,
        "mid-restore candidate failure is fully atomic",
    );

    // The seeded residue survives both failures: a later valid restore still
    // clears it, proving failures never half-teardown.
    let outcome = restore_snapshot(id, 905, &snapshot).unwrap();
    assert!(!outcome.changed);
    assert_eq!(remote_dependency_accounting(id).unwrap(), (0, 0));
    assert_eq!(pending_protocol_replies(id).unwrap().unwrap(), (0, 0));

    destroy_session(id);
}

#[test]
fn await_remote_restore_promotes_to_room_ready_and_settles_disconnected() {
    let snapshot = snapshot_source();

    // Disconnected row: export of an awaiting document rejects, restore
    // promotes and the transport stays Disconnected.
    let id = create_await_remote_room();
    assert_eq!(document_state(id).unwrap(), DocumentState::AwaitRemote);
    assert_eq!(render_state(id).unwrap(), RenderState::Loading);
    let error = export_snapshot(id, 1_000).unwrap_err();
    assert_eq!(error.domain, "snapshot");
    assert_eq!(error.code, "DOCUMENT_INVALID");
    assert_eq!(error.request_id, Some(1_000));
    let revision_before = session_audit(id).unwrap().document_revision;

    let outcome = restore_snapshot(id, 1_001, &snapshot).unwrap();
    assert!(outcome.changed);
    assert_eq!(outcome.document_revision, revision_before + 1);
    assert_eq!(document_state(id).unwrap(), DocumentState::RoomReady);
    assert_eq!(render_state(id).unwrap(), RenderState::Ready);
    assert_eq!(transport_state(id).unwrap(), TransportState::Disconnected);
    assert_eq!(
        get_json(id).unwrap(),
        serde_json::from_str::<serde_json::Value>(JSON_SEED).unwrap(),
    );
    // Once promoted, export works and carries the restored state.
    let exported = export_snapshot(id, 1_002).unwrap();
    assert_eq!(exported, snapshot);
    destroy_session(id);

    // Detached row: restore is allowed and settles the transport back to
    // Disconnected (design transition-table row: AwaitRemote +
    // Detached/Disconnected -> RoomReady + Disconnected).
    let id = create_await_remote_room();
    transport_detach(id, 1_003).unwrap();
    assert_eq!(transport_state(id).unwrap(), TransportState::Detached);
    let outcome = restore_snapshot(id, 1_004, &snapshot).unwrap();
    assert!(outcome.changed);
    assert_eq!(document_state(id).unwrap(), DocumentState::RoomReady);
    assert_eq!(
        transport_state(id).unwrap(),
        TransportState::Disconnected,
        "restore settles the transport to the room's disconnected row",
    );
    // The settled transport issues a fresh generation through Rust's drive.
    assert!(collaboration_drive(id, 1_005, 0)
        .unwrap()
        .generation_to_open
        .is_some());
    destroy_session(id);
}

#[test]
fn local_sessions_reject_snapshot_operations_without_a_scope() {
    let id = create_local_json(JSON_SEED).unwrap();
    let snapshot = snapshot_source();
    let before = session_audit(id).unwrap();
    let client_before = client_id(id).unwrap();

    let error = export_snapshot(id, 1_100).unwrap_err();
    assert_eq!(error.domain, "snapshot");
    assert_eq!(error.code, "SNAPSHOT_SCOPE_MISMATCH");
    assert_eq!(error.request_id, Some(1_100));

    let error = restore_snapshot(id, 1_101, &snapshot).unwrap_err();
    assert_eq!(error.domain, "snapshot");
    assert_eq!(error.code, "SNAPSHOT_SCOPE_MISMATCH");
    assert_eq!(error.request_id, Some(1_101));

    assert_eq!(session_audit(id).unwrap(), before);
    assert_eq!(client_id(id).unwrap(), client_before);
    destroy_session(id);
}
