use std::collections::BTreeMap;

use serde::Serialize;

use super::{AttrSpec, MarkSpec, NodeRole, NodeSpec, Schema};

#[cfg(test)]
std::thread_local! {
    static SCHEMA_FINGERPRINT_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(crate) fn reset_schema_fingerprint_count_for_test() {
    SCHEMA_FINGERPRINT_COUNT.set(0);
}

#[cfg(test)]
pub(crate) fn take_schema_fingerprint_count_for_test() -> usize {
    SCHEMA_FINGERPRINT_COUNT.replace(0)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CanonicalSchema<'a> {
    nodes: BTreeMap<&'a str, CanonicalNode<'a>>,
    marks: BTreeMap<&'a str, CanonicalMark<'a>>,
    mark_order: Vec<&'a str>,
    node_html_tags: BTreeMap<&'a str, &'a str>,
    mark_html_tags: BTreeMap<&'a str, &'a str>,
    preferred_text_block_name: Option<&'a str>,
    fallback_list_item_name: Option<&'a str>,
    document_node_name: &'a str,
    text_node_name: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CanonicalNode<'a> {
    content: &'a str,
    group: Vec<&'a str>,
    attrs: BTreeMap<&'a str, CanonicalAttr>,
    role: &'static str,
    html_tag: Option<&'a str>,
    is_void: bool,
    allow_undeclared_attrs: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CanonicalMark<'a> {
    html_tag: Option<&'a str>,
    attrs: BTreeMap<&'a str, CanonicalAttr>,
    excludes: Option<&'a str>,
    allow_undeclared_attrs: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CanonicalAttr {
    has_default: bool,
    default: CanonicalJsonValue,
}

#[derive(Serialize)]
#[serde(tag = "type", content = "value", rename_all = "camelCase")]
enum CanonicalJsonValue {
    Null,
    Bool(bool),
    Number(String),
    String(String),
    Array(Vec<CanonicalJsonValue>),
    Object(BTreeMap<String, CanonicalJsonValue>),
}

impl From<&serde_json::Value> for CanonicalJsonValue {
    fn from(value: &serde_json::Value) -> Self {
        match value {
            serde_json::Value::Null => Self::Null,
            serde_json::Value::Bool(value) => Self::Bool(*value),
            serde_json::Value::Number(value) => {
                let value = value
                    .as_f64()
                    .expect("JSON numbers are representable as finite binary64 values");
                let normalized = if value == 0.0 { 0.0 } else { value };
                Self::Number(format!("{:016x}", normalized.to_bits()))
            }
            serde_json::Value::String(value) => Self::String(value.clone()),
            serde_json::Value::Array(values) => {
                Self::Array(values.iter().map(CanonicalJsonValue::from).collect())
            }
            serde_json::Value::Object(values) => Self::Object(
                values
                    .iter()
                    .map(|(name, value)| (name.clone(), CanonicalJsonValue::from(value)))
                    .collect(),
            ),
        }
    }
}

impl CanonicalAttr {
    fn from_spec(spec: &AttrSpec) -> Self {
        Self {
            has_default: spec.has_default,
            default: spec
                .default
                .as_ref()
                .map(CanonicalJsonValue::from)
                .unwrap_or(CanonicalJsonValue::Null),
        }
    }
}

impl<'a> From<&'a NodeSpec> for CanonicalNode<'a> {
    fn from(spec: &'a NodeSpec) -> Self {
        let mut group = spec
            .group
            .as_deref()
            .into_iter()
            .flat_map(str::split_whitespace)
            .collect::<Vec<_>>();
        group.sort_unstable();
        group.dedup();

        Self {
            content: spec.content.source(),
            group,
            attrs: spec
                .attrs
                .iter()
                .map(|(name, attr)| (name.as_str(), CanonicalAttr::from_spec(attr)))
                .collect(),
            role: match spec.role {
                NodeRole::Doc => "doc",
                NodeRole::TextBlock => "textBlock",
                NodeRole::List { ordered: true } => "listOrdered",
                NodeRole::List { ordered: false } => "listUnordered",
                NodeRole::ListItem => "listItem",
                NodeRole::Text => "text",
                NodeRole::HardBreak => "hardBreak",
                NodeRole::Inline => "inline",
                NodeRole::Block => "block",
            },
            html_tag: spec.html_tag.as_deref(),
            is_void: spec.is_void,
            allow_undeclared_attrs: spec.allow_undeclared_attrs,
        }
    }
}

impl<'a> From<&'a MarkSpec> for CanonicalMark<'a> {
    fn from(spec: &'a MarkSpec) -> Self {
        Self {
            html_tag: spec.html_tag.as_deref(),
            attrs: spec
                .attrs
                .iter()
                .map(|(name, attr)| (name.as_str(), CanonicalAttr::from_spec(attr)))
                .collect(),
            excludes: spec.excludes.as_deref(),
            allow_undeclared_attrs: spec.allow_undeclared_attrs,
        }
    }
}

impl<'a> From<&'a Schema> for CanonicalSchema<'a> {
    fn from(schema: &'a Schema) -> Self {
        Self {
            nodes: schema
                .nodes
                .iter()
                .map(|(name, spec)| (name.as_str(), CanonicalNode::from(spec)))
                .collect(),
            marks: schema
                .marks
                .iter()
                .map(|(name, spec)| (name.as_str(), CanonicalMark::from(spec)))
                .collect(),
            mark_order: schema.mark_order.iter().map(String::as_str).collect(),
            node_html_tags: schema
                .node_html_tags
                .iter()
                .map(|(tag, name)| (tag.as_str(), name.as_str()))
                .collect(),
            mark_html_tags: schema
                .mark_html_tags
                .iter()
                .map(|(tag, name)| (tag.as_str(), name.as_str()))
                .collect(),
            preferred_text_block_name: schema.preferred_text_block_name.as_deref(),
            fallback_list_item_name: schema.fallback_list_item_name.as_deref(),
            document_node_name: &schema.doc_node_name,
            text_node_name: &schema.text_node_name,
        }
    }
}

pub(crate) fn schema_fingerprint(schema: &Schema) -> String {
    #[cfg(test)]
    SCHEMA_FINGERPRINT_COUNT.set(SCHEMA_FINGERPRINT_COUNT.get().saturating_add(1));
    use sha2::{Digest, Sha256};
    let canonical = CanonicalSchema::from(schema);
    let bytes =
        serde_json::to_vec(&canonical).expect("canonical schema projection is serializable");
    format!("{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use serde::Deserialize;
    use serde_json::{json, Value};

    use super::schema_fingerprint;
    use crate::schema::content_rule::ContentRule;
    use crate::schema::{AttrSpec, NodeRole, NodeSpec, Schema};

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct FingerprintFixtures {
        fingerprints: Vec<FingerprintFixture>,
        equivalent_schemas: Vec<EquivalentFixture>,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct FingerprintFixture {
        name: String,
        schema: Value,
        expected_fingerprint: String,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct EquivalentFixture {
        name: String,
        schemas: Vec<Value>,
        expected_fingerprint: String,
    }

    fn fixture_schema(reverse_keys: bool) -> Value {
        let metadata = if reverse_keys {
            json!({ "z": [{ "b": 2, "a": 1 }], "a": { "d": 4, "c": 3 } })
        } else {
            json!({ "a": { "c": 3, "d": 4 }, "z": [{ "a": 1, "b": 2 }] })
        };
        json!({
            "nodes": [
                { "name": "doc", "content": "paragraph", "role": "doc" },
                {
                    "name": "paragraph",
                    "content": "text*",
                    "group": "secondary block block",
                    "attrs": {
                        "metadata": { "default": metadata },
                        "alignment": { "default": "start" }
                    },
                    "role": "textBlock",
                    "htmlTag": "p"
                },
                { "name": "text", "content": "", "group": "inline", "role": "text" }
            ],
            "marks": []
        })
    }

    fn schema_with_attr(attr: Value) -> Value {
        json!({
            "nodes": [
                { "name": "doc", "content": "paragraph", "role": "doc" },
                { "name": "paragraph", "content": "text*", "role": "textBlock" },
                { "name": "text", "content": "", "role": "text" }
            ],
            "marks": [{ "name": "custom", "attrs": { "value": attr } }]
        })
    }

    fn schema_with_duplicate_tag(first_name: &str) -> Schema {
        let second_name = if first_name == "alpha" {
            "beta"
        } else {
            "alpha"
        };
        Schema::from_json(&json!({
            "nodes": [
                { "name": "doc", "content": "block+", "role": "doc" },
                {
                    "name": first_name,
                    "content": "text*",
                    "group": "block",
                    "role": "textBlock",
                    "htmlTag": "p"
                },
                {
                    "name": second_name,
                    "content": "text*",
                    "group": "block",
                    "role": "textBlock",
                    "htmlTag": "p"
                },
                { "name": "text", "content": "", "role": "text" }
            ],
            "marks": []
        }))
        .unwrap()
    }

    fn schema_with_list_role(ordered: bool) -> Schema {
        Schema::new(
            vec![
                NodeSpec {
                    name: "doc".into(),
                    content: ContentRule::parse("list").unwrap(),
                    group: None,
                    attrs: HashMap::new(),
                    role: NodeRole::Doc,
                    html_tag: None,
                    is_void: false,
                    allow_undeclared_attrs: false,
                },
                NodeSpec {
                    name: "list".into(),
                    content: ContentRule::parse("").unwrap(),
                    group: None,
                    attrs: HashMap::new(),
                    role: NodeRole::List { ordered },
                    html_tag: Some("ol".into()),
                    is_void: false,
                    allow_undeclared_attrs: false,
                },
                NodeSpec {
                    name: "text".into(),
                    content: ContentRule::parse("").unwrap(),
                    group: None,
                    attrs: HashMap::<String, AttrSpec>::new(),
                    role: NodeRole::Text,
                    html_tag: None,
                    is_void: false,
                    allow_undeclared_attrs: false,
                },
            ],
            vec![],
        )
    }

    #[test]
    fn fingerprint_is_independent_of_object_key_order() {
        let left = Schema::from_json(&fixture_schema(false)).unwrap();
        let right = Schema::from_json(&fixture_schema(true)).unwrap();
        assert_eq!(schema_fingerprint(&left), schema_fingerprint(&right));
    }

    #[test]
    fn fingerprint_distinguishes_missing_and_explicit_null_defaults() {
        let missing = Schema::from_json(&schema_with_attr(json!({}))).unwrap();
        let explicit = Schema::from_json(&schema_with_attr(json!({ "default": null }))).unwrap();
        assert_ne!(schema_fingerprint(&missing), schema_fingerprint(&explicit));
    }

    #[test]
    fn fingerprint_changes_with_content_rule() {
        let left = Schema::from_json(&json!({
            "nodes": [
                { "name": "doc", "content": "paragraph", "role": "doc" },
                { "name": "paragraph", "content": "text*", "role": "textBlock" },
                { "name": "text", "content": "", "role": "text" }
            ],
            "marks": []
        }))
        .unwrap();
        let right = Schema::from_json(&json!({
            "nodes": [
                { "name": "doc", "content": "paragraph+", "role": "doc" },
                { "name": "paragraph", "content": "text*", "role": "textBlock" },
                { "name": "text", "content": "", "role": "text" }
            ],
            "marks": []
        }))
        .unwrap();
        assert_ne!(schema_fingerprint(&left), schema_fingerprint(&right));
    }

    #[test]
    fn fingerprint_preserves_duplicate_html_tag_precedence() {
        assert_ne!(
            schema_fingerprint(&schema_with_duplicate_tag("alpha")),
            schema_fingerprint(&schema_with_duplicate_tag("beta"))
        );
    }

    #[test]
    fn fingerprint_preserves_schema_mark_rank() {
        let schema = |marks| {
            Schema::from_json(&serde_json::json!({
                "nodes": [
                    { "name": "doc", "content": "paragraph", "role": "doc" },
                    { "name": "paragraph", "content": "text*", "role": "textBlock" },
                    { "name": "text", "content": "", "role": "text" }
                ],
                "marks": marks
            }))
            .unwrap()
        };
        let first = schema(serde_json::json!([
            {"name": "bold"},
            {"name": "italic"}
        ]));
        let second = schema(serde_json::json!([
            {"name": "italic"},
            {"name": "bold"}
        ]));

        assert_ne!(schema_fingerprint(&first), schema_fingerprint(&second));
    }

    #[test]
    fn fingerprint_distinguishes_ordered_and_unordered_list_roles() {
        assert_ne!(
            schema_fingerprint(&schema_with_list_role(true)),
            schema_fingerprint(&schema_with_list_role(false))
        );
    }

    #[test]
    fn fingerprints_match_checked_in_parity_fixtures() {
        let fixtures: FingerprintFixtures = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/schema-fingerprints.json"
        )))
        .unwrap();

        for fixture in fixtures.fingerprints {
            let schema = Schema::from_json(&fixture.schema).unwrap();
            assert_eq!(
                schema_fingerprint(&schema),
                fixture.expected_fingerprint,
                "fixture {}",
                fixture.name
            );
        }
        for fixture in fixtures.equivalent_schemas {
            for schema in fixture.schemas {
                let schema = Schema::from_json(&schema).unwrap();
                assert_eq!(
                    schema_fingerprint(&schema),
                    fixture.expected_fingerprint,
                    "equivalent fixture {}",
                    fixture.name
                );
            }
        }
    }
}
