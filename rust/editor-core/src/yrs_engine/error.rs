use crate::boundary::BoundaryError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct YrsEngineError {
    pub code: &'static str,
    pub message: String,
    pub limit: Option<usize>,
    pub actual: Option<usize>,
    pub details: Option<serde_json::Value>,
}

pub type YrsEngineResult<T> = Result<T, YrsEngineError>;

impl YrsEngineError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            limit: None,
            actual: None,
            details: None,
        }
    }

    pub fn parse(code: &'static str, error: impl std::fmt::Display) -> Self {
        Self::new(code, error.to_string())
    }

    pub fn limit(code: &'static str, limit: usize, actual: usize) -> Self {
        Self {
            code,
            message: format!("input exceeds limit {limit}: {actual}"),
            limit: Some(limit),
            actual: Some(actual),
            details: None,
        }
    }

    pub fn with_details(mut self, details: serde_json::Value) -> Self {
        self.details = Some(details);
        self
    }
}

impl std::fmt::Display for YrsEngineError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for YrsEngineError {}

impl From<BoundaryError> for YrsEngineError {
    fn from(error: BoundaryError) -> Self {
        Self {
            code: error.code,
            message: error.message,
            limit: error.limit,
            actual: error.actual,
            details: error.details,
        }
    }
}

impl From<YrsEngineError> for BoundaryError {
    fn from(error: YrsEngineError) -> Self {
        Self {
            code: error.code,
            message: error.message,
            limit: error.limit,
            actual: error.actual,
            details: error.details,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::YrsEngineError;
    use crate::boundary::BoundaryError;

    #[test]
    fn error_constructors_keep_stable_codes_and_structured_fields() {
        let new = YrsEngineError::new("ENGINE_FAILED", "engine failed")
            .with_details(serde_json::json!({ "operation": "apply" }));
        assert_eq!(new.code, "ENGINE_FAILED");
        assert_eq!(new.message, "engine failed");
        assert_eq!(new.limit, None);
        assert_eq!(new.actual, None);
        assert_eq!(
            new.details,
            Some(serde_json::json!({ "operation": "apply" }))
        );

        let parse = YrsEngineError::parse("ENGINE_PARSE_FAILED", "invalid update");
        assert_eq!(parse.code, "ENGINE_PARSE_FAILED");
        assert_eq!(parse.message, "invalid update");

        let limit = YrsEngineError::limit("ENGINE_LIMIT_EXCEEDED", 8, 13);
        assert_eq!(limit.code, "ENGINE_LIMIT_EXCEEDED");
        assert_eq!(limit.message, "input exceeds limit 8: 13");
        assert_eq!(limit.limit, Some(8));
        assert_eq!(limit.actual, Some(13));
    }

    #[test]
    fn boundary_error_conversion_is_lossless_in_both_directions() {
        let boundary = BoundaryError {
            code: "DOCUMENT_INVALID",
            message: "document is invalid".into(),
            limit: Some(21),
            actual: Some(34),
            details: Some(serde_json::json!({ "node": "paragraph" })),
        };

        let engine = YrsEngineError::from(boundary);
        assert_eq!(engine.code, "DOCUMENT_INVALID");
        assert_eq!(engine.message, "document is invalid");
        assert_eq!(engine.limit, Some(21));
        assert_eq!(engine.actual, Some(34));
        assert_eq!(
            engine.details,
            Some(serde_json::json!({ "node": "paragraph" }))
        );

        let boundary = BoundaryError::from(engine);
        assert_eq!(boundary.code, "DOCUMENT_INVALID");
        assert_eq!(boundary.message, "document is invalid");
        assert_eq!(boundary.limit, Some(21));
        assert_eq!(boundary.actual, Some(34));
        assert_eq!(
            boundary.details,
            Some(serde_json::json!({ "node": "paragraph" }))
        );
    }
}
