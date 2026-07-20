use serde_json::json;

use crate::boundary::{BoundaryError, ResourceLimits};
use crate::session::{
    CollaborationLimits, DocumentState, ErrorDomain, OperationFailureClass, SessionError,
    TransportState,
};
use crate::yrs_engine::{EditingLimits, OperationError, YrsEngineError};

use super::types::{
    FfiBytesResult, FfiError, FfiJsonResult, FfiUnitResult, ERROR_DOMAINS, OPERATION_ERROR_CODES,
};

fn shared_contract() -> serde_json::Value {
    let fixtures: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../scripts/tests/security-contract-fixtures.json"
    ))
    .unwrap();
    fixtures["ffiV2ErrorContract"].clone()
}

#[test]
fn freezes_document_and_transport_states() {
    assert_eq!(
        [
            DocumentState::LocalReady,
            DocumentState::AwaitRemote,
            DocumentState::RoomReady,
        ]
        .map(DocumentState::as_str),
        ["LocalReady", "AwaitRemote", "RoomReady"]
    );
    assert_eq!(
        [
            TransportState::Detached,
            TransportState::Disconnected,
            TransportState::Connecting,
            TransportState::Handshaking,
            TransportState::Synchronized,
            TransportState::Incompatible,
            TransportState::Destroying,
            TransportState::Destroyed,
        ]
        .map(TransportState::as_str),
        [
            "Detached",
            "Disconnected",
            "Connecting",
            "Handshaking",
            "Synchronized",
            "Incompatible",
            "Destroying",
            "Destroyed",
        ]
    );
}

#[test]
fn freezes_all_error_domains_and_operation_codes() {
    let fixture = shared_contract();
    assert_eq!(
        ERROR_DOMAINS,
        [
            "boundary",
            "document",
            "operation",
            "lifecycle",
            "snapshot",
            "transport",
        ]
    );
    assert_eq!(fixture["domains"], json!(ERROR_DOMAINS));
    assert_eq!(
        OPERATION_ERROR_CODES,
        [
            "ENGINE_NOT_READY",
            "REVISION_MISMATCH",
            "POSITION_INVALID",
            "TRANSACTION_INVALID",
            "OPERATION_INVALID",
            "OPERATION_LIMIT_EXCEEDED",
            "OPERATION_RESOURCE_EXHAUSTED",
            "DOCUMENT_INVALID",
            "DOCUMENT_LIMIT_EXCEEDED",
            "ENGINE_INVARIANT_FAILED",
        ]
    );
    assert_eq!(fixture["operationCodes"], json!(OPERATION_ERROR_CODES));
    assert_eq!(
        fixture["representativeCodes"],
        json!({
            "lifecycle": [
                "ENGINE_DESTROYING",
                "ENGINE_DESTROYED",
                "WHOLE_DOCUMENT_REPLACEMENT_CONNECTED"
            ],
            "snapshot": ["SNAPSHOT_RESTORE_CONNECTED"],
            "transport": ["TRANSPORT_PROTOCOL_INVALID"]
        })
    );
}

#[test]
fn legacy_error_conversions_share_the_complete_code_domain_mapping() {
    let operation_cases = [
        ("ENGINE_NOT_READY", ErrorDomain::Operation),
        ("REVISION_MISMATCH", ErrorDomain::Operation),
        ("POSITION_INVALID", ErrorDomain::Operation),
        ("TRANSACTION_INVALID", ErrorDomain::Operation),
        ("OPERATION_INVALID", ErrorDomain::Operation),
        ("OPERATION_LIMIT_EXCEEDED", ErrorDomain::Operation),
        ("OPERATION_RESOURCE_EXHAUSTED", ErrorDomain::Operation),
        ("DOCUMENT_INVALID", ErrorDomain::Document),
        ("DOCUMENT_LIMIT_EXCEEDED", ErrorDomain::Document),
        ("ENGINE_INVARIANT_FAILED", ErrorDomain::Operation),
    ];
    for (code, expected_domain) in operation_cases {
        let boundary = SessionError::from(BoundaryError {
            code,
            message: code.into(),
            limit: None,
            actual: None,
            details: None,
        });
        assert_eq!(
            boundary.domain, expected_domain,
            "boundary conversion: {code}"
        );

        let engine = SessionError::from(YrsEngineError::new(code, code));
        assert_eq!(engine.domain, expected_domain, "engine conversion: {code}");
    }

    for (code, expected_domain) in [
        ("CONFIG_INVALID", ErrorDomain::Boundary),
        ("DOCUMENT_PARSE_FAILED", ErrorDomain::Document),
        ("SCHEMA_INVALID", ErrorDomain::Document),
        ("MAX_LENGTH_EXCEEDED", ErrorDomain::Document),
        ("SESSION_NOT_FOUND", ErrorDomain::Boundary),
    ] {
        let boundary = SessionError::from(BoundaryError {
            code,
            message: code.into(),
            limit: None,
            actual: None,
            details: None,
        });
        assert_eq!(
            boundary.domain, expected_domain,
            "boundary conversion: {code}"
        );

        let engine = SessionError::from(YrsEngineError::new(code, code));
        assert_eq!(engine.domain, expected_domain, "engine conversion: {code}");
    }
}

#[test]
fn operation_errors_cross_session_conversion_with_frozen_codes_and_domains() {
    for (index, code) in OPERATION_ERROR_CODES.into_iter().enumerate() {
        let failure_class = if code == "OPERATION_RESOURCE_EXHAUSTED" {
            OperationFailureClass::AllocationOrReservation
        } else {
            OperationFailureClass::ExistingStableCode
        };
        let expected_domain = if matches!(code, "DOCUMENT_INVALID" | "DOCUMENT_LIMIT_EXCEEDED") {
            ErrorDomain::Document
        } else {
            ErrorDomain::Operation
        };
        let converted = SessionError::from_operation(
            OperationError {
                code,
                message: code.into(),
                request_id: 100 + index as u64,
                operation_index: None,
                limit: None,
                actual: None,
                details: None,
            },
            failure_class,
        );

        assert_eq!(converted.code, code, "code: {code}");
        assert_eq!(converted.domain, expected_domain, "domain: {code}");
    }
}

#[test]
fn lifecycle_snapshot_and_transport_domains_require_explicit_construction() {
    type ErrorConstructor = fn(YrsEngineError) -> SessionError;
    type ErrorDomainCase = (&'static str, ErrorDomain, ErrorConstructor);

    let cases: [ErrorDomainCase; 3] = [
        (
            "ENGINE_DESTROYED",
            ErrorDomain::Lifecycle,
            SessionError::lifecycle,
        ),
        (
            "SNAPSHOT_RESTORE_CONNECTED",
            ErrorDomain::Snapshot,
            SessionError::snapshot,
        ),
        (
            "TRANSPORT_PROTOCOL_INVALID",
            ErrorDomain::Transport,
            SessionError::transport,
        ),
    ];

    for (code, expected_domain, constructor) in cases {
        let legacy = SessionError::from(YrsEngineError::new(code, code));
        assert_eq!(
            legacy.domain,
            ErrorDomain::Boundary,
            "legacy conversion: {code}"
        );

        let explicit = constructor(YrsEngineError::new(code, code));
        assert_eq!(
            explicit.domain, expected_domain,
            "explicit construction: {code}"
        );
        assert_eq!(explicit.code, code);
    }
}

#[test]
fn ffi_results_enforce_exactly_one_value_or_error() {
    let error = FfiError::new(ErrorDomain::Boundary, "CONFIG_INVALID", "invalid config");

    let json = FfiJsonResult::ok("{}".into());
    assert_eq!(json.value, Some("{}".into()));
    assert_eq!(json.error, None);
    assert!(FfiJsonResult::try_new(None, None).is_err());
    assert!(FfiJsonResult::try_new(Some("{}".into()), Some(error.clone())).is_err());

    let bytes = FfiBytesResult::ok(vec![1, 2, 3]);
    assert_eq!(bytes.value, Some(vec![1, 2, 3]));
    assert_eq!(bytes.error, None);
    assert!(FfiBytesResult::try_new(None, None).is_err());
    assert!(FfiBytesResult::try_new(Some(vec![]), Some(error.clone())).is_err());

    let unit = FfiUnitResult::ok();
    assert_eq!(unit.value, Some(true));
    assert_eq!(unit.error, None);
    assert!(FfiUnitResult::try_new(None, None).is_err());
    assert!(FfiUnitResult::try_new(Some(false), None).is_err());
    assert!(FfiUnitResult::try_new(Some(true), Some(error)).is_err());
}

#[test]
fn ffi_error_conversion_uses_decimal_request_ids_and_true_nullability() {
    let operation = OperationError {
        code: "OPERATION_LIMIT_EXCEEDED",
        message: "operation limit exceeded".into(),
        request_id: u64::MAX,
        operation_index: Some(7),
        limit: Some(256),
        actual: Some(257),
        details: Some(json!({ "field": "maxOperationsPerTransaction" })),
    };
    let rich = FfiError::from(SessionError::from_operation(
        operation,
        OperationFailureClass::ExistingStableCode,
    ));
    assert_eq!(rich.domain, "operation");
    assert_eq!(rich.request_id.as_deref(), Some("18446744073709551615"));
    assert_eq!(rich.operation_index, Some(7));
    assert_eq!(rich.limit, Some(256));
    assert_eq!(rich.actual, Some(257));
    assert_eq!(
        rich.details_json.as_deref(),
        Some(r#"{"field":"maxOperationsPerTransaction"}"#)
    );

    let empty = FfiError::new(ErrorDomain::Lifecycle, "ENGINE_DESTROYED", "destroyed");
    assert_eq!(empty.request_id, None);
    assert_eq!(empty.operation_index, None);
    assert_eq!(empty.limit, None);
    assert_eq!(empty.actual, None);
    assert_eq!(empty.details_json, None);
}

#[test]
fn deterministic_limits_remap_but_allocation_failures_keep_resource_exhausted() {
    let resource_error = || OperationError {
        code: "OPERATION_RESOURCE_EXHAUSTED",
        message: "reservation failed".into(),
        request_id: 9,
        operation_index: Some(3),
        limit: Some(8),
        actual: Some(9),
        details: Some(json!({ "field": "work" })),
    };

    let operation = SessionError::from_operation(
        resource_error(),
        OperationFailureClass::DeterministicOperationLimit,
    );
    assert_eq!(operation.domain, ErrorDomain::Operation);
    assert_eq!(operation.code, "OPERATION_LIMIT_EXCEEDED");

    let document = SessionError::from_operation(
        resource_error(),
        OperationFailureClass::DeterministicDocumentLimit,
    );
    assert_eq!(document.domain, ErrorDomain::Document);
    assert_eq!(document.code, "DOCUMENT_LIMIT_EXCEEDED");

    let allocation = SessionError::from_operation(
        resource_error(),
        OperationFailureClass::AllocationOrReservation,
    );
    assert_eq!(allocation.domain, ErrorDomain::Operation);
    assert_eq!(allocation.code, "OPERATION_RESOURCE_EXHAUSTED");
}

#[test]
fn collaboration_limits_accept_the_ceiling_and_reject_zero_and_one_over() {
    let defaults = CollaborationLimits::default();
    let ceilings = CollaborationLimits::hard_ceiling();
    let fixture = shared_contract();
    assert_eq!(
        defaults.as_pairs_json(),
        fixture["collaborationLimits"]["defaults"]
    );
    assert_eq!(
        ceilings.as_pairs_json(),
        fixture["collaborationLimits"]["ceilings"]
    );
    defaults.validate().unwrap();
    ceilings.validate().unwrap();

    for field in CollaborationLimits::fields() {
        let mut zero = defaults.clone();
        zero.set_for_test(field, 0);
        let error = zero.validate().unwrap_err();
        assert_eq!(error.domain, ErrorDomain::Boundary, "{field}");
        assert_eq!(error.code, "INVALID_RESOURCE_LIMIT", "{field}");

        let mut one_over = defaults.clone();
        one_over.set_for_test(field, ceilings.value(field) + 1);
        let error = one_over.validate().unwrap_err();
        assert_eq!(error.code, "INVALID_RESOURCE_LIMIT", "{field}");
        assert_eq!(error.limit, Some(ceilings.value(field) as u64), "{field}");
        assert_eq!(
            error.actual,
            Some(ceilings.value(field) as u64 + 1),
            "{field}"
        );
    }
}

#[test]
fn session_config_reuses_existing_resolved_limit_types() {
    fn assert_resource_limits(_: &ResourceLimits) {}
    fn assert_editing_limits(_: &EditingLimits) {}
    fn assert_collaboration_limits(_: &CollaborationLimits) {}

    let config = crate::session::EditorSessionConfig::local_for_test();
    assert_resource_limits(&config.resource_limits);
    assert_editing_limits(&config.editing_limits);
    assert_collaboration_limits(&config.collaboration_limits);
    assert_eq!(config.fragment_name, "prosemirror");
    assert_eq!(config.input_filter, None);
}
