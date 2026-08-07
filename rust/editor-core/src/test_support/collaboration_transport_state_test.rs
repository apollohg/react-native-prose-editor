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

#[test]
fn generations_increment_exactly_once_per_accepted_connect() {
    let id = create_ready_room();

    let first = drive_generation(id, 101).unwrap();
    // A drive while an attempt is live reports no generation and does not
    // consume one.
    let parked = collaboration_drive(id, 102, next_legacy_test_now_millis()).unwrap();
    assert_eq!(parked.generation_to_open, None);
    assert_eq!(
        close_socket(id, 103, first, CloseDisposition::Retryable).unwrap(),
        TransportState::Disconnected,
    );

    let second = drive_generation(id, 104).unwrap();
    assert_eq!(
        second,
        first + 1,
        "an accepted connect increments the generation exactly once",
    );
    transport_disconnect(id, 105).unwrap();
    transport_detach(id, 105).unwrap();
    transport_reattach(id, 105).unwrap();

    let third = drive_generation(id, 106).unwrap();
    assert_eq!(
        third,
        second + 1,
        "parked drives must never burn a generation"
    );

    destroy_session(id);
}

#[test]
fn stale_generations_can_neither_advance_nor_regress_state() {
    let id = create_ready_room();
    bridge::attach_runtime(id).unwrap();

    // Open and retire a first generation, then start a second attempt.
    let first = drive_generation(id, 111).unwrap();
    close_socket(id, 112, first, CloseDisposition::Retryable).unwrap();
    let second = drive_generation(id, 113).unwrap();
    let audit = session_audit(id).unwrap();
    assert_eq!(audit.transport_state, TransportState::Connecting);

    // The retired generation is refused for every callback, observably, and
    // both the state and the live generation stay untouched.
    for (request_id, result) in [
        (114, open_socket_and_ack_step1(id, 114, first)),
        (
            115,
            close_socket(id, 115, first, CloseDisposition::Retryable).map(|_| ()),
        ),
        (
            116,
            close_socket(id, 116, first, CloseDisposition::Incompatible).map(|_| ()),
        ),
        (117, mark_synchronized_for_test(id, 117, first)),
    ] {
        let error = result.expect_err("stale generation must be refused");
        assert_eq!(error.domain, "transport", "{error:?}");
        assert_eq!(error.code, TRANSPORT_STALE_GENERATION, "{error:?}");
        assert_eq!(error.request_id, Some(request_id), "{error:?}");
        assert_eq!(session_audit(id).unwrap(), audit);
    }

    // The live generation still works: the attempt was not poisoned.
    open_socket_and_ack_step1(id, 118, second).unwrap();
    assert_eq!(transport_state(id).unwrap(), TransportState::Handshaking);

    // Local disconnect retires the live generation: its late close callback
    // is stale and cannot resurrect or regress the transport.
    transport_disconnect(id, 119).unwrap();
    let error = close_socket(id, 120, second, CloseDisposition::Incompatible).unwrap_err();
    assert_eq!(error.code, TRANSPORT_STALE_GENERATION, "{error:?}");
    assert_eq!(transport_state(id).unwrap(), TransportState::Disconnected);
    // A lifecycle reset makes a fresh Rust drive eligible after a local disconnect.
    transport_detach(id, 121).unwrap();
    transport_reattach(id, 121).unwrap();
    drive_generation(id, 121).unwrap();

    destroy_session(id);
}

#[test]
fn incompatible_ignores_repeated_drives_until_detach_and_reattach() {
    let id = create_ready_room();
    bridge::attach_runtime(id).unwrap();

    let generation = drive_generation(id, 131).unwrap();
    open_socket_and_ack_step1(id, 132, generation).unwrap();
    assert_eq!(
        close_socket(id, 133, generation, CloseDisposition::Incompatible).unwrap(),
        TransportState::Incompatible,
    );

    // Repeated native calls to Rust's drive cannot force a reconnect out of
    // deterministic incompatibility.
    for request_id in [134, 135, 136] {
        let parked = collaboration_drive(id, request_id, next_legacy_test_now_millis()).unwrap();
        assert_eq!(parked.transport_state, TransportState::Incompatible);
        assert_eq!(parked.generation_to_open, None);
        assert_eq!(parked.next_deadline_millis, None);
    }

    // The only escape hatch is the explicit detach/reattach cycle.
    transport_detach(id, 137).unwrap();
    assert_eq!(transport_state(id).unwrap(), TransportState::Detached);
    transport_reattach(id, 138).unwrap();
    assert_eq!(transport_state(id).unwrap(), TransportState::Disconnected);
    let next = drive_generation(id, 139).unwrap();
    assert_eq!(next, generation + 1);

    destroy_session(id);
}

#[test]
fn replacement_policy_gate_follows_real_transport_transitions() {
    let id = create_ready_room();
    bridge::attach_runtime(id).unwrap();

    let assert_connected_refusal = |request_id: u64, json: &str| {
        let error =
            write_json(id, request_id, json, ReplacementHistory::UndoableBoundary).unwrap_err();
        assert_eq!(error.domain, "lifecycle", "{error:?}");
        assert_eq!(
            error.code, "WHOLE_DOCUMENT_REPLACEMENT_CONNECTED",
            "{error:?}"
        );
        assert_eq!(error.request_id, Some(request_id), "{error:?}");
    };

    // Disconnected: allowed.
    assert!(
        write_json(
            id,
            141,
            JSON_REPLACEMENT_A,
            ReplacementHistory::UndoableBoundary
        )
        .unwrap()
        .changed
    );

    // Connecting, Handshaking, Synchronized via real transitions: refused.
    let generation = drive_generation(id, 142).unwrap();
    assert_connected_refusal(143, JSON_REPLACEMENT_B);
    open_socket_and_ack_step1(id, 144, generation).unwrap();
    assert_connected_refusal(145, JSON_REPLACEMENT_B);
    mark_synchronized_for_test(id, 146, generation).unwrap();
    assert_connected_refusal(147, JSON_REPLACEMENT_B);

    // Remote close back to Disconnected: allowed again.
    assert_eq!(
        close_socket(id, 148, generation, CloseDisposition::Retryable).unwrap(),
        TransportState::Disconnected,
    );
    assert!(
        write_json(
            id,
            149,
            JSON_REPLACEMENT_B,
            ReplacementHistory::UndoableBoundary
        )
        .unwrap()
        .changed
    );

    // Local disconnect also reopens the gate.
    let generation = drive_generation(id, 150).unwrap();
    assert_connected_refusal(151, JSON_REPLACEMENT_A);
    transport_disconnect(id, 152).unwrap();
    assert!(
        write_json(
            id,
            153,
            JSON_REPLACEMENT_A,
            ReplacementHistory::UndoableBoundary
        )
        .unwrap()
        .changed
    );
    let _ = generation;

    // Incompatible is an allowed replacement row of the matrix.
    transport_detach(id, 154).unwrap();
    transport_reattach(id, 154).unwrap();
    let generation = drive_generation(id, 154).unwrap();
    close_socket(id, 155, generation, CloseDisposition::Incompatible).unwrap();
    assert!(
        write_json(
            id,
            156,
            JSON_REPLACEMENT_B,
            ReplacementHistory::UndoableBoundary
        )
        .unwrap()
        .changed
    );

    destroy_session(id);
}

#[test]
fn transport_teardown_never_drops_pending_outbox_entries() {
    let id = create_ready_room();
    bridge::attach_runtime(id).unwrap();

    // Enqueue one real captured local edit.
    let revision = bridge::session_audit(id).unwrap().document_revision;
    let envelope = serde_json::json!({
        "version": 1,
        "requestId": "161",
        "baseDocumentRevision": revision.to_string(),
        "text": "offline edit",
    })
    .to_string();
    bridge::submit_input(id, &envelope).unwrap();
    let pending = bridge::outbox_pending(id).unwrap().unwrap();
    assert_eq!(
        pending.0, 1,
        "the edit must be captured before transport work"
    );

    // Close (retryable), close (incompatible), disconnect, and detach all
    // leave the pending offline entry in place for post-reconnect delivery.
    let generation = drive_generation(id, 162).unwrap();
    close_socket(id, 163, generation, CloseDisposition::Retryable).unwrap();
    assert_eq!(bridge::outbox_pending(id).unwrap().unwrap(), pending);

    let generation = drive_generation(id, 164).unwrap();
    close_socket(id, 165, generation, CloseDisposition::Incompatible).unwrap();
    assert_eq!(bridge::outbox_pending(id).unwrap().unwrap(), pending);

    transport_detach(id, 166).unwrap();
    assert_eq!(bridge::outbox_pending(id).unwrap().unwrap(), pending);

    transport_reattach(id, 167).unwrap();
    let generation = drive_generation(id, 168).unwrap();
    open_socket_and_ack_step1(id, 169, generation).unwrap();
    transport_disconnect(id, 170).unwrap();
    assert_eq!(bridge::outbox_pending(id).unwrap().unwrap(), pending);

    // The entry is still deliverable.
    let lease = bridge::lease_next_update(id).unwrap().unwrap();
    assert_eq!(lease.request_id, 161);
    assert!(!lease.update_v1.is_empty());
    bridge::ack_leased_update(id, lease.lease_id).unwrap();

    bridge::destroy_session(id);
}

/// Extension of the teardown matrix rows above: the same transitions
/// that preserve the pending outbox entry clear the transport-scoped
/// awareness peers while the desired local awareness survives every
/// one of them.
#[test]
fn transport_teardown_clears_awareness_peers_and_retains_desired_awareness() {
    use crate::session_initialization_test_support::{
        awareness_peers, desired_awareness, receive_message,
        set_desired_awareness_for_test as set_desired_awareness,
    };
    use yrs::updates::encoder::Encode as _;

    let id = create_ready_room();
    bridge::attach_runtime(id).unwrap();

    // One pending offline edit proves outbox survival stays untouched by
    // the awareness lifecycle.
    let revision = bridge::session_audit(id).unwrap().document_revision;
    let envelope = serde_json::json!({
        "version": 1,
        "requestId": "171",
        "baseDocumentRevision": revision.to_string(),
        "text": "offline edit",
    })
    .to_string();
    bridge::submit_input(id, &envelope).unwrap();
    let pending = bridge::outbox_pending(id).unwrap().unwrap();

    let desired = serde_json::json!({ "name": "survivor" });
    set_desired_awareness(id, 172, &desired.to_string()).unwrap();

    let awareness_frame = |client: u64, clock: u32| -> Vec<u8> {
        let mut clients = std::collections::HashMap::new();
        clients.insert(
            yrs::ClientID::new(client),
            yrs::sync::awareness::AwarenessUpdateEntry {
                clock,
                json: r#"{"name":"transient"}"#.into(),
            },
        );
        yrs::sync::Message::Awareness(yrs::sync::awareness::AwarenessUpdate { clients }).encode_v1()
    };
    let remote_peer_count = |id: u64| {
        awareness_peers(id)
            .unwrap()
            .iter()
            .filter(|peer| !peer.is_local)
            .count()
    };

    // Retryable close.
    let generation = drive_generation(id, 173).unwrap();
    open_socket_and_ack_step1(id, 174, generation).unwrap();
    receive_message(id, 175, generation, &awareness_frame(9_501, 1)).unwrap();
    assert_eq!(remote_peer_count(id), 1);
    close_socket(id, 176, generation, CloseDisposition::Retryable).unwrap();
    assert_eq!(remote_peer_count(id), 0, "retryable close clears peers");
    assert_eq!(desired_awareness(id).unwrap(), Some(desired.clone()));
    assert_eq!(bridge::outbox_pending(id).unwrap().unwrap(), pending);

    // Incompatible close.
    let generation = drive_generation(id, 177).unwrap();
    open_socket_and_ack_step1(id, 178, generation).unwrap();
    receive_message(id, 179, generation, &awareness_frame(9_502, 1)).unwrap();
    assert_eq!(remote_peer_count(id), 1);
    close_socket(id, 180, generation, CloseDisposition::Incompatible).unwrap();
    assert_eq!(remote_peer_count(id), 0, "incompatible close clears peers");
    assert_eq!(desired_awareness(id).unwrap(), Some(desired.clone()));

    // Detach/reattach escape hatch, then a local disconnect.
    transport_detach(id, 181).unwrap();
    transport_reattach(id, 182).unwrap();
    let generation = drive_generation(id, 183).unwrap();
    open_socket_and_ack_step1(id, 184, generation).unwrap();
    receive_message(id, 185, generation, &awareness_frame(9_503, 1)).unwrap();
    assert_eq!(remote_peer_count(id), 1);
    transport_disconnect(id, 186).unwrap();
    assert_eq!(remote_peer_count(id), 0, "local disconnect clears peers");
    assert_eq!(desired_awareness(id).unwrap(), Some(desired));
    assert_eq!(bridge::outbox_pending(id).unwrap().unwrap(), pending);

    bridge::destroy_session(id);
}

#[test]
fn destroy_wins_over_transport_transitions_from_pre_acquired_handles() {
    let id = create_ready_room();
    let holder_handle = transport_handle(id).expect("live session must yield a handle");
    let caller_handle = transport_handle(id).expect("live session must yield a handle");

    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let holder = thread::spawn(move || holder_handle.hold_session_lock(entered_tx, release_rx));
    entered_rx
        .recv_timeout(WAIT_TIMEOUT)
        .expect("holder must enter while alive and take the session lock");

    let destroyer = thread::spawn(move || destroy_session(id));
    // Destroy removes the registry entry (after flipping to Destroying)
    // before it blocks on the held session lock.
    let deadline = Instant::now() + WAIT_TIMEOUT;
    while transport_handle(id).is_some() {
        assert!(
            Instant::now() < deadline,
            "destroy never removed the session from the registry"
        );
        thread::yield_now();
    }

    // Every transport verb on the pre-acquired handle is refused with the
    // frozen lifecycle code while destroy is in flight, without touching
    // transport state.
    assert_handle_refusals(&caller_handle, "ENGINE_DESTROYING");

    release_tx
        .send(())
        .expect("holder must still be waiting for release");
    holder
        .join()
        .expect("holder must not panic")
        .expect("a call linearized before destroy must finish normally");
    destroyer.join().expect("destroy must not panic");

    // After destroy completes the same pre-acquired handle stays refused,
    // for every verb.
    assert_handle_refusals(&caller_handle, "ENGINE_DESTROYED");
    assert!(
        transport_handle(id).is_none(),
        "no new handle may be acquired after destroy"
    );
}

/// All seven transport verbs on a pre-acquired handle must refuse with the
/// given frozen lifecycle code.
fn assert_handle_refusals(handle: &TransportHandle, expected_code: &str) {
    let refusals: [(&str, Result<(), TestError>); 7] = [
        (
            "collaborationDrive",
            handle
                .collaboration_drive(ACTION_REQUEST_ID, next_legacy_test_now_millis())
                .map(|_| ()),
        ),
        (
            "collaborationSocketOpen",
            handle
                .collaboration_socket_open(
                    ACTION_REQUEST_ID,
                    FABRICATED_GENERATION,
                    next_legacy_test_now_millis(),
                )
                .map(|_| ()),
        ),
        (
            "collaborationSocketClose",
            handle
                .collaboration_socket_close(
                    ACTION_REQUEST_ID,
                    FABRICATED_GENERATION,
                    CloseDisposition::Retryable,
                    next_legacy_test_now_millis(),
                )
                .map(|_| ()),
        ),
        ("disconnect", handle.disconnect(ACTION_REQUEST_ID)),
        ("detach", handle.detach(ACTION_REQUEST_ID)),
        ("reattach", handle.reattach(ACTION_REQUEST_ID)),
        (
            "markSynchronized",
            handle.mark_synchronized(ACTION_REQUEST_ID, FABRICATED_GENERATION),
        ),
    ];
    for (op, result) in refusals {
        let error = result.expect_err(&format!("{op}: must refuse with {expected_code}"));
        assert_eq!(error.domain, "lifecycle", "{op}: {error:?}");
        assert_eq!(error.code, expected_code, "{op}: {error:?}");
    }
}

#[test]
fn await_remote_rooms_connect_without_document_promotion() {
    let id = create_await_remote_room();
    bridge::attach_runtime(id).unwrap();
    assert_eq!(
        crate::session_initialization_test_support::document_state(id).unwrap(),
        DocumentState::AwaitRemote
    );

    // The full connect/handshake/synchronize path is transport-only: the
    // AwaitRemote -> RoomReady promotion belongs to Task 9's Step 2 handling.
    let generation = drive_generation(id, 181).unwrap();
    open_socket_and_ack_step1(id, 182, generation).unwrap();
    assert_eq!(
        transport_state(id).unwrap(),
        TransportState::Handshaking,
        "socket open must never mean Synchronized",
    );
    mark_synchronized_guard(id, generation);

    destroy_session(id);
}

/// `mark_synchronized` is generation-checked even on `AwaitRemote` rooms and
/// never promotes the document state.
fn mark_synchronized_guard(id: u64, generation: u64) {
    let error = mark_synchronized_for_test(id, 183, FABRICATED_GENERATION).unwrap_err();
    assert_eq!(error.code, TRANSPORT_STALE_GENERATION, "{error:?}");
    mark_synchronized_for_test(id, 184, generation).unwrap();
    assert_eq!(transport_state(id).unwrap(), TransportState::Synchronized);
    assert_eq!(
        crate::session_initialization_test_support::document_state(id).unwrap(),
        DocumentState::AwaitRemote,
        "transport synchronization must not promote the document state",
    );
}

/// A complete y-sync no-op update message (`[MSG_SYNC, MSG_SYNC_UPDATE,
/// buf(len=2, [0, 0])]`): the standard fixture framing of the canonical
/// empty Update-v1.
const NOOP_SYNC_UPDATE_MESSAGE: [u8; 5] = [0, 2, 2, 0, 0];

/// `receive_message` obeys the same generation discipline as every other
/// callback — stale generations and non-frame-accepting states refuse
/// before any decode work, leaving the full audit untouched.
#[test]
fn receive_message_is_generation_gated_across_all_live_transport_states() {
    use crate::session_initialization_test_support::receive_message;

    for state in LIVE_TRANSPORT_STATES {
        let id = create_ready_room();
        bridge::attach_runtime(id).unwrap();
        let latest = drive_transport(id, Doc::RoomReady, state);
        let current = latest.unwrap_or(FABRICATED_GENERATION);
        let before = session_audit(id).unwrap();
        let before_engine = bridge::session_audit(id).unwrap();
        let label = format!("receiveMessage in {state:?}");

        match state {
            TransportState::Handshaking | TransportState::Synchronized => {
                let outcome =
                    receive_message(id, ACTION_REQUEST_ID, current, &NOOP_SYNC_UPDATE_MESSAGE)
                        .unwrap_or_else(|error| panic!("{label}: expected acceptance: {error:?}"));
                assert!(outcome.close.is_none(), "{label}: {outcome:?}");
                assert!(!outcome.remote_commit_applied, "{label}: {outcome:?}");
                assert_eq!(transport_state(id).unwrap(), state, "{label}");
                assert_eq!(
                    bridge::session_audit(id).unwrap(),
                    before_engine,
                    "{label}: a no-op frame changes nothing",
                );
            }
            TransportState::Connecting => {
                let error =
                    receive_message(id, ACTION_REQUEST_ID, current, &NOOP_SYNC_UPDATE_MESSAGE)
                        .expect_err(&format!("{label}: must refuse"));
                assert_eq!(error.domain, "transport", "{label}: {error:?}");
                assert_eq!(
                    error.code, TRANSPORT_INVALID_TRANSITION,
                    "{label}: {error:?}"
                );
                assert_eq!(session_audit(id).unwrap(), before, "{label}");
            }
            TransportState::Detached
            | TransportState::Disconnected
            | TransportState::Incompatible => {
                let error =
                    receive_message(id, ACTION_REQUEST_ID, current, &NOOP_SYNC_UPDATE_MESSAGE)
                        .expect_err(&format!("{label}: must refuse"));
                assert_eq!(error.domain, "transport", "{label}: {error:?}");
                assert_eq!(error.code, TRANSPORT_STALE_GENERATION, "{label}: {error:?}");
                assert_eq!(session_audit(id).unwrap(), before, "{label}");
            }
        }
        // A stale generation refuses identically in every state, even the
        // frame-accepting ones.
        let error = receive_message(
            id,
            ACTION_REQUEST_ID,
            FABRICATED_GENERATION,
            &NOOP_SYNC_UPDATE_MESSAGE,
        )
        .expect_err(&format!("{label}: fabricated generation must refuse"));
        assert_eq!(error.code, TRANSPORT_STALE_GENERATION, "{label}: {error:?}");
        assert_eq!(
            bridge::session_audit(id).unwrap(),
            before_engine,
            "{label}: refusals leave the engine/outbox audit untouched",
        );
        destroy_session(id);
    }
}

/// Destroyed sessions refuse `receive_message` with the frozen
/// lifecycle codes, exactly like every other transport verb.
#[test]
fn receive_message_refuses_destroyed_sessions_with_lifecycle_codes() {
    use crate::session_initialization_test_support::receive_message;

    let id = create_ready_room();
    bridge::attach_runtime(id).unwrap();
    let generation = drive_generation(id, 191).unwrap();
    open_socket_and_ack_step1(id, 192, generation).unwrap();
    destroy_session(id);

    let error = receive_message(id, 193, generation, &NOOP_SYNC_UPDATE_MESSAGE)
        .expect_err("destroyed sessions must refuse frames");
    assert_eq!(error.domain, "lifecycle", "{error:?}");
    assert_eq!(error.code, "ENGINE_DESTROYED", "{error:?}");
}

#[test]
fn drive_starts_initial_generation_and_owns_exponential_retry_deadlines() {
    let id = create_ready_room_with_runtime();
    let mut now_millis = 0;
    let mut generation = collaboration_drive(id, 20_001, now_millis)
        .unwrap()
        .generation_to_open
        .expect("the initial attached drive opens generation one immediately");
    assert_eq!(generation, 1);

    for delay in [500, 1_000, 2_000, 4_000, 8_000, 16_000, 30_000, 30_000] {
        let deadline = now_millis + delay;
        let closed = collaboration_socket_close(
            id,
            20_002,
            generation,
            CloseDisposition::Retryable,
            now_millis,
        )
        .unwrap();
        assert_eq!(closed.transport_state, TransportState::Disconnected);
        assert_eq!(closed.generation_to_open, None);
        assert_eq!(closed.next_deadline_millis, Some(deadline));

        let before = collaboration_drive(id, 20_003, deadline - 1).unwrap();
        assert_eq!(before.transport_state, TransportState::Disconnected);
        assert_eq!(before.generation_to_open, None);
        assert_eq!(before.next_deadline_millis, Some(deadline));

        let due = collaboration_drive(id, 20_004, deadline).unwrap();
        assert_eq!(due.transport_state, TransportState::Connecting);
        assert_eq!(due.next_deadline_millis, None);
        generation = due
            .generation_to_open
            .expect("only Rust's due drive may issue a retry generation");
        now_millis = deadline;
    }
    assert_eq!(generation, 9, "the 30-second retry delay remains capped");
    destroy_session(id);
}

#[test]
fn retry_deadline_overflow_parks_without_busy_loop_or_outbox_loss_until_reattach() {
    let id = create_ready_room_with_runtime();
    let revision = bridge::session_audit(id).unwrap().document_revision;
    bridge::submit_input(
        id,
        &serde_json::json!({
            "version": 1,
            "requestId": "20",
            "baseDocumentRevision": revision.to_string(),
            "text": " retained",
        })
        .to_string(),
    )
    .unwrap();
    let pending = bridge::outbox_pending(id).unwrap().unwrap();

    let generation = collaboration_drive(id, 20_050, 0)
        .unwrap()
        .generation_to_open
        .unwrap();
    collaboration_socket_open(id, 20_051, generation, 0).unwrap();
    let step1 = lease_outbound(id, 20_052, generation).unwrap().unwrap();
    ack_outbound(id, 20_053, generation, step1.lease_id).unwrap();
    let retained_document = lease_outbound(id, 20_054, generation).unwrap().unwrap();

    let closed = collaboration_socket_close(
        id,
        20_055,
        generation,
        CloseDisposition::Retryable,
        u64::MAX,
    )
    .unwrap();
    assert_eq!(closed.transport_state, TransportState::Disconnected);
    assert_eq!(closed.generation_to_open, None);
    assert_eq!(closed.next_deadline_millis, None);
    assert_eq!(bridge::outbox_pending(id).unwrap().unwrap(), pending);

    for request_id in [20_056, 20_057] {
        let parked = collaboration_drive(id, request_id, u64::MAX).unwrap();
        assert_eq!(parked.transport_state, TransportState::Disconnected);
        assert_eq!(parked.generation_to_open, None);
        assert_eq!(parked.next_deadline_millis, None);
        assert_eq!(bridge::outbox_pending(id).unwrap().unwrap(), pending);
    }

    transport_detach(id, 20_058).unwrap();
    transport_reattach(id, 20_059).unwrap();
    let resumed_generation = collaboration_drive(id, 20_060, u64::MAX)
        .unwrap()
        .generation_to_open
        .expect("reattach resets an exhausted retry schedule for a fresh drive");
    collaboration_socket_open(id, 20_061, resumed_generation, u64::MAX).unwrap();
    let resumed_step1 = lease_outbound(id, 20_062, resumed_generation)
        .unwrap()
        .unwrap();
    ack_outbound(id, 20_063, resumed_generation, resumed_step1.lease_id).unwrap();
    let resumed_document = lease_outbound(id, 20_064, resumed_generation)
        .unwrap()
        .unwrap();
    assert_eq!(resumed_document.frame, retained_document.frame);
    ack_outbound(id, 20_065, resumed_generation, resumed_document.lease_id).unwrap();
    assert_eq!(bridge::outbox_pending(id).unwrap().unwrap(), (0, 0));
    destroy_session(id);
}

#[test]
fn close_detach_and_generation_retirement_release_without_consuming_a_lease() {
    let id = create_ready_room_with_runtime();
    let revision = bridge::session_audit(id).unwrap().document_revision;
    bridge::submit_input(
        id,
        &serde_json::json!({
            "version": 1,
            "requestId": "20",
            "baseDocumentRevision": revision.to_string(),
            "text": " retained",
        })
        .to_string(),
    )
    .unwrap();
    let pending_before = bridge::outbox_pending(id).unwrap().unwrap();
    assert_eq!(pending_before.0, 1);

    let first_generation = collaboration_drive(id, 20_100, 0)
        .unwrap()
        .generation_to_open
        .unwrap();
    collaboration_socket_open(id, 20_101, first_generation, 0).unwrap();
    let step1 = lease_outbound(id, 20_102, first_generation)
        .unwrap()
        .unwrap();
    ack_outbound(id, 20_103, first_generation, step1.lease_id).unwrap();
    let first_document = lease_outbound(id, 20_104, first_generation)
        .unwrap()
        .unwrap();

    let closed = collaboration_socket_close(
        id,
        20_105,
        first_generation,
        CloseDisposition::Retryable,
        10,
    )
    .unwrap();
    assert_eq!(closed.next_deadline_millis, Some(510));
    assert_eq!(bridge::outbox_pending(id).unwrap().unwrap(), pending_before);

    let second_generation = collaboration_drive(id, 20_106, 510)
        .unwrap()
        .generation_to_open
        .unwrap();
    collaboration_socket_open(id, 20_107, second_generation, 510).unwrap();
    let second_step1 = lease_outbound(id, 20_108, second_generation)
        .unwrap()
        .unwrap();
    ack_outbound(id, 20_109, second_generation, second_step1.lease_id).unwrap();
    let second_document = lease_outbound(id, 20_110, second_generation)
        .unwrap()
        .unwrap();
    assert_ne!(second_document.lease_id, first_document.lease_id);
    assert_eq!(second_document.frame, first_document.frame);

    transport_detach(id, 20_111).unwrap();
    assert_eq!(bridge::outbox_pending(id).unwrap().unwrap(), pending_before);
    transport_reattach(id, 20_112).unwrap();
    let third_generation = collaboration_drive(id, 20_113, 511)
        .unwrap()
        .generation_to_open
        .unwrap();
    collaboration_socket_open(id, 20_114, third_generation, 511).unwrap();
    let third_step1 = lease_outbound(id, 20_115, third_generation)
        .unwrap()
        .unwrap();
    ack_outbound(id, 20_116, third_generation, third_step1.lease_id).unwrap();
    let third_document = lease_outbound(id, 20_117, third_generation)
        .unwrap()
        .unwrap();
    assert_ne!(third_document.lease_id, second_document.lease_id);
    assert_eq!(third_document.frame, first_document.frame);
    assert_eq!(bridge::outbox_pending(id).unwrap().unwrap(), pending_before);

    ack_outbound(id, 20_118, third_generation, third_document.lease_id).unwrap();
    assert_eq!(bridge::outbox_pending(id).unwrap().unwrap(), (0, 0));
    destroy_session(id);
}

#[test]
fn stale_ack_nack_and_socket_callbacks_are_observationally_pure() {
    let id = create_ready_room_with_runtime();
    let generation = collaboration_drive(id, 20_200, 0)
        .unwrap()
        .generation_to_open
        .unwrap();
    collaboration_socket_open(id, 20_201, generation, 0).unwrap();
    let retained = lease_outbound(id, 20_202, generation).unwrap().unwrap();
    let before_state = transport_state(id).unwrap();

    for error in [
        ack_outbound(id, 20_203, FABRICATED_GENERATION, retained.lease_id).unwrap_err(),
        nack_outbound(id, 20_204, FABRICATED_GENERATION, retained.lease_id).unwrap_err(),
        collaboration_socket_open(id, 20_205, FABRICATED_GENERATION, 1).unwrap_err(),
        collaboration_socket_close(
            id,
            20_206,
            FABRICATED_GENERATION,
            CloseDisposition::Retryable,
            1,
        )
        .unwrap_err(),
        collaboration_receive(id, 20_207, FABRICATED_GENERATION, &[0xff], 1).unwrap_err(),
    ] {
        assert_eq!(error.code, TRANSPORT_STALE_GENERATION, "{error:?}");
    }

    assert_eq!(transport_state(id).unwrap(), before_state);
    let re_leased = lease_outbound(id, 20_208, generation).unwrap().unwrap();
    assert_eq!(re_leased, retained);
    nack_outbound(id, 20_209, generation, retained.lease_id).unwrap();
    destroy_session(id);
}

#[test]
fn synchronization_resets_retry_and_incompatible_has_no_deadline() {
    let id = create_ready_room_with_runtime();
    let initial = collaboration_drive(id, 20_300, 0)
        .unwrap()
        .generation_to_open
        .unwrap();
    let first_close =
        collaboration_socket_close(id, 20_301, initial, CloseDisposition::Retryable, 0).unwrap();
    assert_eq!(first_close.next_deadline_millis, Some(500));

    let synchronized_generation = collaboration_drive(id, 20_302, 500)
        .unwrap()
        .generation_to_open
        .unwrap();
    collaboration_socket_open(id, 20_303, synchronized_generation, 500).unwrap();
    let synchronized = collaboration_receive(
        id,
        20_304,
        synchronized_generation,
        &Message::Sync(SyncMessage::SyncStep2(vec![0, 0])).encode_v1(),
        500,
    )
    .unwrap();
    assert_eq!(synchronized.transport_state, TransportState::Synchronized);

    let reset = collaboration_socket_close(
        id,
        20_305,
        synchronized_generation,
        CloseDisposition::Retryable,
        500,
    )
    .unwrap();
    assert_eq!(reset.next_deadline_millis, Some(1_000));

    let incompatible_generation = collaboration_drive(id, 20_306, 1_000)
        .unwrap()
        .generation_to_open
        .unwrap();
    let incompatible = collaboration_socket_close(
        id,
        20_307,
        incompatible_generation,
        CloseDisposition::Incompatible,
        1_000,
    )
    .unwrap();
    assert_eq!(incompatible.transport_state, TransportState::Incompatible);
    assert_eq!(incompatible.next_deadline_millis, None);
    let parked = collaboration_drive(id, 20_308, 1_000_000).unwrap();
    assert_eq!(parked.transport_state, TransportState::Incompatible);
    assert_eq!(parked.generation_to_open, None);
    assert_eq!(parked.next_deadline_millis, None);
    destroy_session(id);
}
