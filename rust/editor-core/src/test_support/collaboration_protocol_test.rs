//! Strict standard y-sync frame handling with server Step 2 initialization.
//!
//! Covers the frozen receive flow (bounded classification/decode, reply
//! prebuild + reservation before any engine commit, infallible reply
//! installation), the Step 2 synchronization gate (including the only
//! `AwaitRemote -> RoomReady` promotion path), failure classification law,
//! every new receive ceiling at its exact and one-over boundary, bounded
//! dependency quarantine accounting, no-echo through `receive_message`, and
//! wire interoperability with independent raw Yrs peers in both directions.

use crate::boundary::ResourceLimits;
use crate::collaboration_runtime::protocol::{
    TRANSPORT_DEPENDENCY_LIMIT_EXCEEDED, TRANSPORT_FRAME_LIMIT_EXCEEDED,
    TRANSPORT_PROTOCOL_INVALID, TRANSPORT_REMOTE_INADMISSIBLE, TRANSPORT_REPLY_LIMIT_EXCEEDED,
    TRANSPORT_RESOURCE_EXHAUSTED,
};
use crate::collaboration_runtime::state::{
    TRANSPORT_INVALID_TRANSITION, TRANSPORT_STALE_GENERATION,
};
use crate::native_bridge_test_support as bridge;
use crate::schema::Schema;
use crate::session_initialization_test_support::{
    ack_outbound, collaboration_drive, collaboration_socket_open, create_room_from_json,
    destroy_session, document_state, lease_outbound, receive_message, remote_dependency_accounting,
    render_state, session_audit, set_collaboration_limit_for_test, transport_state,
    CloseDisposition, DocumentState, RenderState, TransportState,
};
use crate::tiptap_schema;
use crate::yrs_engine::{
    DocumentScope, DocumentSnapshot, EditingLimits, InitializationMode, TransactionOrigin,
    TypedCommand, YrsDocumentEngine, YrsEngineConfig,
};
use yrs::sync::{Message, SyncMessage};
use yrs::updates::decoder::Decode;
use yrs::updates::encoder::Encode;
use yrs::{
    diff_updates_v1, encode_state_vector_from_update_v1, Doc, GetString, ReadTxn, StateVector,
    Text, Transact, Update, XmlFragment, XmlOut,
};

const DOCUMENT_ID: &str = "protocol-room";
const LINEAGE_ID: &str = "protocol-lineage";
const JSON_SEED: &str = r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"protocol seed"}]}]}"#;
const FRAGMENT_NAME: &str = "prosemirror";
/// Generation value no session ever issues.
const FABRICATED_GENERATION: u64 = 424_242;
/// Standard empty Update-v1 (zero client blocks, empty delete set): the
/// canonical valid no-op update.
const NOOP_UPDATE_V1: [u8; 2] = [0, 0];

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
/// lineage, so `RawPeer::from_snapshot` peers start state-vector identical.
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

/// Drive and open a current generation, then ACK the queued Sync Step 1 so
/// tests that do not inspect it begin with a drained protocol outbox.
fn handshake(id: u64) -> u64 {
    let initial = collaboration_drive(id, 9_000, 0).unwrap();
    let (generation, now_millis) = match initial.generation_to_open {
        Some(generation) => (generation, 0),
        None => {
            let deadline = initial
                .next_deadline_millis
                .expect("a disconnected retry must expose its Rust deadline");
            let due = collaboration_drive(id, 9_000, deadline).unwrap();
            (
                due.generation_to_open
                    .expect("the due Rust drive must issue a generation"),
                deadline,
            )
        }
    };
    collaboration_socket_open(id, 9_001, generation, now_millis).unwrap();
    let step1 = lease_outbound(id, 9_002, generation)
        .unwrap()
        .expect("socket open must queue Sync Step 1");
    assert!(matches!(
        Message::decode_v1(&step1.frame).unwrap(),
        Message::Sync(SyncMessage::SyncStep1(_))
    ));
    ack_outbound(id, 9_002, generation, step1.lease_id).unwrap();
    generation
}

/// Drive a ready room to `Synchronized` through a real no-op Step 2 frame
/// from a peer sharing the session's snapshot lineage.
fn synchronize_ready_room(id: u64, snapshot: &DocumentSnapshot) -> u64 {
    let generation = handshake(id);
    let our_sv = session_state_vector_bytes(id);
    let server = RawPeer::from_snapshot(snapshot);
    let outcome = receive_message(
        id,
        9_002,
        generation,
        &step2_frame(server.diff_for(&our_sv)),
    )
    .unwrap();
    assert!(outcome.close.is_none(), "{outcome:?}");
    assert_eq!(transport_state(id).unwrap(), TransportState::Synchronized);
    generation
}

fn session_state_vector_bytes(id: u64) -> Vec<u8> {
    let encoded = session_audit(id).unwrap().encoded_state.unwrap_or_default();
    if encoded.is_empty() {
        StateVector::default().encode_v1()
    } else {
        encode_state_vector_from_update_v1(&encoded).unwrap()
    }
}

/// Structural state vector for convergence assertions: encoded state-vector
/// bytes are hash-map-ordered and therefore nondeterministic across
/// independent docs — the design requires semantic equality, not re-encoded
/// byte identity.
fn session_state_vector(id: u64) -> StateVector {
    StateVector::decode_v1(&session_state_vector_bytes(id)).unwrap()
}

/// An independent raw Yrs peer speaking standard y-protocols framing.
struct RawPeer {
    doc: Doc,
}

impl RawPeer {
    fn empty() -> Self {
        Self { doc: Doc::new() }
    }

    fn from_snapshot(snapshot: &DocumentSnapshot) -> Self {
        let peer = Self::empty();
        peer.apply(&snapshot.encoded_state);
        peer
    }

    fn apply(&self, update: &[u8]) {
        self.doc
            .transact_mut()
            .apply_update(Update::decode_v1(update).unwrap())
            .unwrap();
    }

    fn state_vector(&self) -> StateVector {
        self.doc.transact().state_vector()
    }

    fn state_vector_bytes(&self) -> Vec<u8> {
        self.doc.transact().state_vector().encode_v1()
    }

    fn diff_for(&self, remote_state_vector: &[u8]) -> Vec<u8> {
        self.doc
            .transact()
            .encode_state_as_update_v1(&StateVector::decode_v1(remote_state_vector).unwrap())
    }

    fn fragment_string(&self) -> String {
        let txn = self.doc.transact();
        txn.get_xml_fragment(FRAGMENT_NAME)
            .expect("peer must hold the configured fragment")
            .get_string(&txn)
    }

    /// A genuine raw-yrs local edit: push text into the seed paragraph.
    fn push_text(&self, text: &str) {
        let mut txn = self.doc.transact_mut();
        let fragment = txn
            .get_xml_fragment(FRAGMENT_NAME)
            .expect("peer must hold the configured fragment");
        let Some(XmlOut::Element(paragraph)) = fragment.get(&txn, 0) else {
            panic!("seed content must start with a paragraph element");
        };
        let Some(XmlOut::Text(content)) = paragraph.get(&txn, 0) else {
            panic!("seed paragraph must start with a text node");
        };
        content.push(&mut txn, text);
    }
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

fn concat_frames(frames: &[Vec<u8>]) -> Vec<u8> {
    frames.iter().flatten().copied().collect()
}

/// Decode one framed protocol reply the way a standard peer would.
fn decode_step2_reply(reply: &[u8]) -> Vec<u8> {
    let message = Message::decode_v1(reply).expect("reply must decode as a y-sync message");
    match message {
        Message::Sync(SyncMessage::SyncStep2(update)) => update,
        other => panic!("Step 1 reply must be a sync Step 2 message, got {other:?}"),
    }
}

fn incompatible_blockquote_schema() -> Schema {
    Schema::from_json(&serde_json::json!({
        "nodes": [
            {"name":"doc","content":"block+","role":"doc"},
            {"name":"paragraph","content":"inline*","group":"block","role":"textBlock","htmlTag":"p"},
            {"name":"blockquote","content":"inline*","group":"block","role":"block","htmlTag":"blockquote"},
            {"name":"text","content":"","group":"inline","role":"text"}
        ],
        "marks": []
    }))
    .unwrap()
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

#[test]
fn socket_open_queues_step1_as_the_first_leased_protocol_frame() {
    let (id, _snapshot) = create_ready_room();
    local_edit(id, 19_001, " queued before open");
    assert_eq!(bridge::outbox_pending(id).unwrap().unwrap().0, 1);

    let generation = collaboration_drive(id, 19_002, 0)
        .unwrap()
        .generation_to_open
        .expect("the initial drive must issue the generation");
    let opened = collaboration_socket_open(id, 19_003, generation, 0).unwrap();
    assert_eq!(opened.transport_state, TransportState::Handshaking);
    assert_eq!(opened.generation_to_open, None);

    let step1 = lease_outbound(id, 19_004, generation)
        .unwrap()
        .expect("socket open queues Sync Step 1 into the normal lease path");
    assert!(matches!(
        Message::decode_v1(&step1.frame).unwrap(),
        Message::Sync(SyncMessage::SyncStep1(_))
    ));
    ack_outbound(id, 19_005, generation, step1.lease_id).unwrap();

    let document = lease_outbound(id, 19_006, generation)
        .unwrap()
        .expect("the pre-existing document update remains after the Step 1 ACK");
    assert!(matches!(
        Message::decode_v1(&document.frame).unwrap(),
        Message::Sync(SyncMessage::Update(_))
    ));
    ack_outbound(id, 19_007, generation, document.lease_id).unwrap();
    destroy_session(id);
}

#[test]
fn step1_replies_are_read_only_completely_built_and_wire_compatible() {
    let (id, snapshot) = create_ready_room();
    let generation = handshake(id);
    let before = session_audit(id).unwrap();
    let before_engine = bridge::session_audit(id).unwrap();

    let outcome = receive_message(
        id,
        201,
        generation,
        &step1_frame(&StateVector::default().encode_v1()),
    )
    .unwrap();
    assert!(outcome.close.is_none(), "{outcome:?}");
    assert_eq!(outcome.frames_decoded, 1, "{outcome:?}");
    assert_eq!(outcome.replies_enqueued, 1, "{outcome:?}");
    assert!(outcome.reply_bytes_enqueued > 0, "{outcome:?}");
    assert!(!outcome.remote_commit_applied, "{outcome:?}");
    assert!(!outcome.document_promoted, "{outcome:?}");
    // Step 1 handling is read-only: no revision/epoch/history/document change.
    assert_eq!(session_audit(id).unwrap(), before);
    assert_eq!(bridge::session_audit(id).unwrap(), before_engine);
    // The Step 1 frame did not synchronize anything on its own.
    assert_eq!(transport_state(id).unwrap(), TransportState::Handshaking);

    // The reply is a complete standard Step 2 an independent raw peer can
    // apply directly, converging to our exact state.
    let reply = lease_outbound(id, 202, generation)
        .unwrap()
        .expect("Step 1 must retain its complete Step 2 reply");
    assert_eq!(reply.frame.len(), outcome.reply_bytes_enqueued);
    let peer = RawPeer::empty();
    peer.apply(&decode_step2_reply(&reply.frame));
    assert_eq!(peer.state_vector(), session_state_vector(id));
    assert_eq!(
        peer.fragment_string(),
        RawPeer::from_snapshot(&snapshot).fragment_string(),
    );
    ack_outbound(id, 203, generation, reply.lease_id).unwrap();
    assert!(lease_outbound(id, 204, generation).unwrap().is_none());

    destroy_session(id);
}

#[test]
fn socket_open_refuses_sessions_without_an_attached_runtime() {
    let snapshot = snapshot_source();
    let config = serde_json::json!({
        "documentId": snapshot.document_id,
        "lineageId": snapshot.lineage_id,
        "snapshot": snapshot,
    });
    let id = create_room_from_json(&config.to_string()).unwrap();
    let generation = collaboration_drive(id, 210, 0)
        .unwrap()
        .generation_to_open
        .expect("the drive may issue before socket-open runtime admission");

    let error = collaboration_socket_open(id, 211, generation, 0).unwrap_err();
    assert_eq!(error.domain, "boundary", "{error:?}");
    assert_eq!(error.code, "CONFIG_INVALID", "{error:?}");
    assert_eq!(transport_state(id).unwrap(), TransportState::Connecting);

    destroy_session(id);
}

#[test]
fn stale_generations_and_wrong_states_refuse_before_any_decode() {
    let (id, _snapshot) = create_ready_room();
    let _live = handshake(id);
    let before = session_audit(id).unwrap();

    // Malformed bytes prove no decode work happens behind the generation
    // gate: the refusal is stale-generation, never a protocol close.
    let malformed = vec![0xff, 0xff, 0xff];
    let error = receive_message(id, 221, FABRICATED_GENERATION, &malformed).unwrap_err();
    assert_eq!(error.domain, "transport", "{error:?}");
    assert_eq!(error.code, TRANSPORT_STALE_GENERATION, "{error:?}");
    assert_eq!(error.request_id, Some(221), "{error:?}");
    assert_eq!(session_audit(id).unwrap(), before);
    assert_eq!(transport_state(id).unwrap(), TransportState::Handshaking);
    destroy_session(id);

    // Connecting is not a frame-accepting state, even for the live
    // generation.
    let (id, _snapshot) = create_ready_room();
    let generation = collaboration_drive(id, 222, 0)
        .unwrap()
        .generation_to_open
        .expect("the initial drive must issue a generation");
    let error = receive_message(id, 223, generation, &NOOP_UPDATE_V1).unwrap_err();
    assert_eq!(error.code, TRANSPORT_INVALID_TRANSITION, "{error:?}");
    assert_eq!(transport_state(id).unwrap(), TransportState::Connecting);
    destroy_session(id);

    // Closed generations are stale for every later frame.
    let (id, _snapshot) = create_ready_room();
    let generation = handshake(id);
    let outcome = receive_message(id, 224, generation, &[0xff]).unwrap();
    let close = outcome.close.as_ref().expect("malformed frame must close");
    assert_eq!(close.disposition, CloseDisposition::Retryable, "{close:?}");
    let error =
        receive_message(id, 225, generation, &update_frame(NOOP_UPDATE_V1.to_vec())).unwrap_err();
    assert_eq!(error.code, TRANSPORT_STALE_GENERATION, "{error:?}");
    destroy_session(id);
    let _ = generation;
}

#[test]
fn malformed_frames_close_the_generation_with_a_protocol_error() {
    // Strict standard y-protocols framing: truncated frames, unknown
    // message/sync tags, non-protocol messages, empty messages, trailing
    // garbage, and malformed update payloads all classify as protocol
    // errors and close the generation retryably. Task 10 moved awareness
    // (tag 1) and query-awareness (tag 3) to the accepted set — their
    // positive coverage lives in `collaboration_awareness_test.rs` — while
    // auth and custom tags stay rejected.
    let malformed_messages: Vec<(&str, Vec<u8>)> = vec![
        ("empty message", vec![]),
        ("truncated message tag", vec![0x80]),
        ("truncated sync subtag", vec![0]),
        ("truncated step2 payload", vec![0, 1, 5, 1, 2]),
        ("unknown sync subtag", vec![0, 9, 0]),
        ("truncated awareness payload", vec![1, 5, 1, 2]),
        ("auth message tag", vec![2, 1]),
        ("custom message tag", vec![42, 1, 7]),
        ("trailing garbage after a valid frame", {
            let mut bytes = update_frame(NOOP_UPDATE_V1.to_vec());
            bytes.push(0xff);
            bytes
        }),
        (
            "malformed update payload inside valid framing",
            update_frame(vec![0xff, 0xff, 0xff]),
        ),
    ];

    for (label, bytes) in malformed_messages {
        let (id, _snapshot) = create_ready_room();
        let generation = handshake(id);
        let before = session_audit(id).unwrap();
        let before_engine = bridge::session_audit(id).unwrap();

        let outcome = receive_message(id, 231, generation, &bytes).unwrap();
        let close = outcome
            .close
            .as_ref()
            .unwrap_or_else(|| panic!("{label}: must close the generation: {outcome:?}"));
        assert_eq!(
            close.disposition,
            CloseDisposition::Retryable,
            "{label}: {close:?}"
        );
        assert_eq!(close.error.domain, "transport", "{label}: {close:?}");
        assert_eq!(
            close.error.code, TRANSPORT_PROTOCOL_INVALID,
            "{label}: {close:?}"
        );
        assert_eq!(close.error.request_id, Some(231), "{label}: {close:?}");
        assert_eq!(outcome.replies_enqueued, 0, "{label}: {outcome:?}");
        assert!(!outcome.remote_commit_applied, "{label}: {outcome:?}");
        assert_eq!(
            outcome.transport_state,
            TransportState::Disconnected,
            "{label}: a protocol close is retryable"
        );
        assert_eq!(transport_state(id).unwrap(), TransportState::Disconnected);

        let mut expected = before.clone();
        expected.transport_state = TransportState::Disconnected;
        assert_eq!(session_audit(id).unwrap(), expected, "{label}");
        assert_eq!(bridge::session_audit(id).unwrap(), before_engine, "{label}");

        // Retry stays eligible after a protocol close.
        let retry = collaboration_drive(id, 232, 500).unwrap();
        assert!(
            retry.generation_to_open.is_some(),
            "a due Rust drive must issue the retry generation"
        );
        destroy_session(id);
    }
}

#[test]
fn await_remote_valid_step2_initializes_and_synchronizes() {
    let id = create_await_remote_room();
    assert_eq!(document_state(id).unwrap(), DocumentState::AwaitRemote);
    assert_eq!(render_state(id).unwrap(), RenderState::Loading);
    let generation = collaboration_drive(id, 240, 0)
        .unwrap()
        .generation_to_open
        .expect("the initial directive must issue a generation");
    collaboration_socket_open(id, 240, generation, 0).unwrap();

    // Our Step 1 is queued at protocol priority and leased as a standard
    // frame an independent peer consumes directly.
    let step1_lease = lease_outbound(id, 241, generation)
        .unwrap()
        .expect("socket open must retain Sync Step 1");
    let our_step1 = step1_lease.frame.clone();
    ack_outbound(id, 241, generation, step1_lease.lease_id).unwrap();
    let server = RawPeer::from_snapshot(&snapshot_source());
    let our_sv = match Message::decode_v1(&our_step1).unwrap() {
        Message::Sync(SyncMessage::SyncStep1(sv)) => sv.encode_v1(),
        other => panic!("our handshake frame must be sync Step 1, got {other:?}"),
    };

    // The standard server handshake message: Step 2 followed by Step 1.
    let message = concat_frames(&[
        step2_frame(server.diff_for(&our_sv)),
        step1_frame(&server.state_vector_bytes()),
    ]);
    let outcome = receive_message(id, 242, generation, &message).unwrap();
    assert!(outcome.close.is_none(), "{outcome:?}");
    assert_eq!(outcome.frames_decoded, 2, "{outcome:?}");
    assert!(outcome.remote_commit_applied, "{outcome:?}");
    assert!(outcome.document_promoted, "{outcome:?}");
    assert_eq!(outcome.replies_enqueued, 1, "{outcome:?}");
    assert_eq!(outcome.transport_state, TransportState::Synchronized);

    // The ONLY AwaitRemote -> RoomReady promotion path.
    assert_eq!(document_state(id).unwrap(), DocumentState::RoomReady);
    assert_eq!(render_state(id).unwrap(), RenderState::Ready);
    assert_eq!(transport_state(id).unwrap(), TransportState::Synchronized);
    let audit = session_audit(id).unwrap();
    assert_eq!(
        audit.document_json.unwrap(),
        serde_json::from_str::<serde_json::Value>(JSON_SEED).unwrap(),
    );

    // Our reply Step 2 converges the server side (a no-op for it here).
    let reply = lease_outbound(id, 243, generation)
        .unwrap()
        .expect("the peer Step 1 must retain our Step 2 reply");
    server.apply(&decode_step2_reply(&reply.frame));
    ack_outbound(id, 244, generation, reply.lease_id).unwrap();
    assert_eq!(server.state_vector(), session_state_vector(id));

    destroy_session(id);
}

#[test]
fn await_remote_noop_step2_closes_incompatible() {
    let id = create_await_remote_room();
    let generation = handshake(id);
    let empty_server = RawPeer::empty();
    let our_sv = session_state_vector_bytes(id);

    let outcome = receive_message(
        id,
        251,
        generation,
        &step2_frame(empty_server.diff_for(&our_sv)),
    )
    .unwrap();
    let close = outcome
        .close
        .as_ref()
        .expect("no-op Step 2 cannot initialize AwaitRemote");
    assert_eq!(
        close.disposition,
        CloseDisposition::Incompatible,
        "{close:?}"
    );
    assert_eq!(close.error.code, TRANSPORT_REMOTE_INADMISSIBLE, "{close:?}");
    assert!(!outcome.document_promoted, "{outcome:?}");
    assert_eq!(document_state(id).unwrap(), DocumentState::AwaitRemote);
    assert_eq!(render_state(id).unwrap(), RenderState::Loading);
    assert_eq!(transport_state(id).unwrap(), TransportState::Incompatible);

    destroy_session(id);
}

#[test]
fn await_remote_step2_without_configured_fragment_closes_incompatible() {
    let id = create_await_remote_room();
    let generation = handshake(id);

    // A well-formed remote document that never defines our fragment root.
    let foreign = Doc::new();
    {
        let fragment = foreign.get_or_insert_xml_fragment("other-root");
        let mut txn = foreign.transact_mut();
        fragment.insert(&mut txn, 0, yrs::XmlTextPrelim::new("foreign content"));
    }
    let step2 = foreign
        .transact()
        .encode_state_as_update_v1(&StateVector::default());

    let outcome = receive_message(id, 261, generation, &step2_frame(step2)).unwrap();
    let close = outcome
        .close
        .expect("a Step 2 without the configured fragment cannot initialize");
    assert_eq!(
        close.disposition,
        CloseDisposition::Incompatible,
        "{close:?}"
    );
    assert_eq!(close.error.code, TRANSPORT_REMOTE_INADMISSIBLE, "{close:?}");
    // The structured cause is the engine's own admission error.
    let details = close
        .error
        .details
        .as_ref()
        .expect("close must carry details");
    assert_eq!(details["cause"]["code"], "DOCUMENT_INVALID", "{details}");
    assert_eq!(document_state(id).unwrap(), DocumentState::AwaitRemote);
    assert_eq!(transport_state(id).unwrap(), TransportState::Incompatible);

    destroy_session(id);
}

#[test]
fn room_ready_step2_synchronizes_including_noop() {
    // A genuine no-op Step 2 synchronizes a RoomReady handshake.
    let (id, snapshot) = create_ready_room();
    let generation = handshake(id);
    let server = RawPeer::from_snapshot(&snapshot);
    let outcome = receive_message(
        id,
        271,
        generation,
        &step2_frame(server.diff_for(&session_state_vector_bytes(id))),
    )
    .unwrap();
    assert!(outcome.close.is_none(), "{outcome:?}");
    assert!(!outcome.remote_commit_applied, "{outcome:?}");
    assert!(!outcome.document_promoted, "{outcome:?}");
    assert_eq!(transport_state(id).unwrap(), TransportState::Synchronized);
    assert_eq!(document_state(id).unwrap(), DocumentState::RoomReady);
    destroy_session(id);

    // A content-bearing Step 2 synchronizes and applies.
    let (id, snapshot) = create_ready_room();
    let generation = handshake(id);
    let server = RawPeer::from_snapshot(&snapshot);
    server.push_text(" plus server edit");
    let outcome = receive_message(
        id,
        272,
        generation,
        &step2_frame(server.diff_for(&session_state_vector_bytes(id))),
    )
    .unwrap();
    assert!(outcome.close.is_none(), "{outcome:?}");
    assert!(outcome.remote_commit_applied, "{outcome:?}");
    assert_eq!(transport_state(id).unwrap(), TransportState::Synchronized);
    let json = session_audit(id)
        .unwrap()
        .document_json
        .unwrap()
        .to_string();
    assert!(json.contains("plus server edit"), "{json}");
    destroy_session(id);
}

#[test]
fn update_frames_never_synchronize() {
    // RoomReady + Handshaking: an Update applies but cannot synchronize.
    let (id, snapshot) = create_ready_room();
    let generation = handshake(id);
    let server = RawPeer::from_snapshot(&snapshot);
    server.push_text(" via update");
    let update = server.diff_for(&session_state_vector_bytes(id));
    let outcome = receive_message(id, 281, generation, &update_frame(update)).unwrap();
    assert!(outcome.close.is_none(), "{outcome:?}");
    assert!(outcome.remote_commit_applied, "{outcome:?}");
    assert_eq!(
        transport_state(id).unwrap(),
        TransportState::Handshaking,
        "update frames must never enter Synchronized",
    );
    destroy_session(id);

    // AwaitRemote + Handshaking: an Update is admitted per quarantine and
    // admission rules but can neither synchronize nor promote the document.
    let id = create_await_remote_room();
    let generation = handshake(id);
    let server = RawPeer::from_snapshot(&snapshot_source());
    let full = server.diff_for(&session_state_vector_bytes(id));
    let outcome = receive_message(id, 282, generation, &update_frame(full)).unwrap();
    assert!(outcome.close.is_none(), "{outcome:?}");
    assert!(outcome.remote_commit_applied, "{outcome:?}");
    assert!(!outcome.document_promoted, "{outcome:?}");
    assert_eq!(document_state(id).unwrap(), DocumentState::AwaitRemote);
    assert_eq!(transport_state(id).unwrap(), TransportState::Handshaking);

    // Per the frozen transition table, the follow-up Step 2 is now a no-op
    // and therefore closes AwaitRemote initialization as incompatible: only
    // a Step 2 that itself installs the fragment promotes.
    let outcome = receive_message(
        id,
        283,
        generation,
        &step2_frame(server.diff_for(&session_state_vector_bytes(id))),
    )
    .unwrap();
    let close = outcome
        .close
        .as_ref()
        .expect("no-op Step 2 cannot promote AwaitRemote");
    assert_eq!(close.error.code, TRANSPORT_REMOTE_INADMISSIBLE, "{close:?}");
    assert_eq!(document_state(id).unwrap(), DocumentState::AwaitRemote);
    destroy_session(id);
}

include!("collaboration_protocol_test/limits.rs");

include!("collaboration_protocol_test/remote_updates.rs");
