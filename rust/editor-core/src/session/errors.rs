#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ErrorDomain {
    Boundary,
    Document,
    Operation,
    Lifecycle,
    Snapshot,
    Transport,
}

impl ErrorDomain {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Boundary => "boundary",
            Self::Document => "document",
            Self::Operation => "operation",
            Self::Lifecycle => "lifecycle",
            Self::Snapshot => "snapshot",
            Self::Transport => "transport",
        }
    }
}

fn legacy_error_domain(code: &str) -> ErrorDomain {
    match code {
        "DOCUMENT_PARSE_FAILED"
        | "DOCUMENT_INVALID"
        | "DOCUMENT_LIMIT_EXCEEDED"
        | "POSITION_LIMIT_EXCEEDED"
        | "SCHEMA_INVALID"
        | "REQUIRED_ATTRIBUTE_MISSING"
        | "UNKNOWN_MARK"
        | "MAX_LENGTH_EXCEEDED" => ErrorDomain::Document,
        "ENGINE_NOT_READY"
        | "REVISION_MISMATCH"
        | "POSITION_INVALID"
        | "TRANSACTION_INVALID"
        | "OPERATION_INVALID"
        | "OPERATION_LIMIT_EXCEEDED"
        | "OPERATION_RESOURCE_EXHAUSTED"
        | "ENGINE_INVARIANT_FAILED" => ErrorDomain::Operation,
        _ => ErrorDomain::Boundary,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OperationFailureClass {
    ExistingStableCode,
    DeterministicOperationLimit,
    DeterministicDocumentLimit,
    AllocationOrReservation,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SessionError {
    pub(crate) domain: ErrorDomain,
    pub(crate) code: String,
    pub(crate) message: String,
    pub(crate) request_id: Option<u64>,
    pub(crate) operation_index: Option<usize>,
    pub(crate) limit: Option<u64>,
    pub(crate) actual: Option<u64>,
    pub(crate) details: Option<serde_json::Value>,
}

impl SessionError {
    pub(crate) fn new(
        domain: ErrorDomain,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            domain,
            code: code.into(),
            message: message.into(),
            request_id: None,
            operation_index: None,
            limit: None,
            actual: None,
            details: None,
        }
    }

    pub(crate) fn from_operation(
        error: OperationError,
        failure_class: OperationFailureClass,
    ) -> Self {
        let (domain, code) = match (error.code, failure_class) {
            (
                "OPERATION_RESOURCE_EXHAUSTED",
                OperationFailureClass::DeterministicOperationLimit,
            ) => (ErrorDomain::Operation, "OPERATION_LIMIT_EXCEEDED"),
            ("OPERATION_RESOURCE_EXHAUSTED", OperationFailureClass::DeterministicDocumentLimit) => {
                (ErrorDomain::Document, "DOCUMENT_LIMIT_EXCEEDED")
            }
            ("OPERATION_RESOURCE_EXHAUSTED", OperationFailureClass::ExistingStableCode) => {
                (ErrorDomain::Operation, "OPERATION_LIMIT_EXCEEDED")
            }
            _ => (legacy_error_domain(error.code), error.code),
        };
        Self {
            domain,
            code: code.into(),
            message: error.message.into(),
            request_id: Some(error.request_id),
            operation_index: error.operation_index,
            limit: error.limit,
            actual: error.actual,
            details: error.details,
        }
    }

    pub(crate) fn lifecycle(error: YrsEngineError) -> Self {
        Self::from_engine_error(error, ErrorDomain::Lifecycle)
    }

    pub(crate) fn snapshot(error: YrsEngineError) -> Self {
        Self::from_engine_error(error, ErrorDomain::Snapshot)
    }

    pub(crate) fn transport(error: YrsEngineError) -> Self {
        Self::from_engine_error(error, ErrorDomain::Transport)
    }

    fn from_engine_error(error: YrsEngineError, domain: ErrorDomain) -> Self {
        Self {
            domain,
            code: error.code.into(),
            message: error.message,
            request_id: None,
            operation_index: None,
            limit: error.limit.and_then(|value| u64::try_from(value).ok()),
            actual: error.actual.and_then(|value| u64::try_from(value).ok()),
            details: error.details.filter(serde_json::Value::is_object),
        }
    }

    fn with_limit(mut self, limit: usize, actual: usize) -> Self {
        self.limit = u64::try_from(limit).ok();
        self.actual = u64::try_from(actual).ok();
        self
    }

    fn with_details(mut self, details: serde_json::Value) -> Self {
        self.details = details.is_object().then_some(details);
        self
    }
}

impl From<BoundaryError> for SessionError {
    fn from(error: BoundaryError) -> Self {
        let domain = legacy_error_domain(error.code);
        Self {
            domain,
            code: error.code.into(),
            message: error.message,
            request_id: None,
            operation_index: None,
            limit: error.limit.and_then(|value| u64::try_from(value).ok()),
            actual: error.actual.and_then(|value| u64::try_from(value).ok()),
            details: error.details.filter(serde_json::Value::is_object),
        }
    }
}

impl From<YrsEngineError> for SessionError {
    fn from(error: YrsEngineError) -> Self {
        let domain = legacy_error_domain(error.code);
        Self::from_engine_error(error, domain)
    }
}
