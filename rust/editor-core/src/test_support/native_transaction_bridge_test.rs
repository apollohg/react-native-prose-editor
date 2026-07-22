//! Task 7: `NativeTransactionBridge` — the only production local mutation
//! entrance.
//!
//! Covers versioned data-only envelope admission (version, size, unknown and
//! forged-origin fields), bridge-assigned trusted origins, read-only and
//! input-filter policy, the frozen reservation-before-irreversible-write
//! flow with exact-count/exact-byte/one-over/allocation injections on every
//! durable path, and captured-update convergence on twin replicas.

use crate::boundary::ResourceLimits;
use crate::native_bridge_test_support::{
    self as bridge, BridgeTestOutcome, NativeSessionAudit, SessionOptions,
};
use crate::session_initialization_test_support::{set_transport_state_for_test, TransportState};
use crate::tiptap_schema;
use crate::yrs_engine::{
    EditingLimits, InitializationMode, TransactionOrigin, YrsDocumentEngine, YrsEngineConfig,
};

const PLAIN_DOC: &str = r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abcdef"}]}]}"#;
const REPLACEMENT_DOC: &str = r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"replaced body"}]},{"type":"paragraph","content":[{"type":"text","text":"second"}]}]}"#;

const GENEROUS_MESSAGES: usize = 64;
const GENEROUS_BYTES: usize = 1024 * 1024;

fn session_with_runtime() -> u64 {
    bridge::create_session(SessionOptions {
        initial_json: Some(PLAIN_DOC.into()),
        attach_runtime: true,
        ..SessionOptions::default()
    })
    .unwrap()
}

fn revision(id: u64) -> u64 {
    bridge::session_audit(id).unwrap().document_revision
}

fn input_envelope(request_id: u64, base_revision: u64, text: &str) -> String {
    serde_json::json!({
        "version": 1,
        "requestId": request_id,
        "baseDocumentRevision": base_revision,
        "text": text,
    })
    .to_string()
}

fn command_envelope(request_id: u64, base_revision: u64, command: serde_json::Value) -> String {
    serde_json::json!({
        "version": 1,
        "requestId": request_id,
        "baseDocumentRevision": base_revision,
        "command": command,
    })
    .to_string()
}

fn selection_envelope(request_id: u64, base_revision: u64, anchor: u32, head: u32) -> String {
    serde_json::json!({
        "version": 1,
        "requestId": request_id,
        "baseDocumentRevision": base_revision,
        "selection": {
            "type": "text",
            "anchor": { "offset": anchor, "kind": "scalar" },
            "head": { "offset": head, "kind": "scalar" },
        },
    })
    .to_string()
}

fn replace_envelope(request_id: u64, base_revision: u64, json: &str, history: &str) -> String {
    serde_json::json!({
        "version": 1,
        "requestId": request_id,
        "baseDocumentRevision": base_revision,
        "setJson": serde_json::from_str::<serde_json::Value>(json).unwrap(),
        "history": history,
    })
    .to_string()
}

fn assert_transaction_changed(outcome: &BridgeTestOutcome) {
    assert!(
        matches!(
            outcome,
            BridgeTestOutcome::Transaction { changed: true, .. }
        ),
        "expected a changed transaction outcome, got {outcome:?}",
    );
}

fn assert_atomic_rejection(
    id: u64,
    before: &NativeSessionAudit,
    error: &bridge::TestError,
    code: &str,
) {
    assert_eq!(error.code, code, "unexpected rejection code: {error:?}");
    let after = bridge::session_audit(id).unwrap();
    assert_eq!(
        &after, before,
        "rejection must preserve the full session audit"
    );
}

fn twin_replica() -> YrsDocumentEngine {
    // A content-free AwaitRemote replica: seeded exclusively by captured
    // outbound updates, never by locally bootstrapped content.
    YrsDocumentEngine::new(YrsEngineConfig {
        schema: tiptap_schema(),
        fragment_name: "prosemirror".into(),
        initialization_mode: InitializationMode::AwaitRemote,
        resource_limits: ResourceLimits::default(),
        editing_limits: EditingLimits::default(),
        max_length: None,
        scope: Some(crate::yrs_engine::DocumentScope {
            document_id: "bridge-twin".into(),
            lineage_id: "bridge-twin-lineage".into(),
        }),
    })
    .unwrap()
}

#[test]
fn input_commit_is_one_local_input_transaction_with_undo_tracking() {
    let id = session_with_runtime();
    let outcome = bridge::submit_input(id, &input_envelope(1, revision(id), "XY")).unwrap();
    let BridgeTestOutcome::Transaction {
        changed,
        can_undo,
        document_revision,
        ..
    } = outcome
    else {
        panic!("input commit must produce a transaction outcome: {outcome:?}");
    };
    assert!(changed);
    assert!(can_undo, "input commits are tracked by local undo");
    let audit = bridge::session_audit(id).unwrap();
    assert_eq!(audit.document_revision, document_revision);
    assert_eq!(
        audit.last_committed_origin.as_deref(),
        Some(TransactionOrigin::LocalInput.as_tag()),
        "the bridge assigns the trusted local-input origin",
    );
    assert!(audit
        .document_json
        .as_ref()
        .unwrap()
        .to_string()
        .contains("XY"));
    // Exactly one outbox entry, within its admitted bound.
    let (count, bytes) = bridge::outbox_pending(id).unwrap().unwrap();
    assert_eq!(count, 1);
    let bound = bridge::last_reserved_upper_bound(id).unwrap().unwrap();
    assert!(
        bytes <= bound,
        "captured bytes {bytes} exceed bound {bound}"
    );
    bridge::destroy_session(id);
}

#[test]
fn command_entry_applies_planner_commands_with_local_command_origin() {
    let id = session_with_runtime();
    let outcome = bridge::submit_command(
        id,
        &command_envelope(
            11,
            revision(id),
            serde_json::json!({ "type": "toggleBlockquote" }),
        ),
    )
    .unwrap();
    assert_transaction_changed(&outcome);
    let audit = bridge::session_audit(id).unwrap();
    assert_eq!(
        audit.last_committed_origin.as_deref(),
        Some(TransactionOrigin::LocalCommand.as_tag()),
    );
    assert!(audit.can_undo);

    // A structurally inapplicable command is a structured NotApplicable
    // outcome, not an error, and reserves nothing.
    let before = bridge::session_audit(id).unwrap();
    let outcome = bridge::submit_command(
        id,
        &command_envelope(
            12,
            revision(id),
            serde_json::json!({ "type": "outdentListItem" }),
        ),
    )
    .unwrap();
    assert_eq!(outcome, BridgeTestOutcome::NotApplicable);
    assert_eq!(bridge::session_audit(id).unwrap(), before);
    bridge::destroy_session(id);
}

#[test]
fn selection_requests_reserve_nothing_and_enqueue_nothing() {
    let id = session_with_runtime();
    let before = bridge::session_audit(id).unwrap();
    let outcome =
        bridge::submit_selection(id, &selection_envelope(21, revision(id), 1, 3)).unwrap();
    assert_transaction_changed(&outcome);
    let after = bridge::session_audit(id).unwrap();
    assert_eq!(after.document_revision, before.document_revision);
    assert!(after.state_revision > before.state_revision);
    assert_ne!(after.selection, before.selection);
    // Outbox count/bytes unchanged; no reservation was ever recorded.
    assert_eq!(after.outbox_pending_updates, Some(0));
    assert_eq!(after.outbox_pending_bytes, Some(0));
    assert_eq!(bridge::last_reserved_upper_bound(id).unwrap(), None);
    bridge::destroy_session(id);
}

#[test]
fn envelope_admission_rejects_bad_version_unknown_fields_and_forged_origins() {
    let id = session_with_runtime();
    let before = bridge::session_audit(id).unwrap();
    let base = revision(id);

    type SubmitEntry = fn(u64, &str) -> Result<BridgeTestOutcome, bridge::TestError>;
    let submitters: [(&str, SubmitEntry); 4] = [
        ("input", bridge::submit_input),
        ("command", bridge::submit_command),
        ("selection", bridge::submit_selection),
        ("localApi", bridge::submit_local_api),
    ];
    let payloads: [(&str, serde_json::Value); 4] = [
        ("input", serde_json::json!({ "text": "x" })),
        (
            "command",
            serde_json::json!({ "command": { "type": "toggleBlockquote" } }),
        ),
        (
            "selection",
            serde_json::json!({ "selection": { "type": "all" } }),
        ),
        (
            "localApi",
            serde_json::json!({
                "setJson": serde_json::from_str::<serde_json::Value>(PLAIN_DOC).unwrap(),
                "history": "undoableBoundary",
            }),
        ),
    ];

    for ((label, submit), (_, payload)) in submitters.iter().zip(payloads.iter()) {
        let mut valid = serde_json::json!({
            "version": 1,
            "requestId": 31,
            "baseDocumentRevision": base,
        });
        for (key, value) in payload.as_object().unwrap() {
            valid[key] = value.clone();
        }

        // Unsupported version.
        let mut envelope = valid.clone();
        envelope["version"] = serde_json::json!(2);
        let error = submit(id, &envelope.to_string()).unwrap_err();
        assert_atomic_rejection(id, &before, &error, "CONFIG_INVALID");

        // Unknown field.
        let mut envelope = valid.clone();
        envelope["unexpected"] = serde_json::json!(true);
        let error = submit(id, &envelope.to_string()).unwrap_err();
        assert_atomic_rejection(id, &before, &error, "CONFIG_INVALID");

        // A caller-supplied origin is a CONFIG_INVALID-class rejection: there
        // is no origin field in any envelope, so it is an unknown field.
        let mut envelope = valid.clone();
        envelope["origin"] = serde_json::json!("remoteSync");
        let error = submit(id, &envelope.to_string()).unwrap_err();
        assert_atomic_rejection(id, &before, &error, "CONFIG_INVALID");

        // Non-JSON garbage.
        let error = submit(id, "not-json").unwrap_err();
        assert_atomic_rejection(id, &before, &error, "CONFIG_INVALID");

        let _ = label;
    }
    bridge::destroy_session(id);
}

#[test]
fn oversized_envelopes_reject_before_parsing() {
    let id = session_with_runtime();
    let before = bridge::session_audit(id).unwrap();
    let oversized = "x".repeat(ResourceLimits::default().max_input_bytes + 1);
    let error =
        bridge::submit_input(id, &input_envelope(41, revision(id), &oversized)).unwrap_err();
    assert_atomic_rejection(id, &before, &error, "INPUT_LIMIT_EXCEEDED");
    bridge::destroy_session(id);
}

#[test]
fn stale_base_revisions_reject_with_revision_mismatch() {
    let id = session_with_runtime();
    bridge::submit_input(id, &input_envelope(51, revision(id), "a")).unwrap();
    bridge::take_next_update(id).unwrap().unwrap();
    let stale = revision(id) - 1;
    let before = bridge::session_audit(id).unwrap();

    let error = bridge::submit_input(id, &input_envelope(52, stale, "b")).unwrap_err();
    assert_atomic_rejection(id, &before, &error, "REVISION_MISMATCH");
    assert_eq!(error.request_id, Some(52));

    let error = bridge::submit_command(
        id,
        &command_envelope(53, stale, serde_json::json!({ "type": "toggleBlockquote" })),
    )
    .unwrap_err();
    assert_atomic_rejection(id, &before, &error, "REVISION_MISMATCH");

    let error = bridge::submit_selection(id, &selection_envelope(54, stale, 0, 1)).unwrap_err();
    assert_atomic_rejection(id, &before, &error, "REVISION_MISMATCH");

    let error = bridge::submit_local_api(
        id,
        &replace_envelope(55, stale, REPLACEMENT_DOC, "undoableBoundary"),
    )
    .unwrap_err();
    assert_atomic_rejection(id, &before, &error, "REVISION_MISMATCH");
    bridge::destroy_session(id);
}

#[test]
fn read_only_policy_rejects_input_and_command_but_admits_selection_and_local_api() {
    let id = bridge::create_session(SessionOptions {
        initial_json: Some(PLAIN_DOC.into()),
        read_only: true,
        attach_runtime: true,
        ..SessionOptions::default()
    })
    .unwrap();
    let before = bridge::session_audit(id).unwrap();

    let error = bridge::submit_input(id, &input_envelope(61, revision(id), "x")).unwrap_err();
    assert_atomic_rejection(id, &before, &error, "MUTATION_REJECTED");
    let error = bridge::submit_command(
        id,
        &command_envelope(
            62,
            revision(id),
            serde_json::json!({ "type": "toggleBlockquote" }),
        ),
    )
    .unwrap_err();
    assert_atomic_rejection(id, &before, &error, "MUTATION_REJECTED");

    // Selection is state-only and remains available (legacy read-only never
    // gated selection movement).
    let outcome =
        bridge::submit_selection(id, &selection_envelope(63, revision(id), 0, 2)).unwrap();
    assert_transaction_changed(&outcome);
    // Local-API requests are the legacy `Source::Api` pass-through.
    let outcome = bridge::submit_local_api(
        id,
        &replace_envelope(64, revision(id), REPLACEMENT_DOC, "undoableBoundary"),
    )
    .unwrap();
    assert!(matches!(
        outcome,
        BridgeTestOutcome::Replacement { changed: true, .. }
    ));
    bridge::destroy_session(id);
}

#[test]
fn input_filter_keeps_matching_characters_and_filters_whole_commits() {
    let id = bridge::create_session(SessionOptions {
        initial_json: Some(PLAIN_DOC.into()),
        input_filter: Some("[0-9]".into()),
        attach_runtime: true,
        ..SessionOptions::default()
    })
    .unwrap();

    let outcome = bridge::submit_input(id, &input_envelope(71, revision(id), "a1b2")).unwrap();
    assert_transaction_changed(&outcome);
    let audit = bridge::session_audit(id).unwrap();
    let text = audit.document_json.as_ref().unwrap().to_string();
    assert!(
        text.contains("12"),
        "filtered insert must keep only digits: {text}"
    );
    assert!(!text.contains("a1b2"));
    bridge::take_next_update(id).unwrap().unwrap();

    // A fully filtered commit is a structured no-op: no document change, no
    // history entry, no reservation, no outbox entry.
    let before = bridge::session_audit(id).unwrap();
    let outcome = bridge::submit_input(id, &input_envelope(72, revision(id), "abc")).unwrap();
    assert!(
        matches!(
            outcome,
            BridgeTestOutcome::Transaction { changed: false, .. }
        ),
        "fully filtered input must be an unchanged transaction: {outcome:?}",
    );
    assert_eq!(bridge::session_audit(id).unwrap(), before);
    bridge::destroy_session(id);
}

#[test]
fn local_api_replacement_honors_history_mode_and_connected_policy() {
    // UndoableBoundary: exactly one undoable local-API boundary.
    let id = session_with_runtime();
    let outcome = bridge::submit_local_api(
        id,
        &replace_envelope(81, revision(id), REPLACEMENT_DOC, "undoableBoundary"),
    )
    .unwrap();
    assert!(matches!(
        outcome,
        BridgeTestOutcome::Replacement { changed: true, .. }
    ));
    let audit = bridge::session_audit(id).unwrap();
    assert!(audit.can_undo);
    assert_eq!(
        audit.last_committed_origin.as_deref(),
        Some(TransactionOrigin::LocalApi.as_tag()),
    );
    assert!(bridge::undo(id, 82).unwrap());
    let restored = bridge::session_audit(id).unwrap();
    assert!(restored
        .document_json
        .as_ref()
        .unwrap()
        .to_string()
        .contains("abcdef"));
    bridge::destroy_session(id);

    // ResetAndClear: non-undoable, clears local history.
    let id = session_with_runtime();
    bridge::submit_input(id, &input_envelope(83, revision(id), "h")).unwrap();
    assert!(bridge::session_audit(id).unwrap().can_undo);
    bridge::submit_local_api(
        id,
        &replace_envelope(84, revision(id), REPLACEMENT_DOC, "resetAndClear"),
    )
    .unwrap();
    let audit = bridge::session_audit(id).unwrap();
    assert!(!audit.can_undo, "resetAndClear clears local history");
    assert!(!audit.can_redo);
    bridge::destroy_session(id);

    // Connected transports reject with the frozen lifecycle code and a
    // fully preserved audit.
    let id = session_with_runtime();
    set_transport_state_for_test(id, TransportState::Synchronized).unwrap();
    let before = bridge::session_audit(id).unwrap();
    let error = bridge::submit_local_api(
        id,
        &replace_envelope(85, revision(id), REPLACEMENT_DOC, "undoableBoundary"),
    )
    .unwrap_err();
    assert_eq!(error.domain, "lifecycle");
    assert_atomic_rejection(id, &before, &error, "WHOLE_DOCUMENT_REPLACEMENT_CONNECTED");
    bridge::destroy_session(id);
}

/// Every durable path emits exactly one bounded entry whose captured
/// Update-v1 replays to convergence on an independent twin replica.
#[test]
fn captured_updates_converge_on_a_twin_replica_across_all_durable_paths() {
    let id = session_with_runtime();
    let mut twin = twin_replica();
    let mut replay_request = 9_000_u64;

    // Seed the twin with the session's initial state.
    let initial = bridge::session_audit(id).unwrap().encoded_state.unwrap();
    twin.apply_remote_update_v1(replay_request, &initial)
        .unwrap();

    let mut drain_and_replay = |label: &str| {
        let bound = bridge::last_reserved_upper_bound(id).unwrap().unwrap();
        let (_, update) = bridge::take_next_update(id).unwrap().unwrap();
        assert!(
            update.len() <= bound,
            "{label}: captured {} exceeds admitted bound {bound}",
            update.len(),
        );
        replay_request += 1;
        twin.apply_remote_update_v1(replay_request, &update)
            .unwrap();
        let audit = bridge::session_audit(id).unwrap();
        assert_eq!(
            twin.document_json(),
            audit.document_json,
            "{label}: twin must converge from the captured update alone",
        );
        assert!(
            bridge::take_next_update(id).unwrap().is_none(),
            "{label}: exactly one entry per durable commit",
        );
    };

    bridge::submit_input(id, &input_envelope(91, revision(id), "typed")).unwrap();
    drain_and_replay("input transaction");

    bridge::submit_command(
        id,
        &command_envelope(
            92,
            revision(id),
            serde_json::json!({ "type": "toggleBlockquote" }),
        ),
    )
    .unwrap();
    drain_and_replay("command");

    assert!(bridge::undo(id, 93).unwrap());
    drain_and_replay("undo");

    assert!(bridge::redo(id, 94).unwrap());
    drain_and_replay("redo");

    bridge::submit_local_api(
        id,
        &replace_envelope(95, revision(id), REPLACEMENT_DOC, "undoableBoundary"),
    )
    .unwrap();
    drain_and_replay("replace (UndoableBoundary)");

    bridge::submit_local_api(
        id,
        &replace_envelope(96, revision(id), PLAIN_DOC, "resetAndClear"),
    )
    .unwrap();
    drain_and_replay("reset (ResetAndClear)");

    bridge::destroy_session(id);
}

#[test]
fn history_pop_reservation_bound_is_the_exact_captured_length() {
    let id = session_with_runtime();
    bridge::submit_input(id, &input_envelope(101, revision(id), "pop")).unwrap();
    bridge::take_next_update(id).unwrap().unwrap();

    let probed = bridge::probe_history_pop_bytes(id, 102, true)
        .unwrap()
        .unwrap();
    assert!(bridge::undo(id, 102).unwrap());
    let bound = bridge::last_reserved_upper_bound(id).unwrap().unwrap();
    let (_, update) = bridge::take_next_update(id).unwrap().unwrap();
    assert_eq!(
        update.len(),
        probed,
        "probe must predict the captured pop bytes"
    );
    assert_eq!(
        update.len(),
        bound,
        "pop reservations are exact-length bounds"
    );
    bridge::destroy_session(id);
}

#[derive(Clone, Copy, Debug)]
enum DurablePath {
    InputTransaction,
    Command,
    Undo,
    Redo,
    ReplaceBoundary,
    ResetAndClear,
}

const DURABLE_PATHS: [DurablePath; 6] = [
    DurablePath::InputTransaction,
    DurablePath::Command,
    DurablePath::Undo,
    DurablePath::Redo,
    DurablePath::ReplaceBoundary,
    DurablePath::ResetAndClear,
];

/// Build a fresh attached session prepared so the path's durable operation is
/// available, with the outbox drained and generous ceilings.
fn prepared_session(path: DurablePath) -> u64 {
    let id = session_with_runtime();
    bridge::set_outbox_ceilings(id, GENEROUS_MESSAGES, GENEROUS_BYTES).unwrap();
    match path {
        DurablePath::InputTransaction
        | DurablePath::Command
        | DurablePath::ReplaceBoundary
        | DurablePath::ResetAndClear => {}
        DurablePath::Undo => {
            bridge::submit_input(id, &input_envelope(900, revision(id), "u")).unwrap();
        }
        DurablePath::Redo => {
            bridge::submit_input(id, &input_envelope(901, revision(id), "r")).unwrap();
            assert!(bridge::undo(id, 902).unwrap());
        }
    }
    while bridge::take_next_update(id).unwrap().is_some() {}
    id
}

fn probe_bound(id: u64, path: DurablePath, request_id: u64) -> usize {
    match path {
        DurablePath::InputTransaction => {
            bridge::probe_input_upper_bound(id, &input_envelope(request_id, revision(id), "sat"))
                .unwrap()
                .unwrap()
        }
        DurablePath::Command => bridge::probe_command_upper_bound(
            id,
            &command_envelope(
                request_id,
                revision(id),
                serde_json::json!({ "type": "toggleBlockquote" }),
            ),
        )
        .unwrap()
        .unwrap(),
        DurablePath::Undo => bridge::probe_history_pop_bytes(id, request_id, true)
            .unwrap()
            .unwrap(),
        DurablePath::Redo => bridge::probe_history_pop_bytes(id, request_id, false)
            .unwrap()
            .unwrap(),
        DurablePath::ReplaceBoundary => {
            bridge::probe_replace_json_upper_bound(id, request_id, REPLACEMENT_DOC, false).unwrap()
        }
        DurablePath::ResetAndClear => {
            bridge::probe_replace_json_upper_bound(id, request_id, REPLACEMENT_DOC, true).unwrap()
        }
    }
}

fn run_durable(id: u64, path: DurablePath, request_id: u64) -> Result<(), bridge::TestError> {
    let base = revision(id);
    match path {
        DurablePath::InputTransaction => {
            bridge::submit_input(id, &input_envelope(request_id, base, "sat")).map(|_| ())
        }
        DurablePath::Command => bridge::submit_command(
            id,
            &command_envelope(
                request_id,
                base,
                serde_json::json!({ "type": "toggleBlockquote" }),
            ),
        )
        .map(|_| ()),
        DurablePath::Undo => bridge::undo(id, request_id).map(|popped| assert!(popped)),
        DurablePath::Redo => bridge::redo(id, request_id).map(|popped| assert!(popped)),
        DurablePath::ReplaceBoundary => bridge::submit_local_api(
            id,
            &replace_envelope(request_id, base, REPLACEMENT_DOC, "undoableBoundary"),
        )
        .map(|_| ()),
        DurablePath::ResetAndClear => bridge::submit_local_api(
            id,
            &replace_envelope(request_id, base, REPLACEMENT_DOC, "resetAndClear"),
        )
        .map(|_| ()),
    }
}

#[test]
fn exact_byte_and_exact_count_reservations_are_admitted_on_every_durable_path() {
    for path in DURABLE_PATHS {
        // Exact bytes: ceiling equals the conservative bound.
        let id = prepared_session(path);
        let bound = probe_bound(id, path, 1_000);
        bridge::set_outbox_ceilings(id, GENEROUS_MESSAGES, bound).unwrap();
        run_durable(id, path, 1_000)
            .unwrap_or_else(|error| panic!("{path:?} exact-byte must succeed: {error:?}"));
        let (count, bytes) = bridge::outbox_pending(id).unwrap().unwrap();
        assert_eq!(count, 1, "{path:?}");
        assert!(bytes <= bound, "{path:?}: {bytes} > {bound}");
        bridge::destroy_session(id);

        // Exact count: ceiling of one admits exactly one pending update.
        let id = prepared_session(path);
        bridge::set_outbox_ceilings(id, 1, GENEROUS_BYTES).unwrap();
        run_durable(id, path, 1_001)
            .unwrap_or_else(|error| panic!("{path:?} exact-count must succeed: {error:?}"));
        assert_eq!(
            bridge::outbox_pending(id).unwrap().unwrap().0,
            1,
            "{path:?}",
        );
        bridge::destroy_session(id);
    }
}

#[test]
fn one_over_byte_reservations_reject_atomically_on_every_durable_path() {
    for path in DURABLE_PATHS {
        let id = prepared_session(path);
        let bound = probe_bound(id, path, 1_100);
        assert!(
            bound > 0,
            "{path:?}: durable paths always carry a nonzero bound"
        );
        bridge::set_outbox_ceilings(id, GENEROUS_MESSAGES, bound - 1).unwrap();
        let before = bridge::session_audit(id).unwrap();
        let error =
            run_durable(id, path, 1_100).expect_err(&format!("{path:?} one-over-byte must reject"));
        assert_atomic_rejection(id, &before, &error, "OPERATION_LIMIT_EXCEEDED");
        bridge::destroy_session(id);
    }
}

#[test]
fn one_over_count_reservations_reject_atomically_on_every_durable_path() {
    for path in DURABLE_PATHS {
        let id = prepared_session(path);
        // Saturate the single admitted slot with a real pending update where
        // history state allows it; redo cannot survive a filler edit, so its
        // one-over case uses an exhausted zero ceiling instead.
        match path {
            DurablePath::Redo => {
                bridge::set_outbox_ceilings(id, 0, GENEROUS_BYTES).unwrap();
            }
            _ => {
                bridge::submit_input(id, &input_envelope(1_200, revision(id), "fill")).unwrap();
                bridge::set_outbox_ceilings(id, 1, GENEROUS_BYTES).unwrap();
            }
        }
        let before = bridge::session_audit(id).unwrap();
        let error = run_durable(id, path, 1_201)
            .expect_err(&format!("{path:?} one-over-count must reject"));
        assert_atomic_rejection(id, &before, &error, "OPERATION_LIMIT_EXCEEDED");
        bridge::destroy_session(id);
    }
}

#[test]
fn allocation_failure_injection_rejects_atomically_on_every_durable_path() {
    for path in DURABLE_PATHS {
        let id = prepared_session(path);
        let before = bridge::session_audit(id).unwrap();
        bridge::set_outbox_allocation_failure(true);
        let result = run_durable(id, path, 1_300);
        bridge::set_outbox_allocation_failure(false);
        let error = result.expect_err(&format!("{path:?} allocation failure must reject"));
        assert_atomic_rejection(id, &before, &error, "OPERATION_RESOURCE_EXHAUSTED");
        // Recovery: the identical operation succeeds once allocation recovers.
        run_durable(id, path, 1_301)
            .unwrap_or_else(|error| panic!("{path:?} must recover after injection: {error:?}"));
        assert_eq!(
            bridge::outbox_pending(id).unwrap().unwrap().0,
            1,
            "{path:?}"
        );
        bridge::destroy_session(id);
    }
}
