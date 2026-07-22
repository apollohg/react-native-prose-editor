use crate::session::{ErrorDomain, SessionError};

/// The one JSON representation for every u64-shaped v2 wire field.
pub(crate) fn decimal_u64(value: u64) -> serde_json::Value {
    serde_json::Value::String(value.to_string())
}

/// Parse only the frozen decimal u64 spelling: ASCII digits, no leading zero
/// except zero itself, and no sign, exponent, fraction, or whitespace.
pub(crate) fn parse_canonical_u64(value: &str) -> Option<u64> {
    let bytes = value.as_bytes();
    if bytes.is_empty()
        || (bytes.len() > 1 && bytes[0] == b'0')
        || !bytes.iter().all(u8::is_ascii_digit)
    {
        return None;
    }
    value.parse().ok()
}

pub(crate) fn deserialize_canonical_u64<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = <String as serde::Deserialize>::deserialize(deserializer)?;
    parse_canonical_u64(&value)
        .ok_or_else(|| serde::de::Error::custom("expected a canonical decimal u64 string"))
}

/// Frozen non-retryable transport code exposed across the FFI boundary when
/// the current editor identity cannot advance its awareness clock safely.
pub const AWARENESS_CLOCK_EXHAUSTED: &str = "AWARENESS_CLOCK_EXHAUSTED";

#[cfg(test)]
pub(crate) const ERROR_DOMAINS: [&str; 6] = [
    "boundary",
    "document",
    "operation",
    "lifecycle",
    "snapshot",
    "transport",
];

#[cfg(test)]
pub(crate) const OPERATION_ERROR_CODES: [&str; 10] = [
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
];

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FfiError {
    pub domain: String,
    pub code: String,
    pub message: String,
    pub request_id: Option<String>,
    pub operation_index: Option<String>,
    pub limit: Option<String>,
    pub actual: Option<String>,
    pub details_json: Option<String>,
}

impl FfiError {
    pub(crate) fn new(
        domain: ErrorDomain,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            domain: domain.as_str().into(),
            code: code.into(),
            message: message.into(),
            request_id: None,
            operation_index: None,
            limit: None,
            actual: None,
            details_json: None,
        }
    }
}

impl From<SessionError> for FfiError {
    fn from(error: SessionError) -> Self {
        Self {
            domain: error.domain.as_str().into(),
            code: error.code,
            message: error.message,
            request_id: error.request_id.map(|value| value.to_string()),
            operation_index: error
                .operation_index
                .and_then(|value| u64::try_from(value).ok())
                .map(|value| value.to_string()),
            limit: error.limit.map(|value| value.to_string()),
            actual: error.actual.map(|value| value.to_string()),
            details_json: error
                .details
                .filter(serde_json::Value::is_object)
                .and_then(|details| serde_json::to_string(&details).ok()),
        }
    }
}

fn exactly_one<T>(value: &Option<T>, error: &Option<FfiError>) -> Result<(), &'static str> {
    if value.is_some() == error.is_some() {
        return Err("FFI result must contain exactly one of value or error");
    }
    Ok(())
}

macro_rules! ffi_result {
    ($name:ident, $value:ty) => {
        #[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
        pub struct $name {
            pub value: Option<$value>,
            pub error: Option<FfiError>,
        }

        impl $name {
            pub(crate) fn try_new(
                value: Option<$value>,
                error: Option<FfiError>,
            ) -> Result<Self, &'static str> {
                exactly_one(&value, &error)?;
                Ok(Self { value, error })
            }

            pub(crate) fn ok(value: $value) -> Self {
                Self::try_new(Some(value), None).expect("value-only result is valid")
            }

            pub(crate) fn err(error: FfiError) -> Self {
                Self::try_new(None, Some(error)).expect("error-only result is valid")
            }
        }
    };
}

ffi_result!(FfiJsonResult, String);
ffi_result!(FfiBytesResult, Vec<u8>);

/// One exported document snapshot: the five-field manifest as JSON plus the
/// encoded state as direct bytes (never a JSON number array).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FfiSnapshotExport {
    pub metadata_json: String,
    pub encoded_state: Vec<u8>,
}

ffi_result!(FfiSnapshotExportResult, FfiSnapshotExport);

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FfiUnitResult {
    pub value: Option<bool>,
    pub error: Option<FfiError>,
}

impl FfiUnitResult {
    pub(crate) fn try_new(
        value: Option<bool>,
        error: Option<FfiError>,
    ) -> Result<Self, &'static str> {
        if value == Some(false) {
            return Err("FFI unit success must use Some(true)");
        }
        exactly_one(&value, &error)?;
        Ok(Self { value, error })
    }

    pub(crate) fn ok() -> Self {
        Self::try_new(Some(true), None).expect("unit success is valid")
    }

    pub(crate) fn err(error: FfiError) -> Self {
        Self::try_new(None, Some(error)).expect("error-only result is valid")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::boundary::ResourceLimits;
    use crate::collaboration_runtime::awareness::AwarenessContext;
    use crate::collaboration_runtime::CollaborationRuntime;
    use crate::session::{CollaborationLimits, TransportState};
    use crate::yrs_engine::{
        EditingLimits, InitializationMode, YrsDocumentEngine, YrsEngineConfig,
    };

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
    fn awareness_clock_exhaustion_code_and_identity_recovery_details_are_frozen() {
        let limits = CollaborationLimits::default();
        let mut engine = engine();
        let mut runtime = CollaborationRuntime::new(&limits);
        runtime
            .set_desired_awareness(
                91,
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
        let session_error = runtime
            .set_desired_awareness(
                92,
                r#"{"name":"after"}"#,
                AwarenessContext {
                    engine: &mut engine,
                    transport_state: TransportState::Synchronized,
                    limits: &limits,
                },
            )
            .unwrap_err();
        assert_eq!(session_error.code, AWARENESS_CLOCK_EXHAUSTED);
        let error = FfiError::from(session_error);

        assert_eq!(error.domain, "transport");
        assert_eq!(error.code, "AWARENESS_CLOCK_EXHAUSTED");
        assert_eq!(error.request_id.as_deref(), Some("92"));
        assert!(error.message.contains("fresh editor identity is required"));
        let details: serde_json::Value =
            serde_json::from_str(error.details_json.as_deref().unwrap()).unwrap();
        assert_eq!(details["requiresFreshEditorIdentity"], true);
        assert_eq!(details["retryable"], false);
    }
}
