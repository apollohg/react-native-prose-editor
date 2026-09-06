pub mod content_rule;
#[allow(dead_code)]
mod fingerprint;
pub mod presets;

#[allow(unused_imports)]
pub(crate) use fingerprint::schema_fingerprint;
#[cfg(test)]
pub(crate) use fingerprint::{
    reset_schema_fingerprint_count_for_test, take_schema_fingerprint_count_for_test,
};

use std::cell::Cell;
use std::collections::{HashMap, HashSet, VecDeque};

use crate::boundary::{BoundaryError, BoundaryResult, ResourceLimits};
use crate::model::{Document, Fragment, Node};
use crate::schema::content_rule::{
    ContentRule, ContentRuleError, WorkBudget, DEFAULT_RUNTIME_WORK_LIMIT,
};

#[cfg(test)]
std::thread_local! {
    static SCHEMA_FROM_JSON_COUNT: Cell<usize> = const { Cell::new(0) };
}

#[cfg(test)]
pub(crate) fn reset_schema_from_json_count_for_test() {
    SCHEMA_FROM_JSON_COUNT.set(0);
}

#[cfg(test)]
pub(crate) fn take_schema_from_json_count_for_test() -> usize {
    SCHEMA_FROM_JSON_COUNT.replace(0)
}

#[derive(Debug)]
enum SchemaValidationError {
    Semantic(String),
    ResourceExhausted(String),
}

impl SchemaValidationError {
    fn semantic(message: impl Into<String>) -> Self {
        Self::Semantic(message.into())
    }

    fn resource(message: impl Into<String>) -> Self {
        Self::ResourceExhausted(message.into())
    }

    fn message(self) -> String {
        match self {
            Self::Semantic(message) | Self::ResourceExhausted(message) => message,
        }
    }
}

/// A schema defines the set of node types and mark types available in a document.
///
/// Node and mark names are plain strings, allowing the same schema structure to
/// support different naming conventions (e.g. camelCase for Tiptap, snake_case
/// for ProseMirror).
#[derive(Debug, Clone)]
pub struct Schema {
    nodes: HashMap<String, NodeSpec>,
    node_order: Vec<String>,
    marks: HashMap<String, MarkSpec>,
    mark_order: Vec<String>,
    node_html_tags: HashMap<String, String>,
    html_rules_by_tag: HashMap<String, Vec<String>>,
    json_node_types: HashMap<String, Vec<String>>,
    mark_html_tags: HashMap<String, String>,
    preferred_text_block_name: Option<String>,
    fallback_list_item_name: Option<String>,
    groups: HashMap<String, Vec<String>>,
    symbol_role_masks: HashMap<String, u8>,
    doc_node_name: String,
    text_node_name: String,
}

const OPAQUE_INLINE_ROLE: u8 = 1;
const OPAQUE_BLOCK_ROLE: u8 = 2;

/// Specification for a node type within a schema.
#[derive(Debug, Clone)]
pub struct NodeSpec {
    pub name: String,
    pub content: ContentRule,
    pub group: Option<String>,
    pub attrs: HashMap<String, AttrSpec>,
    pub role: NodeRole,
    pub html_tag: Option<String>,
    pub html_rules: Option<HtmlRules>,
    pub json_projection: Option<NodeJsonProjection>,
    /// If `true`, this node has no editable content (e.g. horizontal rule, hard break).
    pub is_void: bool,
    /// Whether collapsed backspace may delete this void block from an adjacent caret.
    pub deletable_on_backspace: Option<bool>,
    /// If `true`, JSON ingestion (`set_json`/`insert_content_json`) admits attrs
    /// on this node that are not declared in `attrs`, instead of filtering them
    /// out. Default `false`. This is an opt-in escape hatch for node types with
    /// an intentional pass-through-metadata contract (e.g. the `mention` node,
    /// which carries arbitrary app-defined attrs such as `id`/`kind`). Every
    /// other node type is filtered to its schema-declared attrs, matching the
    /// HTML ingestion path (`extract_node_attrs`).
    pub allow_undeclared_attrs: bool,
}

#[derive(Debug, Clone)]
pub struct HtmlRules {
    pub tag: String,
    pub static_attrs: Vec<(String, String)>,
    pub attr_map: Vec<(String, String)>,
}

#[derive(Debug, Clone)]
pub struct NodeJsonProjection {
    pub node_type: String,
    pub attrs: HashMap<String, serde_json::Value>,
}

pub(crate) fn json_projection_values_equal(
    left: &serde_json::Value,
    right: &serde_json::Value,
) -> bool {
    match (left.as_number(), right.as_number()) {
        (Some(left), Some(right)) => json_projection_numbers_equal(left, right),
        _ => left == right,
    }
}

fn json_projection_numbers_equal(left: &serde_json::Number, right: &serde_json::Number) -> bool {
    if left.is_f64() {
        return left
            .as_f64()
            .is_some_and(|left| json_projection_float_matches(left, right));
    }
    if right.is_f64() {
        return right
            .as_f64()
            .is_some_and(|right| json_projection_float_matches(right, left));
    }
    left.as_i64()
        .zip(right.as_i64())
        .is_some_and(|(left, right)| left == right)
        || left
            .as_u64()
            .zip(right.as_u64())
            .is_some_and(|(left, right)| left == right)
}

fn json_projection_float_matches(value: f64, number: &serde_json::Number) -> bool {
    if number.is_f64() {
        return number.as_f64() == Some(value);
    }
    if let Some(integer) = number.as_i64() {
        return integer_is_exact_binary64(integer.unsigned_abs()) && (integer as f64) == value;
    }
    number
        .as_u64()
        .is_some_and(|integer| integer_is_exact_binary64(integer) && (integer as f64) == value)
}

fn integer_is_exact_binary64(magnitude: u64) -> bool {
    if magnitude == 0 {
        return true;
    }
    let significant_bits = u64::BITS - magnitude.leading_zeros();
    significant_bits <= 53 || magnitude.trailing_zeros() >= significant_bits - 53
}

fn legacy_heading_projection_name(projection: &NodeJsonProjection) -> Option<String> {
    if projection.node_type != "heading" {
        return None;
    }
    let level = match projection.attrs.get("level")? {
        serde_json::Value::Number(number) => number
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
        serde_json::Value::String(value) => (value.len() <= 3)
            .then(|| value.parse::<u8>().ok())
            .flatten(),
        _ => None,
    }?;
    (1..=6).contains(&level).then(|| format!("h{level}"))
}

/// The semantic role of a node, used by transactions and rendering to handle
/// node types generically without hardcoding names.
#[derive(Debug, Clone)]
pub enum NodeRole {
    Doc,
    TextBlock,
    List { ordered: bool },
    ListItem,
    Text,
    HardBreak,
    Inline,
    Block,
}

/// Specification for a mark type within a schema.
#[derive(Debug, Clone)]
pub struct MarkSpec {
    pub name: String,
    /// Optional validated HTML tag used for importing and exporting this mark.
    pub html_tag: Option<String>,
    pub attrs: HashMap<String, AttrSpec>,
    /// Marks in the `excludes` set cannot coexist with this mark on the same
    /// text range. `None` means no exclusions.
    pub excludes: Option<String>,
    /// If `true`, JSON ingestion (`set_json`/`insert_content_json`) admits attrs
    /// on this mark that are not declared in `attrs`, instead of filtering them
    /// out. Default `false`. This is an opt-in escape hatch mirroring
    /// `NodeSpec::allow_undeclared_attrs` for mark types with an intentional
    /// pass-through-metadata contract. Every other mark type is filtered to
    /// its schema-declared attrs.
    pub allow_undeclared_attrs: bool,
}

/// Specification for a single attribute on a node or mark type.
#[derive(Debug, Clone, Default)]
pub struct AttrSpec {
    pub default: Option<serde_json::Value>,
    pub has_default: bool,
    pub constraints: std::collections::BTreeMap<String, serde_json::Value>,
}

fn attribute_values_equal(left: &serde_json::Value, right: &serde_json::Value) -> bool {
    match (left, right) {
        (serde_json::Value::Array(a), serde_json::Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| attribute_values_equal(a, b))
        }
        (serde_json::Value::Object(a), serde_json::Value::Object(b)) => {
            a.len() == b.len()
                && a.iter().all(|(key, value)| {
                    b.get(key)
                        .is_some_and(|other| attribute_values_equal(value, other))
                })
        }
        _ => json_projection_values_equal(left, right),
    }
}

impl AttrSpec {
    fn from_json(value: &serde_json::Value) -> Result<Self, BoundaryError> {
        let mut spec = Self {
            default: value.get("default").cloned(),
            has_default: value.get("default").is_some(),
            ..Self::default()
        };
        if let Some(fields) = value.as_object() {
            for (key, value) in fields {
                if key == "default" {
                    continue;
                }
                if !["type", "enum", "min", "max"].contains(&key.as_str()) {
                    return Err(BoundaryError::new(
                        "SCHEMA_INVALID",
                        "unknown attribute constraint",
                    ));
                }
                spec.constraints.insert(key.clone(), value.clone());
            }
        }
        spec.validate_definition()
            .map_err(|message| BoundaryError::new("SCHEMA_INVALID", message))?;
        Ok(spec)
    }

    pub fn declared_type(&self) -> Option<&str> {
        self.constraints.get("type").and_then(|v| v.as_str())
    }

    pub fn validate_value(&self, value: &serde_json::Value) -> Result<(), String> {
        let valid = match self.declared_type() {
            Some("string") => value.is_string(),
            Some("number") => value.is_number(),
            Some("boolean") => value.is_boolean(),
            Some("array") => value.is_array(),
            Some("object") => value.is_object(),
            None => true,
            _ => false,
        };
        if !valid {
            return Err("attribute has invalid type".into());
        }
        if let Some(values) = self.constraints.get("enum").and_then(|v| v.as_array()) {
            if !values
                .iter()
                .any(|candidate| attribute_values_equal(candidate, value))
            {
                return Err("attribute outside enum".into());
            }
        }
        let size = value
            .as_f64()
            .or_else(|| value.as_str().map(|s| s.chars().count() as f64))
            .or_else(|| value.as_array().map(|a| a.len() as f64));
        for (key, minimum) in [("min", true), ("max", false)] {
            if let Some(bound) = self.constraints.get(key).and_then(|v| v.as_f64()) {
                if size.map_or(
                    true,
                    |size| if minimum { size < bound } else { size > bound },
                ) {
                    return Err("attribute outside bounds".into());
                }
            }
        }
        Ok(())
    }

    fn validate_definition(&self) -> Result<(), String> {
        if self.constraints.contains_key("type")
            && !matches!(
                self.declared_type(),
                Some("string" | "number" | "boolean" | "object" | "array")
            )
        {
            return Err("invalid attribute type".into());
        }
        for key in ["min", "max"] {
            if let Some(value) = self.constraints.get(key) {
                let bound = value.as_f64().ok_or("invalid attribute bound")?;
                if !matches!(self.declared_type(), Some("number" | "string" | "array"))
                    || (self.declared_type() != Some("number")
                        && (bound < 0.0 || bound.fract() != 0.0))
                {
                    return Err("invalid attribute bounds".into());
                }
            }
        }
        if let (Some(min), Some(max)) = (
            self.constraints.get("min").and_then(|v| v.as_f64()),
            self.constraints.get("max").and_then(|v| v.as_f64()),
        ) {
            if min > max {
                return Err("invalid attribute bounds".into());
            }
        }
        if let Some(values) = self.constraints.get("enum") {
            let values = values
                .as_array()
                .filter(|v| !v.is_empty())
                .ok_or("invalid attribute enum")?;
            let kind = std::mem::discriminant(&values[0]);
            for value in values {
                if std::mem::discriminant(value) != kind {
                    return Err("attribute enum values must share one JSON type".into());
                }
                self.validate_value(value)?;
            }
        }
        if let Some(value) = &self.default {
            self.validate_value(value)?;
        }
        Ok(())
    }
}

const DEFAULT_DOCUMENT_MAX_DEPTH: usize = 128;
const DEFAULT_DOCUMENT_MAX_NODES: usize = 10_000;
const DEFAULT_DOCUMENT_MAX_WORK: usize = 10_000;
pub(crate) const MAX_SCHEMA_METADATA_DEPTH: usize = 128;

struct DefaultConstructionBudget {
    work: Cell<usize>,
    nodes: Cell<usize>,
}

impl DefaultConstructionBudget {
    fn consume_work(&self) -> bool {
        if self.work.get() >= DEFAULT_DOCUMENT_MAX_WORK {
            return false;
        }
        self.work.set(self.work.get() + 1);
        true
    }
}

include!("construction.rs");
include!("queries.rs");
include!("json.rs");

fn consume_schema_work(
    budget: &WorkBudget,
    amount: usize,
    message: &'static str,
) -> Result<(), SchemaValidationError> {
    if budget.consume_n(amount) {
        Ok(())
    } else {
        Err(SchemaValidationError::resource(message))
    }
}

fn consume_schema_boundary_work(
    budget: &WorkBudget,
    amount: usize,
    work_limit: usize,
) -> BoundaryResult<()> {
    if budget.consume_n(amount) {
        Ok(())
    } else {
        Err(schema_boundary_error(
            SchemaValidationError::resource("schema metadata work budget exceeded"),
            work_limit,
        ))
    }
}

fn admit_schema_string(value: &str, budget: &WorkBudget, work_limit: usize) -> BoundaryResult<()> {
    consume_schema_boundary_work(budget, value.len().saturating_add(1), work_limit)
}

fn admit_schema_groups(groups: &str, budget: &WorkBudget, work_limit: usize) -> BoundaryResult<()> {
    let mut in_token = false;
    for character in groups.chars() {
        consume_schema_boundary_work(budget, character.len_utf8(), work_limit)?;
        if character.is_whitespace() {
            in_token = false;
        } else if !in_token {
            consume_schema_boundary_work(budget, 1, work_limit)?;
            in_token = true;
        }
    }
    Ok(())
}

fn admit_schema_attrs(
    attrs: Option<&serde_json::Value>,
    budget: &WorkBudget,
    work_limit: usize,
) -> BoundaryResult<()> {
    let Some(attrs) = attrs.and_then(serde_json::Value::as_object) else {
        return Ok(());
    };
    for (name, spec) in attrs {
        consume_schema_boundary_work(budget, 1, work_limit)?;
        admit_schema_string(name, budget, work_limit)?;
        if let Some(default) = spec.get("default") {
            admit_schema_value(default, budget, work_limit, 1)?;
        }
        for key in ["type", "enum", "min", "max"] {
            if let Some(value) = spec.get(key) {
                admit_schema_value(value, budget, work_limit, 1)?;
            }
        }
    }
    Ok(())
}

fn parse_html_rules(
    value: Option<&serde_json::Value>,
    budget: &WorkBudget,
    work_limit: usize,
) -> BoundaryResult<Option<HtmlRules>> {
    let Some(value) = value else {
        return Ok(None);
    };
    let object = value.as_object().ok_or_else(|| {
        BoundaryError::new("SCHEMA_INVALID", "node atom HTML rules must be an object")
    })?;
    if object
        .keys()
        .any(|key| !matches!(key.as_str(), "tag" | "staticAttrs" | "attrMap"))
    {
        return Err(BoundaryError::new(
            "SCHEMA_INVALID",
            "node atom HTML rules contain an unknown field",
        ));
    }
    let tag = object
        .get("tag")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            BoundaryError::new("SCHEMA_INVALID", "node atom HTML rules missing 'tag'")
        })?;
    admit_schema_string(tag, budget, work_limit)?;

    let parse_map = |name: &str| -> BoundaryResult<Vec<(String, String)>> {
        let Some(value) = object.get(name) else {
            return Ok(Vec::new());
        };
        let map = value.as_object().ok_or_else(|| {
            BoundaryError::new(
                "SCHEMA_INVALID",
                format!("node atom HTML rules '{name}' must be an object"),
            )
        })?;
        let mut entries = Vec::with_capacity(map.len());
        for (key, value) in map {
            let value = value.as_str().ok_or_else(|| {
                BoundaryError::new(
                    "SCHEMA_INVALID",
                    format!("node atom HTML rules '{name}' values must be strings"),
                )
            })?;
            admit_schema_string(key, budget, work_limit)?;
            admit_schema_string(value, budget, work_limit)?;
            entries.push((key.clone(), value.to_string()));
        }
        entries.sort_by(|left, right| left.0.cmp(&right.0));
        Ok(entries)
    };

    Ok(Some(HtmlRules {
        tag: tag.to_string(),
        static_attrs: parse_map("staticAttrs")?,
        attr_map: parse_map("attrMap")?,
    }))
}

fn admit_schema_value(
    value: &serde_json::Value,
    budget: &WorkBudget,
    work_limit: usize,
    depth: usize,
) -> BoundaryResult<()> {
    if depth > MAX_SCHEMA_METADATA_DEPTH {
        return Err(schema_boundary_error(
            SchemaValidationError::resource("schema metadata nesting work budget exceeded"),
            work_limit,
        ));
    }
    consume_schema_boundary_work(budget, 1, work_limit)?;
    match value {
        serde_json::Value::String(value) => admit_schema_string(value, budget, work_limit),
        serde_json::Value::Array(values) => {
            for value in values {
                admit_schema_value(value, budget, work_limit, depth + 1)?;
            }
            Ok(())
        }
        serde_json::Value::Object(values) => {
            for (name, value) in values {
                admit_schema_string(name, budget, work_limit)?;
                admit_schema_value(value, budget, work_limit, depth + 1)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

pub(crate) fn is_safe_html_tag(tag: &str) -> bool {
    let mut chars = tag.chars();
    matches!(chars.next(), Some('a'..='z'))
        && chars.all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-')
}

pub(crate) fn is_safe_html_attr(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(ch) if ch.is_ascii_alphabetic() || ch == '_' || ch == ':')
        && chars.all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | ':' | '.' | '-'))
}

pub(crate) fn is_safe_atom_html_tag(tag: &str) -> bool {
    const DENIED: &[&str] = &[
        "script", "style", "iframe", "object", "embed", "link", "meta", "base", "title", "head",
        "html", "body", "form", "textarea", "select", "option", "button", "area", "br", "col",
        "hr", "img", "input", "param", "source", "track", "wbr",
    ];
    is_atom_html_identifier(tag) && !DENIED.contains(&tag)
}

pub(crate) fn is_safe_atom_html_attr(name: &str) -> bool {
    const DENIED: &[&str] = &[
        "style",
        "srcdoc",
        "href",
        "src",
        "srcset",
        "action",
        "formaction",
    ];
    is_atom_html_identifier(name) && !name.starts_with("on") && !DENIED.contains(&name)
}

fn is_atom_html_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    matches!(chars.next(), Some('a'..='z'))
        && chars.all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-')
}

fn content_rule_schema_error(name: &str, error: ContentRuleError) -> SchemaValidationError {
    match error {
        ContentRuleError::Semantic(message) => SchemaValidationError::semantic(format!(
            "content rule parse error for {name}: {message}"
        )),
        ContentRuleError::ResourceExhausted(message) => SchemaValidationError::resource(format!(
            "content rule parse error for {name}: {message}"
        )),
    }
}

fn schema_boundary_error(error: SchemaValidationError, work_limit: usize) -> BoundaryError {
    match error {
        SchemaValidationError::Semantic(message) => BoundaryError::new("SCHEMA_INVALID", message),
        SchemaValidationError::ResourceExhausted(message) => {
            let mut error =
                BoundaryError::limit("SCHEMA_INVALID", work_limit, work_limit.saturating_add(1));
            error.message = message;
            error.details = Some(serde_json::json!({ "phase": "schemaWork" }));
            error
        }
    }
}

fn default_node_priority(node: &NodeSpec) -> (u8, &str) {
    let priority = match node.role {
        NodeRole::TextBlock
            if node.html_tag.as_deref() == Some("p") || node.name == "paragraph" =>
        {
            0
        }
        NodeRole::TextBlock => 1,
        _ => 2,
    };
    (priority, node.name.as_str())
}

/// Check whether an `excludes` field covers a given mark name.
///
/// - `None` → no exclusions.
/// - `Some("_")` → excludes all marks.
/// - Otherwise, space-separated list of mark names.
fn mark_excluded_by(excludes: &Option<String>, mark_name: &str) -> bool {
    match excludes {
        None => false,
        Some(exc) => {
            if exc == "_" {
                return true;
            }
            exc.split_whitespace().any(|e| e == mark_name)
        }
    }
}
