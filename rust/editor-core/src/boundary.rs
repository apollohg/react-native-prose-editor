use serde::{Deserialize, Serialize};

pub(crate) const HARD_MAX_INPUT_BYTES: usize = 64 * 1024 * 1024;
pub(crate) const HARD_MAX_DOCUMENT_DEPTH: usize = 1_024;

const DOCUMENT_STACK_RED_ZONE_BYTES: usize = 256 * 1024;
const DOCUMENT_STACK_SEGMENT_BYTES: usize = 8 * 1024 * 1024;

/// Run a bounded document lifecycle operation on a segmented stack whenever
/// the caller cannot supply enough native stack for admitted depth 1024.
pub(crate) fn with_document_stack<T>(operation: impl FnOnce() -> T) -> T {
    stacker::maybe_grow(
        DOCUMENT_STACK_RED_ZONE_BYTES,
        DOCUMENT_STACK_SEGMENT_BYTES,
        operation,
    )
}

pub(crate) fn deserialize_non_null_option<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

pub type BoundaryResult<T> = Result<T, BoundaryError>;

/// A deeply nested JSON value whose destructor drains child containers
/// iteratively. `serde_json::Value` otherwise drops through the container
/// tree recursively, which can overflow after an admitted deep parse.
pub(crate) struct StackSafeJsonValue(serde_json::Value);

impl StackSafeJsonValue {
    pub(crate) fn new(value: serde_json::Value) -> Self {
        Self(value)
    }

    pub(crate) fn as_value(&self) -> &serde_json::Value {
        &self.0
    }
}

impl std::fmt::Debug for StackSafeJsonValue {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("StackSafeJsonValue(..)")
    }
}

impl Clone for StackSafeJsonValue {
    fn clone(&self) -> Self {
        Self(clone_json_value_stack_safe(&self.0))
    }
}

impl PartialEq for StackSafeJsonValue {
    fn eq(&self, other: &Self) -> bool {
        json_values_equal_stack_safe(&self.0, &other.0)
    }
}

impl PartialEq<serde_json::Value> for StackSafeJsonValue {
    fn eq(&self, other: &serde_json::Value) -> bool {
        json_values_equal_stack_safe(&self.0, other)
    }
}

impl Eq for StackSafeJsonValue {}

impl Drop for StackSafeJsonValue {
    fn drop(&mut self) {
        let mut pending = vec![std::mem::take(&mut self.0)];
        while let Some(mut value) = pending.pop() {
            match &mut value {
                serde_json::Value::Array(values) => pending.append(values),
                serde_json::Value::Object(values) => {
                    pending.extend(std::mem::take(values).into_values());
                }
                _ => {}
            }
        }
    }
}

pub(crate) fn drop_json_value_stack_safe(value: serde_json::Value) {
    drop(StackSafeJsonValue::new(value));
}

pub(crate) fn clone_json_value_stack_safe(value: &serde_json::Value) -> serde_json::Value {
    enum Frame<'a> {
        Visit(&'a serde_json::Value),
        BuildArray(usize),
        BuildObject(Vec<String>),
    }

    let mut frames = vec![Frame::Visit(value)];
    let mut built = Vec::new();
    while let Some(frame) = frames.pop() {
        match frame {
            Frame::Visit(value) => match value {
                serde_json::Value::Null => built.push(serde_json::Value::Null),
                serde_json::Value::Bool(value) => built.push(serde_json::Value::Bool(*value)),
                serde_json::Value::Number(value) => {
                    built.push(serde_json::Value::Number(value.clone()));
                }
                serde_json::Value::String(value) => {
                    built.push(serde_json::Value::String(value.clone()));
                }
                serde_json::Value::Array(values) => {
                    frames.push(Frame::BuildArray(values.len()));
                    frames.extend(values.iter().rev().map(Frame::Visit));
                }
                serde_json::Value::Object(values) => {
                    frames.push(Frame::BuildObject(values.keys().cloned().collect()));
                    frames.extend(values.values().rev().map(Frame::Visit));
                }
            },
            Frame::BuildArray(len) => {
                let first = built
                    .len()
                    .checked_sub(len)
                    .expect("JSON clone frame stack is balanced");
                let values = built.split_off(first);
                built.push(serde_json::Value::Array(values));
            }
            Frame::BuildObject(keys) => {
                let first = built
                    .len()
                    .checked_sub(keys.len())
                    .expect("JSON clone frame stack is balanced");
                let values = built.split_off(first);
                built.push(serde_json::Value::Object(
                    keys.into_iter().zip(values).collect(),
                ));
            }
        }
    }
    built.pop().expect("JSON clone produces one root")
}

pub(crate) fn clone_json_object_stack_safe(
    values: &std::collections::HashMap<String, serde_json::Value>,
) -> std::collections::HashMap<String, serde_json::Value> {
    values
        .iter()
        .map(|(key, value)| (key.clone(), clone_json_value_stack_safe(value)))
        .collect()
}

pub(crate) fn drop_json_object_values_stack_safe(
    values: &mut std::collections::HashMap<String, serde_json::Value>,
) {
    for value in values.values_mut() {
        drop_json_value_stack_safe(std::mem::take(value));
    }
}

pub(crate) fn json_values_equal_stack_safe(
    left: &serde_json::Value,
    right: &serde_json::Value,
) -> bool {
    let mut pending = vec![(left, right)];
    while let Some((left, right)) = pending.pop() {
        match (left, right) {
            (serde_json::Value::Null, serde_json::Value::Null) => {}
            (serde_json::Value::Bool(left), serde_json::Value::Bool(right)) if left == right => {}
            (serde_json::Value::Number(left), serde_json::Value::Number(right))
                if left == right => {}
            (serde_json::Value::String(left), serde_json::Value::String(right))
                if left == right => {}
            (serde_json::Value::Array(left), serde_json::Value::Array(right))
                if left.len() == right.len() =>
            {
                pending.extend(left.iter().zip(right));
            }
            (serde_json::Value::Object(left), serde_json::Value::Object(right))
                if left.len() == right.len() =>
            {
                for ((left_key, left_value), (right_key, right_value)) in
                    left.iter().zip(right.iter())
                {
                    if left_key != right_key {
                        return false;
                    }
                    pending.push((left_value, right_value));
                }
            }
            _ => return false,
        }
    }
    true
}

pub(crate) fn json_objects_equal_stack_safe(
    left: &std::collections::HashMap<String, serde_json::Value>,
    right: &std::collections::HashMap<String, serde_json::Value>,
) -> bool {
    left.len() == right.len()
        && left.iter().all(|(key, left_value)| {
            right
                .get(key)
                .is_some_and(|right_value| json_values_equal_stack_safe(left_value, right_value))
        })
}

pub(crate) fn serialize_json_value_stack_safe(
    value: &serde_json::Value,
    initial_capacity: usize,
) -> Vec<u8> {
    enum Frame<'a> {
        Value(&'a serde_json::Value),
        String(&'a str),
        Raw(&'static [u8]),
    }

    let mut output = Vec::with_capacity(initial_capacity);
    let mut frames = vec![Frame::Value(value)];
    while let Some(frame) = frames.pop() {
        match frame {
            Frame::Raw(bytes) => output.extend_from_slice(bytes),
            Frame::String(value) => serde_json::to_writer(&mut output, value)
                .expect("JSON strings always serialize to an in-memory buffer"),
            Frame::Value(value) => match value {
                serde_json::Value::Null => output.extend_from_slice(b"null"),
                serde_json::Value::Bool(true) => output.extend_from_slice(b"true"),
                serde_json::Value::Bool(false) => output.extend_from_slice(b"false"),
                serde_json::Value::Number(value) => {
                    output.extend_from_slice(value.to_string().as_bytes());
                }
                serde_json::Value::String(value) => serde_json::to_writer(&mut output, value)
                    .expect("JSON strings always serialize to an in-memory buffer"),
                serde_json::Value::Array(values) => {
                    output.push(b'[');
                    frames.push(Frame::Raw(b"]"));
                    for (index, value) in values.iter().enumerate().rev() {
                        if index + 1 < values.len() {
                            frames.push(Frame::Raw(b","));
                        }
                        frames.push(Frame::Value(value));
                    }
                }
                serde_json::Value::Object(values) => {
                    output.push(b'{');
                    frames.push(Frame::Raw(b"}"));
                    for (index, (key, value)) in values.iter().enumerate().rev() {
                        if index + 1 < values.len() {
                            frames.push(Frame::Raw(b","));
                        }
                        frames.push(Frame::Value(value));
                        frames.push(Frame::Raw(b":"));
                        frames.push(Frame::String(key));
                    }
                }
            },
        }
    }
    output
}

/// ProseMirror nodes add at most one object and one `content` array per
/// semantic level. Attributes or mark attributes can independently consume
/// the configured metadata depth at the deepest node. Fixed slack covers the
/// root/mark wrappers while keeping the pre-deserialization bound derived
/// from the already-resolved semantic contract.
pub(crate) fn document_json_container_depth_limit(
    max_document_depth: usize,
) -> BoundaryResult<usize> {
    max_document_depth
        .checked_mul(3)
        .and_then(|depth| depth.checked_add(16))
        .ok_or_else(|| {
            BoundaryError::new(
                "DOCUMENT_LIMIT_EXCEEDED",
                "document JSON container-depth limit overflow",
            )
        })
}

/// Parse a bounded JSON value without serde's fixed 128-container ceiling.
/// A lexical, iterative preflight proves a finite container bound first;
/// serde-stacker then keeps serde's own recursive implementation off the
/// caller's finite native stack.
pub(crate) fn parse_json_value_stack_safe(
    input: &str,
    max_container_depth: usize,
    reported_limit: usize,
    limit_code: &'static str,
    parse_code: &'static str,
) -> BoundaryResult<StackSafeJsonValue> {
    admit_json_container_depth(input, max_container_depth)
        .map_err(|actual| BoundaryError::limit(limit_code, reported_limit, actual))?;

    let mut deserializer = serde_json::Deserializer::from_str(input);
    deserializer.disable_recursion_limit();
    let value = serde_json::Value::deserialize(serde_stacker::Deserializer::new(&mut deserializer))
        .map(StackSafeJsonValue::new)
        .map_err(|error| BoundaryError::parse(parse_code, error))?;
    deserializer
        .end()
        .map_err(|error| BoundaryError::parse(parse_code, error))?;
    Ok(value)
}

fn admit_json_container_depth(input: &str, limit: usize) -> Result<(), usize> {
    let mut containers = Vec::with_capacity(limit.min(64));
    let mut in_string = false;
    let mut escaped = false;
    for byte in input.bytes() {
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'{' => {
                containers.push(b'}');
                if containers.len() > limit {
                    return Err(containers.len());
                }
            }
            b'[' => {
                containers.push(b']');
                if containers.len() > limit {
                    return Err(containers.len());
                }
            }
            b'}' | b']' => {
                if containers.pop() != Some(byte) {
                    return Ok(());
                }
            }
            _ => {}
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum JsonMeterDimension {
    Bytes,
    Work,
    Depth,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct JsonMeterError {
    pub(crate) dimension: JsonMeterDimension,
    pub(crate) limit: usize,
    pub(crate) actual: usize,
}

pub(crate) struct JsonValueMeter {
    byte_limit: usize,
    work_limit: usize,
    depth_limit: usize,
    bytes: usize,
    work: usize,
}

impl JsonValueMeter {
    pub(crate) fn new(
        byte_limit: usize,
        work_limit: usize,
        depth_limit: usize,
        initial_bytes: usize,
    ) -> Self {
        Self {
            byte_limit,
            work_limit,
            depth_limit,
            bytes: initial_bytes,
            work: 0,
        }
    }

    pub(crate) fn bytes(&self) -> usize {
        self.bytes
    }

    pub(crate) fn charge_bytes(&mut self, amount: usize) -> Result<(), JsonMeterError> {
        let actual = self.bytes.saturating_add(amount);
        if actual > self.byte_limit {
            return Err(JsonMeterError {
                dimension: JsonMeterDimension::Bytes,
                limit: self.byte_limit,
                actual,
            });
        }
        self.bytes = actual;
        Ok(())
    }

    fn admit_value(&mut self, depth: usize) -> Result<(), JsonMeterError> {
        if depth > self.depth_limit {
            return Err(JsonMeterError {
                dimension: JsonMeterDimension::Depth,
                limit: self.depth_limit,
                actual: depth,
            });
        }
        let actual = self.work.saturating_add(1);
        if actual > self.work_limit {
            return Err(JsonMeterError {
                dimension: JsonMeterDimension::Work,
                limit: self.work_limit,
                actual,
            });
        }
        self.work = actual;
        Ok(())
    }

    pub(crate) fn admit_object(
        &mut self,
        attrs: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<(), JsonMeterError> {
        enum Frame<'a> {
            Attrs(
                std::collections::hash_map::Iter<'a, String, serde_json::Value>,
                usize,
            ),
            Value(&'a serde_json::Value, usize),
            Array(std::slice::Iter<'a, serde_json::Value>, usize),
            Object(serde_json::map::Iter<'a>, usize),
        }
        self.charge_bytes(2)?;
        if !attrs.is_empty() {
            self.charge_bytes(attrs.len() - 1)?;
        }
        let mut stack = vec![Frame::Attrs(attrs.iter(), 1)];
        while let Some(frame) = stack.pop() {
            match frame {
                Frame::Attrs(mut values, depth) => {
                    if let Some((key, value)) = values.next() {
                        self.admit_value(depth)?;
                        self.count_string(key)?;
                        self.charge_bytes(1)?;
                        stack.push(Frame::Attrs(values, depth));
                        stack.push(Frame::Value(value, depth));
                    }
                }
                Frame::Value(value, depth) => match value {
                    serde_json::Value::Null => self.charge_bytes(4)?,
                    serde_json::Value::Bool(value) => {
                        self.charge_bytes(if *value { 4 } else { 5 })?
                    }
                    serde_json::Value::Number(value) => {
                        self.charge_bytes(value.to_string().len())?
                    }
                    serde_json::Value::String(value) => self.count_string(value)?,
                    serde_json::Value::Array(values) => {
                        self.charge_bytes(2)?;
                        if !values.is_empty() {
                            self.charge_bytes(values.len() - 1)?;
                        }
                        let child_depth = depth.saturating_add(1);
                        stack.push(Frame::Array(values.iter(), child_depth));
                    }
                    serde_json::Value::Object(values) => {
                        self.charge_bytes(2)?;
                        if !values.is_empty() {
                            self.charge_bytes(values.len() - 1)?;
                        }
                        let child_depth = depth.saturating_add(1);
                        stack.push(Frame::Object(values.iter(), child_depth));
                    }
                },
                Frame::Array(mut values, depth) => {
                    if let Some(value) = values.next() {
                        self.admit_value(depth)?;
                        stack.push(Frame::Array(values, depth));
                        stack.push(Frame::Value(value, depth));
                    }
                }
                Frame::Object(mut values, depth) => {
                    if let Some((key, value)) = values.next() {
                        self.admit_value(depth)?;
                        self.count_string(key)?;
                        self.charge_bytes(1)?;
                        stack.push(Frame::Object(values, depth));
                        stack.push(Frame::Value(value, depth));
                    }
                }
            }
        }
        Ok(())
    }

    fn count_string(&mut self, value: &str) -> Result<(), JsonMeterError> {
        self.charge_bytes(value.len().saturating_add(2))?;
        if !string_requires_json_escape(value.as_bytes()) {
            return Ok(());
        }
        for byte in value.bytes() {
            let extra = match byte {
                b'"' | b'\\' | b'\x08' | b'\t' | b'\n' | b'\x0c' | b'\r' => 1,
                0x00..=0x1f => 5,
                _ => 0,
            };
            if extra != 0 {
                self.charge_bytes(extra)?;
            }
        }
        Ok(())
    }
}

fn string_requires_json_escape(bytes: &[u8]) -> bool {
    const ONES: u64 = 0x0101_0101_0101_0101;
    const HIGHS: u64 = 0x8080_8080_8080_8080;
    const CONTROLS: u64 = 0x2020_2020_2020_2020;
    const QUOTES: u64 = 0x2222_2222_2222_2222;
    const BACKSLASHES: u64 = 0x5c5c_5c5c_5c5c_5c5c;

    let mut chunks = bytes.chunks_exact(8);
    for chunk in &mut chunks {
        let word = u64::from_ne_bytes(chunk.try_into().expect("exact chunk length"));
        let has_control = word.wrapping_sub(CONTROLS) & !word & HIGHS != 0;
        let quote_lanes = word ^ QUOTES;
        let has_quote = quote_lanes.wrapping_sub(ONES) & !quote_lanes & HIGHS != 0;
        let backslash_lanes = word ^ BACKSLASHES;
        let has_backslash = backslash_lanes.wrapping_sub(ONES) & !backslash_lanes & HIGHS != 0;
        if has_control || has_quote || has_backslash {
            return true;
        }
    }
    chunks
        .remainder()
        .iter()
        .any(|byte| *byte < 0x20 || matches!(*byte, b'"' | b'\\'))
}

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

#[cfg(test)]
mod json_meter_tests {
    use std::collections::HashMap;

    use serde_json::json;

    use super::{JsonMeterDimension, JsonValueMeter};

    #[test]
    fn deep_json_trailing_input_unwinds_on_a_constrained_stack() {
        let outcome = std::thread::Builder::new()
            .name("deep-json-trailing-input".into())
            .stack_size(192 * 1024)
            .spawn(|| {
                let depth = 1_024;
                let mut input = "[".repeat(depth);
                input.push('0');
                input.push_str(&"]".repeat(depth));
                input.push_str(" trailing");
                super::parse_json_value_stack_safe(
                    &input,
                    depth,
                    depth,
                    "DOCUMENT_LIMIT_EXCEEDED",
                    "DOCUMENT_INVALID",
                )
                .map(|_| ())
                .map_err(|error| error.code)
            })
            .expect("constrained-stack thread should spawn")
            .join()
            .expect("deep trailing-input parse must not overflow");

        assert_eq!(outcome, Err("DOCUMENT_INVALID"));
    }

    #[test]
    fn json_value_meter_matches_compact_json_and_enforces_exact_bytes() {
        let attrs = HashMap::from([
            ("escaped".into(), json!("quote\" slash\\ line\n 😀")),
            ("number".into(), json!(-123.5)),
            ("nested".into(), json!({ "a": [true, null, 7] })),
        ]);
        let expected = serde_json::to_vec(&attrs).unwrap().len();
        let mut exact = JsonValueMeter::new(expected, 64, 16, 0);
        exact.admit_object(&attrs).unwrap();
        assert_eq!(exact.bytes(), expected);

        let mut one_under = JsonValueMeter::new(expected - 1, 64, 16, 0);
        let error = one_under.admit_object(&attrs).unwrap_err();
        assert_eq!(error.dimension, JsonMeterDimension::Bytes);
        assert_eq!(error.limit, expected - 1);
        assert!(error.actual > error.limit);
    }

    #[test]
    fn json_value_meter_enforces_depth_and_work_before_descent() {
        let nested = HashMap::from([("value".into(), json!([[0]]))]);
        JsonValueMeter::new(1024, 8, 3, 0)
            .admit_object(&nested)
            .unwrap();
        let error = JsonValueMeter::new(1024, 8, 2, 0)
            .admit_object(&nested)
            .unwrap_err();
        assert_eq!(error.dimension, JsonMeterDimension::Depth);
        assert_eq!(error.actual, 3);

        let wide = HashMap::from([("value".into(), json!([1, 2, 3, 4]))]);
        let error = JsonValueMeter::new(1024, 4, 8, 0)
            .admit_object(&wide)
            .unwrap_err();
        assert_eq!(error.dimension, JsonMeterDimension::Work);
        assert_eq!(error.actual, 5);
    }

    #[test]
    fn json_value_meter_rejects_exhausted_work_before_scanning_the_next_key() {
        let attrs = HashMap::from([("x".repeat(128 * 1024), json!(null))]);
        let mut meter = JsonValueMeter::new(usize::MAX, 0, 8, 0);
        let error = meter.admit_object(&attrs).unwrap_err();
        assert_eq!(error.dimension, JsonMeterDimension::Work);
        assert_eq!(error.actual, 1);
        assert_eq!(meter.bytes(), 2);
    }
}
