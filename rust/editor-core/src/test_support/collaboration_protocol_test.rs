//! Task 9: strict standard y-sync frame handling with server Step 2
//! initialization.
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
    begin_connect, create_room_from_json, destroy_session, document_state, receive_message,
    remote_dependency_accounting, render_state, session_audit, set_collaboration_limit_for_test,
    socket_opened, sync_step1_frame, take_next_protocol_reply, transport_state, CloseDisposition,
    DocumentState, RenderState, TransportState,
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

/// `begin_connect` + `socket_opened`: the transport is `Handshaking` and the
/// returned generation is live.
fn handshake(id: u64) -> u64 {
    let generation = begin_connect(id, 9_000).unwrap();
    socket_opened(id, 9_001, generation).unwrap();
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
        "requestId": request_id,
        "baseDocumentRevision": revision,
        "text": text,
    })
    .to_string();
    bridge::submit_input(id, &envelope).unwrap();
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
    let (reply_request_id, reply) = take_next_protocol_reply(id).unwrap().unwrap();
    assert_eq!(reply_request_id, 201);
    assert_eq!(reply.len(), outcome.reply_bytes_enqueued);
    let peer = RawPeer::empty();
    peer.apply(&decode_step2_reply(&reply));
    assert_eq!(peer.state_vector(), session_state_vector(id));
    assert_eq!(
        peer.fragment_string(),
        RawPeer::from_snapshot(&snapshot).fragment_string(),
    );
    assert!(take_next_protocol_reply(id).unwrap().is_none());

    destroy_session(id);
}

#[test]
fn receive_refuses_sessions_without_an_attached_runtime() {
    let snapshot = snapshot_source();
    let config = serde_json::json!({
        "documentId": snapshot.document_id,
        "lineageId": snapshot.lineage_id,
        "snapshot": snapshot,
    });
    let id = create_room_from_json(&config.to_string()).unwrap();
    let generation = handshake(id);

    let error = receive_message(id, 211, generation, &NOOP_UPDATE_V1).unwrap_err();
    assert_eq!(error.domain, "boundary", "{error:?}");
    assert_eq!(error.code, "CONFIG_INVALID", "{error:?}");
    assert_eq!(transport_state(id).unwrap(), TransportState::Handshaking);

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
    let generation = begin_connect(id, 222).unwrap();
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
        begin_connect(id, 232).unwrap();
        destroy_session(id);
    }
}

#[test]
fn await_remote_valid_step2_initializes_and_synchronizes() {
    let id = create_await_remote_room();
    assert_eq!(document_state(id).unwrap(), DocumentState::AwaitRemote);
    assert_eq!(render_state(id).unwrap(), RenderState::Loading);
    let generation = handshake(id);

    // Our Step 1 is a standard frame an independent peer consumes directly.
    let our_step1 = sync_step1_frame(id, 241).unwrap();
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
    let (_, reply) = take_next_protocol_reply(id).unwrap().unwrap();
    server.apply(&decode_step2_reply(&reply));
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
    take_next_protocol_reply(id).unwrap().unwrap();

    set_collaboration_limit_for_test(id, "maxAggregateResponseBytes", reply_bytes).unwrap();
    let outcome = receive_message(id, 312, generation, &step1).unwrap();
    assert!(outcome.close.is_none(), "{outcome:?}");
    assert_eq!(outcome.reply_bytes_enqueued, reply_bytes, "{outcome:?}");
    take_next_protocol_reply(id).unwrap().unwrap();

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
    assert!(take_next_protocol_reply(id).unwrap().is_none());
    assert_eq!(bridge::session_audit(id).unwrap(), before_engine);

    destroy_session(id);
}

#[test]
fn reply_reservation_failures_close_before_any_engine_commit() {
    // A message coupling a Step 1 (reply owed) with a genuine remote update:
    // if the reply cannot be reserved, the update must NOT have been
    // committed — reservation strictly precedes the engine commit, which is
    // what makes reply failure after a remote commit impossible.
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
    // One shared outbox slot: the offline edit below fills it completely.
    bridge::set_outbox_ceilings(id, 1, 10 * 1024 * 1024).unwrap();
    local_edit(id, 401, "offline edit");
    let (pending_count, _) = bridge::outbox_pending(id).unwrap().unwrap();
    assert_eq!(pending_count, 1, "the offline edit must fill the only slot");

    // Reconnect: the peer's Step 1 cannot reserve its reply — retryable.
    let generation = handshake(id);
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
    let (request_id, _) = bridge::take_next_update(id).unwrap().unwrap();
    assert_eq!(request_id, 401);
    assert_eq!(bridge::outbox_pending(id).unwrap().unwrap(), (0, 0));

    // The same session reconnects (begin_connect accepted — not wedged in
    // Incompatible) and completes the full handshake.
    let generation = synchronize_ready_room(id, &snapshot);
    let outcome = receive_message(id, 403, generation, &step1).unwrap();
    assert!(outcome.close.is_none(), "{outcome:?}");
    assert_eq!(outcome.replies_enqueued, 1, "{outcome:?}");
    assert_eq!(transport_state(id).unwrap(), TransportState::Synchronized);

    destroy_session(id);
}

#[test]
fn bounded_dependencies_stay_quarantined_inside_the_engine() {
    let (prefix, delta_b) = dependent_room_updates();
    let (id, snapshot) = create_ready_room();
    let generation = synchronize_ready_room(id, &snapshot);
    let before_json = session_audit(id).unwrap().document_json.unwrap();
    assert_eq!(remote_dependency_accounting(id).unwrap(), (0, 0));

    // The dependency-pending update stays quarantined inside the engine; the
    // runtime holds only byte/work accounting, never a second payload copy.
    let outcome = receive_message(id, 331, generation, &update_frame(delta_b.clone())).unwrap();
    assert!(outcome.close.is_none(), "{outcome:?}");
    assert!(!outcome.remote_commit_applied, "{outcome:?}");
    let (retained_bytes, retained_work) = remote_dependency_accounting(id).unwrap();
    assert_eq!(retained_bytes, delta_b.len());
    assert_eq!(retained_work, delta_b.len() as u64);
    assert_eq!(
        session_audit(id).unwrap().document_json.unwrap(),
        before_json
    );
    assert_eq!(transport_state(id).unwrap(), TransportState::Synchronized);

    // Completing the dependency converges and clears the accounting.
    let outcome = receive_message(id, 332, generation, &update_frame(prefix)).unwrap();
    assert!(outcome.close.is_none(), "{outcome:?}");
    assert!(outcome.remote_commit_applied, "{outcome:?}");
    assert_eq!(remote_dependency_accounting(id).unwrap(), (0, 0));
    let json = session_audit(id)
        .unwrap()
        .document_json
        .unwrap()
        .to_string();
    assert!(json.contains("ab"), "{json}");

    destroy_session(id);
}

#[test]
fn first_one_over_dependency_update_is_rejected_before_commit() {
    let (_prefix, incoming) = dependent_room_updates();
    let (id, snapshot) = create_ready_room();
    let generation = synchronize_ready_room(id, &snapshot);
    let before_session = session_audit(id).unwrap();
    let before_engine = bridge::session_audit(id).unwrap();
    let before_dependencies = remote_dependency_accounting(id).unwrap();
    assert_eq!(before_dependencies, (0, 0));

    set_collaboration_limit_for_test(id, "maxPendingDependencyUpdateBytes", incoming.len() - 1)
        .unwrap();
    let outcome = receive_message(id, 333, generation, &update_frame(incoming)).unwrap();
    let close = outcome
        .close
        .as_ref()
        .expect("the first one-over dependency candidate must close");
    assert_eq!(
        close.disposition,
        CloseDisposition::Incompatible,
        "{close:?}"
    );
    assert_eq!(
        close.error.code, TRANSPORT_DEPENDENCY_LIMIT_EXCEEDED,
        "{close:?}"
    );
    assert_eq!(
        close.error.details.as_ref().unwrap()["field"],
        "maxPendingDependencyUpdateBytes",
    );
    assert!(!outcome.remote_commit_applied, "{outcome:?}");

    let mut expected_session = before_session;
    expected_session.transport_state = TransportState::Incompatible;
    assert_eq!(session_audit(id).unwrap(), expected_session);
    assert_eq!(
        bridge::session_audit(id).unwrap(),
        before_engine,
        "canonical JSON, encoded state, revisions, history, selection, and outbox must be unchanged",
    );
    assert_eq!(
        remote_dependency_accounting(id).unwrap(),
        before_dependencies,
        "the refused candidate must not rewrite quarantine or charge work",
    );
    destroy_session(id);
}

#[test]
fn recovery_update_is_judged_by_drained_candidate_state() {
    let (recovery, incoming) = dependent_room_updates();
    let (id, snapshot) = create_ready_room();
    let generation = synchronize_ready_room(id, &snapshot);

    let outcome = receive_message(id, 334, generation, &update_frame(incoming.clone())).unwrap();
    assert!(outcome.close.is_none(), "{outcome:?}");
    assert_eq!(
        remote_dependency_accounting(id).unwrap(),
        (incoming.len(), incoming.len() as u64),
    );

    // The candidate drains the quarantine, so neither retained bytes nor
    // accumulated pending work may be charged for this recovery update.
    set_collaboration_limit_for_test(id, "maxPendingDependencyUpdateBytes", 0).unwrap();
    set_collaboration_limit_for_test(id, "maxPendingDependencyUpdateWork", 0).unwrap();
    let outcome = receive_message(id, 335, generation, &update_frame(recovery)).unwrap();
    assert!(outcome.close.is_none(), "{outcome:?}");
    assert!(outcome.remote_commit_applied, "{outcome:?}");
    assert_eq!(remote_dependency_accounting(id).unwrap(), (0, 0));
    let json = session_audit(id)
        .unwrap()
        .document_json
        .unwrap()
        .to_string();
    assert!(json.contains("ab"), "{json}");
    destroy_session(id);
}

#[test]
fn rejected_dependency_work_never_ratchets_or_rewrites_quarantine() {
    let (recovery, delta_b, delta_c) = dependent_room_update_chain();
    let (id, snapshot) = create_ready_room();
    let generation = synchronize_ready_room(id, &snapshot);

    let outcome = receive_message(id, 336, generation, &update_frame(delta_b.clone())).unwrap();
    assert!(outcome.close.is_none(), "{outcome:?}");
    let before_dependencies = remote_dependency_accounting(id).unwrap();
    assert_eq!(before_dependencies, (delta_b.len(), delta_b.len() as u64));
    set_collaboration_limit_for_test(id, "maxPendingDependencyUpdateWork", delta_b.len()).unwrap();
    let before_session = session_audit(id).unwrap();
    let before_engine = bridge::session_audit(id).unwrap();

    let outcome = receive_message(id, 337, generation, &update_frame(delta_c)).unwrap();
    let close = outcome
        .close
        .as_ref()
        .expect("one-over candidate work must close");
    assert_eq!(
        close.disposition,
        CloseDisposition::Incompatible,
        "{close:?}"
    );
    assert_eq!(
        close.error.code, TRANSPORT_DEPENDENCY_LIMIT_EXCEEDED,
        "{close:?}"
    );
    assert_eq!(
        close.error.details.as_ref().unwrap()["field"],
        "maxPendingDependencyUpdateWork",
    );
    assert!(!outcome.remote_commit_applied, "{outcome:?}");

    let mut expected_session = before_session;
    expected_session.transport_state = TransportState::Incompatible;
    assert_eq!(session_audit(id).unwrap(), expected_session);
    assert_eq!(bridge::session_audit(id).unwrap(), before_engine);
    assert_eq!(
        remote_dependency_accounting(id).unwrap(),
        before_dependencies,
        "the refusal must preserve retained bytes and accumulated work",
    );

    // Probe the preserved quarantine directly: recovery may publish `b`,
    // but the refused `c` must not have been installed into the candidate.
    let slot = crate::registry::get_session(id).expect("session must remain registered");
    slot.with_alive(|session| {
        let prepared = session
            .engine
            .prepare_remote_update_v1(338, &recovery)
            .unwrap();
        assert!(!prepared.has_pending_dependencies());
        session
            .engine
            .commit_prepared_remote_update(prepared)
            .unwrap();
        let json = session.engine.document_json().unwrap().to_string();
        assert!(json.contains("ab"), "{json}");
        assert!(!json.contains("abc"), "{json}");
    })
    .unwrap();
    destroy_session(id);
}

#[test]
fn dependency_byte_and_work_ceilings_close_as_incompatible() {
    let (_prefix, delta_b) = dependent_room_updates();
    let quarantined_len = delta_b.len();

    // Byte ceiling: exact retained size passes.
    let (id, snapshot) = create_ready_room();
    let generation = synchronize_ready_room(id, &snapshot);
    set_collaboration_limit_for_test(id, "maxPendingDependencyUpdateBytes", quarantined_len)
        .unwrap();
    let outcome = receive_message(id, 341, generation, &update_frame(delta_b.clone())).unwrap();
    assert!(outcome.close.is_none(), "{outcome:?}");
    destroy_session(id);

    // Byte ceiling: one under the retained size closes as incompatible.
    let (id, snapshot) = create_ready_room();
    let generation = synchronize_ready_room(id, &snapshot);
    set_collaboration_limit_for_test(id, "maxPendingDependencyUpdateBytes", quarantined_len - 1)
        .unwrap();
    let outcome = receive_message(id, 342, generation, &update_frame(delta_b.clone())).unwrap();
    let close = outcome
        .close
        .as_ref()
        .expect("dependency byte overflow must close");
    assert_eq!(
        close.disposition,
        CloseDisposition::Incompatible,
        "{close:?}"
    );
    assert_eq!(
        close.error.code, TRANSPORT_DEPENDENCY_LIMIT_EXCEEDED,
        "{close:?}"
    );
    assert_eq!(
        close.error.details.as_ref().unwrap()["field"],
        "maxPendingDependencyUpdateBytes",
    );
    assert_eq!(transport_state(id).unwrap(), TransportState::Incompatible);
    destroy_session(id);

    // Work ceiling: work accumulates across quarantined admissions even when
    // the merged retained bytes do not grow, and closes one over the limit.
    let (id, snapshot) = create_ready_room();
    let generation = synchronize_ready_room(id, &snapshot);
    set_collaboration_limit_for_test(
        id,
        "maxPendingDependencyUpdateWork",
        2 * quarantined_len - 1,
    )
    .unwrap();
    let outcome = receive_message(id, 343, generation, &update_frame(delta_b.clone())).unwrap();
    assert!(outcome.close.is_none(), "{outcome:?}");
    let outcome = receive_message(id, 344, generation, &update_frame(delta_b.clone())).unwrap();
    let close = outcome
        .close
        .as_ref()
        .expect("dependency work overflow must close");
    assert_eq!(
        close.disposition,
        CloseDisposition::Incompatible,
        "{close:?}"
    );
    assert_eq!(
        close.error.code, TRANSPORT_DEPENDENCY_LIMIT_EXCEEDED,
        "{close:?}"
    );
    assert_eq!(
        close.error.details.as_ref().unwrap()["field"],
        "maxPendingDependencyUpdateWork",
    );
    destroy_session(id);

    // Work ceiling: the exact accumulated work passes.
    let (id, snapshot) = create_ready_room();
    let generation = synchronize_ready_room(id, &snapshot);
    set_collaboration_limit_for_test(id, "maxPendingDependencyUpdateWork", 2 * quarantined_len)
        .unwrap();
    receive_message(id, 345, generation, &update_frame(delta_b.clone())).unwrap();
    let outcome = receive_message(id, 346, generation, &update_frame(delta_b)).unwrap();
    assert!(outcome.close.is_none(), "{outcome:?}");
    destroy_session(id);
}

#[test]
fn permanently_inadmissible_remote_state_preserves_the_engine_audit() {
    let mut foreign = YrsDocumentEngine::new(YrsEngineConfig {
        schema: incompatible_blockquote_schema(),
        fragment_name: FRAGMENT_NAME.into(),
        initialization_mode: InitializationMode::LocalEmpty,
        resource_limits: ResourceLimits::default(),
        editing_limits: EditingLimits::default(),
        max_length: None,
        scope: None,
    })
    .unwrap();
    foreign
        .import_json(
            &serde_json::json!({"type":"doc","content":[{"type":"blockquote","content":[{"type":"text","text":"invalid in target"}]}]}).to_string(),
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let invalid_update = foreign.encoded_state().unwrap();

    let (id, snapshot) = create_ready_room();
    let generation = synchronize_ready_room(id, &snapshot);
    let before = session_audit(id).unwrap();
    let before_engine = bridge::session_audit(id).unwrap();

    let outcome = receive_message(id, 351, generation, &update_frame(invalid_update)).unwrap();
    let close = outcome
        .close
        .expect("schema-invalid remote state must close the generation");
    assert_eq!(
        close.disposition,
        CloseDisposition::Incompatible,
        "{close:?}"
    );
    assert_eq!(close.error.code, TRANSPORT_REMOTE_INADMISSIBLE, "{close:?}");
    let details = close.error.details.as_ref().unwrap();
    assert_eq!(details["cause"]["code"], "DOCUMENT_INVALID", "{details}");

    let mut expected = before.clone();
    expected.transport_state = TransportState::Incompatible;
    assert_eq!(session_audit(id).unwrap(), expected);
    assert_eq!(bridge::session_audit(id).unwrap(), before_engine);

    destroy_session(id);
}

#[test]
fn local_operation_errors_do_not_disconnect_a_synchronized_transport() {
    let (id, snapshot) = create_ready_room();
    let generation = synchronize_ready_room(id, &snapshot);

    // A failing local edit (stale base revision) is an operation error and
    // must leave the healthy transport untouched.
    let stale_envelope = serde_json::json!({
        "version": 1,
        "requestId": 361,
        "baseDocumentRevision": 999_999,
        "text": "stale local edit",
    })
    .to_string();
    let error = bridge::submit_input(id, &stale_envelope).unwrap_err();
    assert_eq!(error.code, "REVISION_MISMATCH", "{error:?}");
    assert_eq!(transport_state(id).unwrap(), TransportState::Synchronized);

    // The generation stays live: the next frame is still accepted.
    let outcome =
        receive_message(id, 362, generation, &update_frame(NOOP_UPDATE_V1.to_vec())).unwrap();
    assert!(outcome.close.is_none(), "{outcome:?}");
    assert_eq!(transport_state(id).unwrap(), TransportState::Synchronized);

    destroy_session(id);
}

#[test]
fn remote_commits_are_never_echoed_and_local_edits_still_enqueue_once() {
    let (id, snapshot) = create_ready_room();
    let generation = synchronize_ready_room(id, &snapshot);
    assert_eq!(bridge::outbox_pending(id).unwrap().unwrap(), (0, 0));

    // A committed remote update produces no outbox document entry.
    let server = RawPeer::from_snapshot(&snapshot);
    server.push_text(" no echo");
    let outcome = receive_message(
        id,
        371,
        generation,
        &update_frame(server.diff_for(&session_state_vector_bytes(id))),
    )
    .unwrap();
    assert!(outcome.remote_commit_applied, "{outcome:?}");
    assert_eq!(
        bridge::outbox_pending(id).unwrap().unwrap(),
        (0, 0),
        "remote commits must never be echoed as document updates",
    );
    assert!(take_next_protocol_reply(id).unwrap().is_none());

    // A local edit while Synchronized still enqueues exactly one bounded
    // document update, and the two paths coexist.
    local_edit(id, 372, "local while synchronized");
    let (pending_count, pending_bytes) = bridge::outbox_pending(id).unwrap().unwrap();
    assert_eq!(pending_count, 1);
    assert!(pending_bytes > 0);
    let (request_id, update) = bridge::take_next_update(id).unwrap().unwrap();
    assert_eq!(request_id, 372);

    // The captured update converges the independent peer.
    server.apply(&update);
    assert_eq!(server.state_vector(), session_state_vector(id));
    assert_eq!(bridge::outbox_pending(id).unwrap().unwrap(), (0, 0));

    destroy_session(id);
}

#[test]
fn update_exchange_converges_with_an_independent_raw_peer() {
    let (id, snapshot) = create_ready_room();
    let generation = synchronize_ready_room(id, &snapshot);
    let server = RawPeer::from_snapshot(&snapshot);

    // Their edit -> our runtime.
    server.push_text(" from peer");
    let outcome = receive_message(
        id,
        381,
        generation,
        &update_frame(server.diff_for(&session_state_vector_bytes(id))),
    )
    .unwrap();
    assert!(outcome.remote_commit_applied, "{outcome:?}");

    // Our edit -> their doc.
    local_edit(id, 382, " from us");
    let (_, update) = bridge::take_next_update(id).unwrap().unwrap();
    server.apply(&update);

    // Both directions converge to state-vector equality.
    assert_eq!(server.state_vector(), session_state_vector(id));
    let json = session_audit(id)
        .unwrap()
        .document_json
        .unwrap()
        .to_string();
    assert!(json.contains("from peer"), "{json}");

    destroy_session(id);
}

// ---------------------------------------------------------------------------
// Dependency-quarantine byte/work ceilings are charged from the prepared
// post-state before commit can mutate the live quarantine.
// ---------------------------------------------------------------------------

/// `(prefix, delta_b, delta_c)`: both deltas depend on content only present
/// in earlier states, so a receiver holding the seed must quarantine each
/// until the prefix (and for `delta_c`, also `delta_b`) arrives.
fn dependent_room_update_chain() -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let mut source = source_engine();
    source
        .apply_command(101, TypedCommand::InsertText { text: "a".into() })
        .unwrap();
    let after_a = source.encoded_state().unwrap();
    source
        .apply_command(102, TypedCommand::InsertText { text: "b".into() })
        .unwrap();
    let after_b = source.encoded_state().unwrap();
    source
        .apply_command(103, TypedCommand::InsertText { text: "c".into() })
        .unwrap();
    let after_c = source.encoded_state().unwrap();
    let after_a_sv = encode_state_vector_from_update_v1(&after_a).unwrap();
    let after_b_sv = encode_state_vector_from_update_v1(&after_b).unwrap();
    let delta_b = diff_updates_v1(&after_b, &after_a_sv).unwrap();
    let delta_c = diff_updates_v1(&after_c, &after_b_sv).unwrap();
    (after_a, delta_b, delta_c)
}

#[test]
fn prepared_remote_update_drop_is_observationally_pure() {
    let (_, delta_b, _) = dependent_room_update_chain();
    let (id, snapshot) = create_ready_room();
    synchronize_ready_room(id, &snapshot);

    let before_audit = session_audit(id).unwrap();
    let before_dependencies = remote_dependency_accounting(id).unwrap();
    let before_encoded = bridge::session_audit(id).unwrap().encoded_state;

    let slot = crate::registry::get_session(id).expect("session must remain registered");
    slot.with_alive(|session| {
        let prepared = session
            .engine
            .prepare_remote_update_v1(360, &delta_b)
            .unwrap();
        assert_eq!(prepared.retained_dependency_bytes(), delta_b.len());
        assert!(prepared.has_pending_dependencies());
        drop(prepared);
    })
    .unwrap();

    assert_eq!(session_audit(id).unwrap(), before_audit);
    assert_eq!(
        remote_dependency_accounting(id).unwrap(),
        before_dependencies
    );
    assert_eq!(
        bridge::session_audit(id).unwrap().encoded_state,
        before_encoded
    );
    destroy_session(id);
}

#[test]
fn dependency_byte_ceiling_refuses_before_any_quarantine_mutation() {
    let (prefix, delta_b, delta_c) = dependent_room_update_chain();
    let retained = delta_b.len();

    // One over: the exact merged candidate crosses the retained-byte
    // ceiling. The refusal must leave the quarantine byte-identical and the
    // work counter untouched — candidate admission, not a post-commit
    // apology.
    let (id, snapshot) = create_ready_room();
    let generation = synchronize_ready_room(id, &snapshot);
    let outcome = receive_message(id, 361, generation, &update_frame(delta_b.clone())).unwrap();
    assert!(outcome.close.is_none(), "{outcome:?}");
    assert_eq!(
        remote_dependency_accounting(id).unwrap(),
        (retained, retained as u64),
    );
    set_collaboration_limit_for_test(id, "maxPendingDependencyUpdateBytes", retained).unwrap();
    let before = session_audit(id).unwrap();
    let before_engine = bridge::session_audit(id).unwrap();
    let expected_retained = Update::merge_updates(vec![
        Update::decode_v1(&delta_b).unwrap(),
        Update::decode_v1(&delta_c).unwrap(),
    ])
    .encode_v1()
    .len();
    let slot = crate::registry::get_session(id).expect("session must remain registered");
    slot.with_alive(|session| {
        let before_bytes = session.engine.pending_remote_dependency_bytes();
        let prepared = session
            .engine
            .prepare_remote_update_v1(362, &delta_c)
            .unwrap();
        assert_eq!(prepared.retained_dependency_bytes(), expected_retained);
        assert!(prepared.has_pending_dependencies());
        drop(prepared);
        assert_eq!(
            session.engine.pending_remote_dependency_bytes(),
            before_bytes
        );
    })
    .unwrap();

    let outcome = receive_message(id, 362, generation, &update_frame(delta_c.clone())).unwrap();
    let close = outcome
        .close
        .as_ref()
        .expect("one over the candidate byte ceiling must close");
    assert_eq!(
        close.disposition,
        CloseDisposition::Incompatible,
        "{close:?}"
    );
    assert_eq!(
        close.error.code, TRANSPORT_DEPENDENCY_LIMIT_EXCEEDED,
        "{close:?}"
    );
    assert_eq!(
        close.error.details.as_ref().unwrap()["field"],
        "maxPendingDependencyUpdateBytes",
    );
    // Zero mutation: retained bytes and charged work are exactly the
    // pre-refusal figures, and the full audits match (the deliberate
    // generation close aside).
    assert_eq!(
        remote_dependency_accounting(id).unwrap(),
        (retained, retained as u64),
        "the refused payload was never retained or charged",
    );
    let mut expected = before.clone();
    expected.transport_state = TransportState::Incompatible;
    assert_eq!(session_audit(id).unwrap(), expected);
    assert_eq!(bridge::session_audit(id).unwrap(), before_engine);
    destroy_session(id);

    // Exact: the prepared candidate at the ceiling admits.
    let (id, snapshot) = create_ready_room();
    let generation = synchronize_ready_room(id, &snapshot);
    receive_message(id, 363, generation, &update_frame(delta_b.clone())).unwrap();
    set_collaboration_limit_for_test(id, "maxPendingDependencyUpdateBytes", expected_retained)
        .unwrap();
    let outcome = receive_message(id, 364, generation, &update_frame(delta_c.clone())).unwrap();
    assert!(outcome.close.is_none(), "{outcome:?}");
    assert_eq!(
        remote_dependency_accounting(id).unwrap().0,
        expected_retained,
        "the exact admitted candidate stays pending",
    );
    destroy_session(id);

    // After pruning (the dependency completes and the quarantine drains),
    // the identical update succeeds — the refusal above was purely the
    // retained candidate charge, never the payload's content.
    let (id, snapshot) = create_ready_room();
    let generation = synchronize_ready_room(id, &snapshot);
    receive_message(id, 365, generation, &update_frame(delta_b.clone())).unwrap();
    let slot = crate::registry::get_session(id).expect("session must remain registered");
    slot.with_alive(|session| {
        let before_bytes = session.engine.pending_remote_dependency_bytes();
        let prepared = session
            .engine
            .prepare_remote_update_v1(366, &prefix)
            .unwrap();
        assert_eq!(prepared.retained_dependency_bytes(), 0);
        assert!(!prepared.has_pending_dependencies());
        drop(prepared);
        assert_eq!(
            session.engine.pending_remote_dependency_bytes(),
            before_bytes
        );
    })
    .unwrap();
    let outcome = receive_message(id, 366, generation, &update_frame(prefix)).unwrap();
    assert!(outcome.close.is_none(), "{outcome:?}");
    assert_eq!(remote_dependency_accounting(id).unwrap(), (0, 0));
    let outcome = receive_message(id, 367, generation, &update_frame(delta_c)).unwrap();
    assert!(outcome.close.is_none(), "{outcome:?}");
    assert!(outcome.remote_commit_applied, "{outcome:?}");
    assert_eq!(remote_dependency_accounting(id).unwrap(), (0, 0));
    let json = session_audit(id)
        .unwrap()
        .document_json
        .unwrap()
        .to_string();
    assert!(json.contains("abc"), "{json}");
    destroy_session(id);
}

#[test]
fn dependency_work_ceiling_refusal_never_ratchets_the_counter() {
    let (_prefix, delta_b, delta_c) = dependent_room_update_chain();
    let retained = delta_b.len();

    // One over the prepared candidate's work ceiling: the refusal must not
    // ratchet the counter (the review's permanent-ratchet defect).
    let (id, snapshot) = create_ready_room();
    let generation = synchronize_ready_room(id, &snapshot);
    receive_message(id, 371, generation, &update_frame(delta_b.clone())).unwrap();
    assert_eq!(
        remote_dependency_accounting(id).unwrap(),
        (retained, retained as u64),
    );
    set_collaboration_limit_for_test(id, "maxPendingDependencyUpdateWork", retained).unwrap();
    let outcome = receive_message(id, 372, generation, &update_frame(delta_c.clone())).unwrap();
    let close = outcome
        .close
        .as_ref()
        .expect("one over the candidate work ceiling must close");
    assert_eq!(
        close.disposition,
        CloseDisposition::Incompatible,
        "{close:?}"
    );
    assert_eq!(
        close.error.code, TRANSPORT_DEPENDENCY_LIMIT_EXCEEDED,
        "{close:?}"
    );
    assert_eq!(
        close.error.details.as_ref().unwrap()["field"],
        "maxPendingDependencyUpdateWork",
    );
    assert_eq!(
        remote_dependency_accounting(id).unwrap(),
        (retained, retained as u64),
        "the refused admission never ratcheted the work counter",
    );
    destroy_session(id);

    // Exact: the accumulated work at the ceiling admits.
    let (id, snapshot) = create_ready_room();
    let generation = synchronize_ready_room(id, &snapshot);
    receive_message(id, 373, generation, &update_frame(delta_b.clone())).unwrap();
    let exact_work = retained + delta_c.len();
    set_collaboration_limit_for_test(id, "maxPendingDependencyUpdateWork", exact_work).unwrap();
    let outcome = receive_message(id, 374, generation, &update_frame(delta_c)).unwrap();
    assert!(outcome.close.is_none(), "{outcome:?}");
    assert_eq!(
        remote_dependency_accounting(id).unwrap().1,
        exact_work as u64,
    );
    destroy_session(id);
}
