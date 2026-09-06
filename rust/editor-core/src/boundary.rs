use memchr::memchr2;
use serde::{Deserialize, Serialize};

pub(crate) const HARD_MAX_INPUT_BYTES: usize = 64 * 1024 * 1024;
pub(crate) const HARD_MAX_DOCUMENT_DEPTH: usize = 1_024;

// A whole-root ReplaceStructure compilation retains the source and preview
// trees while lowering an admitted 1,024-deep document.  Its bounded peak is
// larger than the generic import/read paths covered by the original 8 MiB
// segment, so reserve the next fixed tier at the FFI lifecycle boundary.
const DOCUMENT_STACK_SEGMENT_BYTES: usize = 16 * 1024 * 1024;
// A valid ProseMirror document uses an object, a content array, and a child
// object per semantic level. Keeping only five container levels on the caller
// stack leaves the deep-document path on the fixed segmented stack while
// avoiding an allocation for ordinary shallow documents.
const CALLER_STACK_JSON_CONTAINER_DEPTH: usize = 16;

std::thread_local! {
    static DOCUMENT_STACK_DEPTH: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

struct DocumentStackScope;

impl DocumentStackScope {
    fn enter() -> (Self, bool) {
        let outermost = DOCUMENT_STACK_DEPTH.with(|depth| {
            let current = depth.get();
            depth.set(
                current
                    .checked_add(1)
                    .expect("document stack boundary nesting exceeds usize"),
            );
            current == 0
        });
        (Self, outermost)
    }
}

impl Drop for DocumentStackScope {
    fn drop(&mut self) {
        DOCUMENT_STACK_DEPTH.with(|depth| {
            if let Some(remaining) = depth.get().checked_sub(1) {
                depth.set(remaining);
            }
        });
    }
}

#[cfg(test)]
std::thread_local! {
    static DOCUMENT_STACK_SEGMENT_GROWS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static JSON_CONTAINER_DEPTH_PREFLIGHTS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static JSON_CONTAINER_DEPTH_PREFLIGHT_BYTES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
fn record_document_stack_segment_grow_for_test() {
    DOCUMENT_STACK_SEGMENT_GROWS.with(|count| count.set(count.get().saturating_add(1)));
}

#[cfg(test)]
fn reset_document_stack_segment_grows_for_test() {
    DOCUMENT_STACK_SEGMENT_GROWS.with(|count| count.set(0));
}

#[cfg(test)]
fn document_stack_segment_grows_for_test() -> usize {
    DOCUMENT_STACK_SEGMENT_GROWS.with(|count| count.get())
}

#[cfg(test)]
fn record_json_container_depth_preflight_for_test() {
    JSON_CONTAINER_DEPTH_PREFLIGHTS.with(|count| count.set(count.get().saturating_add(1)));
}

#[cfg(test)]
fn reset_json_container_depth_preflights_for_test() {
    JSON_CONTAINER_DEPTH_PREFLIGHTS.with(|count| count.set(0));
}

#[cfg(test)]
fn json_container_depth_preflights_for_test() -> usize {
    JSON_CONTAINER_DEPTH_PREFLIGHTS.with(|count| count.get())
}

#[cfg(test)]
fn record_json_container_depth_preflight_byte_for_test() {
    JSON_CONTAINER_DEPTH_PREFLIGHT_BYTES.with(|count| count.set(count.get().saturating_add(1)));
}

#[cfg(test)]
fn reset_json_container_depth_preflight_bytes_for_test() {
    JSON_CONTAINER_DEPTH_PREFLIGHT_BYTES.with(|count| count.set(0));
}

#[cfg(test)]
fn json_container_depth_preflight_bytes_for_test() -> usize {
    JSON_CONTAINER_DEPTH_PREFLIGHT_BYTES.with(|count| count.get())
}

/// Run every bounded document lifecycle operation on a segmented stack sized
/// for admitted depth 1024.
pub(crate) fn with_document_stack<T>(operation: impl FnOnce() -> T) -> T {
    let (_scope, outermost) = DocumentStackScope::enter();
    if outermost {
        #[cfg(test)]
        record_document_stack_segment_grow_for_test();
        stacker::grow(DOCUMENT_STACK_SEGMENT_BYTES, operation)
    } else {
        operation()
    }
}

/// Run shallow JSON document work on the caller stack, reserving the fixed
/// segmented stack for inputs whose lexical container depth can exercise the
/// deep-document lifecycle.
pub(crate) fn with_document_stack_for_json_container_depth<T>(
    container_depth: usize,
    operation: impl FnOnce() -> T,
) -> T {
    if container_depth <= CALLER_STACK_JSON_CONTAINER_DEPTH {
        operation()
    } else {
        with_document_stack(operation)
    }
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
pub(crate) struct StackSafeJsonValue {
    value: serde_json::Value,
    container_depth: usize,
}

impl StackSafeJsonValue {
    pub(crate) fn new(value: serde_json::Value) -> Self {
        Self::with_container_depth(value, 0)
    }

    fn with_container_depth(value: serde_json::Value, container_depth: usize) -> Self {
        Self {
            value,
            container_depth,
        }
    }

    pub(crate) fn as_value(&self) -> &serde_json::Value {
        &self.value
    }

    pub(crate) fn container_depth(&self) -> usize {
        self.container_depth
    }
}

impl std::fmt::Debug for StackSafeJsonValue {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("StackSafeJsonValue(..)")
    }
}

impl Clone for StackSafeJsonValue {
    fn clone(&self) -> Self {
        Self::with_container_depth(
            clone_json_value_stack_safe(&self.value),
            self.container_depth,
        )
    }
}

impl PartialEq for StackSafeJsonValue {
    fn eq(&self, other: &Self) -> bool {
        json_values_equal_stack_safe(&self.value, &other.value)
    }
}

impl PartialEq<serde_json::Value> for StackSafeJsonValue {
    fn eq(&self, other: &serde_json::Value) -> bool {
        json_values_equal_stack_safe(&self.value, other)
    }
}

impl Eq for StackSafeJsonValue {}

impl Drop for StackSafeJsonValue {
    fn drop(&mut self) {
        let mut pending = vec![std::mem::take(&mut self.value)];
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn document_stack_reuses_nested_segment_and_resets_after_unwind() {
        reset_document_stack_segment_grows_for_test();

        with_document_stack(|| {
            assert_eq!(document_stack_segment_grows_for_test(), 1);
            with_document_stack(|| {
                assert_eq!(document_stack_segment_grows_for_test(), 1);
            });

            let child_thread_grows = std::thread::spawn(|| {
                reset_document_stack_segment_grows_for_test();
                with_document_stack(|| {
                    assert_eq!(document_stack_segment_grows_for_test(), 1);
                    with_document_stack(|| {
                        assert_eq!(document_stack_segment_grows_for_test(), 1);
                    });
                });
                document_stack_segment_grows_for_test()
            })
            .join()
            .expect("child thread stack boundary should not panic");
            assert_eq!(child_thread_grows, 1);
        });
        assert_eq!(document_stack_segment_grows_for_test(), 1);

        with_document_stack(|| {});
        assert_eq!(document_stack_segment_grows_for_test(), 2);

        assert!(std::panic::catch_unwind(|| {
            with_document_stack(|| panic!("intentional stack-boundary unwind"));
        })
        .is_err());
        assert_eq!(document_stack_segment_grows_for_test(), 3);

        with_document_stack(|| {});
        assert_eq!(document_stack_segment_grows_for_test(), 4);
    }

    #[test]
    fn shallow_json_document_work_skips_the_segmented_stack() {
        reset_document_stack_segment_grows_for_test();

        with_document_stack_for_json_container_depth(3, || {});

        assert_eq!(document_stack_segment_grows_for_test(), 0);
    }

    #[test]
    fn deep_json_document_work_keeps_the_segmented_stack() {
        reset_document_stack_segment_grows_for_test();

        with_document_stack_for_json_container_depth(CALLER_STACK_JSON_CONTAINER_DEPTH + 1, || {});

        assert_eq!(document_stack_segment_grows_for_test(), 1);
    }

    #[test]
    fn shallow_large_json_uses_the_depth_preflight() {
        reset_json_container_depth_preflights_for_test();
        let input = serde_json::json!({
            "type": "doc",
            "content": [{
                "type": "benchmarkOpaqueBlock",
                "attrs": { "payload": "x".repeat(256 * 1024) },
            }],
        })
        .to_string();

        let value = parse_json_value_stack_safe(
            &input,
            64,
            64,
            "DOCUMENT_LIMIT_EXCEEDED",
            "DOCUMENT_INVALID",
        )
        .expect("shallow opaque JSON should parse");

        assert_eq!(value.container_depth(), 4);
        assert_eq!(json_container_depth_preflights_for_test(), 1);
    }

    #[test]
    fn shallow_large_json_preflight_skips_opaque_string_bodies() {
        reset_json_container_depth_preflight_bytes_for_test();
        let input = serde_json::json!({
            "type": "doc",
            "content": [{
                "type": "benchmarkOpaqueBlock",
                "attrs": { "payload": "x".repeat(256 * 1024) },
            }],
        })
        .to_string();

        parse_json_value_stack_safe(
            &input,
            64,
            64,
            "DOCUMENT_LIMIT_EXCEEDED",
            "DOCUMENT_INVALID",
        )
        .expect("shallow opaque JSON should parse");

        assert!(
            json_container_depth_preflight_bytes_for_test() < 128,
            "the preflight must skip opaque string bodies"
        );
    }
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
    let container_depth = admit_json_container_depth(input, max_container_depth)
        .map_err(|actual| BoundaryError::limit(limit_code, reported_limit, actual))?;

    let mut deserializer = serde_json::Deserializer::from_str(input);
    deserializer.disable_recursion_limit();
    let value = serde_json::Value::deserialize(serde_stacker::Deserializer::new(&mut deserializer))
        .map(|value| StackSafeJsonValue::with_container_depth(value, container_depth))
        .map_err(|error| BoundaryError::parse(parse_code, error))?;
    deserializer
        .end()
        .map_err(|error| BoundaryError::parse(parse_code, error))?;
    Ok(value)
}

fn admit_json_container_depth(input: &str, limit: usize) -> Result<usize, usize> {
    #[cfg(test)]
    record_json_container_depth_preflight_for_test();
    let bytes = input.as_bytes();
    let mut containers = Vec::with_capacity(limit.min(64));
    let mut maximum_depth = 0;
    let mut index = 0;
    while index < bytes.len() {
        #[cfg(test)]
        record_json_container_depth_preflight_byte_for_test();
        match bytes[index] {
            b'"' => {
                index += 1;
                loop {
                    let Some(next) = memchr2(b'"', b'\\', &bytes[index..]) else {
                        index = bytes.len();
                        break;
                    };
                    index += next;
                    #[cfg(test)]
                    record_json_container_depth_preflight_byte_for_test();
                    if bytes[index] == b'"' {
                        index += 1;
                        break;
                    }
                    index += 1;
                    if index == bytes.len() {
                        break;
                    }
                    #[cfg(test)]
                    record_json_container_depth_preflight_byte_for_test();
                    index += 1;
                }
                continue;
            }
            b'{' => {
                containers.push(b'}');
                if containers.len() > limit {
                    return Err(containers.len());
                }
                maximum_depth = maximum_depth.max(containers.len());
            }
            b'[' => {
                containers.push(b']');
                if containers.len() > limit {
                    return Err(containers.len());
                }
                maximum_depth = maximum_depth.max(containers.len());
            }
            b'}' | b']' => {
                if containers.pop() != Some(bytes[index]) {
                    return Ok(maximum_depth);
                }
            }
            _ => {}
        }
        index += 1;
    }
    Ok(maximum_depth)
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

include!("boundary/limits.rs");

#[cfg(test)]
#[path = "boundary/json_meter_tests.rs"]
mod json_meter_tests;
