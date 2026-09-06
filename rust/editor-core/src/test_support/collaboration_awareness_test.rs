//! Bounded awareness and peer projections.
//!
//! Covers awareness/query-awareness handling through the `receive_message` classification
//! pipeline, runtime ownership of desired local awareness across transport lifecycle
//! (including the reconnect re-publish that mitigates the tombstone-migration gap),
//! deterministic renewal/expiry clocks driven exclusively through
//! `collaboration_drive(now_millis)`, tombstone semantics through the runtime path, sticky
//! cursor projections that recompute with document revisions, and every awareness ceiling
//! at its exact and one-over boundary with atomic rejection. Wire compatibility is proven
//! against independent raw `yrs::sync::Awareness` peers in both directions; encoded
//! awareness bytes are hash-map-ordered, so assertions always compare decoded structures.

use std::collections::HashMap;

use crate::boundary::ResourceLimits;
use crate::collaboration_runtime::awareness::{
    AWARENESS_EXPIRY_MILLIS, AWARENESS_RENEWAL_INTERVAL_MILLIS,
};
use crate::collaboration_runtime::protocol::{
    TRANSPORT_AWARENESS_LIMIT_EXCEEDED, TRANSPORT_FRAME_LIMIT_EXCEEDED, TRANSPORT_PROTOCOL_INVALID,
    TRANSPORT_REPLY_LIMIT_EXCEEDED, TRANSPORT_RESOURCE_EXHAUSTED,
};
use crate::collaboration_runtime::CollaborationRuntime;
use crate::ffi_v2::collaboration as v2_collaboration;
use crate::native_bridge_test_support as bridge;
use crate::session::{CollaborationLimits, TransportState as RuntimeTransportState};
use crate::session_initialization_test_support::{
    ack_outbound, awareness_peers, clear_desired_awareness, collaboration_drive,
    collaboration_receive, collaboration_socket_close, collaboration_socket_open,
    create_room_from_json, desired_awareness, destroy_session, document_state, lease_outbound,
    pending_protocol_replies, receive_message, restore_snapshot, session_audit,
    set_collaboration_limit_for_test, set_desired_awareness_for_test as set_desired_awareness,
    transport_detach, transport_disconnect, transport_reattach, transport_state, AwarenessPeerInfo,
    CloseDisposition, DocumentState, ReceiveOutcome, SessionAudit, TestError, TransportState,
};
use crate::tiptap_schema;
use crate::yrs_engine::{
    DocumentScope, DocumentSnapshot, EditingLimits, InitializationMode, TransactionOrigin,
    YrsDocumentEngine, YrsEngineConfig,
};
use serde_json::{json, Value};
use yrs::sync::awareness::{Awareness, AwarenessUpdate, AwarenessUpdateEntry};
use yrs::sync::Message;
use yrs::updates::decoder::Decode;
use yrs::updates::encoder::Encode;
use yrs::{Assoc, ClientID, Doc, ReadTxn, StickyIndex, Transact, Update, XmlFragment, XmlOut};

const DOCUMENT_ID: &str = "awareness-room";
const LINEAGE_ID: &str = "awareness-lineage";
const JSON_SEED: &str = r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"awareness seed"}]}]}"#;
const FRAGMENT_NAME: &str = "prosemirror";

fn source_engine() -> YrsDocumentEngine {
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
        .import_json(JSON_SEED, TransactionOrigin::DocumentImport)
        .unwrap();
    source
}

fn snapshot_source() -> DocumentSnapshot {
    source_engine().export_snapshot().unwrap()
}

/// One shared snapshot per test: the returned snapshot IS the session's
/// lineage, so raw peers built from it start state-vector identical.
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

/// Drive a generation now or at the exact retry deadline Rust returns.
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

/// Open through the production callback and ACK the queued Sync Step 1 for
/// setups that need a drained protocol outbox.
fn open_socket_and_ack_step1(id: u64, request_id: u64, generation: u64, now_millis: u64) {
    collaboration_socket_open(id, request_id, generation, now_millis).unwrap();
    let step1 = lease_outbound(id, request_id, generation)
        .unwrap()
        .expect("socket open must queue Sync Step 1");
    ack_outbound(id, request_id, generation, step1.lease_id).unwrap();
}

/// The production-shaped handshake setup, with the socket-open Step 1
/// explicitly ACKed so later protocol assertions start from an empty queue.
fn handshake_at(id: u64, now_millis: u64) -> (u64, u64) {
    let (generation, opened_at) = drive_generation(id, 9_000, now_millis);
    open_socket_and_ack_step1(id, 9_001, generation, opened_at);
    (generation, opened_at)
}

fn handshake(id: u64) -> u64 {
    handshake_at(id, 0).0
}

/// Drive a ready room to `Synchronized` through a real no-op Step 2 from a
/// raw peer sharing the session's snapshot lineage.
fn synchronize_ready_room(id: u64, snapshot: &DocumentSnapshot) -> u64 {
    synchronize_ready_room_at(id, snapshot, 0).0
}

fn synchronize_ready_room_at(id: u64, snapshot: &DocumentSnapshot, now_millis: u64) -> (u64, u64) {
    let (generation, opened_at) = handshake_at(id, now_millis);
    let server = raw_doc_from_snapshot(snapshot);
    let step2 = server
        .transact()
        .encode_state_as_update_v1(&yrs::StateVector::default());
    let directive = collaboration_receive(
        id,
        9_002,
        generation,
        &Message::Sync(yrs::sync::SyncMessage::SyncStep2(step2)).encode_v1(),
        opened_at,
    )
    .unwrap();
    assert!(!directive.remote_commit_applied, "{directive:?}");
    assert_eq!(transport_state(id).unwrap(), TransportState::Synchronized);
    (generation, opened_at)
}

/// Reconnect when a retained protocol reply precedes the new socket-open
/// Sync Step 1. Both frames must be leased and ACKed in FIFO order before
/// the normal Step 2 completion; a generic empty-outbox handshake cannot
/// safely assume its first lease is Step 1 here.
fn synchronize_ready_room_after_draining_retained_protocol_reply(
    id: u64,
    snapshot: &DocumentSnapshot,
) -> u64 {
    let (generation, opened_at) = drive_generation(id, 9_003, 0);
    collaboration_socket_open(id, 9_004, generation, opened_at).unwrap();

    let retained_reply = lease_outbound(id, 9_005, generation)
        .unwrap()
        .expect("the prior protocol reply must stay queued across reconnect");
    ack_outbound(id, 9_005, generation, retained_reply.lease_id).unwrap();

    let step1 = lease_outbound(id, 9_006, generation)
        .unwrap()
        .expect("socket open must queue Sync Step 1 after the retained reply");
    assert!(matches!(
        Message::decode_v1(&step1.frame).unwrap(),
        Message::Sync(yrs::sync::SyncMessage::SyncStep1(_))
    ));
    ack_outbound(id, 9_006, generation, step1.lease_id).unwrap();

    let server = raw_doc_from_snapshot(snapshot);
    let step2 = server
        .transact()
        .encode_state_as_update_v1(&yrs::StateVector::default());
    let directive = collaboration_receive(
        id,
        9_007,
        generation,
        &Message::Sync(yrs::sync::SyncMessage::SyncStep2(step2)).encode_v1(),
        opened_at,
    )
    .unwrap();
    assert!(!directive.remote_commit_applied, "{directive:?}");
    assert_eq!(transport_state(id).unwrap(), TransportState::Synchronized);
    generation
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AwarenessDriveOutcome {
    renewed_local: bool,
    outbound_changed: bool,
    expired_peers: Vec<u64>,
    peers_changed: bool,
    next_deadline_millis: Option<u64>,
}

/// Test-local projection of the production directive for legacy awareness
/// assertions. It never bypasses the driver: outbound change is derived from
/// the real protocol outbox before and after `collaboration_drive`.
fn drive_awareness(
    id: u64,
    request_id: u64,
    now_millis: u64,
) -> Result<AwarenessDriveOutcome, TestError> {
    let protocol_before = pending_protocol_replies(id)?;
    let directive = collaboration_drive(id, request_id, now_millis)?;
    let protocol_after = pending_protocol_replies(id)?;
    Ok(AwarenessDriveOutcome {
        renewed_local: directive.renewed_local,
        outbound_changed: protocol_before != protocol_after,
        expired_peers: directive.expired_peers,
        peers_changed: directive.peers_changed,
        next_deadline_millis: directive.next_deadline_millis,
    })
}

fn raw_doc_from_snapshot(snapshot: &DocumentSnapshot) -> Doc {
    let doc = Doc::new();
    doc.transact_mut()
        .apply_update(Update::decode_v1(&snapshot.encoded_state).unwrap())
        .unwrap();
    doc
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

fn query_awareness_message() -> Vec<u8> {
    Message::AwarenessQuery.encode_v1()
}

/// Decode one framed protocol reply as an awareness update the way a
/// standard peer would; assertions then compare the decoded structure.
fn decode_awareness_reply(reply: &[u8]) -> AwarenessUpdate {
    match Message::decode_v1(reply).expect("reply must decode as a y-protocols message") {
        Message::Awareness(update) => update,
        other => panic!("expected an awareness reply, got {other:?}"),
    }
}

fn drain_protocol_replies(id: u64, generation: u64) -> Vec<Vec<u8>> {
    let mut replies = Vec::new();
    while pending_protocol_replies(id).unwrap().unwrap_or((0, 0)).0 > 0 {
        let lease = lease_outbound(id, 90_000 + replies.len() as u64, generation)
            .unwrap()
            .expect("a pending protocol reply must be retained by an outbound lease");
        replies.push(lease.frame);
        ack_outbound(
            id,
            91_000 + replies.len() as u64,
            generation,
            lease.lease_id,
        )
        .unwrap();
    }
    replies
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

// Protocol integration: awareness and query-awareness through receive_message

#[test]
fn awareness_updates_project_peers_without_touching_document_state() {
    let (id, snapshot) = create_ready_room();
    let generation = synchronize_ready_room(id, &snapshot);
    let before = session_audit(id).unwrap();
    let before_engine = bridge::session_audit(id).unwrap();

    let state = json!({ "name": "peer one" });
    let outcome = receive_message(
        id,
        301,
        generation,
        &awareness_message(&[(4_242, 1, &state.to_string())]),
    )
    .unwrap();
    assert!(outcome.close.is_none(), "{outcome:?}");
    assert_eq!(outcome.frames_decoded, 1, "{outcome:?}");
    assert_eq!(outcome.replies_enqueued, 0, "{outcome:?}");
    assert!(!outcome.remote_commit_applied, "{outcome:?}");
    assert!(!outcome.document_promoted, "{outcome:?}");
    assert_eq!(outcome.transport_state, TransportState::Synchronized);

    let peers = remote_peers(id);
    assert_eq!(peers.len(), 1, "{peers:?}");
    assert_eq!(peers[0].client_id, 4_242);
    assert_eq!(peers[0].clock, 1);
    assert_eq!(peers[0].state, state);
    assert_eq!(
        peers[0].cursor, None,
        "cursor-less state projects no cursor"
    );
    assert!(local_peer(id).is_none(), "no desired state was published");

    // Awareness never touches durable document state, revisions, history,
    // document outbox, or document/transport lifecycle.
    assert_eq!(session_audit(id).unwrap(), before);
    assert_eq!(bridge::session_audit(id).unwrap(), before_engine);
    assert_eq!(bridge::outbox_pending(id).unwrap().unwrap().0, 0);
    assert_eq!(document_state(id).unwrap(), DocumentState::RoomReady);
    destroy_session(id);
}

#[test]
fn awareness_frames_never_synchronize_a_handshaking_transport() {
    let (id, _snapshot) = create_ready_room();
    let generation = handshake(id);

    let outcome = receive_message(
        id,
        311,
        generation,
        &awareness_message(&[(7_007, 1, r#"{"name":"early"}"#)]),
    )
    .unwrap();
    assert!(outcome.close.is_none(), "{outcome:?}");
    assert!(!outcome.document_promoted, "{outcome:?}");
    assert_eq!(
        outcome.transport_state,
        TransportState::Handshaking,
        "Synchronized entry stays Step-2-only",
    );
    assert_eq!(transport_state(id).unwrap(), TransportState::Handshaking);
    let peers = remote_peers(id);
    assert_eq!(peers.len(), 1, "{peers:?}");
    assert_eq!(peers[0].client_id, 7_007);
    destroy_session(id);
}

#[test]
fn query_awareness_reply_answers_completely_and_interoperates_with_a_raw_peer() {
    let (id, snapshot) = create_ready_room();
    let generation = synchronize_ready_room(id, &snapshot);
    let desired = json!({ "name": "local author" });
    set_desired_awareness(id, 321, &desired.to_string()).unwrap();
    let local_client = local_peer(id).expect("desired state projects locally");
    drain_protocol_replies(id, generation);

    // One live peer and one peer that announced then tombstoned itself.
    let live_state = json!({ "name": "alive" });
    receive_message(
        id,
        322,
        generation,
        &awareness_message(&[(6_001, 3, &live_state.to_string())]),
    )
    .unwrap();
    receive_message(
        id,
        323,
        generation,
        &awareness_message(&[(6_002, 1, r#"{"name":"gone"}"#), (6_002, 2, "null")]),
    )
    .unwrap();

    let outcome = receive_message(id, 324, generation, &query_awareness_message()).unwrap();
    assert!(outcome.close.is_none(), "{outcome:?}");
    assert_eq!(outcome.replies_enqueued, 1, "{outcome:?}");
    let replies = drain_protocol_replies(id, generation);
    assert_eq!(replies.len(), 1, "{replies:?}");
    let reply = decode_awareness_reply(&replies[0]);

    // Complete answer: our local state plus every live peer; the tombstoned
    // client is excluded per standard y-protocols semantics.
    let mut clients: Vec<u64> = reply.clients.keys().map(|client| client.get()).collect();
    clients.sort_unstable();
    let mut expected = vec![local_client.client_id, 6_001];
    expected.sort_unstable();
    assert_eq!(clients, expected, "{reply:?}");
    let live_entry = &reply.clients[&ClientID::new(6_001)];
    assert_eq!(live_entry.clock, 3);
    assert_eq!(
        serde_json::from_str::<Value>(live_entry.json.as_ref()).unwrap(),
        live_state,
    );

    // Raw-peer interop: a standard yrs awareness applying our reply sees the
    // same live states.
    let mut raw = Awareness::new(Doc::new());
    raw.apply_update(reply).unwrap();
    assert_eq!(
        raw.state::<Value>(ClientID::new(local_client.client_id)),
        Some(desired),
    );
    assert_eq!(raw.state::<Value>(ClientID::new(6_001)), Some(live_state));
    assert_eq!(raw.state::<Value>(ClientID::new(6_002)), None);
    destroy_session(id);
}

#[test]
fn awareness_and_query_frames_count_against_the_frame_ceiling() {
    let (id, snapshot) = create_ready_room();
    let generation = synchronize_ready_room(id, &snapshot);
    set_collaboration_limit_for_test(id, "maxFramesPerMessage", 1).unwrap();

    let message: Vec<u8> = [
        awareness_message(&[(5_001, 1, r#"{"n":1}"#)]),
        query_awareness_message(),
    ]
    .concat();
    let outcome = receive_message(id, 331, generation, &message).unwrap();
    let close = outcome.close.as_ref().expect("one frame over must close");
    assert_eq!(
        close.disposition,
        CloseDisposition::Incompatible,
        "{close:?}"
    );
    assert_eq!(
        close.error.code, TRANSPORT_FRAME_LIMIT_EXCEEDED,
        "{close:?}"
    );
    assert!(
        remote_peers(id).is_empty(),
        "a rejected message installs nothing",
    );
    destroy_session(id);
}

#[test]
fn malformed_awareness_frames_close_the_generation_retryably() {
    let malformed: Vec<(&str, Vec<u8>)> = vec![
        // MSG_AWARENESS with an empty buffer payload.
        ("empty awareness payload", vec![1, 0]),
        // MSG_AWARENESS whose buffer truncates mid-payload.
        ("truncated awareness payload", vec![1, 5, 1, 2]),
        // Well-framed buffer that is not a decodable AwarenessUpdate.
        ("corrupt awareness update", vec![1, 3, 0xff, 0xff, 0xff]),
        // Well-formed update carrying a non-JSON state payload.
        (
            "non-json awareness state",
            awareness_message(&[(8_001, 1, "{not json")]),
        ),
    ];
    for (label, bytes) in malformed {
        let (id, snapshot) = create_ready_room();
        let generation = synchronize_ready_room(id, &snapshot);
        let before = session_audit(id).unwrap();

        let outcome = receive_message(id, 341, generation, &bytes).unwrap();
        let close = outcome
            .close
            .as_ref()
            .unwrap_or_else(|| panic!("{label}: must close the generation: {outcome:?}"));
        assert_eq!(
            close.disposition,
            CloseDisposition::Retryable,
            "{label}: {close:?}"
        );
        assert_eq!(
            close.error.code, TRANSPORT_PROTOCOL_INVALID,
            "{label}: {close:?}"
        );
        assert_eq!(close.error.request_id, Some(341), "{label}: {close:?}");
        assert!(remote_peers(id).is_empty(), "{label}: nothing may install");

        let mut expected = before.clone();
        expected.transport_state = TransportState::Disconnected;
        assert_eq!(session_audit(id).unwrap(), expected, "{label}");
        destroy_session(id);
    }
}

// Awareness ceilings: exact and one-over, atomic rejection

/// Shared shape of every deterministic awareness-ceiling refusal: the
/// generation closes as `Incompatible` with the awareness limit code, the
/// charged `CollaborationLimits` field rides in the structured cause, the
/// close clears the transport-scoped peers (design rule — so nothing of the
/// refused update can be observed either), and the engine/session audit is
/// untouched apart from the transport state.
fn assert_awareness_ceiling_close(
    id: u64,
    outcome: &ReceiveOutcome,
    field: &str,
    before: &SessionAudit,
) {
    let close = outcome
        .close
        .as_ref()
        .unwrap_or_else(|| panic!("{field}: one over must close: {outcome:?}"));
    assert_eq!(
        close.disposition,
        CloseDisposition::Incompatible,
        "{field}: {close:?}"
    );
    assert_eq!(
        close.error.code, TRANSPORT_AWARENESS_LIMIT_EXCEEDED,
        "{field}: {close:?}"
    );
    let details = close.error.details.as_ref().unwrap();
    assert_eq!(details["cause"]["details"]["field"], field, "{details:?}");
    assert!(
        remote_peers(id).is_empty(),
        "{field}: the failure close clears transport-scoped peers, so nothing \
         of the refused update survives",
    );
    let mut expected = before.clone();
    expected.transport_state = TransportState::Incompatible;
    assert_eq!(&session_audit(id).unwrap(), &expected, "{field}");
}

#[test]
fn awareness_peer_count_ceiling_admits_exact_and_rejects_one_over() {
    let (id, snapshot) = create_ready_room();
    let generation = synchronize_ready_room(id, &snapshot);
    set_collaboration_limit_for_test(id, "maxAwarenessPeers", 2).unwrap();

    // Exactly at the ceiling is admitted.
    receive_message(
        id,
        351,
        generation,
        &awareness_message(&[(6_101, 1, r#"{"i":1}"#), (6_102, 1, r#"{"i":2}"#)]),
    )
    .unwrap();
    assert_eq!(remote_peers(id).len(), 2);
    let before = session_audit(id).unwrap();

    // One peer over closes deterministically.
    let outcome = receive_message(
        id,
        352,
        generation,
        &awareness_message(&[(6_103, 1, r#"{"i":3}"#)]),
    )
    .unwrap();
    assert_awareness_ceiling_close(id, &outcome, "maxAwarenessPeers", &before);
    destroy_session(id);

    // A single update whose combined entries land one over is rejected as a
    // whole (atomic admission through the codec; the close then clears the
    // transport scope).
    let (id, snapshot) = create_ready_room();
    let generation = synchronize_ready_room(id, &snapshot);
    set_collaboration_limit_for_test(id, "maxAwarenessPeers", 2).unwrap();
    let before = session_audit(id).unwrap();
    let outcome = receive_message(
        id,
        353,
        generation,
        &awareness_message(&[
            (6_201, 1, r#"{"i":1}"#),
            (6_202, 1, r#"{"i":2}"#),
            (6_203, 1, r#"{"i":3}"#),
        ]),
    )
    .unwrap();
    assert_awareness_ceiling_close(id, &outcome, "maxAwarenessPeers", &before);
    destroy_session(id);
}

#[test]
fn awareness_peer_bytes_ceiling_admits_exact_and_rejects_one_over() {
    let exact_state = format!(r#"{{"pad":"{}"}}"#, "x".repeat(20));
    let over_state = format!(r#"{{"pad":"{}"}}"#, "x".repeat(21));

    let (id, snapshot) = create_ready_room();
    let generation = synchronize_ready_room(id, &snapshot);
    set_collaboration_limit_for_test(id, "maxAwarenessPeerBytes", exact_state.len()).unwrap();

    receive_message(
        id,
        361,
        generation,
        &awareness_message(&[(6_301, 1, &exact_state)]),
    )
    .unwrap();
    assert_eq!(
        remote_peers(id).len(),
        1,
        "exact per-peer bytes are admitted"
    );
    let before = session_audit(id).unwrap();

    let outcome = receive_message(
        id,
        362,
        generation,
        &awareness_message(&[(6_302, 1, &over_state)]),
    )
    .unwrap();
    assert_awareness_ceiling_close(id, &outcome, "maxAwarenessPeerBytes", &before);
    destroy_session(id);
}

#[test]
fn awareness_aggregate_bytes_ceiling_admits_exact_and_rejects_one_over() {
    let state_a = r#"{"i":1}"#;
    let state_b_exact = r#"{"i":22}"#;
    let state_b_over = r#"{"i":333}"#;
    let ceiling = state_a.len() + state_b_exact.len();

    let (id, snapshot) = create_ready_room();
    let generation = synchronize_ready_room(id, &snapshot);
    set_collaboration_limit_for_test(id, "maxAwarenessBytes", ceiling).unwrap();

    receive_message(
        id,
        371,
        generation,
        &awareness_message(&[(6_401, 1, state_a)]),
    )
    .unwrap();
    receive_message(
        id,
        372,
        generation,
        &awareness_message(&[(6_402, 1, state_b_exact)]),
    )
    .unwrap();
    assert_eq!(remote_peers(id).len(), 2, "exact aggregate is admitted");
    let before = session_audit(id).unwrap();

    // Growing peer B by one byte pushes the aggregate one over the ceiling.
    let outcome = receive_message(
        id,
        373,
        generation,
        &awareness_message(&[(6_402, 2, state_b_over)]),
    )
    .unwrap();
    assert_awareness_ceiling_close(id, &outcome, "maxAwarenessBytes", &before);
    destroy_session(id);

    // The aggregate ceiling also bounds the raw inbound payload before any
    // decode work.
    let (id, snapshot) = create_ready_room();
    let generation = synchronize_ready_room(id, &snapshot);
    set_collaboration_limit_for_test(id, "maxAwarenessBytes", 8).unwrap();
    let before = session_audit(id).unwrap();
    let outcome = receive_message(
        id,
        374,
        generation,
        &awareness_message(&[(6_403, 1, r#"{"pad":"xxxxxxxxxxxxxxxx"}"#)]),
    )
    .unwrap();
    assert_awareness_ceiling_close(id, &outcome, "maxAwarenessBytes", &before);
    destroy_session(id);
}

#[test]
fn query_awareness_replies_are_bounded_and_reserved_like_every_protocol_reply() {
    // Aggregate response ceiling: measured exact passes, one under it closes
    // deterministically.
    let (id, snapshot) = create_ready_room();
    let generation = synchronize_ready_room(id, &snapshot);
    set_desired_awareness(id, 381, &json!({ "name": "measured" }).to_string()).unwrap();
    drain_protocol_replies(id, generation);

    let outcome = receive_message(id, 382, generation, &query_awareness_message()).unwrap();
    assert!(outcome.close.is_none(), "{outcome:?}");
    let reply_bytes = outcome.reply_bytes_enqueued;
    assert!(reply_bytes > 0, "{outcome:?}");
    drain_protocol_replies(id, generation);

    set_collaboration_limit_for_test(id, "maxAggregateResponseBytes", reply_bytes).unwrap();
    let outcome = receive_message(id, 383, generation, &query_awareness_message()).unwrap();
    assert!(
        outcome.close.is_none(),
        "exact reply bytes pass: {outcome:?}"
    );
    assert_eq!(outcome.reply_bytes_enqueued, reply_bytes, "{outcome:?}");
    drain_protocol_replies(id, generation);

    set_collaboration_limit_for_test(id, "maxAggregateResponseBytes", reply_bytes - 1).unwrap();
    let outcome = receive_message(id, 384, generation, &query_awareness_message()).unwrap();
    let close = outcome.close.as_ref().expect("one byte over must close");
    assert_eq!(
        close.disposition,
        CloseDisposition::Incompatible,
        "{close:?}"
    );
    assert_eq!(
        close.error.code, TRANSPORT_REPLY_LIMIT_EXCEEDED,
        "{close:?}"
    );
    assert_eq!(
        pending_protocol_replies(id).unwrap(),
        Some((0, 0)),
        "a refused reply must not enqueue",
    );
    destroy_session(id);

    // Outbox saturation is retryable per the Task 9 saturation ruling: the
    // shared queue drains on delivery, so retry can change the result.
    let (id, snapshot) = create_ready_room();
    let generation = synchronize_ready_room(id, &snapshot);
    bridge::set_outbox_ceilings(id, 0, 0).unwrap();
    let outcome = receive_message(id, 385, generation, &query_awareness_message()).unwrap();
    let close = outcome.close.as_ref().expect("saturated outbox must close");
    assert_eq!(close.disposition, CloseDisposition::Retryable, "{close:?}");
    assert_eq!(
        close.error.code, TRANSPORT_REPLY_LIMIT_EXCEEDED,
        "{close:?}"
    );
    destroy_session(id);
}

include!("collaboration_awareness_test/publication.rs");

include!("collaboration_awareness_test/renewal_and_expiry.rs");

include!("collaboration_awareness_test/cursor_projection.rs");
