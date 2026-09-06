use super::*;
use crate::boundary::ResourceLimits;
use crate::collaboration_runtime::awareness::AwarenessContext;
use crate::session::{CollaborationLimits, TransportState};
use crate::yrs_engine::{EditingLimits, InitializationMode, YrsDocumentEngine, YrsEngineConfig};
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
        .set_desired_awareness_for_test(
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
        failure.error.details.as_ref().unwrap()["cause"]["details"]["requiresFreshEditorIdentity"],
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
