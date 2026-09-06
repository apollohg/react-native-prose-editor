#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResourceLimits {
    pub max_input_bytes: usize,
    pub max_document_nodes: usize,
    pub max_document_depth: usize,
    pub max_schema_nodes: usize,
    pub max_schema_expression_bytes: usize,
    pub max_collaboration_message_bytes: usize,
    pub max_encoded_state_bytes: usize,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ResourceLimitOverrides {
    #[serde(default, deserialize_with = "deserialize_non_null_option")]
    pub(crate) max_input_bytes: Option<usize>,
    #[serde(default, deserialize_with = "deserialize_non_null_option")]
    pub(crate) max_document_nodes: Option<usize>,
    #[serde(default, deserialize_with = "deserialize_non_null_option")]
    pub(crate) max_document_depth: Option<usize>,
    #[serde(default, deserialize_with = "deserialize_non_null_option")]
    pub(crate) max_schema_nodes: Option<usize>,
    #[serde(default, deserialize_with = "deserialize_non_null_option")]
    pub(crate) max_schema_expression_bytes: Option<usize>,
    #[serde(default, deserialize_with = "deserialize_non_null_option")]
    pub(crate) max_collaboration_message_bytes: Option<usize>,
    #[serde(default, deserialize_with = "deserialize_non_null_option")]
    pub(crate) max_encoded_state_bytes: Option<usize>,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            max_input_bytes: 20 * 1024 * 1024,
            max_document_nodes: 100_000,
            max_document_depth: 256,
            max_schema_nodes: 1_024,
            max_schema_expression_bytes: 64 * 1024,
            max_collaboration_message_bytes: 10 * 1024 * 1024,
            max_encoded_state_bytes: 50 * 1024 * 1024,
        }
    }
}

impl ResourceLimits {
    // Not reachable from production call paths after the Task 16C legacy runtime
    // removal; exercised by crate tests.
    #[allow(dead_code)]
    pub fn try_from_config(value: Option<&serde_json::Value>) -> BoundaryResult<Self> {
        let overrides = match value {
            Some(value) => serde_json::from_value::<ResourceLimitOverrides>(value.clone())
                .map_err(|error| BoundaryError::parse("INVALID_RESOURCE_LIMIT", error))?,
            None => ResourceLimitOverrides::default(),
        };
        Self::resolve(overrides)
    }

    pub(crate) fn resolve(overrides: ResourceLimitOverrides) -> BoundaryResult<Self> {
        let defaults = Self::default();
        let limits = Self {
            max_input_bytes: overrides
                .max_input_bytes
                .unwrap_or(defaults.max_input_bytes),
            max_document_nodes: overrides
                .max_document_nodes
                .unwrap_or(defaults.max_document_nodes),
            max_document_depth: overrides
                .max_document_depth
                .unwrap_or(defaults.max_document_depth),
            max_schema_nodes: overrides
                .max_schema_nodes
                .unwrap_or(defaults.max_schema_nodes),
            max_schema_expression_bytes: overrides
                .max_schema_expression_bytes
                .unwrap_or(defaults.max_schema_expression_bytes),
            max_collaboration_message_bytes: overrides
                .max_collaboration_message_bytes
                .unwrap_or(defaults.max_collaboration_message_bytes),
            max_encoded_state_bytes: overrides
                .max_encoded_state_bytes
                .unwrap_or(defaults.max_encoded_state_bytes),
        };

        limits.validate()?;
        Ok(limits)
    }

    pub(crate) fn validate(&self) -> BoundaryResult<()> {
        for (name, actual, ceiling) in [
            ("maxInputBytes", self.max_input_bytes, HARD_MAX_INPUT_BYTES),
            ("maxDocumentNodes", self.max_document_nodes, 1_000_000),
            (
                "maxDocumentDepth",
                self.max_document_depth,
                HARD_MAX_DOCUMENT_DEPTH,
            ),
            ("maxSchemaNodes", self.max_schema_nodes, 10_000),
            (
                "maxSchemaExpressionBytes",
                self.max_schema_expression_bytes,
                1024 * 1024,
            ),
            (
                "maxCollaborationMessageBytes",
                self.max_collaboration_message_bytes,
                64 * 1024 * 1024,
            ),
            (
                "maxEncodedStateBytes",
                self.max_encoded_state_bytes,
                256 * 1024 * 1024,
            ),
        ] {
            if actual == 0 || actual > ceiling {
                return Err(BoundaryError {
                    code: "INVALID_RESOURCE_LIMIT",
                    message: format!("{name} must be a positive integer no greater than {ceiling}"),
                    limit: Some(ceiling),
                    actual: Some(actual),
                    details: Some(serde_json::json!({ "field": name })),
                });
            }
        }
        Ok(())
    }

    fn limit_for(&self, kind: InputKind) -> usize {
        match kind {
            InputKind::CollaborationMessage => self.max_collaboration_message_bytes,
            InputKind::EncodedState => self.max_encoded_state_bytes,
            InputKind::Config | InputKind::DocumentJson | InputKind::Html => self.max_input_bytes,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BoundaryError {
    pub code: &'static str,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actual: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

impl BoundaryError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            limit: None,
            actual: None,
            details: None,
        }
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

    pub fn parse(code: &'static str, error: impl std::fmt::Display) -> Self {
        Self::new(code, error.to_string())
    }

    pub fn code(&self) -> &'static str {
        self.code
    }
}

impl std::fmt::Display for BoundaryError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for BoundaryError {}

#[derive(Debug)]
pub struct BoundedInput<'a> {
    value: &'a str,
}

#[derive(Debug, Clone, Copy)]
pub enum InputKind {
    Config,
    DocumentJson,
    Html,
    // Not reachable from production call paths after the Task 16C legacy runtime
    // removal; exercised by crate tests.
    #[allow(dead_code)]
    CollaborationMessage,
    // Not reachable from production call paths after the Task 16C legacy runtime
    // removal; exercised by crate tests.
    #[allow(dead_code)]
    EncodedState,
}

impl<'a> BoundedInput<'a> {
    pub fn new(value: &'a str, kind: InputKind, limits: &ResourceLimits) -> BoundaryResult<Self> {
        let limit = limits.limit_for(kind);
        if value.len() > limit {
            return Err(BoundaryError::limit(
                "INPUT_LIMIT_EXCEEDED",
                limit,
                value.len(),
            ));
        }
        Ok(Self { value })
    }

    pub fn as_str(&self) -> &'a str {
        self.value
    }
}
