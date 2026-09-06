//! Generation-owned transport transitions.
//!
//! Covers the design's required-transition table as an exhaustive
//! document-state x transport-state x action matrix (including destroy rows
//! and pre-acquired handles), generation discipline (monotonic increments,
//! stale callbacks as observable no-ops), Rust-owned retry disposition with
//! the `Incompatible` detach/reattach escape hatch, the replacement policy
//! gate driven by real transitions, and outbox preservation across
//! transport teardown.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use crate::boundary::ResourceLimits;
use crate::collaboration_runtime::state::{
    TRANSPORT_INVALID_TRANSITION, TRANSPORT_NOT_ROOM_BOUND, TRANSPORT_STALE_GENERATION,
};
use crate::native_bridge_test_support as bridge;
use crate::session_initialization_test_support::{
    ack_outbound, collaboration_drive, collaboration_receive, collaboration_socket_close,
    collaboration_socket_open, create_local_json, create_room_from_json, destroy_session,
    lease_outbound, mark_synchronized_for_test, nack_outbound, session_audit,
    set_transport_state_for_test, transport_detach, transport_disconnect, transport_handle,
    transport_reattach, transport_state, write_json, CloseDisposition, DocumentState, TestError,
    TransportHandle, TransportState,
};
use crate::tiptap_schema;
use crate::yrs_engine::{
    DocumentScope, EditingLimits, InitializationMode, ReplacementHistory, TransactionOrigin,
    YrsDocumentEngine, YrsEngineConfig,
};
use yrs::sync::{Message, SyncMessage};
use yrs::updates::encoder::Encode;

const JSON_SEED: &str = r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"transport seed"}]}]}"#;
const JSON_REPLACEMENT_A: &str = r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"replacement a"}]}]}"#;
const JSON_REPLACEMENT_B: &str = r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"replacement b"}]}]}"#;

/// Generation value no session ever issues (attempts count up from one).
const FABRICATED_GENERATION: u64 = 424_242;
/// Request id used while driving a cell to its starting state.
const SETUP_REQUEST_ID: u64 = 9_000;
/// Request id used for the action under test; refusals must echo it.
const ACTION_REQUEST_ID: u64 = 9_100;
const WAIT_TIMEOUT: Duration = Duration::from_secs(10);

static LEGACY_TEST_NOW_MILLIS: AtomicU64 = AtomicU64::new(0);

/// The older state-matrix scenarios do not otherwise own a deterministic
/// clock. Give every production-shaped callback a strictly increasing test
/// timestamp so retry drives happen after their returned deadlines.
fn next_legacy_test_now_millis() -> u64 {
    LEGACY_TEST_NOW_MILLIS.fetch_add(100_000, Ordering::Relaxed) + 100_000
}

/// Issue a current generation only through Rust's drive directive.
fn drive_generation(id: u64, request_id: u64) -> Result<u64, TestError> {
    collaboration_drive(id, request_id, next_legacy_test_now_millis()).and_then(|directive| {
        directive.generation_to_open.ok_or_else(|| TestError {
            domain: "transport",
            code: "TEST_EXPECTED_GENERATION".into(),
            request_id: Some(request_id),
            details: None,
        })
    })
}

/// Socket-open setup enters the production queue and explicitly ACKs Sync
/// Step 1 where the higher-level scenario needs a drained outbox.
fn open_socket_and_ack_step1(id: u64, request_id: u64, generation: u64) -> Result<(), TestError> {
    collaboration_socket_open(id, request_id, generation, next_legacy_test_now_millis())?;
    let step1 = lease_outbound(id, request_id, generation)?.ok_or_else(|| TestError {
        domain: "transport",
        code: "TEST_EXPECTED_STEP1".into(),
        request_id: Some(request_id),
        details: None,
    })?;
    ack_outbound(id, request_id, generation, step1.lease_id)
}

/// Report a close through the production callback and retain its state-only
/// shape for existing transition assertions.
fn close_socket(
    id: u64,
    request_id: u64,
    generation: u64,
    disposition: CloseDisposition,
) -> Result<TransportState, TestError> {
    collaboration_socket_close(
        id,
        request_id,
        generation,
        disposition,
        next_legacy_test_now_millis(),
    )
    .map(|directive| directive.transport_state)
}

/// The six transport states a live (non-destroyed) session can hold. The
/// `Destroying`/`Destroyed` rows of the design table are exercised through
/// the destroy action and pre-acquired handles: `with_alive` refuses them
/// with the frozen lifecycle codes before any transport logic runs.
const LIVE_TRANSPORT_STATES: [TransportState; 6] = [
    TransportState::Detached,
    TransportState::Disconnected,
    TransportState::Connecting,
    TransportState::Handshaking,
    TransportState::Synchronized,
    TransportState::Incompatible,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Doc {
    Local,
    AwaitRemote,
    RoomReady,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    Drive,
    Reattach,
    SocketOpenedCurrent,
    SocketOpenedStale,
    SocketClosedCurrentRetryable,
    SocketClosedCurrentIncompatible,
    SocketClosedStaleRetryable,
    SocketClosedStaleIncompatible,
    Disconnect,
    Detach,
    MarkSynchronizedCurrent,
    MarkSynchronizedStale,
    Destroy,
}

const ALL_ACTIONS: [Action; 13] = [
    Action::Drive,
    Action::Reattach,
    Action::SocketOpenedCurrent,
    Action::SocketOpenedStale,
    Action::SocketClosedCurrentRetryable,
    Action::SocketClosedCurrentIncompatible,
    Action::SocketClosedStaleRetryable,
    Action::SocketClosedStaleIncompatible,
    Action::Disconnect,
    Action::Detach,
    Action::MarkSynchronizedCurrent,
    Action::MarkSynchronizedStale,
    Action::Destroy,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Expected {
    /// Transition accepted; only the transport state may change, to this.
    Accepted(TransportState),
    /// Structured transport-domain refusal with this code; nothing changes.
    Refused(&'static str),
}

fn snapshot_source() -> crate::yrs_engine::DocumentSnapshot {
    let mut source = YrsDocumentEngine::new(YrsEngineConfig {
        schema: tiptap_schema(),
        fragment_name: "prosemirror".into(),
        initialization_mode: InitializationMode::LocalEmpty,
        resource_limits: ResourceLimits::default(),
        editing_limits: EditingLimits::default(),
        max_length: None,
        scope: Some(DocumentScope {
            document_id: "transport-room".into(),
            lineage_id: "transport-lineage".into(),
        }),
    })
    .unwrap();
    source
        .import_json(JSON_SEED, TransactionOrigin::DocumentImport)
        .unwrap();
    source.export_snapshot().unwrap()
}

fn create_ready_room() -> u64 {
    let snapshot = snapshot_source();
    let config = serde_json::json!({
        "documentId": snapshot.document_id,
        "lineageId": snapshot.lineage_id,
        "snapshot": snapshot,
    });
    create_room_from_json(&config.to_string()).unwrap()
}

fn create_ready_room_with_runtime() -> u64 {
    let id = create_ready_room();
    bridge::attach_runtime(id).unwrap();
    id
}

fn create_await_remote_room() -> u64 {
    create_room_from_json(r#"{"documentId":"transport-room","lineageId":"transport-lineage"}"#)
        .unwrap()
}

fn create_for(doc: Doc) -> u64 {
    match doc {
        Doc::Local => create_local_json(JSON_SEED).unwrap(),
        Doc::AwaitRemote => {
            let id = create_await_remote_room();
            bridge::attach_runtime(id).unwrap();
            id
        }
        Doc::RoomReady => create_ready_room_with_runtime(),
    }
}

/// Drive a freshly created session to the cell's starting transport state.
/// Room-bound rows use only real transitions; `LocalReady` rows other than
/// the constructed `Detached` are unreachable by construction and are forced
/// through the documented test hook (they carry no live attempt). Returns
/// the latest generation issued during setup, if any.
fn drive_transport(id: u64, doc: Doc, target: TransportState) -> Option<u64> {
    if doc == Doc::Local {
        assert_eq!(transport_state(id).unwrap(), TransportState::Detached);
        if target != TransportState::Detached {
            set_transport_state_for_test(id, target).unwrap();
        }
        return None;
    }
    assert_eq!(transport_state(id).unwrap(), TransportState::Disconnected);
    match target {
        TransportState::Disconnected => None,
        TransportState::Detached => {
            transport_detach(id, SETUP_REQUEST_ID).unwrap();
            None
        }
        TransportState::Connecting => Some(drive_generation(id, SETUP_REQUEST_ID).unwrap()),
        TransportState::Handshaking => {
            let generation = drive_generation(id, SETUP_REQUEST_ID).unwrap();
            open_socket_and_ack_step1(id, SETUP_REQUEST_ID, generation).unwrap();
            Some(generation)
        }
        TransportState::Synchronized => {
            let generation = drive_generation(id, SETUP_REQUEST_ID).unwrap();
            open_socket_and_ack_step1(id, SETUP_REQUEST_ID, generation).unwrap();
            mark_synchronized_for_test(id, SETUP_REQUEST_ID, generation).unwrap();
            Some(generation)
        }
        TransportState::Incompatible => {
            let generation = drive_generation(id, SETUP_REQUEST_ID).unwrap();
            assert_eq!(
                close_socket(
                    id,
                    SETUP_REQUEST_ID,
                    generation,
                    CloseDisposition::Incompatible
                )
                .unwrap(),
                TransportState::Incompatible,
            );
            Some(generation)
        }
    }
}

/// The complete expected-outcome table: one explicit cell for every
/// document-state x live-transport-state x action combination.
fn expected(doc: Doc, state: TransportState, action: Action) -> Expected {
    use TransportState::{
        Connecting, Detached, Disconnected, Handshaking, Incompatible, Synchronized,
    };
    let active = matches!(state, Connecting | Handshaking | Synchronized);
    // LocalReady rows are hook-forced without a live attempt, so every
    // generation-carrying callback on them is stale by definition.
    let live_attempt = doc != Doc::Local && active;
    match action {
        Action::Drive => match (doc, state) {
            (Doc::RoomReady | Doc::AwaitRemote, Disconnected) => Expected::Accepted(Connecting),
            _ => Expected::Accepted(state),
        },
        Action::Reattach => match (doc, state) {
            (Doc::Local, _) => Expected::Refused(TRANSPORT_NOT_ROOM_BOUND),
            (_, Detached | Disconnected) => Expected::Accepted(Disconnected),
            _ => Expected::Refused(TRANSPORT_INVALID_TRANSITION),
        },
        Action::SocketOpenedCurrent => {
            if !live_attempt {
                Expected::Refused(TRANSPORT_STALE_GENERATION)
            } else if state == Connecting {
                Expected::Accepted(Handshaking)
            } else {
                Expected::Refused(TRANSPORT_INVALID_TRANSITION)
            }
        }
        Action::SocketOpenedStale => Expected::Refused(TRANSPORT_STALE_GENERATION),
        Action::SocketClosedCurrentRetryable => {
            if live_attempt {
                Expected::Accepted(Disconnected)
            } else {
                Expected::Refused(TRANSPORT_STALE_GENERATION)
            }
        }
        Action::SocketClosedCurrentIncompatible => {
            if live_attempt {
                Expected::Accepted(Incompatible)
            } else {
                Expected::Refused(TRANSPORT_STALE_GENERATION)
            }
        }
        Action::SocketClosedStaleRetryable | Action::SocketClosedStaleIncompatible => {
            Expected::Refused(TRANSPORT_STALE_GENERATION)
        }
        Action::Disconnect => {
            if active {
                Expected::Accepted(Disconnected)
            } else {
                Expected::Refused(TRANSPORT_INVALID_TRANSITION)
            }
        }
        Action::Detach => Expected::Accepted(Detached),
        Action::MarkSynchronizedCurrent => {
            if !live_attempt {
                Expected::Refused(TRANSPORT_STALE_GENERATION)
            } else if state == Handshaking {
                Expected::Accepted(Synchronized)
            } else {
                Expected::Refused(TRANSPORT_INVALID_TRANSITION)
            }
        }
        Action::MarkSynchronizedStale => Expected::Refused(TRANSPORT_STALE_GENERATION),
        Action::Destroy => unreachable!("destroy cells are asserted separately"),
    }
}

/// The action name every refusal's structured details must echo.
fn wire_action(action: Action) -> &'static str {
    match action {
        Action::Drive => "collaborationDrive",
        Action::Reattach => "reattach",
        Action::SocketOpenedCurrent | Action::SocketOpenedStale => "socketOpened",
        Action::SocketClosedCurrentRetryable
        | Action::SocketClosedCurrentIncompatible
        | Action::SocketClosedStaleRetryable
        | Action::SocketClosedStaleIncompatible => "socketClosed",
        Action::Disconnect => "disconnect",
        Action::Detach => "detach",
        Action::MarkSynchronizedCurrent | Action::MarkSynchronizedStale => "markSynchronized",
        Action::Destroy => unreachable!("destroy cells are asserted separately"),
    }
}

fn run_action(id: u64, action: Action, current: u64) -> Result<(), TestError> {
    match action {
        Action::Drive => {
            collaboration_drive(id, ACTION_REQUEST_ID, next_legacy_test_now_millis()).map(|_| ())
        }
        Action::Reattach => transport_reattach(id, ACTION_REQUEST_ID),
        Action::SocketOpenedCurrent => open_socket_and_ack_step1(id, ACTION_REQUEST_ID, current),
        Action::SocketOpenedStale => {
            open_socket_and_ack_step1(id, ACTION_REQUEST_ID, FABRICATED_GENERATION).map(|_| ())
        }
        Action::SocketClosedCurrentRetryable => {
            close_socket(id, ACTION_REQUEST_ID, current, CloseDisposition::Retryable).map(|_| ())
        }
        Action::SocketClosedCurrentIncompatible => close_socket(
            id,
            ACTION_REQUEST_ID,
            current,
            CloseDisposition::Incompatible,
        )
        .map(|_| ()),
        Action::SocketClosedStaleRetryable => close_socket(
            id,
            ACTION_REQUEST_ID,
            FABRICATED_GENERATION,
            CloseDisposition::Retryable,
        )
        .map(|_| ()),
        Action::SocketClosedStaleIncompatible => close_socket(
            id,
            ACTION_REQUEST_ID,
            FABRICATED_GENERATION,
            CloseDisposition::Incompatible,
        )
        .map(|_| ()),
        Action::Disconnect => transport_disconnect(id, ACTION_REQUEST_ID),
        Action::Detach => transport_detach(id, ACTION_REQUEST_ID),
        Action::MarkSynchronizedCurrent => {
            mark_synchronized_for_test(id, ACTION_REQUEST_ID, current)
        }
        Action::MarkSynchronizedStale => {
            mark_synchronized_for_test(id, ACTION_REQUEST_ID, FABRICATED_GENERATION)
        }
        Action::Destroy => unreachable!("destroy cells are asserted separately"),
    }
}

fn assert_destroy_cell(id: u64, current: u64, label: &str) {
    destroy_session(id);
    let refusals: [(&str, Result<(), TestError>); 7] = [
        (
            "collaborationDrive",
            collaboration_drive(id, ACTION_REQUEST_ID, next_legacy_test_now_millis()).map(|_| ()),
        ),
        (
            "collaborationSocketOpen",
            collaboration_socket_open(
                id,
                ACTION_REQUEST_ID,
                current,
                next_legacy_test_now_millis(),
            )
            .map(|_| ()),
        ),
        (
            "collaborationSocketClose",
            collaboration_socket_close(
                id,
                ACTION_REQUEST_ID,
                current,
                CloseDisposition::Retryable,
                next_legacy_test_now_millis(),
            )
            .map(|_| ()),
        ),
        ("disconnect", transport_disconnect(id, ACTION_REQUEST_ID)),
        ("detach", transport_detach(id, ACTION_REQUEST_ID)),
        ("reattach", transport_reattach(id, ACTION_REQUEST_ID)),
        (
            "markSynchronized",
            mark_synchronized_for_test(id, ACTION_REQUEST_ID, current),
        ),
    ];
    for (op, result) in refusals {
        let error = result.expect_err(&format!("{label} {op}: destroyed session must refuse"));
        assert_eq!(error.domain, "lifecycle", "{label} {op}: {error:?}");
        assert_eq!(error.code, "ENGINE_DESTROYED", "{label} {op}: {error:?}");
    }
}

fn run_matrix(doc: Doc) {
    for state in LIVE_TRANSPORT_STATES {
        for action in ALL_ACTIONS {
            let label = format!("{doc:?} + {state:?} + {action:?}");
            let id = create_for(doc);
            let latest = drive_transport(id, doc, state);
            assert_eq!(transport_state(id).unwrap(), state, "{label}: setup");
            let current = latest.unwrap_or(FABRICATED_GENERATION);

            if action == Action::Destroy {
                assert_destroy_cell(id, current, &label);
                continue;
            }

            let before = session_audit(id).unwrap();
            let before_engine = bridge::session_audit(id).unwrap();
            match expected(doc, state, action) {
                Expected::Accepted(new_state) => {
                    run_action(id, action, current).unwrap_or_else(|error| {
                        panic!("{label}: expected acceptance, got {error:?}")
                    });
                    assert_eq!(transport_state(id).unwrap(), new_state, "{label}");
                    let mut want = before.clone();
                    want.transport_state = new_state;
                    assert_eq!(
                        session_audit(id).unwrap(),
                        want,
                        "{label}: an accepted transition may change nothing but transport state",
                    );
                    assert_eq!(
                        bridge::session_audit(id).unwrap(),
                        before_engine,
                        "{label}: engine/outbox audit must be untouched",
                    );
                }
                Expected::Refused(code) => {
                    let error = run_action(id, action, current)
                        .expect_err(&format!("{label}: expected refusal {code}"));
                    assert_eq!(error.domain, "transport", "{label}: {error:?}");
                    assert_eq!(error.code, code, "{label}: {error:?}");
                    assert_eq!(
                        error.request_id,
                        Some(ACTION_REQUEST_ID),
                        "{label}: {error:?}"
                    );
                    // Structured details always report the refused action
                    // and the machine's ACTUAL state (never a hardcoded one).
                    let details = error
                        .details
                        .as_ref()
                        .unwrap_or_else(|| panic!("{label}: refusal must carry details"));
                    assert_eq!(details["action"], wire_action(action), "{label}");
                    assert_eq!(details["transportState"], format!("{state:?}"), "{label}");
                    assert_eq!(
                        session_audit(id).unwrap(),
                        before,
                        "{label}: a refusal must leave the full session audit untouched",
                    );
                    assert_eq!(
                        bridge::session_audit(id).unwrap(),
                        before_engine,
                        "{label}: a refusal must leave the engine/outbox audit untouched",
                    );
                }
            }
            destroy_session(id);
        }
    }
}

#[test]
fn task8_lifecycle_contract_local_ready_matrix_has_an_explicit_outcome_for_every_cell() {
    run_matrix(Doc::Local);
}

#[test]
fn task8_lifecycle_contract_await_remote_matrix_has_an_explicit_outcome_for_every_cell() {
    run_matrix(Doc::AwaitRemote);
}

#[test]
fn task8_lifecycle_contract_room_ready_matrix_has_an_explicit_outcome_for_every_cell() {
    run_matrix(Doc::RoomReady);
}

include!("collaboration_transport_state_test/transitions.rs");

include!("collaboration_transport_state_test/retry_and_leases.rs");
