use serde_json::{json, Map, Value};

use crate::model::{Document, Node};
use crate::schema::Schema;

/// Serialize a document to ProseMirror JSON format using the given schema.
///
/// The output matches the ProseMirror JSON representation:
/// ```json
/// {
///   "type": "doc",
///   "content": [
///     { "type": "paragraph", "content": [{ "type": "text", "text": "Hello" }] }
///   ]
/// }
/// ```
///
/// Node and mark type names are taken verbatim from the document tree (which
/// should already use the naming convention of the schema that created it).
/// Attrs are included only when non-empty, and default-valued attrs (per the
/// schema spec) are omitted.
pub fn to_prosemirror_json(doc: &Document, schema: &Schema) -> Value {
    node_to_json(doc.root(), schema)
}

fn node_to_json(node: &Node, schema: &Schema) -> Value {
    let mut obj = Map::new();
    obj.insert("type".to_string(), json!(node.node_type()));

    if node.is_text() {
        obj.insert("text".to_string(), json!(node.text_str().unwrap_or("")));

        if !node.marks().is_empty() {
            let marks_json: Vec<Value> = node
                .marks()
                .iter()
                .map(|m| {
                    let mut mark_obj = Map::new();
                    mark_obj.insert("type".to_string(), json!(m.mark_type()));
                    if !m.attrs().is_empty() {
                        mark_obj.insert("attrs".to_string(), json!(m.attrs()));
                    }
                    Value::Object(mark_obj)
                })
                .collect();
            obj.insert("marks".to_string(), Value::Array(marks_json));
        }
    } else if node.is_element() {
        // Include non-default attrs
        let attrs_json = build_attrs_json(node, schema);
        if !attrs_json.is_empty() {
            obj.insert("attrs".to_string(), Value::Object(attrs_json));
        }

        // Include content if non-empty
        if let Some(content) = node.content() {
            if content.child_count() > 0 {
                let children: Vec<Value> =
                    content.iter().map(|c| node_to_json(c, schema)).collect();
                obj.insert("content".to_string(), Value::Array(children));
            }
        }
    } else {
        // Void node — include non-default attrs, no content
        let attrs_json = build_attrs_json(node, schema);
        if !attrs_json.is_empty() {
            obj.insert("attrs".to_string(), Value::Object(attrs_json));
        }
    }

    Value::Object(obj)
}

/// Build the attrs JSON object for a node, omitting attributes whose values
/// match the schema-defined defaults.
fn build_attrs_json(node: &Node, schema: &Schema) -> Map<String, Value> {
    let mut attrs_map = Map::new();
    let spec = schema.node(node.node_type());

    for (key, value) in node.attrs() {
        // Check if this value equals the schema default — if so, omit it
        let is_default = spec
            .and_then(|s| s.attrs.get(key))
            .and_then(|a| a.default.as_ref())
            .map(|d| d == value)
            .unwrap_or(false);

        if !is_default {
            attrs_map.insert(key.clone(), value.clone());
        }
    }

    attrs_map
}
