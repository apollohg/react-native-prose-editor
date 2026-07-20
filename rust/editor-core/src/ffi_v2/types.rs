use crate::session::{ErrorDomain, SessionError};

pub(crate) const ERROR_DOMAINS: [&str; 6] = [
    "boundary",
    "document",
    "operation",
    "lifecycle",
    "snapshot",
    "transport",
];

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
pub(crate) struct FfiError {
    pub domain: String,
    pub code: String,
    pub message: String,
    pub request_id: Option<String>,
    pub operation_index: Option<u64>,
    pub limit: Option<u64>,
    pub actual: Option<u64>,
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
                .and_then(|value| u64::try_from(value).ok()),
            limit: error.limit,
            actual: error.actual,
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
        pub(crate) struct $name {
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

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub(crate) struct FfiUnitResult {
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
