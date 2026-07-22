#[cfg(test)]
use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt;

use serde_json::{Map, Value};

use crate::boundary::ResourceLimits;
use crate::model::{Document, Fragment, Mark, Node};
use crate::schema::content_rule::WorkBudget;
use crate::schema::Schema;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors returned by `from_prosemirror_json`.
#[derive(Debug, Clone)]
pub enum JsonParseError {
    /// A node/mark type in the JSON was not found in the schema.
    UnknownType(String),
    /// A mark is never eligible for opaque preservation.
    UnknownMark(String),
    ResourceLimit {
        limit: usize,
        actual: usize,
    },
    /// The JSON structure is invalid (e.g. missing "type" field).
    InvalidStructure(String),
}

impl fmt::Display for JsonParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            JsonParseError::UnknownType(name) => {
                write!(f, "unknown node/mark type: \"{}\"", name)
            }
            JsonParseError::UnknownMark(name) => write!(f, "unknown mark type: \"{}\"", name),
            JsonParseError::ResourceLimit { limit, actual } => {
                write!(f, "document parse work exceeds limit {limit}: {actual}")
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
    // Not reachable from production call paths after the Task 16C legacy runtime
    // removal; exercised by crate tests.
    #[allow(dead_code)]
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
// Not reachable from production call paths after the Task 16C legacy runtime
// removal; exercised by crate tests.
#[allow(dead_code)]
pub fn from_prosemirror_json(
    json: &Value,
    schema: &Schema,
    mode: UnknownTypeMode,
) -> Result<Document, JsonParseError> {
    from_prosemirror_json_with_limits(json, schema, mode, &ResourceLimits::default())
}

pub fn from_prosemirror_json_with_limits(
    json: &Value,
    schema: &Schema,
    mode: UnknownTypeMode,
    limits: &ResourceLimits,
) -> Result<Document, JsonParseError> {
    let mut budget = ParseBudget::new(limits.max_document_nodes, limits.max_document_depth);
    let root = parse_node(json, schema, mode, "block", &mut budget)?;
    Ok(Document::new(root))
}

struct ParseBudget {
    nodes: usize,
    max_nodes: usize,
    max_depth: usize,
    placement: WorkBudget,
}

impl ParseBudget {
    fn new(max_nodes: usize, max_depth: usize) -> Self {
        Self {
            nodes: 0,
            max_nodes,
            max_depth,
            placement: WorkBudget::new(max_nodes.saturating_mul(64)),
        }
    }

    fn admit_node(&mut self, depth: usize) -> Result<(), JsonParseError> {
        if depth > self.max_depth {
            return Err(JsonParseError::ResourceLimit {
                limit: self.max_depth,
                actual: depth,
            });
        }
        self.nodes = self.nodes.saturating_add(1);
        if self.nodes > self.max_nodes {
            return Err(JsonParseError::ResourceLimit {
                limit: self.max_nodes,
                actual: self.nodes,
            });
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Internal parsing
// ---------------------------------------------------------------------------

fn parse_node(
    json: &Value,
    schema: &Schema,
    mode: UnknownTypeMode,
    placement: &'static str,
    budget: &mut ParseBudget,
) -> Result<Node, JsonParseError> {
    enum Frame<'json, 'schema> {
        Visit {
            json: &'json Value,
            depth: usize,
            placement: &'static str,
        },
        BuildElement {
            type_name: String,
            attrs: HashMap<String, Value>,
            parent: &'schema crate::schema::NodeSpec,
            child_count: usize,
        },
    }

    let mut frames = vec![Frame::Visit {
        json,
        depth: 1,
        placement,
    }];
    let mut built = Vec::new();
    while let Some(frame) = frames.pop() {
        match frame {
            Frame::Visit {
                json,
                depth,
                placement,
            } => {
                budget.admit_node(depth)?;
                let obj = json.as_object().ok_or_else(|| {
                    JsonParseError::InvalidStructure("node must be a JSON object".into())
                })?;
                let raw_type = obj.get("type").and_then(Value::as_str).ok_or_else(|| {
                    JsonParseError::InvalidStructure(
                        "node must have a string \"type\" field".into(),
                    )
                })?;
                let heading_level = (raw_type == "heading")
                    .then(|| {
                        obj.get("attrs")
                            .and_then(Value::as_object)
                            .and_then(|attrs| parse_heading_level_value(attrs.get("level")))
                    })
                    .flatten();
                let type_name = heading_level
                    .map(|level| format!("h{level}"))
                    .unwrap_or_else(|| raw_type.to_string());

                if type_name == "text" {
                    built.push(parse_text_node(obj, schema, mode)?);
                    continue;
                }

                let Some(spec) = schema.node(&type_name) else {
                    match mode {
                        UnknownTypeMode::Error => {
                            return Err(JsonParseError::UnknownType(type_name));
                        }
                        UnknownTypeMode::Preserve => {
                            built.push(build_opaque_json_node(&type_name, json, placement));
                        }
                        UnknownTypeMode::Skip => {
                            built.push(Node::void("__skip".to_string(), HashMap::new()));
                        }
                    }
                    continue;
                };

                let mut attrs = parse_attrs(obj, spec);
                if heading_level.is_some() {
                    attrs.remove("level");
                }
                if spec.is_void {
                    built.push(Node::void(type_name, attrs));
                    continue;
                }

                let children: &[Value] = match obj.get("content") {
                    Some(value) => value.as_array().map(Vec::as_slice).ok_or_else(|| {
                        JsonParseError::InvalidStructure("\"content\" must be an array".into())
                    })?,
                    None => &[],
                };
                frames.push(Frame::BuildElement {
                    type_name,
                    attrs,
                    parent: spec,
                    child_count: children.len(),
                });
                let child_depth = depth.saturating_add(1);
                for child in children.iter().rev() {
                    frames.push(Frame::Visit {
                        json: child,
                        depth: child_depth,
                        placement: "unknown",
                    });
                }
            }
            Frame::BuildElement {
                type_name,
                attrs,
                parent,
                child_count,
            } => {
                let first_child = built.len().checked_sub(child_count).ok_or_else(|| {
                    JsonParseError::InvalidStructure("document parser child stack underflow".into())
                })?;
                let children = built
                    .split_off(first_child)
                    .into_iter()
                    .filter(|child| child.node_type() != "__skip")
                    .collect();
                let children =
                    resolve_opaque_placements(children, parent, schema, &budget.placement)?;
                built.push(Node::element(type_name, attrs, Fragment::from(children)));
            }
        }
    }
    if built.len() != 1 {
        return Err(JsonParseError::InvalidStructure(
            "document parser did not produce one root".into(),
        ));
    }
    Ok(built.pop().expect("one parsed root"))
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
    _mode: UnknownTypeMode,
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
            return Err(JsonParseError::UnknownMark(mark_type.to_string()));
        }

        let attrs = parse_mark_attrs(mark_obj, schema.mark(mark_type));
        marks.push(Mark::new(mark_type.to_string(), attrs));
    }

    Ok(marks)
}

/// Parse mark attributes from a mark JSON object — only attrs the schema
/// declares for this mark, unless the spec opts into undeclared attrs
/// (parity with the node path's parse_attrs). Marks whose type is unknown
/// to the schema get no attrs at all.
fn parse_mark_attrs(
    mark_obj: &serde_json::Map<String, Value>,
    spec: Option<&crate::schema::MarkSpec>,
) -> HashMap<String, Value> {
    let Some(spec) = spec else {
        return HashMap::new();
    };
    let mut attrs = spec
        .attrs
        .iter()
        .filter_map(|(name, attr)| attr.default.clone().map(|value| (name.clone(), value)))
        .collect::<HashMap<_, _>>();
    if let Some(Value::Object(attrs_obj)) = mark_obj.get("attrs") {
        for (name, value) in attrs_obj {
            if spec.allow_undeclared_attrs || spec.attrs.contains_key(name) {
                attrs.insert(name.clone(), value.clone());
            }
        }
    }
    attrs
}

/// Parse node attributes from a node's JSON object, filling in schema defaults
/// for any missing attributes.
fn parse_attrs(
    obj: &serde_json::Map<String, Value>,
    spec: &crate::schema::NodeSpec,
) -> HashMap<String, Value> {
    let mut attrs = super::default_node_attrs(spec);

    // Overlay with values from JSON — only attrs the schema declares for this
    // node (parity with the HTML path's extract_node_attrs), unless the spec
    // opts into undeclared attrs (e.g. mention nodes carrying app metadata).
    if let Some(Value::Object(json_attrs)) = obj.get("attrs") {
        for (key, value) in json_attrs {
            if spec.allow_undeclared_attrs || spec.attrs.contains_key(key) {
                attrs.insert(key.clone(), value.clone());
            }
        }
    }

    attrs
}

#[derive(Clone, Copy)]
enum PlacementChoice {
    Known,
    Inline,
    Block,
}

fn resolve_opaque_placements(
    mut children: Vec<Node>,
    parent: &crate::schema::NodeSpec,
    schema: &Schema,
    budget: &WorkBudget,
) -> Result<Vec<Node>, JsonParseError> {
    if !children
        .iter()
        .any(|node| node.node_type() == "__opaque_json")
    {
        return Ok(children);
    }
    let choices = parent
        .content
        .choose_matches(
            &children,
            |child, symbol| placement_choice(child, symbol, schema),
            budget,
        )
        .map_err(|_| JsonParseError::ResourceLimit {
            limit: children.len().saturating_mul(64),
            actual: children.len().saturating_mul(64).saturating_add(1),
        })?
        .ok_or_else(|| {
            JsonParseError::InvalidStructure(format!(
                "opaque children are incompatible with parent '{}'",
                parent.name
            ))
        })?;
    for (index, choice) in choices.into_iter().enumerate() {
        let placement = match choice {
            PlacementChoice::Known => continue,
            PlacementChoice::Inline => "inline",
            PlacementChoice::Block => "block",
        };
        let original_type = children[index]
            .attrs()
            .get("original_type")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        let original_json = children[index]
            .attrs()
            .get("original_json")
            .cloned()
            .unwrap_or(Value::Null);
        children[index] = build_opaque_json_node(&original_type, &original_json, placement);
    }
    Ok(children)
}

fn placement_choice(child: &Node, symbol: &str, schema: &Schema) -> Option<PlacementChoice> {
    if child.node_type() != "__opaque_json" {
        return schema
            .node_matches_symbol(child.node_type(), symbol)
            .then_some(PlacementChoice::Known);
    }
    let inline = schema.symbol_accepts_opaque_placement(symbol, "inline");
    let block = schema.symbol_accepts_opaque_placement(symbol, "block");
    if block {
        Some(PlacementChoice::Block)
    } else if inline {
        Some(PlacementChoice::Inline)
    } else {
        None
    }
}

/// Build an opaque node for an unknown type (Preserve mode).
///
/// The original JSON is stored in the attrs so it can survive round-trips.
fn build_opaque_json_node(type_name: &str, original_json: &Value, placement: &str) -> Node {
    let mut attrs = HashMap::new();
    attrs.insert(
        "original_type".to_string(),
        Value::String(type_name.to_string()),
    );
    attrs.insert("original_json".to_string(), original_json.clone());
    attrs.insert(
        "opaque_placement".to_string(),
        Value::String(placement.to_string()),
    );
    Node::void("__opaque_json".to_string(), attrs)
}

pub(crate) fn rehydrate_reserved_html_opaque(document: &Document) -> Document {
    Document::new(rehydrate_reserved_html_opaque_node(document.root()))
}

pub(crate) fn normalized_wire_json_node_type(tag: &str, attrs: &Map<String, Value>) -> String {
    if tag == "heading" {
        let level = attrs.get("level").and_then(|value| match value {
            Value::Number(number) => number
                .as_u64()
                .and_then(|value| u8::try_from(value).ok())
                .or_else(|| number.as_i64().and_then(|value| u8::try_from(value).ok()))
                .or_else(|| {
                    number.as_f64().and_then(|value| {
                        (value.is_finite() && value.fract() == 0.0)
                            .then(|| u8::try_from(value as i64).ok())
                            .flatten()
                    })
                }),
            Value::String(value) => parse_wire_heading_level_str(value),
            _ => None,
        });
        if let Some(level @ 1..=6) = level {
            return format!("h{level}");
        }
    }
    tag.to_string()
}

fn rehydrate_reserved_html_opaque_node(node: &Node) -> Node {
    if let Some(attrs) = reserved_html_opaque_attrs(node) {
        return Node::void("__opaque".to_string(), attrs);
    }
    if node.is_text() {
        return Node::text(
            node.text_str().unwrap_or_default().to_string(),
            node.marks().to_vec(),
        );
    }
    if node.is_void() {
        return Node::void(node.node_type().to_string(), node.attrs().clone());
    }
    let children = node
        .content()
        .into_iter()
        .flat_map(Fragment::iter)
        .map(rehydrate_reserved_html_opaque_node)
        .collect();
    Node::element(
        node.node_type().to_string(),
        node.attrs().clone(),
        Fragment::from(children),
    )
}

fn reserved_html_opaque_attrs(node: &Node) -> Option<HashMap<String, Value>> {
    if node.node_type() != "__opaque_json" {
        return None;
    }
    let original = node.attrs().get("original_json")?.as_object()?;
    if original.get("type")?.as_str()? != "__opaque" {
        return None;
    }
    let attrs = original.get("attrs")?.as_object()?;
    Some(
        attrs
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect(),
    )
}

#[cfg(test)]
fn normalize_json_aliases(value: &Value) -> Cow<'_, Value> {
    match value {
        Value::Array(values) => normalize_json_array_aliases(value, values),
        Value::Object(object) => normalize_json_object_aliases(value, object),
        _ => Cow::Borrowed(value),
    }
}

#[cfg(test)]
fn normalize_json_array_aliases<'a>(value: &'a Value, values: &'a [Value]) -> Cow<'a, Value> {
    let mut normalized = None;
    for (index, child) in values.iter().enumerate() {
        match (normalized.as_mut(), normalize_json_aliases(child)) {
            (None, Cow::Borrowed(_)) => {}
            (None, Cow::Owned(child)) => {
                let mut next = Vec::with_capacity(values.len());
                next.extend_from_slice(&values[..index]);
                next.push(child);
                normalized = Some(next);
            }
            (Some(next), Cow::Borrowed(child)) => next.push(child.clone()),
            (Some(next), Cow::Owned(child)) => next.push(child),
        }
    }

    normalized
        .map(|values| Cow::Owned(Value::Array(values)))
        .unwrap_or(Cow::Borrowed(value))
}

#[cfg(test)]
fn normalize_json_object_aliases<'a>(
    value: &'a Value,
    object: &'a Map<String, Value>,
) -> Cow<'a, Value> {
    let mut normalized = None;
    for (index, (key, child)) in object.iter().enumerate() {
        match (normalized.as_mut(), normalize_json_aliases(child)) {
            (None, Cow::Borrowed(_)) => {}
            (None, Cow::Owned(child)) => {
                let mut next = Map::with_capacity(object.len());
                next.extend(
                    object
                        .iter()
                        .take(index)
                        .map(|(key, value)| (key.clone(), value.clone())),
                );
                next.insert(key.clone(), child);
                normalized = Some(next);
            }
            (Some(next), Cow::Borrowed(child)) => {
                next.insert(key.clone(), child.clone());
            }
            (Some(next), Cow::Owned(child)) => {
                next.insert(key.clone(), child);
            }
        }
    }

    let type_name = object.get("type").and_then(Value::as_str);
    if type_name == Some("heading") {
        let level = object
            .get("attrs")
            .and_then(Value::as_object)
            .and_then(|attrs| parse_heading_level_value(attrs.get("level")));
        if let Some(level) = level {
            let normalized = normalized.get_or_insert_with(|| object.clone());
            normalized.insert("type".to_string(), Value::String(format!("h{level}")));
            if let Some(Value::Object(attrs)) = normalized.get_mut("attrs") {
                attrs.remove("level");
                if attrs.is_empty() {
                    normalized.remove("attrs");
                }
            }
        }
    }

    normalized
        .map(|object| Cow::Owned(Value::Object(object)))
        .unwrap_or(Cow::Borrowed(value))
}

fn parse_heading_level_value(value: Option<&Value>) -> Option<u8> {
    let value = value?;
    let level = match value {
        Value::Number(number) => number.as_u64().and_then(|value| u8::try_from(value).ok())?,
        Value::String(value) => parse_wire_heading_level_str(value)?,
        _ => return None,
    };

    if (1..=6).contains(&level) {
        Some(level)
    } else {
        None
    }
}

pub(crate) fn parse_wire_heading_level_str(value: &str) -> Option<u8> {
    // A u8 can contain at most three decimal digits. Capping before parsing
    // prevents arbitrarily long leading-zero strings from becoming an
    // unmetered scan in Yrs-backed normalization.
    (value.len() <= 3)
        .then(|| value.parse::<u8>().ok())
        .flatten()
        .filter(|level| (1..=6).contains(level))
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use serde_json::json;

    use super::normalize_json_aliases;

    #[test]
    fn canonical_json_alias_normalization_borrows_the_original_tree() {
        let canonical = json!({
            "type": "doc",
            "content": [{
                "type": "h2",
                "content": [{ "type": "text", "text": "Already canonical" }]
            }, {
                "type": "paragraph",
                "content": [{ "type": "text", "text": "No aliases" }]
            }]
        });

        assert!(matches!(
            normalize_json_aliases(&canonical),
            Cow::Borrowed(value) if std::ptr::eq(value, &canonical)
        ));
    }

    #[test]
    fn legacy_heading_alias_normalization_preserves_recursive_rewrites() {
        let legacy = json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "attrs": {
                    "metadata": {
                        "type": "heading",
                        "attrs": { "level": "3", "id": "nested" }
                    }
                }
            }, {
                "type": "heading",
                "attrs": { "level": 2, "id": "visible" },
                "content": [{ "type": "text", "text": "Legacy" }]
            }]
        });
        let expected = json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "attrs": {
                    "metadata": {
                        "type": "h3",
                        "attrs": { "id": "nested" }
                    }
                }
            }, {
                "type": "h2",
                "attrs": { "id": "visible" },
                "content": [{ "type": "text", "text": "Legacy" }]
            }]
        });

        assert_eq!(
            normalize_json_aliases(&legacy),
            Cow::Owned::<serde_json::Value>(expected)
        );
    }
}
