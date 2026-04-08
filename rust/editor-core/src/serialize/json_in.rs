use std::collections::HashMap;
use std::fmt;

use serde_json::{Map, Value};

use crate::model::{Document, Fragment, Mark, Node};
use crate::schema::Schema;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors returned by `from_prosemirror_json`.
#[derive(Debug, Clone)]
pub enum JsonParseError {
    /// A node/mark type in the JSON was not found in the schema.
    UnknownType(String),
    /// The JSON structure is invalid (e.g. missing "type" field).
    InvalidStructure(String),
}

impl fmt::Display for JsonParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            JsonParseError::UnknownType(name) => {
                write!(f, "unknown node/mark type: \"{}\"", name)
            }
            JsonParseError::InvalidStructure(msg) => {
                write!(f, "invalid JSON structure: {}", msg)
            }
        }
    }
}

impl std::error::Error for JsonParseError {}

// ---------------------------------------------------------------------------
// Options
// ---------------------------------------------------------------------------

/// How to handle node types that are not found in the schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UnknownTypeMode {
    /// Return an error when an unknown type is encountered.
    #[default]
    Error,
    /// Preserve unknown nodes as opaque void nodes with the original JSON
    /// retained in attrs.
    Preserve,
    /// Silently drop unknown nodes from the output.
    Skip,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Parse a ProseMirror JSON value into a Document tree using the given schema.
///
/// The JSON should be a ProseMirror document object:
/// ```json
/// { "type": "doc", "content": [ ... ] }
/// ```
///
/// The `mode` parameter controls how unknown node/mark types are handled.
pub fn from_prosemirror_json(
    json: &Value,
    schema: &Schema,
    mode: UnknownTypeMode,
) -> Result<Document, JsonParseError> {
    let normalized = normalize_json_aliases(json);
    let root = parse_node(&normalized, schema, mode)?;
    Ok(Document::new(root))
}

// ---------------------------------------------------------------------------
// Internal parsing
// ---------------------------------------------------------------------------

fn parse_node(
    json: &Value,
    schema: &Schema,
    mode: UnknownTypeMode,
) -> Result<Node, JsonParseError> {
    let obj = json
        .as_object()
        .ok_or_else(|| JsonParseError::InvalidStructure("node must be a JSON object".into()))?;

    let type_name = obj.get("type").and_then(|v| v.as_str()).ok_or_else(|| {
        JsonParseError::InvalidStructure("node must have a string \"type\" field".into())
    })?;

    // Text node: special case
    if type_name == "text" {
        return parse_text_node(obj, schema, mode);
    }

    // Look up the type in the schema
    let spec = schema.node(type_name);

    if spec.is_none() {
        match mode {
            UnknownTypeMode::Error => {
                return Err(JsonParseError::UnknownType(type_name.to_string()));
            }
            UnknownTypeMode::Preserve => {
                return Ok(build_opaque_json_node(type_name, json));
            }
            UnknownTypeMode::Skip => {
                // Signal to the caller that this node should be dropped.
                // We use a sentinel — a void node with a special type.
                return Ok(Node::void("__skip".to_string(), HashMap::new()));
            }
        }
    }

    let spec = spec.unwrap();
    let attrs = parse_attrs(obj, spec);

    if spec.is_void {
        // Void node — no content
        return Ok(Node::void(type_name.to_string(), attrs));
    }

    // Element node — parse children
    let children = parse_content(obj, schema, mode)?;
    Ok(Node::element(
        type_name.to_string(),
        attrs,
        Fragment::from(children),
    ))
}

/// Parse a text node from a JSON object.
fn parse_text_node(
    obj: &serde_json::Map<String, Value>,
    schema: &Schema,
    mode: UnknownTypeMode,
) -> Result<Node, JsonParseError> {
    let text = obj.get("text").and_then(|v| v.as_str()).ok_or_else(|| {
        JsonParseError::InvalidStructure("text node must have a string \"text\" field".into())
    })?;

    let marks = parse_marks(obj, schema, mode)?;
    Ok(Node::text(text.to_string(), marks))
}

/// Parse marks from a node's JSON object.
fn parse_marks(
    obj: &serde_json::Map<String, Value>,
    schema: &Schema,
    mode: UnknownTypeMode,
) -> Result<Vec<Mark>, JsonParseError> {
    let marks_val = match obj.get("marks") {
        Some(v) => v,
        None => return Ok(Vec::new()),
    };

    let marks_arr = marks_val
        .as_array()
        .ok_or_else(|| JsonParseError::InvalidStructure("\"marks\" must be an array".into()))?;

    let mut marks = Vec::with_capacity(marks_arr.len());
    for mark_json in marks_arr {
        let mark_obj = mark_json.as_object().ok_or_else(|| {
            JsonParseError::InvalidStructure("each mark must be a JSON object".into())
        })?;

        let mark_type = mark_obj
            .get("type")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                JsonParseError::InvalidStructure("mark must have a string \"type\" field".into())
            })?;

        // Check if mark exists in schema
        if schema.mark(mark_type).is_none() {
            match mode {
                UnknownTypeMode::Error => {
                    return Err(JsonParseError::UnknownType(mark_type.to_string()));
                }
                UnknownTypeMode::Preserve => {
                    // Keep the mark even though it's unknown
                }
                UnknownTypeMode::Skip => {
                    continue; // Drop the unknown mark
                }
            }
        }

        let attrs = parse_mark_attrs(mark_obj);
        marks.push(Mark::new(mark_type.to_string(), attrs));
    }

    Ok(marks)
}

/// Parse mark attributes from a mark JSON object.
fn parse_mark_attrs(mark_obj: &serde_json::Map<String, Value>) -> HashMap<String, Value> {
    match mark_obj.get("attrs") {
        Some(Value::Object(attrs_obj)) => attrs_obj
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
        _ => HashMap::new(),
    }
}

/// Parse node attributes from a node's JSON object, filling in schema defaults
/// for any missing attributes.
fn parse_attrs(
    obj: &serde_json::Map<String, Value>,
    spec: &crate::schema::NodeSpec,
) -> HashMap<String, Value> {
    let mut attrs = HashMap::new();

    // Start with schema defaults
    for (key, attr_spec) in &spec.attrs {
        if let Some(default) = &attr_spec.default {
            attrs.insert(key.clone(), default.clone());
        }
    }

    // Overlay with values from JSON
    if let Some(Value::Object(json_attrs)) = obj.get("attrs") {
        for (key, value) in json_attrs {
            attrs.insert(key.clone(), value.clone());
        }
    }

    attrs
}

/// Parse the "content" array of a node.
fn parse_content(
    obj: &serde_json::Map<String, Value>,
    schema: &Schema,
    mode: UnknownTypeMode,
) -> Result<Vec<Node>, JsonParseError> {
    let content_val = match obj.get("content") {
        Some(v) => v,
        None => return Ok(Vec::new()),
    };

    let content_arr = content_val
        .as_array()
        .ok_or_else(|| JsonParseError::InvalidStructure("\"content\" must be an array".into()))?;

    let mut children = Vec::with_capacity(content_arr.len());
    for child_json in content_arr {
        let child = parse_node(child_json, schema, mode)?;
        // Skip sentinel nodes (from UnknownTypeMode::Skip)
        if child.node_type() != "__skip" {
            children.push(child);
        }
    }

    Ok(children)
}

/// Build an opaque node for an unknown type (Preserve mode).
///
/// The original JSON is stored in the attrs so it can survive round-trips.
fn build_opaque_json_node(type_name: &str, original_json: &Value) -> Node {
    let mut attrs = HashMap::new();
    attrs.insert(
        "original_type".to_string(),
        Value::String(type_name.to_string()),
    );
    attrs.insert("original_json".to_string(), original_json.clone());
    Node::void("__opaque_json".to_string(), attrs)
}

fn normalize_json_aliases(value: &Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.iter().map(normalize_json_aliases).collect()),
        Value::Object(object) => normalize_json_object_aliases(object),
        other => other.clone(),
    }
}

fn normalize_json_object_aliases(object: &Map<String, Value>) -> Value {
    let mut normalized = object
        .iter()
        .map(|(key, value)| (key.clone(), normalize_json_aliases(value)))
        .collect::<Map<String, Value>>();

    let type_name = normalized.get("type").and_then(Value::as_str);
    if type_name == Some("heading") {
        let level = normalized
            .get("attrs")
            .and_then(Value::as_object)
            .and_then(|attrs| parse_heading_level_value(attrs.get("level")));
        if let Some(level) = level {
            normalized.insert("type".to_string(), Value::String(format!("h{level}")));
            if let Some(Value::Object(attrs)) = normalized.get_mut("attrs") {
                attrs.remove("level");
                if attrs.is_empty() {
                    normalized.remove("attrs");
                }
            }
        }
    }

    Value::Object(normalized)
}

fn parse_heading_level_value(value: Option<&Value>) -> Option<u8> {
    let value = value?;
    let level = match value {
        Value::Number(number) => number.as_u64().and_then(|value| u8::try_from(value).ok())?,
        Value::String(value) => value.parse::<u8>().ok()?,
        _ => return None,
    };

    if (1..=6).contains(&level) {
        Some(level)
    } else {
        None
    }
}
