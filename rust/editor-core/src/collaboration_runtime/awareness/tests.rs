use super::*;
use serde_json::json;

const REQUEST_ID: u64 = 88;

fn runtime() -> CollaborationRuntime {
    CollaborationRuntime::new(&CollaborationLimits::default())
}

fn engine() -> YrsDocumentEngine {
    use crate::boundary::ResourceLimits;
    use crate::yrs_engine::{EditingLimits, InitializationMode, YrsEngineConfig};
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

fn context<'a>(
    engine: &'a mut YrsDocumentEngine,
    transport_state: TransportState,
    limits: &'a CollaborationLimits,
) -> AwarenessContext<'a> {
    AwarenessContext {
        engine,
        transport_state,
        limits,
    }
}

#[test]
fn awareness_limits_mirror_the_session_collaboration_limit_fields() {
    let limits = CollaborationLimits {
        max_awareness_peers: 3,
        max_awareness_peer_bytes: 64,
        max_awareness_bytes: 256,
        ..CollaborationLimits::default()
    };
    assert_eq!(
        awareness_limits(&limits),
        AwarenessLimits {
            max_awareness_peers: 3,
            max_awareness_peer_bytes: 64,
            max_awareness_bytes: 256,
        },
    );
}

#[test]
fn local_intent_peer_byte_ceiling_rejects_before_json_deserialization() {
    let mut runtime = runtime();
    let mut engine = engine();
    let limits = CollaborationLimits {
        max_awareness_peer_bytes: 64,
        ..CollaborationLimits::default()
    };
    // This is deliberately invalid JSON. If serde_json is reached, the
    // refusal changes to AWARENESS_STATE_INVALID instead of the frozen
    // maxAwarenessPeerBytes resource-limit envelope.
    let oversized_intent = "[".repeat(limits.max_awareness_peer_bytes + 1);

    let error = runtime
        .set_awareness_intent(
            REQUEST_ID,
            &oversized_intent,
            context(&mut engine, TransportState::Disconnected, &limits),
        )
        .unwrap_err();

    assert_eq!(error.domain, ErrorDomain::Boundary, "{error:?}");
    assert_eq!(error.code, "INPUT_LIMIT_EXCEEDED", "{error:?}");
    assert_eq!(error.request_id, Some(REQUEST_ID), "{error:?}");
    assert_eq!(error.limit, Some(64), "{error:?}");
    assert_eq!(error.actual, Some(65), "{error:?}");
    assert_eq!(error.message, "input exceeds limit 64: 65", "{error:?}");
    assert_eq!(
        error.details.as_ref().unwrap()["field"],
        "maxAwarenessPeerBytes",
        "{error:?}",
    );
    assert_eq!(runtime.desired_awareness(), None);
    assert_eq!(runtime.outbox().pending_protocol_reply_count(), 0);
}

#[test]
fn next_deadline_is_the_minimum_of_renewal_and_earliest_expiry() {
    let mut state = AwarenessRuntimeState::new();
    assert_eq!(
        state.next_deadline_millis(TransportState::Synchronized),
        None
    );

    state.peer_activity.insert(7, 1_000);
    state.peer_activity.insert(8, 2_000);
    assert_eq!(
        state.next_deadline_millis(TransportState::Disconnected),
        Some(1_000 + AWARENESS_EXPIRY_MILLIS),
    );

    state.desired_state = Some(json!({"n": 1}));
    state.last_local_publish_millis = Some(4_000);
    // Renewal is earlier than the earliest expiry.
    assert_eq!(
        state.next_deadline_millis(TransportState::Synchronized),
        Some(4_000 + AWARENESS_RENEWAL_INTERVAL_MILLIS),
    );
    // A disconnected transport never schedules renewal.
    assert_eq!(
        state.next_deadline_millis(TransportState::Disconnected),
        Some(1_000 + AWARENESS_EXPIRY_MILLIS),
    );
    // An unpublished desired state on a synchronized transport is due at
    // the current deterministic time.
    state.last_local_publish_millis = None;
    state.now_millis = 9_000;
    assert_eq!(
        state.next_deadline_millis(TransportState::Synchronized),
        Some(9_000),
    );
}

#[test]
fn task8_fourth_remediation_local_max_timestamp_has_no_deadline() {
    let mut state = AwarenessRuntimeState::new();
    state.now_millis = u64::MAX;
    state.desired_state = Some(json!({"name": "local"}));
    state.last_local_publish_millis = Some(u64::MAX);

    assert_eq!(
        state.next_deadline_millis(TransportState::Synchronized),
        None,
    );
}

#[test]
fn task8_fourth_remediation_remote_max_timestamp_has_no_deadline() {
    let mut state = AwarenessRuntimeState::new();
    state.now_millis = u64::MAX;
    state.peer_activity.insert(7, u64::MAX);

    assert_eq!(
        state.next_deadline_millis(TransportState::Disconnected),
        None,
    );
}

#[test]
fn task8_fourth_remediation_mixed_deadlines_keep_the_representable_candidate() {
    let mut state = AwarenessRuntimeState::new();
    state.desired_state = Some(json!({"name": "local"}));
    state.last_local_publish_millis = Some(u64::MAX);
    state.peer_activity.insert(7, 1_000);
    assert_eq!(
        state.next_deadline_millis(TransportState::Synchronized),
        Some(1_000 + AWARENESS_EXPIRY_MILLIS),
    );

    state.last_local_publish_millis = Some(2_000);
    state.peer_activity.clear();
    state.peer_activity.insert(7, u64::MAX);
    assert_eq!(
        state.next_deadline_millis(TransportState::Synchronized),
        Some(2_000 + AWARENESS_RENEWAL_INTERVAL_MILLIS),
    );
}

#[test]
fn task8_fourth_remediation_tick_at_equal_max_does_no_false_clock_work() {
    let mut runtime = runtime();
    let mut engine = engine();
    let limits = CollaborationLimits::default();
    runtime.awareness.now_millis = u64::MAX;
    runtime.awareness.desired_state = Some(json!({"name": "local"}));
    runtime.awareness.last_local_publish_millis = Some(u64::MAX);
    runtime.awareness.peer_activity.insert(7, u64::MAX);

    let outcome = runtime
        .tick(
            REQUEST_ID,
            u64::MAX,
            context(&mut engine, TransportState::Synchronized, &limits),
        )
        .unwrap();

    assert!(!outcome.renewed_local, "{outcome:?}");
    assert!(!outcome.outbound_changed, "{outcome:?}");
    assert!(outcome.expired_peers.is_empty(), "{outcome:?}");
    assert!(!outcome.peers_changed, "{outcome:?}");
    assert_eq!(outcome.next_deadline_millis, None, "{outcome:?}");
    assert_eq!(runtime.awareness.last_local_publish_millis, Some(u64::MAX));
    assert_eq!(runtime.awareness.peer_activity.get(&7), Some(&u64::MAX));
    assert_eq!(runtime.outbox().pending_protocol_reply_count(), 0);
}

#[test]
fn reset_for_restore_clears_peer_bookkeeping_and_restarts_the_renewal_clock() {
    let mut state = AwarenessRuntimeState::new();
    state.now_millis = 9_000;
    state.desired_state = Some(json!({"n": 1}));
    state.last_local_publish_millis = Some(4_000);
    state.peer_activity.insert(7, 1_000);
    state.peer_activity.insert(8, 2_000);

    state.reset_for_restore();

    // The desired state survives; prior-store deadlines are gone.
    assert_eq!(state.desired_state, Some(json!({"n": 1})));
    assert!(state.peer_activity.is_empty());
    assert_eq!(state.last_local_publish_millis, None);
    // With no broadcast for the new store, a synchronized renewal is due
    // at the current deterministic time, never on the prior store's
    // schedule.
    assert_eq!(
        state.next_deadline_millis(TransportState::Synchronized),
        Some(9_000),
    );
    assert_eq!(
        state.next_deadline_millis(TransportState::Disconnected),
        None
    );
}

#[test]
fn peer_cursor_projection_requires_two_resolvable_sticky_points() {
    let mut engine = engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"unit seed"}]}]}"#,
            crate::yrs_engine::TransactionOrigin::DocumentImport,
        )
        .unwrap();

    // Non-object states, missing cursors, and malformed points degrade.
    assert_eq!(peer_cursor_projection(&engine, &json!("plain")), None);
    assert_eq!(peer_cursor_projection(&engine, &json!({"name": "x"})), None);
    assert_eq!(
        peer_cursor_projection(
            &engine,
            &json!({"cursor": {"anchor": {"bogus": true}, "head": {"bogus": true}}}),
        ),
        None,
    );
    assert_eq!(
        peer_cursor_projection(&engine, &json!({"cursor": {"anchor": 1}})),
        None,
        "a cursor missing its head never resolves half a projection",
    );
}

#[test]
fn broadcast_reservation_errors_split_per_the_saturation_ruling() {
    let saturated = broadcast_reservation_error(
        OutboxReservationError::Saturated {
            field: super::super::outbox::OUTBOX_MESSAGES_FIELD,
            limit: 2,
            actual: 3,
        },
        REQUEST_ID,
    );
    assert_eq!(saturated.code, TRANSPORT_REPLY_LIMIT_EXCEEDED);
    assert_eq!(saturated.domain, ErrorDomain::Transport);
    assert_eq!(saturated.request_id, Some(REQUEST_ID));
    assert_eq!(saturated.limit, Some(2));
    assert_eq!(saturated.actual, Some(3));

    let allocation = broadcast_reservation_error(OutboxReservationError::Allocation, REQUEST_ID);
    assert_eq!(allocation.code, TRANSPORT_RESOURCE_EXHAUSTED);
    assert_eq!(allocation.request_id, Some(REQUEST_ID));
}

#[test]
fn set_desired_awareness_rejects_invalid_json_atomically() {
    let mut runtime = runtime();
    let mut engine = engine();
    let limits = CollaborationLimits::default();

    let error = runtime
        .set_desired_awareness_for_test(
            REQUEST_ID,
            "{broken",
            context(&mut engine, TransportState::Disconnected, &limits),
        )
        .unwrap_err();
    assert_eq!(error.code, AWARENESS_STATE_INVALID);
    assert_eq!(error.request_id, Some(REQUEST_ID));
    assert_eq!(runtime.desired_awareness(), None);

    runtime
        .set_desired_awareness_for_test(
            REQUEST_ID,
            r#"{"name":"kept"}"#,
            context(&mut engine, TransportState::Disconnected, &limits),
        )
        .unwrap();
    assert_eq!(runtime.desired_awareness(), Some(&json!({"name": "kept"})));
    // Disconnected transports retain without broadcasting.
    assert_eq!(runtime.outbox().pending_protocol_reply_count(), 0);
}

#[test]
fn tick_expires_tracked_peers_and_reports_the_next_deadline() {
    let mut runtime = runtime();
    let mut engine = engine();
    let limits = CollaborationLimits::default();

    // Install two peers through the runtime path so activity is tracked.
    runtime
        .tick(
            REQUEST_ID,
            0,
            context(&mut engine, TransportState::Disconnected, &limits),
        )
        .unwrap();
    let update = |client: u64, clock: u32| {
        use yrs::updates::encoder::Encode as _;
        let mut clients = std::collections::HashMap::new();
        clients.insert(
            yrs::ClientID::new(client),
            yrs::sync::awareness::AwarenessUpdateEntry {
                clock,
                json: r#"{"u":1}"#.into(),
            },
        );
        yrs::sync::awareness::AwarenessUpdate { clients }.encode_v1()
    };
    runtime
        .apply_awareness_frame(&mut engine, &limits, &update(21, 1))
        .unwrap();
    let outcome = runtime
        .tick(
            REQUEST_ID,
            10_000,
            context(&mut engine, TransportState::Disconnected, &limits),
        )
        .unwrap();
    assert_eq!(outcome.expired_peers, Vec::<u64>::new());
    runtime
        .apply_awareness_frame(&mut engine, &limits, &update(22, 1))
        .unwrap();

    // Exactly at the boundary peer 21 expires; peer 22 (seen at 10s)
    // stays and owns the next deadline.
    let outcome = runtime
        .tick(
            REQUEST_ID,
            AWARENESS_EXPIRY_MILLIS,
            context(&mut engine, TransportState::Disconnected, &limits),
        )
        .unwrap();
    assert_eq!(outcome.expired_peers, vec![21]);
    assert!(!outcome.renewed_local);
    assert_eq!(
        outcome.next_deadline_millis,
        Some(10_000 + AWARENESS_EXPIRY_MILLIS),
    );
    assert_eq!(runtime.peers(&mut engine).len(), 1);
}

#[test]
fn task8_third_remediation_runtime_renewal_marks_local_peer_changed() {
    let mut runtime = runtime();
    let mut engine = engine();
    let limits = CollaborationLimits::default();

    runtime
        .set_desired_awareness_for_test(
            REQUEST_ID,
            r#"{"name":"renewed"}"#,
            context(&mut engine, TransportState::Synchronized, &limits),
        )
        .unwrap();
    let before = runtime.peers(&mut engine);

    let outcome = runtime
        .tick(
            REQUEST_ID,
            AWARENESS_RENEWAL_INTERVAL_MILLIS,
            context(&mut engine, TransportState::Synchronized, &limits),
        )
        .unwrap();
    let after = runtime.peers(&mut engine);

    assert!(outcome.renewed_local, "{outcome:?}");
    assert!(outcome.outbound_changed, "{outcome:?}");
    assert!(outcome.expired_peers.is_empty(), "{outcome:?}");
    assert!(outcome.peers_changed, "{outcome:?}");
    assert_eq!(before.len(), 1);
    assert_eq!(after.len(), 1);
    assert!(after[0].clock > before[0].clock, "{before:?} -> {after:?}");
}

fn assert_clock_exhausted(error: &SessionError) {
    assert_eq!(error.domain, ErrorDomain::Transport, "{error:?}");
    assert_eq!(error.code, "AWARENESS_CLOCK_EXHAUSTED", "{error:?}");
    assert_eq!(error.request_id, Some(REQUEST_ID), "{error:?}");
    assert!(
        error.message.contains("fresh editor identity is required"),
        "{error:?}",
    );
    assert_eq!(
        error.details.as_ref().unwrap()["requiresFreshEditorIdentity"],
        true,
        "{error:?}",
    );
    assert_eq!(
        error.details.as_ref().unwrap()["retryable"],
        false,
        "{error:?}",
    );
}

fn runtime_with_clock(clock: u32) -> (CollaborationRuntime, YrsDocumentEngine) {
    let mut runtime = runtime();
    let mut engine = engine();
    let limits = CollaborationLimits::default();
    runtime
        .set_desired_awareness_for_test(
            REQUEST_ID,
            r#"{"name":"before"}"#,
            context(&mut engine, TransportState::Disconnected, &limits),
        )
        .unwrap();
    engine.awareness().set_live_local_clock_for_test(clock);
    (runtime, engine)
}

#[test]
fn set_and_renew_report_clock_exhaustion_without_mutating_state_or_outbox() {
    let limits = CollaborationLimits::default();
    for clock in [u32::MAX - 1, u32::MAX] {
        let (mut runtime, mut engine) = runtime_with_clock(clock);
        let before_peers = runtime.peers(&mut engine);
        let before_replies = runtime.outbox().pending_protocol_reply_count();

        let error = runtime
            .set_desired_awareness_for_test(
                REQUEST_ID,
                r#"{"name":"after"}"#,
                context(&mut engine, TransportState::Synchronized, &limits),
            )
            .unwrap_err();
        assert_clock_exhausted(&error);
        assert_eq!(runtime.peers(&mut engine), before_peers);
        assert_eq!(
            runtime.outbox().pending_protocol_reply_count(),
            before_replies,
        );

        runtime.awareness.last_local_publish_millis = Some(0);
        let error = runtime
            .tick(
                REQUEST_ID,
                AWARENESS_RENEWAL_INTERVAL_MILLIS,
                context(&mut engine, TransportState::Synchronized, &limits),
            )
            .unwrap_err();
        assert_clock_exhausted(&error);
        assert_eq!(runtime.peers(&mut engine), before_peers);
        assert_eq!(
            runtime.outbox().pending_protocol_reply_count(),
            before_replies,
        );
    }
}

#[test]
fn reconnect_publication_reports_clock_exhaustion_without_enqueuing() {
    let limits = CollaborationLimits::default();
    for clock in [u32::MAX - 1, u32::MAX] {
        let (mut runtime, mut engine) = runtime_with_clock(clock);
        let before_peers = runtime.peers(&mut engine);

        let error = runtime
            .prepare_handshake_republish(&mut engine, &limits)
            .unwrap_err();

        assert_eq!(error.code, "AWARENESS_CLOCK_EXHAUSTED", "{error:?}");
        assert_eq!(runtime.peers(&mut engine), before_peers);
        assert_eq!(runtime.outbox().pending_protocol_reply_count(), 0);
    }
}
