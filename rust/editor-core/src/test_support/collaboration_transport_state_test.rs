//! Task 8: generation-owned transport transitions.
//!
//! Covers the design's required-transition table as an exhaustive
//! document-state x transport-state x action matrix (including destroy rows
//! and pre-acquired handles), generation discipline (monotonic increments,
//! stale callbacks as observable no-ops), Rust-owned retry disposition with
//! the `Incompatible` detach/reattach escape hatch, the Task 5 replacement
//! policy gate driven by real transitions, and outbox preservation across
//! transport teardown.

use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use crate::boundary::ResourceLimits;
use crate::collaboration_runtime::state::{
    TRANSPORT_INCOMPATIBLE, TRANSPORT_INVALID_TRANSITION, TRANSPORT_NOT_ROOM_BOUND,
    TRANSPORT_STALE_GENERATION,
};
use crate::native_bridge_test_support as bridge;
use crate::session_initialization_test_support::{
    begin_connect, create_local_json, create_room_from_json, destroy_session,
    mark_synchronized_for_test, session_audit, set_transport_state_for_test, socket_closed,
    socket_opened, transport_detach, transport_disconnect, transport_handle, transport_reattach,
    transport_state, write_json, CloseDisposition, DocumentState, SyncDirective, TestError,
    TransportHandle, TransportState,
};
use crate::tiptap_schema;
use crate::yrs_engine::{
    DocumentScope, EditingLimits, InitializationMode, ReplacementHistory, TransactionOrigin,
    YrsDocumentEngine, YrsEngineConfig,
};

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
    BeginConnect,
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
    Action::BeginConnect,
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

fn create_await_remote_room() -> u64 {
    create_room_from_json(r#"{"documentId":"transport-room","lineageId":"transport-lineage"}"#)
        .unwrap()
}

fn create_for(doc: Doc) -> u64 {
    match doc {
        Doc::Local => create_local_json(JSON_SEED).unwrap(),
        Doc::AwaitRemote => create_await_remote_room(),
        Doc::RoomReady => create_ready_room(),
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
        TransportState::Connecting => Some(begin_connect(id, SETUP_REQUEST_ID).unwrap()),
        TransportState::Handshaking => {
            let generation = begin_connect(id, SETUP_REQUEST_ID).unwrap();
            assert_eq!(
                socket_opened(id, SETUP_REQUEST_ID, generation).unwrap(),
                SyncDirective::SendSyncStep1,
            );
            Some(generation)
        }
        TransportState::Synchronized => {
            let generation = begin_connect(id, SETUP_REQUEST_ID).unwrap();
            socket_opened(id, SETUP_REQUEST_ID, generation).unwrap();
            mark_synchronized_for_test(id, SETUP_REQUEST_ID, generation).unwrap();
            Some(generation)
        }
        TransportState::Incompatible => {
            let generation = begin_connect(id, SETUP_REQUEST_ID).unwrap();
            assert_eq!(
                socket_closed(
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
        Action::BeginConnect => match (doc, state) {
            (Doc::Local, _) => Expected::Refused(TRANSPORT_NOT_ROOM_BOUND),
            (_, Disconnected) => Expected::Accepted(Connecting),
            (_, Incompatible) => Expected::Refused(TRANSPORT_INCOMPATIBLE),
            (_, Detached | Connecting | Handshaking | Synchronized) => {
                Expected::Refused(TRANSPORT_INVALID_TRANSITION)
            }
        },
        Action::Reattach => match (doc, state) {
            (Doc::Local, _) => Expected::Refused(TRANSPORT_NOT_ROOM_BOUND),
            (_, Detached) => Expected::Accepted(Disconnected),
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
        Action::Detach => {
            if state == Detached {
                Expected::Refused(TRANSPORT_INVALID_TRANSITION)
            } else {
                Expected::Accepted(Detached)
            }
        }
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
        Action::BeginConnect => "beginConnect",
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
        Action::BeginConnect => begin_connect(id, ACTION_REQUEST_ID).map(|_| ()),
        Action::Reattach => transport_reattach(id, ACTION_REQUEST_ID),
        Action::SocketOpenedCurrent => {
            socket_opened(id, ACTION_REQUEST_ID, current).map(|directive| {
                assert_eq!(
                    directive,
                    SyncDirective::SendSyncStep1,
                    "socket open must demand a Sync Step 1 send",
                );
            })
        }
        Action::SocketOpenedStale => {
            socket_opened(id, ACTION_REQUEST_ID, FABRICATED_GENERATION).map(|_| ())
        }
        Action::SocketClosedCurrentRetryable => {
            socket_closed(id, ACTION_REQUEST_ID, current, CloseDisposition::Retryable).map(|_| ())
        }
        Action::SocketClosedCurrentIncompatible => socket_closed(
            id,
            ACTION_REQUEST_ID,
            current,
            CloseDisposition::Incompatible,
        )
        .map(|_| ()),
        Action::SocketClosedStaleRetryable => socket_closed(
            id,
            ACTION_REQUEST_ID,
            FABRICATED_GENERATION,
            CloseDisposition::Retryable,
        )
        .map(|_| ()),
        Action::SocketClosedStaleIncompatible => socket_closed(
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
            "beginConnect",
            begin_connect(id, ACTION_REQUEST_ID).map(|_| ()),
        ),
        (
            "socketOpened",
            socket_opened(id, ACTION_REQUEST_ID, current).map(|_| ()),
        ),
        (
            "socketClosed",
            socket_closed(id, ACTION_REQUEST_ID, current, CloseDisposition::Retryable).map(|_| ()),
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
fn local_ready_matrix_has_an_explicit_outcome_for_every_cell() {
    run_matrix(Doc::Local);
}

#[test]
fn await_remote_matrix_has_an_explicit_outcome_for_every_cell() {
    run_matrix(Doc::AwaitRemote);
}

#[test]
fn room_ready_matrix_has_an_explicit_outcome_for_every_cell() {
    run_matrix(Doc::RoomReady);
}

#[test]
fn generations_increment_exactly_once_per_accepted_connect() {
    let id = create_ready_room();

    let first = begin_connect(id, 101).unwrap();
    // A refused begin_connect must not consume a generation.
    let error = begin_connect(id, 102).unwrap_err();
    assert_eq!(error.code, TRANSPORT_INVALID_TRANSITION, "{error:?}");
    assert_eq!(
        socket_closed(id, 103, first, CloseDisposition::Retryable).unwrap(),
        TransportState::Disconnected,
    );

    let second = begin_connect(id, 104).unwrap();
    assert_eq!(
        second,
        first + 1,
        "an accepted connect increments the generation exactly once",
    );
    transport_disconnect(id, 105).unwrap();

    let third = begin_connect(id, 106).unwrap();
    assert_eq!(third, second + 1, "refusals must never burn a generation");

    destroy_session(id);
}

#[test]
fn stale_generations_can_neither_advance_nor_regress_state() {
    let id = create_ready_room();

    // Open and retire a first generation, then start a second attempt.
    let first = begin_connect(id, 111).unwrap();
    socket_closed(id, 112, first, CloseDisposition::Retryable).unwrap();
    let second = begin_connect(id, 113).unwrap();
    let audit = session_audit(id).unwrap();
    assert_eq!(audit.transport_state, TransportState::Connecting);

    // The retired generation is refused for every callback, observably, and
    // both the state and the live generation stay untouched.
    for (request_id, result) in [
        (114, socket_opened(id, 114, first).map(|_| ())),
        (
            115,
            socket_closed(id, 115, first, CloseDisposition::Retryable).map(|_| ()),
        ),
        (
            116,
            socket_closed(id, 116, first, CloseDisposition::Incompatible).map(|_| ()),
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
    assert_eq!(
        socket_opened(id, 118, second).unwrap(),
        SyncDirective::SendSyncStep1,
    );
    assert_eq!(transport_state(id).unwrap(), TransportState::Handshaking);

    // Local disconnect retires the live generation: its late close callback
    // is stale and cannot resurrect or regress the transport.
    transport_disconnect(id, 119).unwrap();
    let error = socket_closed(id, 120, second, CloseDisposition::Incompatible).unwrap_err();
    assert_eq!(error.code, TRANSPORT_STALE_GENERATION, "{error:?}");
    assert_eq!(transport_state(id).unwrap(), TransportState::Disconnected);
    // Retry stays Rust-eligible after a local disconnect.
    begin_connect(id, 121).unwrap();

    destroy_session(id);
}

#[test]
fn incompatible_refuses_javascript_reconnects_until_detach_and_reattach() {
    let id = create_ready_room();

    let generation = begin_connect(id, 131).unwrap();
    socket_opened(id, 132, generation).unwrap();
    assert_eq!(
        socket_closed(id, 133, generation, CloseDisposition::Incompatible).unwrap(),
        TransportState::Incompatible,
    );

    // A JS retry timer cannot force a reconnect out of deterministic
    // incompatibility, no matter how often it asks.
    for request_id in [134, 135, 136] {
        let error = begin_connect(id, request_id).unwrap_err();
        assert_eq!(error.domain, "transport", "{error:?}");
        assert_eq!(error.code, TRANSPORT_INCOMPATIBLE, "{error:?}");
        assert_eq!(transport_state(id).unwrap(), TransportState::Incompatible);
    }

    // The only escape hatch is the explicit detach/reattach cycle.
    transport_detach(id, 137).unwrap();
    assert_eq!(transport_state(id).unwrap(), TransportState::Detached);
    transport_reattach(id, 138).unwrap();
    assert_eq!(transport_state(id).unwrap(), TransportState::Disconnected);
    let next = begin_connect(id, 139).unwrap();
    assert_eq!(next, generation + 1);

    destroy_session(id);
}

#[test]
fn replacement_policy_gate_follows_real_transport_transitions() {
    let id = create_ready_room();

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
    let generation = begin_connect(id, 142).unwrap();
    assert_connected_refusal(143, JSON_REPLACEMENT_B);
    socket_opened(id, 144, generation).unwrap();
    assert_connected_refusal(145, JSON_REPLACEMENT_B);
    mark_synchronized_for_test(id, 146, generation).unwrap();
    assert_connected_refusal(147, JSON_REPLACEMENT_B);

    // Remote close back to Disconnected: allowed again.
    assert_eq!(
        socket_closed(id, 148, generation, CloseDisposition::Retryable).unwrap(),
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
    let generation = begin_connect(id, 150).unwrap();
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

    // Incompatible is an allowed replacement row of the Task 5 matrix.
    let generation = begin_connect(id, 154).unwrap();
    socket_closed(id, 155, generation, CloseDisposition::Incompatible).unwrap();
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
    let generation = begin_connect(id, 162).unwrap();
    socket_closed(id, 163, generation, CloseDisposition::Retryable).unwrap();
    assert_eq!(bridge::outbox_pending(id).unwrap().unwrap(), pending);

    let generation = begin_connect(id, 164).unwrap();
    socket_closed(id, 165, generation, CloseDisposition::Incompatible).unwrap();
    assert_eq!(bridge::outbox_pending(id).unwrap().unwrap(), pending);

    transport_detach(id, 166).unwrap();
    assert_eq!(bridge::outbox_pending(id).unwrap().unwrap(), pending);

    transport_reattach(id, 167).unwrap();
    let generation = begin_connect(id, 168).unwrap();
    socket_opened(id, 169, generation).unwrap();
    transport_disconnect(id, 170).unwrap();
    assert_eq!(bridge::outbox_pending(id).unwrap().unwrap(), pending);

    // The entry is still deliverable.
    let (request_id, update) = bridge::take_next_update(id).unwrap().unwrap();
    assert_eq!(request_id, 161);
    assert!(!update.is_empty());

    bridge::destroy_session(id);
}

/// Task 10 extension of the teardown matrix rows above: the same
/// transitions that preserve the pending outbox entry clear the
/// transport-scoped awareness peers while the desired local awareness
/// survives every one of them.
#[test]
fn transport_teardown_clears_awareness_peers_and_retains_desired_awareness() {
    use crate::session_initialization_test_support::{
        awareness_peers, desired_awareness, receive_message, set_desired_awareness,
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
    let generation = begin_connect(id, 173).unwrap();
    socket_opened(id, 174, generation).unwrap();
    receive_message(id, 175, generation, &awareness_frame(9_501, 1)).unwrap();
    assert_eq!(remote_peer_count(id), 1);
    socket_closed(id, 176, generation, CloseDisposition::Retryable).unwrap();
    assert_eq!(remote_peer_count(id), 0, "retryable close clears peers");
    assert_eq!(desired_awareness(id).unwrap(), Some(desired.clone()));
    assert_eq!(bridge::outbox_pending(id).unwrap().unwrap(), pending);

    // Incompatible close.
    let generation = begin_connect(id, 177).unwrap();
    socket_opened(id, 178, generation).unwrap();
    receive_message(id, 179, generation, &awareness_frame(9_502, 1)).unwrap();
    assert_eq!(remote_peer_count(id), 1);
    socket_closed(id, 180, generation, CloseDisposition::Incompatible).unwrap();
    assert_eq!(remote_peer_count(id), 0, "incompatible close clears peers");
    assert_eq!(desired_awareness(id).unwrap(), Some(desired.clone()));

    // Detach/reattach escape hatch, then a local disconnect.
    transport_detach(id, 181).unwrap();
    transport_reattach(id, 182).unwrap();
    let generation = begin_connect(id, 183).unwrap();
    socket_opened(id, 184, generation).unwrap();
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
            "beginConnect",
            handle.begin_connect(ACTION_REQUEST_ID).map(|_| ()),
        ),
        (
            "socketOpened",
            handle
                .socket_opened(ACTION_REQUEST_ID, FABRICATED_GENERATION)
                .map(|_| ()),
        ),
        (
            "socketClosed",
            handle
                .socket_closed(
                    ACTION_REQUEST_ID,
                    FABRICATED_GENERATION,
                    CloseDisposition::Retryable,
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
    assert_eq!(
        crate::session_initialization_test_support::document_state(id).unwrap(),
        DocumentState::AwaitRemote
    );

    // The full connect/handshake/synchronize path is transport-only: the
    // AwaitRemote -> RoomReady promotion belongs to Task 9's Step 2 handling.
    let generation = begin_connect(id, 181).unwrap();
    assert_eq!(
        socket_opened(id, 182, generation).unwrap(),
        SyncDirective::SendSyncStep1,
        "socket open means Handshaking plus a demanded Step 1 send",
    );
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

/// Task 9: `receive_message` obeys the same generation discipline as every
/// other callback — stale generations and non-frame-accepting states refuse
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

/// Task 9: destroyed sessions refuse `receive_message` with the frozen
/// lifecycle codes, exactly like every other transport verb.
#[test]
fn receive_message_refuses_destroyed_sessions_with_lifecycle_codes() {
    use crate::session_initialization_test_support::receive_message;

    let id = create_ready_room();
    bridge::attach_runtime(id).unwrap();
    let generation = begin_connect(id, 191).unwrap();
    socket_opened(id, 192, generation).unwrap();
    destroy_session(id);

    let error = receive_message(id, 193, generation, &NOOP_SYNC_UPDATE_MESSAGE)
        .expect_err("destroyed sessions must refuse frames");
    assert_eq!(error.domain, "lifecycle", "{error:?}");
    assert_eq!(error.code, "ENGINE_DESTROYED", "{error:?}");
}
