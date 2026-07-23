//! Task 10: bounded awareness and peer projections.
//!
//! Covers awareness/query-awareness handling through the Task 9
//! `receive_message` classification pipeline, runtime ownership of desired
//! local awareness across transport lifecycle (including the reconnect
//! re-publish that mitigates the Task 6 tombstone-migration gap),
//! deterministic renewal/expiry clocks driven exclusively through
//! `tick(now_millis)`, tombstone semantics through the runtime path, sticky
//! cursor projections that recompute with document revisions, and every
//! awareness ceiling at its exact and one-over boundary with atomic
//! rejection. Wire compatibility is proven against independent raw
//! `yrs::sync::Awareness` peers in both directions; encoded awareness bytes
//! are hash-map-ordered, so assertions always compare decoded structures.

use std::collections::HashMap;

use crate::boundary::ResourceLimits;
use crate::collaboration_runtime::awareness::{
    AWARENESS_EXPIRY_MILLIS, AWARENESS_RENEWAL_INTERVAL_MILLIS,
};
use crate::collaboration_runtime::protocol::{
    TRANSPORT_AWARENESS_LIMIT_EXCEEDED, TRANSPORT_FRAME_LIMIT_EXCEEDED, TRANSPORT_PROTOCOL_INVALID,
    TRANSPORT_REPLY_LIMIT_EXCEEDED,
};
use crate::collaboration_runtime::CollaborationRuntime;
use crate::ffi_v2::collaboration as v2_collaboration;
use crate::native_bridge_test_support as bridge;
use crate::session::{CollaborationLimits, TransportState as RuntimeTransportState};
use crate::session_initialization_test_support::{
    awareness_peers, awareness_tick, begin_connect, clear_desired_awareness, create_room_from_json,
    desired_awareness, destroy_session, document_state, receive_message, restore_snapshot,
    session_audit, set_collaboration_limit_for_test, set_desired_awareness, socket_closed,
    socket_opened, take_next_protocol_reply, transport_detach, transport_disconnect,
    transport_reattach, transport_state, AwarenessPeerInfo, CloseDisposition, DocumentState,
    ReceiveOutcome, SessionAudit, TransportState,
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

/// `begin_connect` + `socket_opened`: the transport is `Handshaking`.
fn handshake(id: u64) -> u64 {
    let generation = begin_connect(id, 9_000).unwrap();
    socket_opened(id, 9_001, generation).unwrap();
    generation
}

/// Drive a ready room to `Synchronized` through a real no-op Step 2 from a
/// raw peer sharing the session's snapshot lineage.
fn synchronize_ready_room(id: u64, snapshot: &DocumentSnapshot) -> u64 {
    let generation = handshake(id);
    let server = raw_doc_from_snapshot(snapshot);
    let step2 = server
        .transact()
        .encode_state_as_update_v1(&yrs::StateVector::default());
    let outcome = receive_message(
        id,
        9_002,
        generation,
        &Message::Sync(yrs::sync::SyncMessage::SyncStep2(step2)).encode_v1(),
    )
    .unwrap();
    assert!(outcome.close.is_none(), "{outcome:?}");
    assert_eq!(transport_state(id).unwrap(), TransportState::Synchronized);
    generation
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

fn drain_protocol_replies(id: u64) -> Vec<Vec<u8>> {
    let mut replies = Vec::new();
    while let Some((_, message)) = take_next_protocol_reply(id).unwrap() {
        replies.push(message);
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

// ---------------------------------------------------------------------------
// Protocol integration: awareness and query-awareness through receive_message
// ---------------------------------------------------------------------------

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
    drain_protocol_replies(id);

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
    let replies = drain_protocol_replies(id);
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

// ---------------------------------------------------------------------------
// Awareness ceilings: exact and one-over, atomic rejection
// ---------------------------------------------------------------------------

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
    drain_protocol_replies(id);

    let outcome = receive_message(id, 382, generation, &query_awareness_message()).unwrap();
    assert!(outcome.close.is_none(), "{outcome:?}");
    let reply_bytes = outcome.reply_bytes_enqueued;
    assert!(reply_bytes > 0, "{outcome:?}");
    drain_protocol_replies(id);

    set_collaboration_limit_for_test(id, "maxAggregateResponseBytes", reply_bytes).unwrap();
    let outcome = receive_message(id, 383, generation, &query_awareness_message()).unwrap();
    assert!(
        outcome.close.is_none(),
        "exact reply bytes pass: {outcome:?}"
    );
    assert_eq!(outcome.reply_bytes_enqueued, reply_bytes, "{outcome:?}");
    drain_protocol_replies(id);

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
        take_next_protocol_reply(id).unwrap(),
        None,
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

// ---------------------------------------------------------------------------
// Desired local awareness: ownership, validation, lifecycle
// ---------------------------------------------------------------------------

#[test]
fn set_desired_awareness_publishes_immediately_and_bounds_the_state_at_set_time() {
    let (id, snapshot) = create_ready_room();
    let _generation = synchronize_ready_room(id, &snapshot);

    let desired = json!({ "name": "author", "color": "#204060" });
    set_desired_awareness(id, 401, &desired.to_string()).unwrap();
    assert_eq!(desired_awareness(id).unwrap(), Some(desired.clone()));
    let local = local_peer(id).expect("published local state projects");
    assert_eq!(local.state, desired);

    // The synchronized transport broadcasts the state immediately; a raw
    // peer applying the frame sees exactly the desired state.
    let replies = drain_protocol_replies(id);
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
    let _generation = synchronize_ready_room(id, &snapshot);
    let desired = json!({ "name": "leaving" });
    set_desired_awareness(id, 411, &desired.to_string()).unwrap();
    let local = local_peer(id).unwrap();
    let replies = drain_protocol_replies(id);
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
    let replies = drain_protocol_replies(id);
    assert_eq!(replies.len(), 1, "{replies:?}");
    raw.apply_update(decode_awareness_reply(&replies[0]))
        .unwrap();
    assert_eq!(raw.state::<Value>(ClientID::new(local.client_id)), None);

    // Clearing twice stays an idempotent no-op with nothing to broadcast.
    clear_desired_awareness(id, 413).unwrap();
    assert_eq!(drain_protocol_replies(id).len(), 0);
    destroy_session(id);
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
    for reply in drain_protocol_replies(id) {
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
    socket_closed(id, 422, generation, CloseDisposition::Retryable).unwrap();
    assert_eq!(desired_awareness(id).unwrap(), Some(desired.clone()));

    // Reconnect + handshake completion re-publishes with a fresh clock; the
    // peer that tombstoned us sees us again with a strictly newer clock.
    let generation = synchronize_ready_room(id, &snapshot);
    let _ = generation;
    let replies = drain_protocol_replies(id);
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
            .set_desired_awareness(
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
    let mut generation = synchronize_ready_room(id, &snapshot);
    let desired = json!({ "name": "twice resilient" });
    set_desired_awareness(id, 471, &desired.to_string()).unwrap();
    let local_client = ClientID::new(local_peer(id).unwrap().client_id);

    // A raw peer observes the initial publish: the induction base clock.
    let mut raw = Awareness::new(Doc::new());
    for reply in drain_protocol_replies(id) {
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

        socket_closed(id, close_request, generation, CloseDisposition::Retryable).unwrap();
        assert_eq!(
            desired_awareness(id).unwrap(),
            Some(desired.clone()),
            "cycle {cycle}: desired local awareness survives the close",
        );

        generation = synchronize_ready_room(id, &snapshot);
        let replies = drain_protocol_replies(id);
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
            socket_closed(id, 431, generation, CloseDisposition::Retryable).unwrap();
        }),
        ("incompatible close", |id, generation| {
            socket_closed(id, 432, generation, CloseDisposition::Incompatible).unwrap();
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

// ---------------------------------------------------------------------------
// Tombstone semantics through the runtime path
// ---------------------------------------------------------------------------

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
        .set_desired_awareness(
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

// ---------------------------------------------------------------------------
// Deterministic renewal and expiry clocks
// ---------------------------------------------------------------------------

#[test]
fn tick_renews_local_awareness_at_exactly_the_renewal_interval() {
    let (id, snapshot) = create_ready_room();
    let _generation = synchronize_ready_room(id, &snapshot);
    awareness_tick(id, 501, 0).unwrap();
    set_desired_awareness(id, 502, &json!({ "name": "renewer" }).to_string()).unwrap();
    let published_clock = local_peer(id).unwrap().clock;
    drain_protocol_replies(id);

    // One millisecond before the interval: no renewal, deadline reported.
    let outcome = awareness_tick(id, 503, AWARENESS_RENEWAL_INTERVAL_MILLIS - 1).unwrap();
    assert!(!outcome.renewed_local, "{outcome:?}");
    assert_eq!(
        outcome.next_deadline_millis,
        Some(AWARENESS_RENEWAL_INTERVAL_MILLIS),
        "{outcome:?}"
    );
    assert_eq!(drain_protocol_replies(id).len(), 0);
    assert_eq!(local_peer(id).unwrap().clock, published_clock);

    // Exactly at the interval: the local state renews with a fresh clock and
    // the renewed frame is enqueued for broadcast.
    let outcome = awareness_tick(id, 504, AWARENESS_RENEWAL_INTERVAL_MILLIS).unwrap();
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
    let replies = drain_protocol_replies(id);
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
    awareness_tick(id, 541, 0).unwrap();

    // Baseline: one successful publish; its clock is the last successful one.
    let desired = json!({ "name": "pre-fill" });
    set_desired_awareness(id, 542, &desired.to_string()).unwrap();
    let local = local_peer(id).unwrap();
    let local_client = ClientID::new(local.client_id);
    let published_clock = local.clock;
    let mut raw = Awareness::new(Doc::new());
    for reply in drain_protocol_replies(id) {
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
    awareness_tick(id, 545, 5_000).unwrap();
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
    assert_eq!(take_next_protocol_reply(id).unwrap(), None);
    assert_eq!(
        bridge::outbox_pending(id).unwrap().unwrap().0,
        1,
        "only the fill edit may pend",
    );

    // (c): the publish clock did not advance — the renewal deadline stays
    // anchored at the last successful broadcast (t=0), not at the refusal
    // (t=5_000), and the boundary tick still finds renewal due: it attempts
    // the broadcast and is refused the same retryable way.
    let outcome = awareness_tick(id, 547, AWARENESS_RENEWAL_INTERVAL_MILLIS - 1).unwrap();
    assert!(!outcome.renewed_local, "{outcome:?}");
    assert_eq!(
        outcome.next_deadline_millis,
        Some(AWARENESS_RENEWAL_INTERVAL_MILLIS),
        "a refused publish must not push the renewal deadline out: {outcome:?}",
    );
    let error = awareness_tick(id, 548, AWARENESS_RENEWAL_INTERVAL_MILLIS).unwrap_err();
    assert_eq!(error.code, TRANSPORT_REPLY_LIMIT_EXCEEDED, "{error:?}");
    assert_eq!(transport_state(id).unwrap(), TransportState::Synchronized);
    assert_eq!(take_next_protocol_reply(id).unwrap(), None);

    // (d): drain through the normal pickup seam, then a retried tick heals
    // the broadcast end-to-end with a fresh clock past the last successful
    // publish — the raw peer sees the retained state again.
    let (request_id, _) = bridge::take_next_update(id).unwrap().unwrap();
    assert_eq!(request_id, 544);
    assert_eq!(bridge::outbox_pending(id).unwrap().unwrap(), (0, 0));
    let outcome = awareness_tick(id, 549, AWARENESS_RENEWAL_INTERVAL_MILLIS).unwrap();
    assert!(outcome.renewed_local, "{outcome:?}");
    assert_eq!(
        outcome.next_deadline_millis,
        Some(AWARENESS_RENEWAL_INTERVAL_MILLIS * 2),
        "{outcome:?}",
    );
    let replies = drain_protocol_replies(id);
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
    let replies = drain_protocol_replies(id);
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
fn tick_never_renews_without_a_synchronized_transport_or_desired_state() {
    // No desired state: nothing renews, no deadline is requested.
    let (id, snapshot) = create_ready_room();
    let _generation = synchronize_ready_room(id, &snapshot);
    awareness_tick(id, 511, 0).unwrap();
    let outcome = awareness_tick(id, 512, AWARENESS_RENEWAL_INTERVAL_MILLIS).unwrap();
    assert!(!outcome.renewed_local, "{outcome:?}");
    assert_eq!(outcome.next_deadline_millis, None, "{outcome:?}");
    destroy_session(id);

    // Desired state but a closed transport: the state is retained, but
    // nothing broadcasts while disconnected.
    let (id, snapshot) = create_ready_room();
    let generation = synchronize_ready_room(id, &snapshot);
    set_desired_awareness(id, 513, &json!({ "name": "offline" }).to_string()).unwrap();
    socket_closed(id, 514, generation, CloseDisposition::Retryable).unwrap();
    drain_protocol_replies(id);
    let outcome = awareness_tick(id, 515, AWARENESS_RENEWAL_INTERVAL_MILLIS * 3).unwrap();
    assert!(!outcome.renewed_local, "{outcome:?}");
    assert_eq!(outcome.next_deadline_millis, None, "{outcome:?}");
    assert_eq!(drain_protocol_replies(id).len(), 0);
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
    awareness_tick(id, 521, 0).unwrap();
    receive_message(
        id,
        522,
        generation,
        &awareness_message(&[(6_801, 1, r#"{"name":"mortal"}"#)]),
    )
    .unwrap();

    // One millisecond before the deadline: the peer survives.
    let outcome = awareness_tick(id, 523, AWARENESS_EXPIRY_MILLIS - 1).unwrap();
    assert!(outcome.expired_peers.is_empty(), "{outcome:?}");
    assert_eq!(
        outcome.next_deadline_millis,
        Some(AWARENESS_EXPIRY_MILLIS),
        "{outcome:?}"
    );
    assert_eq!(remote_peers(id).len(), 1);

    // Exactly at the deadline: the peer expires, leaves peers(), and leaves
    // the complete query answer.
    let outcome = awareness_tick(id, 524, AWARENESS_EXPIRY_MILLIS).unwrap();
    assert_eq!(outcome.expired_peers, vec![6_801], "{outcome:?}");
    assert!(remote_peers(id).is_empty());
    receive_message(id, 525, generation, &query_awareness_message()).unwrap();
    let replies = drain_protocol_replies(id);
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
    awareness_tick(id, 531, 0).unwrap();
    receive_message(
        id,
        532,
        generation,
        &awareness_message(&[(6_901, 1, r#"{"name":"heartbeat"}"#)]),
    )
    .unwrap();

    // A renewed announcement (fresh clock) at t=20s pushes the deadline out.
    awareness_tick(id, 533, 20_000).unwrap();
    receive_message(
        id,
        534,
        generation,
        &awareness_message(&[(6_901, 2, r#"{"name":"heartbeat"}"#)]),
    )
    .unwrap();
    let outcome = awareness_tick(id, 535, AWARENESS_EXPIRY_MILLIS).unwrap();
    assert!(
        outcome.expired_peers.is_empty(),
        "activity at 20s keeps the peer alive at 30s: {outcome:?}",
    );
    assert_eq!(
        outcome.next_deadline_millis,
        Some(20_000 + AWARENESS_EXPIRY_MILLIS),
        "{outcome:?}"
    );
    let outcome = awareness_tick(id, 536, 20_000 + AWARENESS_EXPIRY_MILLIS).unwrap();
    assert_eq!(outcome.expired_peers, vec![6_901], "{outcome:?}");
    destroy_session(id);
}

// ---------------------------------------------------------------------------
// Cursor projections
// ---------------------------------------------------------------------------

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
    assert_eq!(local.state["state"], intent["state"]);
    assert_eq!(local.state["focused"], true);
    assert!(local.state["cursor"].is_object(), "{local:?}");
    assert_eq!(local.cursor, Some((7, 7)), "{local:?}");
    drain_protocol_replies(id);

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
        assert!(
            take_next_protocol_reply(id).unwrap().is_none(),
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
    let frame = v2_collaboration::editor_v2_collaboration_take_outbound(
        id.to_string(),
        generation.to_string(),
    );
    assert!(frame.error.is_none(), "{frame:?}");
    assert!(!frame.value.unwrap_or_default().is_empty());

    socket_closed(id, 614, generation, CloseDisposition::Retryable).unwrap();
    restore_snapshot(id, 615, &snapshot).unwrap();
    assert_eq!(local_peer(id).unwrap().cursor, Some((7, 7)));

    // A same-scope snapshot minted by a different Yrs client cannot resolve
    // the old sticky targets. The peer remains, but its cursor is omitted.
    let foreign_snapshot = snapshot_source();
    restore_snapshot(id, 616, &foreign_snapshot).unwrap();
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

// ---------------------------------------------------------------------------
// Task 18A: security-review findings C1/I1/I2 through the receive pipeline.
// ---------------------------------------------------------------------------

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
    let replies = drain_protocol_replies(id);
    assert_eq!(replies.len(), 1, "{replies:?}");
    let reply = decode_awareness_reply(&replies[0]);
    assert!(
        reply.clients.is_empty(),
        "no storm client is ever replayed: {reply:?}",
    );

    // The storm stamped no activity deadlines: a tick far in the future
    // expires nothing.
    let outcome = awareness_tick(id, 433, 10_000_000).unwrap();
    assert!(outcome.expired_peers.is_empty(), "{outcome:?}");
    destroy_session(id);
}
