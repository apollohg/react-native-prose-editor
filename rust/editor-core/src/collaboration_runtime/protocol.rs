//! Strict standard y-sync protocol handling (Task 9).
//!
//! One bounded entry point — [`CollaborationRuntime::receive_message`] —
//! composes the sealed seams built by Tasks 6–8 and owns nothing else:
//!
//! - generation discipline and the `Handshaking`/`Synchronized` admission
//!   gate come from the Task 8 state machine (checked before ANY decode);
//! - Sync Step 1 replies are completely built through the engine's
//!   read-only `encode_diff_v1` and reserved through the Task 7
//!   `reserve_protocol_replies` seam BEFORE any engine commit, so
//!   reply/outbox failure after a remote commit is impossible;
//! - document effects flow exclusively through the Task 6 sealed
//!   `prepare_remote_update_v1`/`commit_prepared_remote_update` split. The
//!   protocol layer never touches a `yrs::Doc`, a transaction, or an
//!   `Update` application.
//!
//! An accepted current-generation Sync Step 2 is the ONLY synchronization
//! gate, including the server-owned `AwaitRemote -> RoomReady` promotion.
//! Task 10 extended the same classification pipeline to the standard
//! y-protocols awareness (tag 1) and query-awareness (tag 3) messages:
//! awareness payloads apply through the sealed Task 6 `AwarenessCodec`
//! (never touching document state, revisions, sync gating, or the document
//! outbox), query replies are prebuilt and reserved exactly like Step 1
//! replies, and completing the handshake re-publishes the desired local
//! awareness with a fresh clock. Auth and custom message types remain
//! protocol errors.

#![allow(
    clippy::result_large_err,
    reason = "SessionError is the established unboxed session error envelope"
)]

use serde_json::json;

use yrs::encoding::read::{Cursor, Read};
use yrs::encoding::write::Write;
use yrs::sync::protocol::{
    MSG_AWARENESS, MSG_QUERY_AWARENESS, MSG_SYNC, MSG_SYNC_STEP_1, MSG_SYNC_STEP_2, MSG_SYNC_UPDATE,
};
use yrs::updates::encoder::{Encoder, EncoderV1};

use crate::ffi_v2::types::AWARENESS_CLOCK_EXHAUSTED;
use crate::session::{
    CollaborationLimits, DocumentState, ErrorDomain, OperationFailureClass, SessionError,
    TransportState,
};
use crate::yrs_engine::{EngineCommit, OperationError, YrsDocumentEngine, YrsEngineError};

use super::outbox::OutboxReservationError;
use super::state::{SocketCloseDisposition, TransportGeneration, TransportStateMachine};
use super::CollaborationRuntime;

/// Malformed protocol framing or update encoding; the generation closes
/// retryably (a fresh handshake re-synchronizes through Step 1/Step 2).
/// Frozen representative transport code in the shared error contract.
pub const TRANSPORT_PROTOCOL_INVALID: &str = "TRANSPORT_PROTOCOL_INVALID";
/// Inbound frame byte/count ceilings; deterministic, so incompatible.
pub const TRANSPORT_FRAME_LIMIT_EXCEEDED: &str = "TRANSPORT_FRAME_LIMIT_EXCEEDED";
/// Aggregate reply admission (response ceiling or saturated outbox);
/// deterministic, so incompatible.
pub const TRANSPORT_REPLY_LIMIT_EXCEEDED: &str = "TRANSPORT_REPLY_LIMIT_EXCEEDED";
/// Schema-invalid, over-limit, or otherwise permanently inadmissible remote
/// document state (including failed server-owned initialization); the
/// underlying engine error rides along as the structured cause.
pub const TRANSPORT_REMOTE_INADMISSIBLE: &str = "TRANSPORT_REMOTE_INADMISSIBLE";
/// Engine-reported dependency-quarantine byte/work accounting exceeded the
/// configured pending-update ceilings; deterministic, so incompatible.
pub const TRANSPORT_DEPENDENCY_LIMIT_EXCEEDED: &str = "TRANSPORT_DEPENDENCY_LIMIT_EXCEEDED";
/// Inbound awareness content exceeded a configured awareness ceiling
/// (peer count, per-peer bytes, aggregate bytes); deterministic per-message
/// admission, so incompatible (Task 9 saturation ruling).
pub const TRANSPORT_AWARENESS_LIMIT_EXCEEDED: &str = "TRANSPORT_AWARENESS_LIMIT_EXCEEDED";
/// Recoverable allocation/reservation exhaustion; retry may succeed.
pub const TRANSPORT_RESOURCE_EXHAUSTED: &str = "TRANSPORT_RESOURCE_EXHAUSTED";
/// Residual engine admission failures (defensive invariants, derived-state
/// preparation); a reconnect resynchronizes from scratch, so retryable.
pub const TRANSPORT_REMOTE_APPLY_FAILED: &str = "TRANSPORT_REMOTE_APPLY_FAILED";

/// Wire action every structured refusal/close reports.
const RECEIVE_ACTION: &str = "receiveMessage";
/// `CollaborationLimits` field names charged by the receive path.
const MAX_FRAME_BYTES_FIELD: &str = "maxFrameBytes";
const MAX_FRAMES_PER_MESSAGE_FIELD: &str = "maxFramesPerMessage";
const MAX_AGGREGATE_RESPONSE_BYTES_FIELD: &str = "maxAggregateResponseBytes";
const MAX_PENDING_DEPENDENCY_BYTES_FIELD: &str = "maxPendingDependencyUpdateBytes";
const MAX_PENDING_DEPENDENCY_WORK_FIELD: &str = "maxPendingDependencyUpdateWork";

/// The session collaborators one receive composes; built by the session's
/// field-disjoint split borrow.
pub(crate) struct ReceiveContext<'a> {
    pub(crate) transport: &'a mut TransportStateMachine,
    pub(crate) engine: &'a mut YrsDocumentEngine,
    pub(crate) document_state: &'a mut DocumentState,
    pub(crate) limits: &'a CollaborationLimits,
}

/// What one accepted receive did. Refusals (stale generation, wrong state,
/// no runtime) return `Err` without touching anything; accepted messages
/// always return an outcome, whose disposition reports whether the frame
/// content forced the generation closed.
#[derive(Debug)]
pub(crate) struct ReceiveOutcome {
    pub(crate) frames_decoded: usize,
    pub(crate) replies_enqueued: usize,
    pub(crate) reply_bytes_enqueued: usize,
    pub(crate) remote_commit_applied: bool,
    pub(crate) document_promoted: bool,
    pub(crate) transport_state: TransportState,
    pub(crate) disposition: ReceiveDisposition,
}

#[derive(Debug)]
pub(crate) enum ReceiveDisposition {
    /// Frames accepted; the socket stays open.
    Continue,
    /// The message classified as a failure: the generation was closed with
    /// the Rust-owned disposition and this structured error.
    CloseGeneration {
        close: SocketCloseDisposition,
        error: SessionError,
    },
}

/// One decoded standard protocol frame; payloads stay raw for the sealed
/// engine seams (the engine owns all Update/state-vector/awareness decoding
/// semantics).
#[derive(Debug, PartialEq, Eq)]
enum ProtocolFrame {
    SyncStep1(Vec<u8>),
    SyncStep2(Vec<u8>),
    SyncUpdate(Vec<u8>),
    /// Raw encoded `AwarenessUpdate` payload (y-protocols tag 1).
    Awareness(Vec<u8>),
    /// Query-awareness request (y-protocols tag 3, payload-less).
    AwarenessQuery,
}

/// Internal failure envelope: how to close the generation, and why.
#[derive(Debug)]
struct ReceiveFailure {
    close: SocketCloseDisposition,
    error: SessionError,
}

impl CollaborationRuntime {
    /// Bounded standard y-sync frame handling for one inbound transport
    /// message. Frozen flow order: generation gate -> bounded
    /// classification/decode -> reply prebuild + reservation ->
    /// candidate admission and same-engine commit (Step 2 gate included)
    /// -> infallible reply installation.
    pub(crate) fn receive_message(
        &mut self,
        request_id: u64,
        generation: TransportGeneration,
        context: ReceiveContext<'_>,
        bytes: &[u8],
    ) -> Result<ReceiveOutcome, SessionError> {
        let ReceiveContext {
            transport,
            engine,
            document_state,
            limits,
        } = context;
        // Generation + state gate before ANY decode work: only the live
        // generation in Handshaking/Synchronized may cause work.
        transport.admit_receive(request_id, generation)?;

        let mut outcome = ReceiveOutcome {
            frames_decoded: 0,
            replies_enqueued: 0,
            reply_bytes_enqueued: 0,
            remote_commit_applied: false,
            document_promoted: false,
            transport_state: transport.state(),
            disposition: ReceiveDisposition::Continue,
        };

        let result = self.process_admitted_message(
            request_id,
            generation,
            transport,
            engine,
            document_state,
            limits,
            bytes,
            &mut outcome,
        );
        match result {
            Ok(()) => {
                outcome.transport_state = transport.state();
                Ok(outcome)
            }
            Err(failure) => {
                let closed = transport
                    .socket_closed(request_id, generation, failure.close)
                    .expect(
                        "the admitted live generation must remain closable for its own failure",
                    );
                // Task 10 lifecycle rule: every generation close clears the
                // transport-scoped peers while desired awareness survives.
                self.clear_transport_peers(engine);
                outcome.transport_state = closed;
                outcome.disposition = ReceiveDisposition::CloseGeneration {
                    close: failure.close,
                    error: failure.error,
                };
                Ok(outcome)
            }
        }
    }

    /// Everything after the admission gate, in the frozen flow order.
    #[expect(
        clippy::too_many_arguments,
        reason = "one-shot composition of the session's split borrows; \
                  bundling them again would only re-wrap ReceiveContext"
    )]
    fn process_admitted_message(
        &mut self,
        request_id: u64,
        generation: TransportGeneration,
        transport: &mut TransportStateMachine,
        engine: &mut YrsDocumentEngine,
        document_state: &mut DocumentState,
        limits: &CollaborationLimits,
        bytes: &[u8],
        outcome: &mut ReceiveOutcome,
    ) -> Result<(), ReceiveFailure> {
        // Bounded classification/decode: message bytes, then frame count.
        if bytes.len() > limits.max_frame_bytes {
            return Err(limit_failure(
                request_id,
                TRANSPORT_FRAME_LIMIT_EXCEEDED,
                MAX_FRAME_BYTES_FIELD,
                limits.max_frame_bytes as u64,
                bytes.len() as u64,
            ));
        }
        let frames = decode_protocol_frames(request_id, bytes, limits.max_frames_per_message)?;
        outcome.frames_decoded = frames.len();

        // Pre-admit and reserve every reply BEFORE any engine commit
        // (Step 1 idiom, shared by query-awareness answers and the
        // handshake-completion awareness re-publish).
        let mut replies: Vec<Vec<u8>> = Vec::new();
        let mut reply_bytes_total = 0usize;
        let admit_reply = |message: Vec<u8>,
                           replies: &mut Vec<Vec<u8>>,
                           reply_bytes_total: &mut usize|
         -> Result<(), ReceiveFailure> {
            *reply_bytes_total = reply_bytes_total.saturating_add(message.len());
            if *reply_bytes_total > limits.max_aggregate_response_bytes {
                return Err(limit_failure(
                    request_id,
                    TRANSPORT_REPLY_LIMIT_EXCEEDED,
                    MAX_AGGREGATE_RESPONSE_BYTES_FIELD,
                    limits.max_aggregate_response_bytes as u64,
                    *reply_bytes_total as u64,
                ));
            }
            replies.push(message);
            Ok(())
        };
        for frame in &frames {
            match frame {
                ProtocolFrame::SyncStep1(remote_state_vector) => {
                    let diff = engine
                        .encode_diff_v1(request_id, remote_state_vector)
                        .map_err(|error| classify_reply_build_error(request_id, error))?;
                    admit_reply(
                        frame_sync_message(MSG_SYNC_STEP_2, &diff),
                        &mut replies,
                        &mut reply_bytes_total,
                    )?;
                }
                ProtocolFrame::AwarenessQuery => {
                    // The complete answer per standard semantics: every live
                    // state (local included), built through the codec.
                    let answer = engine
                        .awareness()
                        .encode_full_update_v1()
                        .map_err(|error| classify_awareness_error(request_id, error))?;
                    admit_reply(
                        frame_awareness_message(&answer),
                        &mut replies,
                        &mut reply_bytes_total,
                    )?;
                }
                ProtocolFrame::SyncStep2(_)
                | ProtocolFrame::SyncUpdate(_)
                | ProtocolFrame::Awareness(_) => {}
            }
        }
        // A Step 2 on a Handshaking transport is the handshake-completion
        // point: prebuild the desired-awareness re-publish (fresh clock —
        // the designed mitigation for the Task 6 tombstone-migration gap)
        // so it rides the same reservation. If the Step 2 later fails, the
        // generation closes and the unconsumed reservation releases.
        let republish_included = if transport.state() == TransportState::Handshaking
            && frames
                .iter()
                .any(|frame| matches!(frame, ProtocolFrame::SyncStep2(_)))
        {
            match self
                .prepare_handshake_republish(engine, limits)
                .map_err(|error| classify_awareness_error(request_id, error))?
            {
                Some(message) => {
                    admit_reply(message, &mut replies, &mut reply_bytes_total)?;
                    true
                }
                None => false,
            }
        } else {
            false
        };
        let reservation = if replies.is_empty() {
            None
        } else {
            Some(
                self.outbox
                    .reserve_protocol_replies(replies.len(), reply_bytes_total)
                    .map_err(|error| classify_reservation_error(request_id, error))?,
            )
        };

        // Candidate admission and same-engine commit, frame by frame, with
        // the Step 2 synchronization gate applied at commit time.
        for frame in &frames {
            match frame {
                ProtocolFrame::SyncStep1(_) | ProtocolFrame::AwarenessQuery => {}
                ProtocolFrame::SyncStep2(update) => {
                    let commit = self.admit_remote_update(request_id, engine, limits, update)?;
                    if commit.changed {
                        outcome.remote_commit_applied = true;
                    }
                    if transport.state() == TransportState::Handshaking {
                        self.apply_step2_synchronization_gate(
                            request_id,
                            generation,
                            transport,
                            document_state,
                            commit.changed,
                            outcome,
                        )?;
                    }
                    // In `Synchronized`, a Step 2 is semantically an update:
                    // admitted above, no transport or document transition.
                }
                ProtocolFrame::SyncUpdate(update) => {
                    // Update frames NEVER synchronize or promote, in any
                    // state: quarantine/admission rules only.
                    let commit = self.admit_remote_update(request_id, engine, limits, update)?;
                    if commit.changed {
                        outcome.remote_commit_applied = true;
                    }
                }
                ProtocolFrame::Awareness(payload) => {
                    // Awareness frames never touch document state, sync
                    // gating, or the document outbox: codec application
                    // plus runtime activity stamping only.
                    self.apply_awareness_frame(engine, limits, payload)
                        .map_err(|error| classify_awareness_error(request_id, error))?;
                }
            }
        }

        // Infallible reply installation: capacity and storage were reserved
        // before the first commit.
        if let Some(reservation) = reservation {
            outcome.replies_enqueued = replies.len();
            outcome.reply_bytes_enqueued = reply_bytes_total;
            self.outbox
                .install_protocol_replies(reservation, request_id, replies);
            if republish_included {
                self.mark_local_awareness_published();
            }
        }
        Ok(())
    }

    /// The transition-table Step 2 rows for a `Handshaking` transport. An
    /// accepted current-generation Step 2 is the ONLY synchronization gate:
    /// `AwaitRemote` promotes to `RoomReady` exactly when the Step 2 itself
    /// installed a valid configured fragment.
    fn apply_step2_synchronization_gate(
        &mut self,
        request_id: u64,
        generation: TransportGeneration,
        transport: &mut TransportStateMachine,
        document_state: &mut DocumentState,
        commit_changed: bool,
        outcome: &mut ReceiveOutcome,
    ) -> Result<(), ReceiveFailure> {
        let synchronize = |transport: &mut TransportStateMachine| {
            transport
                .mark_synchronized(request_id, generation)
                .expect("Step 2 synchronization must be legal for a live Handshaking transport");
        };
        match *document_state {
            DocumentState::AwaitRemote => {
                if commit_changed {
                    // The sealed prepare path already proved the configured
                    // fragment installed as schema-valid content.
                    *document_state = DocumentState::RoomReady;
                    outcome.document_promoted = true;
                    synchronize(transport);
                    Ok(())
                } else {
                    // No-op Step 2 (or one whose fragment never installed):
                    // server-owned initialization deterministically failed.
                    Err(ReceiveFailure {
                        close: SocketCloseDisposition::Incompatible,
                        error: transport_error(
                            request_id,
                            TRANSPORT_REMOTE_INADMISSIBLE,
                            "Sync Step 2 did not install the configured document fragment; \
                             server-owned initialization cannot complete",
                            json!({ "action": RECEIVE_ACTION, "reason": "emptyInitialization" }),
                        ),
                    })
                }
            }
            DocumentState::RoomReady => {
                // Valid Step 2, including a genuine no-op: synchronized.
                synchronize(transport);
                Ok(())
            }
            DocumentState::LocalReady => unreachable!(
                "a Handshaking transport with a live generation requires an accepted \
                 begin_connect, which refuses local-only (LocalReady) sessions"
            ),
        }
    }

    /// Prepare + commit one remote Update-v1 through the sealed Task 6
    /// seams, admitting dependency-quarantine byte/work ceilings against the
    /// exact prepared post-state before commit. The runtime never retains a
    /// second payload copy: bytes stay inside the engine, the runtime keeps
    /// only the byte-unit work counter.
    fn admit_remote_update(
        &mut self,
        request_id: u64,
        engine: &mut YrsDocumentEngine,
        limits: &CollaborationLimits,
        update: &[u8],
    ) -> Result<EngineCommit, ReceiveFailure> {
        // Preparation owns temporary decode/merge buffers under the engine's
        // maxEncodedStateBytes resource ceiling. The dependency ceilings
        // below charge only the retained post-update candidate and its
        // accumulated pending work.
        let prepared = engine
            .prepare_remote_update_v1(request_id, update)
            .map_err(|error| classify_admission_error(engine, request_id, update, error))?;
        let candidate_bytes = prepared.retained_dependency_bytes();
        let candidate_work = if prepared.has_pending_dependencies() {
            self.remote_dependency_work
                .checked_add(update.len() as u64)
                .ok_or_else(|| dependency_work_overflow(request_id, limits))?
        } else {
            0
        };
        admit_dependency_candidate(request_id, candidate_bytes, candidate_work, limits)?;

        let commit = engine
            .commit_prepared_remote_update(prepared)
            .map_err(|error| classify_admission_error(engine, request_id, update, error))?;
        self.remote_dependency_work = candidate_work;
        Ok(commit)
    }
}

fn admit_dependency_candidate(
    request_id: u64,
    candidate_bytes: usize,
    candidate_work: u64,
    limits: &CollaborationLimits,
) -> Result<(), ReceiveFailure> {
    if candidate_bytes > limits.max_pending_dependency_update_bytes {
        return Err(limit_failure(
            request_id,
            TRANSPORT_DEPENDENCY_LIMIT_EXCEEDED,
            MAX_PENDING_DEPENDENCY_BYTES_FIELD,
            limits.max_pending_dependency_update_bytes as u64,
            candidate_bytes as u64,
        ));
    }
    if candidate_work > limits.max_pending_dependency_update_work as u64 {
        return Err(limit_failure(
            request_id,
            TRANSPORT_DEPENDENCY_LIMIT_EXCEEDED,
            MAX_PENDING_DEPENDENCY_WORK_FIELD,
            limits.max_pending_dependency_update_work as u64,
            candidate_work,
        ));
    }
    Ok(())
}

fn dependency_work_overflow(request_id: u64, limits: &CollaborationLimits) -> ReceiveFailure {
    limit_failure(
        request_id,
        TRANSPORT_DEPENDENCY_LIMIT_EXCEEDED,
        MAX_PENDING_DEPENDENCY_WORK_FIELD,
        limits.max_pending_dependency_update_work as u64,
        u64::MAX,
    )
}

/// The framed Sync Step 1 message owed after `socket_opened`, built from
/// the engine's read-only state vector.
pub(crate) fn sync_step1_message(
    engine: &YrsDocumentEngine,
    request_id: u64,
) -> Result<Vec<u8>, SessionError> {
    let state_vector = engine.encode_state_vector_v1(request_id).map_err(|error| {
        SessionError::from_operation(error, OperationFailureClass::ExistingStableCode)
    })?;
    Ok(frame_sync_message(MSG_SYNC_STEP_1, &state_vector))
}

/// Standard y-protocols framing of one sync submessage:
/// `[MSG_SYNC, subtag, buf(payload)]`, byte-identical to `yrs::sync`
/// message encoding.
fn frame_sync_message(subtag: u8, payload: &[u8]) -> Vec<u8> {
    let mut encoder = EncoderV1::new();
    encoder.write_var(MSG_SYNC);
    encoder.write_var(subtag);
    encoder.write_buf(payload);
    encoder.to_vec()
}

/// Standard y-protocols framing of one awareness message:
/// `[MSG_AWARENESS, buf(update)]`, byte-identical to
/// `yrs::sync::Message::Awareness` encoding.
pub(crate) fn frame_awareness_message(update_v1: &[u8]) -> Vec<u8> {
    let mut encoder = EncoderV1::new();
    encoder.write_var(MSG_AWARENESS);
    encoder.write_buf(update_v1);
    encoder.to_vec()
}

/// Standard y-protocols framing of one document update:
/// `[MSG_SYNC, MSG_SYNC_UPDATE, buf(update)]`, byte-identical to
/// `yrs::sync::Message::Sync(SyncMessage::Update)` encoding. The outbox
/// stores raw update-v1 bytes (Task 7); wire frames are wrapped only at
/// pickup time so every outbound frame is a complete y-protocols message.
pub(crate) fn frame_sync_update_message(update_v1: &[u8]) -> Vec<u8> {
    frame_sync_message(MSG_SYNC_UPDATE, update_v1)
}

/// Strict bounded decode of one inbound transport message into protocol
/// frames: standard sync frames plus awareness (tag 1) and query-awareness
/// (tag 3). Anything else — truncation, trailing bytes, unknown
/// message/sync tags, auth/custom messages, or an empty message —
/// classifies as a protocol error. Frame payloads are kept raw; their
/// update/state-vector/awareness semantics belong to the engine.
fn decode_protocol_frames(
    request_id: u64,
    bytes: &[u8],
    max_frames_per_message: usize,
) -> Result<Vec<ProtocolFrame>, ReceiveFailure> {
    if bytes.is_empty() {
        return Err(protocol_failure(request_id, "emptyMessage"));
    }
    let mut cursor = Cursor::new(bytes);
    let mut frames = Vec::new();
    while cursor.has_content() {
        if frames.len() == max_frames_per_message {
            return Err(limit_failure(
                request_id,
                TRANSPORT_FRAME_LIMIT_EXCEEDED,
                MAX_FRAMES_PER_MESSAGE_FIELD,
                max_frames_per_message as u64,
                max_frames_per_message as u64 + 1,
            ));
        }
        let message_tag: u8 = cursor
            .read_var()
            .map_err(|_| protocol_failure(request_id, "messageTag"))?;
        frames.push(match message_tag {
            MSG_SYNC => {
                let sync_tag: u8 = cursor
                    .read_var()
                    .map_err(|_| protocol_failure(request_id, "syncTag"))?;
                let payload = cursor
                    .read_buf()
                    .map_err(|_| protocol_failure(request_id, "payload"))?
                    .to_vec();
                match sync_tag {
                    MSG_SYNC_STEP_1 => ProtocolFrame::SyncStep1(payload),
                    MSG_SYNC_STEP_2 => ProtocolFrame::SyncStep2(payload),
                    MSG_SYNC_UPDATE => ProtocolFrame::SyncUpdate(payload),
                    _ => return Err(protocol_failure(request_id, "unsupportedSyncType")),
                }
            }
            MSG_AWARENESS => {
                // The payload stays raw: whether it decodes as an
                // `AwarenessUpdate` is the codec's call (a failure there
                // classifies as the same protocol error).
                let payload = cursor
                    .read_buf()
                    .map_err(|_| protocol_failure(request_id, "awarenessPayload"))?
                    .to_vec();
                ProtocolFrame::Awareness(payload)
            }
            MSG_QUERY_AWARENESS => ProtocolFrame::AwarenessQuery,
            _ => return Err(protocol_failure(request_id, "unsupportedMessageType")),
        });
    }
    Ok(frames)
}

/// Failure-classification law for engine admission errors, keyed on the
/// frozen operation codes plus a bounded post-hoc encoding preflight: a
/// `DOCUMENT_INVALID` whose payload also fails the engine's structural
/// preflight is malformed encoding (protocol error); one whose payload is
/// well-formed is permanently inadmissible content.
fn classify_admission_code(
    code: &str,
    encoding_malformed: bool,
) -> (SocketCloseDisposition, &'static str) {
    match code {
        "OPERATION_RESOURCE_EXHAUSTED" => (
            SocketCloseDisposition::Retryable,
            TRANSPORT_RESOURCE_EXHAUSTED,
        ),
        "DOCUMENT_LIMIT_EXCEEDED" | "OPERATION_LIMIT_EXCEEDED" => (
            SocketCloseDisposition::Incompatible,
            TRANSPORT_REMOTE_INADMISSIBLE,
        ),
        "DOCUMENT_INVALID" if encoding_malformed => (
            SocketCloseDisposition::Retryable,
            TRANSPORT_PROTOCOL_INVALID,
        ),
        "DOCUMENT_INVALID" => (
            SocketCloseDisposition::Incompatible,
            TRANSPORT_REMOTE_INADMISSIBLE,
        ),
        _ => (
            SocketCloseDisposition::Retryable,
            TRANSPORT_REMOTE_APPLY_FAILED,
        ),
    }
}

fn classify_admission_error(
    engine: &YrsDocumentEngine,
    request_id: u64,
    update: &[u8],
    error: OperationError,
) -> ReceiveFailure {
    let encoding_malformed = error.code == "DOCUMENT_INVALID"
        && engine
            .preflight_remote_update_v1(request_id, update)
            .is_err();
    let (close, code) = classify_admission_code(error.code, encoding_malformed);
    ReceiveFailure {
        close,
        error: transport_error(
            request_id,
            code,
            "remote update admission failed",
            json!({ "action": RECEIVE_ACTION, "cause": operation_cause(&error) }),
        ),
    }
}

/// Failure-classification law for the sealed awareness codec, mirroring the
/// Task 9 table: malformed encoding (including non-JSON state payloads) is
/// a protocol error and closes retryably; the deterministic per-message
/// awareness ceilings close as incompatible; residual codec failures close
/// retryably like every other apply failure.
fn classify_awareness_code(code: &str) -> (SocketCloseDisposition, &'static str) {
    match code {
        AWARENESS_CLOCK_EXHAUSTED => (
            SocketCloseDisposition::Incompatible,
            AWARENESS_CLOCK_EXHAUSTED,
        ),
        "INPUT_LIMIT_EXCEEDED" => (
            SocketCloseDisposition::Incompatible,
            TRANSPORT_AWARENESS_LIMIT_EXCEEDED,
        ),
        "COLLABORATION_DECODE_FAILED" => (
            SocketCloseDisposition::Retryable,
            TRANSPORT_PROTOCOL_INVALID,
        ),
        _ => (
            SocketCloseDisposition::Retryable,
            TRANSPORT_REMOTE_APPLY_FAILED,
        ),
    }
}

fn classify_awareness_error(request_id: u64, error: YrsEngineError) -> ReceiveFailure {
    let (close, code) = classify_awareness_code(error.code);
    ReceiveFailure {
        close,
        error: transport_error(
            request_id,
            code,
            "awareness frame handling failed",
            json!({
                "action": RECEIVE_ACTION,
                "cause": {
                    "code": error.code,
                    "message": error.message,
                    "limit": error.limit,
                    "actual": error.actual,
                    "details": error.details,
                },
            }),
        ),
    }
}

/// Reply prebuild failures: a malformed remote state vector is a protocol
/// error; an engine byte-ceiling refusal is a deterministic reply limit.
fn classify_reply_build_error(request_id: u64, error: OperationError) -> ReceiveFailure {
    if error.code == "DOCUMENT_INVALID" {
        return ReceiveFailure {
            close: SocketCloseDisposition::Retryable,
            error: transport_error(
                request_id,
                TRANSPORT_PROTOCOL_INVALID,
                "Sync Step 1 carried a malformed remote state vector",
                json!({ "action": RECEIVE_ACTION, "cause": operation_cause(&error) }),
            ),
        };
    }
    let (close, code) = classify_admission_code(error.code, false);
    ReceiveFailure {
        close,
        error: transport_error(
            request_id,
            if code == TRANSPORT_REMOTE_INADMISSIBLE {
                TRANSPORT_REPLY_LIMIT_EXCEEDED
            } else {
                code
            },
            "Sync Step 1 reply could not be built",
            json!({ "action": RECEIVE_ACTION, "cause": operation_cause(&error) }),
        ),
    }
}

fn classify_reservation_error(request_id: u64, error: OutboxReservationError) -> ReceiveFailure {
    match error {
        // The reply queue shares the outbox ceilings with pending offline
        // document updates, which drain on delivery — retry CAN change the
        // result, so saturation closes retryably (unlike the deterministic
        // per-message ceilings). Otherwise an offline-full outbox would
        // wedge the transport in `Incompatible` across detach/reattach.
        OutboxReservationError::Saturated {
            field,
            limit,
            actual,
        } => ReceiveFailure {
            close: SocketCloseDisposition::Retryable,
            error: ceiling_error(
                request_id,
                TRANSPORT_REPLY_LIMIT_EXCEEDED,
                field,
                limit as u64,
                actual as u64,
            ),
        },
        OutboxReservationError::Allocation => ReceiveFailure {
            close: SocketCloseDisposition::Retryable,
            error: transport_error(
                request_id,
                TRANSPORT_RESOURCE_EXHAUSTED,
                "protocol reply capacity could not be reserved",
                json!({ "action": RECEIVE_ACTION, "reason": "replyReservation" }),
            ),
        },
    }
}

/// Deterministic configured-ceiling violation: retrying the same message
/// against the same configuration cannot change the result, so it closes
/// as incompatible.
fn limit_failure(
    request_id: u64,
    code: &'static str,
    field: &str,
    limit: u64,
    actual: u64,
) -> ReceiveFailure {
    ReceiveFailure {
        close: SocketCloseDisposition::Incompatible,
        error: ceiling_error(request_id, code, field, limit, actual),
    }
}

/// Structured configured-ceiling error: the charged field plus both
/// boundary values, in the details and on the envelope.
fn ceiling_error(
    request_id: u64,
    code: &'static str,
    field: &str,
    limit: u64,
    actual: u64,
) -> SessionError {
    let mut error = transport_error(
        request_id,
        code,
        format!("{field} exceeded while receiving a protocol message"),
        json!({
            "action": RECEIVE_ACTION,
            "field": field,
            "limit": limit,
            "actual": actual,
        }),
    );
    error.limit = Some(limit);
    error.actual = Some(actual);
    error
}

fn protocol_failure(request_id: u64, reason: &'static str) -> ReceiveFailure {
    ReceiveFailure {
        close: SocketCloseDisposition::Retryable,
        error: transport_error(
            request_id,
            TRANSPORT_PROTOCOL_INVALID,
            "inbound bytes are not a well-formed y-sync message",
            json!({ "action": RECEIVE_ACTION, "reason": reason }),
        ),
    }
}

fn transport_error(
    request_id: u64,
    code: &str,
    message: impl Into<String>,
    details: serde_json::Value,
) -> SessionError {
    let mut error = SessionError::new(ErrorDomain::Transport, code, message.into());
    error.request_id = Some(request_id);
    error.details = Some(details);
    error
}

/// The engine error as a structured cause payload.
fn operation_cause(error: &OperationError) -> serde_json::Value {
    serde_json::to_value(error).unwrap_or_else(|_| json!({ "code": error.code }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::boundary::ResourceLimits;
    use crate::collaboration_runtime::awareness::AwarenessContext;
    use crate::session::{CollaborationLimits, TransportState};
    use crate::yrs_engine::{
        EditingLimits, InitializationMode, YrsDocumentEngine, YrsEngineConfig,
    };
    use yrs::sync::awareness::AwarenessUpdate;
    use yrs::sync::{Message, SyncMessage};
    use yrs::updates::encoder::Encode;
    use yrs::StateVector;

    const REQUEST_ID: u64 = 77;

    fn yrs_frame(message: SyncMessage) -> Vec<u8> {
        Message::Sync(message).encode_v1()
    }

    fn empty_awareness_update() -> AwarenessUpdate {
        AwarenessUpdate {
            clients: std::collections::HashMap::new(),
        }
    }

    fn engine() -> YrsDocumentEngine {
        YrsDocumentEngine::new(YrsEngineConfig {
            schema: crate::schema::presets::tiptap_schema(),
            fragment_name: "prosemirror".into(),
            initialization_mode: InitializationMode::LocalEmpty,
            resource_limits: ResourceLimits::default(),
            editing_limits: EditingLimits::default(),
            max_length: None,
            scope: None,
        })
        .unwrap()
    }

    #[test]
    fn framing_is_byte_identical_to_yrs_sync_message_encoding() {
        let state_vector = StateVector::default().encode_v1();
        assert_eq!(
            frame_sync_message(MSG_SYNC_STEP_1, &state_vector),
            yrs_frame(SyncMessage::SyncStep1(StateVector::default())),
        );
        let payload = vec![0, 0];
        assert_eq!(
            frame_sync_message(MSG_SYNC_STEP_2, &payload),
            yrs_frame(SyncMessage::SyncStep2(payload.clone())),
        );
        assert_eq!(
            frame_sync_message(MSG_SYNC_UPDATE, &payload),
            yrs_frame(SyncMessage::Update(payload)),
        );
        assert_eq!(
            frame_awareness_message(&empty_awareness_update().encode_v1()),
            Message::Awareness(empty_awareness_update()).encode_v1(),
        );
    }

    #[test]
    fn decode_accepts_exact_multi_frame_messages_and_keeps_raw_payloads() {
        let state_vector = StateVector::default().encode_v1();
        let awareness_payload = empty_awareness_update().encode_v1();
        let message: Vec<u8> = [
            yrs_frame(SyncMessage::SyncStep1(StateVector::default())),
            yrs_frame(SyncMessage::SyncStep2(vec![0, 0])),
            yrs_frame(SyncMessage::Update(vec![1, 2, 3])),
            Message::Awareness(empty_awareness_update()).encode_v1(),
            Message::AwarenessQuery.encode_v1(),
        ]
        .concat();

        let frames = decode_protocol_frames(REQUEST_ID, &message, 5).unwrap();
        assert_eq!(
            frames,
            vec![
                ProtocolFrame::SyncStep1(state_vector),
                ProtocolFrame::SyncStep2(vec![0, 0]),
                ProtocolFrame::SyncUpdate(vec![1, 2, 3]),
                ProtocolFrame::Awareness(awareness_payload),
                ProtocolFrame::AwarenessQuery,
            ],
        );

        // One frame over the exact count boundary is a deterministic
        // frame-limit close, not a protocol error — awareness and
        // query-awareness frames count like every other frame.
        let failure = decode_protocol_frames(REQUEST_ID, &message, 4).unwrap_err();
        assert_eq!(failure.close, SocketCloseDisposition::Incompatible);
        assert_eq!(failure.error.code, TRANSPORT_FRAME_LIMIT_EXCEEDED);
        assert_eq!(
            failure.error.details.as_ref().unwrap()["field"],
            MAX_FRAMES_PER_MESSAGE_FIELD,
        );
    }

    #[test]
    fn decode_strictly_rejects_everything_that_is_not_a_protocol_frame() {
        // Task 10 extended the decoder to awareness (tag 1) and
        // query-awareness (tag 3); auth, custom, and malformed frames stay
        // protocol errors.
        let cases: [(&str, Vec<u8>); 8] = [
            ("empty", vec![]),
            ("truncated message tag", vec![0x80]),
            ("truncated sync tag", vec![0]),
            ("truncated payload", vec![0, 1, 5, 1, 2]),
            ("unknown sync tag", vec![0, 9, 0]),
            ("auth tag", vec![2, 1]),
            ("truncated awareness payload", vec![1, 5, 1, 2]),
            ("trailing byte", {
                let mut bytes = yrs_frame(SyncMessage::Update(vec![0, 0]));
                bytes.push(0xff);
                bytes
            }),
        ];
        for (label, bytes) in cases {
            let failure = decode_protocol_frames(REQUEST_ID, &bytes, 64)
                .expect_err(&format!("{label}: must reject"));
            assert_eq!(
                failure.close,
                SocketCloseDisposition::Retryable,
                "{label}: protocol errors close retryably",
            );
            assert_eq!(failure.error.code, TRANSPORT_PROTOCOL_INVALID, "{label}");
            assert_eq!(failure.error.request_id, Some(REQUEST_ID), "{label}");
        }
    }

    #[test]
    fn awareness_classification_splits_ceilings_from_malformed_encoding() {
        use SocketCloseDisposition::{Incompatible, Retryable};
        let cases = [
            (
                "INPUT_LIMIT_EXCEEDED",
                Incompatible,
                TRANSPORT_AWARENESS_LIMIT_EXCEEDED,
            ),
            (
                "COLLABORATION_DECODE_FAILED",
                Retryable,
                TRANSPORT_PROTOCOL_INVALID,
            ),
            (
                "COLLABORATION_APPLY_FAILED",
                Retryable,
                TRANSPORT_REMOTE_APPLY_FAILED,
            ),
            (
                "AWARENESS_CLOCK_EXHAUSTED",
                Incompatible,
                "AWARENESS_CLOCK_EXHAUSTED",
            ),
        ];
        for (engine_code, close, transport_code) in cases {
            assert_eq!(
                classify_awareness_code(engine_code),
                (close, transport_code),
                "{engine_code}",
            );
        }

        // The structured cause carries the codec's field details through to
        // the wire error.
        let failure = classify_awareness_error(
            REQUEST_ID,
            YrsEngineError::limit("INPUT_LIMIT_EXCEEDED", 2, 3)
                .with_details(json!({ "field": "maxAwarenessPeers" })),
        );
        assert_eq!(failure.close, Incompatible);
        assert_eq!(failure.error.code, TRANSPORT_AWARENESS_LIMIT_EXCEEDED);
        let details = failure.error.details.as_ref().unwrap();
        assert_eq!(details["cause"]["details"]["field"], "maxAwarenessPeers");
        assert_eq!(details["cause"]["limit"], 2);
        assert_eq!(details["cause"]["actual"], 3);

        let limits = CollaborationLimits::default();
        let mut engine = engine();
        let mut runtime = CollaborationRuntime::new(&limits);
        runtime
            .set_desired_awareness(
                REQUEST_ID,
                r#"{"name":"before"}"#,
                AwarenessContext {
                    engine: &mut engine,
                    transport_state: TransportState::Disconnected,
                    limits: &limits,
                },
            )
            .unwrap();
        engine
            .awareness()
            .set_live_local_clock_for_test(u32::MAX - 1);
        let production_error = runtime
            .prepare_handshake_republish(&mut engine, &limits)
            .unwrap_err();
        let failure = classify_awareness_error(REQUEST_ID, production_error);
        assert_eq!(failure.close, Incompatible);
        assert_eq!(failure.error.code, "AWARENESS_CLOCK_EXHAUSTED");
        assert_eq!(failure.error.domain, ErrorDomain::Transport);
        assert_eq!(failure.error.request_id, Some(REQUEST_ID));
        assert_eq!(
            failure.error.details.as_ref().unwrap()["cause"]["details"]
                ["requiresFreshEditorIdentity"],
            true,
        );
        assert_eq!(
            failure.error.details.as_ref().unwrap()["cause"]["details"]["retryable"],
            false,
        );
    }

    #[test]
    fn admission_classification_covers_every_engine_code_class() {
        use SocketCloseDisposition::{Incompatible, Retryable};
        let cases = [
            (
                "OPERATION_RESOURCE_EXHAUSTED",
                false,
                Retryable,
                TRANSPORT_RESOURCE_EXHAUSTED,
            ),
            (
                "DOCUMENT_LIMIT_EXCEEDED",
                false,
                Incompatible,
                TRANSPORT_REMOTE_INADMISSIBLE,
            ),
            (
                "OPERATION_LIMIT_EXCEEDED",
                false,
                Incompatible,
                TRANSPORT_REMOTE_INADMISSIBLE,
            ),
            (
                "DOCUMENT_INVALID",
                true,
                Retryable,
                TRANSPORT_PROTOCOL_INVALID,
            ),
            (
                "DOCUMENT_INVALID",
                false,
                Incompatible,
                TRANSPORT_REMOTE_INADMISSIBLE,
            ),
            // Residual class: defensive invariants and derived-state
            // failures close retryably with their own code.
            (
                "ENGINE_INVARIANT_FAILED",
                false,
                Retryable,
                TRANSPORT_REMOTE_APPLY_FAILED,
            ),
            (
                "POSITION_INVALID",
                false,
                Retryable,
                TRANSPORT_REMOTE_APPLY_FAILED,
            ),
        ];
        for (engine_code, malformed, close, transport_code) in cases {
            assert_eq!(
                classify_admission_code(engine_code, malformed),
                (close, transport_code),
                "{engine_code} malformed={malformed}",
            );
        }
    }
}
