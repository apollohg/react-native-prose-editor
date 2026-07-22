use crate::model::{Document, Node};
use crate::schema::{NodeRole, Schema};

/// Serialize a document to an HTML string using the given schema for tag mappings.
///
/// The root "doc" node is not emitted — only its children are serialized.
pub fn to_html(doc: &Document, schema: &Schema) -> String {
    enum Frame<'a> {
        Node(&'a Node),
        Close(&'a str),
    }

    let mut buf = String::new();
    let mut frames = Vec::new();
    if let Some(content) = doc.root().content() {
        frames.extend(content.iter().rev().map(Frame::Node));
    }
    while let Some(frame) = frames.pop() {
        match frame {
            Frame::Close(tag) => {
                buf.push_str("</");
                buf.push_str(tag);
                buf.push('>');
            }
            Frame::Node(node) if node.is_text() => {
                let text = node.text_str().unwrap_or("");
                for mark in node.marks() {
                    serialize_mark_open(mark, schema, &mut buf);
                }
                escape_html(text, &mut buf);
                for mark in node.marks().iter().rev() {
                    let tag = mark_tag(mark, schema);
                    buf.push_str("</");
                    buf.push_str(tag.as_str());
                    buf.push('>');
                }
            }
            Frame::Node(node) => {
                let spec = schema.node(node.node_type());
                let html_tag = spec.and_then(|spec| spec.html_tag.as_deref());
                if node.is_void() {
                    if node.node_type() == "mention" {
                        serialize_mention_node(node, &mut buf);
                    } else if node.node_type() == "__opaque" {
                        serialize_opaque_node(node, &mut buf);
                    } else if let Some(tag) = html_tag {
                        buf.push('<');
                        buf.push_str(tag);
                        if let Some(spec) = spec {
                            serialize_node_attrs(node, spec, &mut buf);
                        }
                        buf.push('>');
                    }
                    continue;
                }
                if let Some(tag) = html_tag {
                    buf.push('<');
                    buf.push_str(tag);
                    if let Some(spec) = spec {
                        serialize_node_attrs(node, spec, &mut buf);
                    }
                    buf.push('>');
                    frames.push(Frame::Close(tag));
                }
                if let Some(content) = node.content() {
                    frames.extend(content.iter().rev().map(Frame::Node));
                }
            }
        }
    }
    buf
}

fn serialize_node_attrs(node: &Node, spec: &crate::schema::NodeSpec, buf: &mut String) {
    let mut keys = spec.attrs.keys().collect::<Vec<_>>();
    keys.sort_unstable();
    for key in keys {
        let Some(value) = node.attrs().get(key) else {
            continue;
        };
        if value.is_null() {
            continue;
        }
        if key == "start"
            && matches!(spec.role, NodeRole::List { ordered: true })
            && value.as_u64() == Some(1)
        {
            continue;
        }

        let rendered = if let Some(string_value) = value.as_str() {
            string_value.to_string()
        } else if let Some(bool_value) = value.as_bool() {
            bool_value.to_string()
        } else if let Some(number_value) = value.as_i64() {
            number_value.to_string()
        } else if let Some(number_value) = value.as_u64() {
            number_value.to_string()
        } else if let Some(number_value) = value.as_f64() {
            number_value.to_string()
        } else {
            continue;
        };

        buf.push(' ');
        buf.push_str(key);
        buf.push_str("=\"");
        escape_html(&rendered, buf);
        buf.push('"');
    }
}

fn serialize_mark_open(mark: &crate::model::Mark, schema: &Schema, buf: &mut String) {
    let tag = mark_tag(mark, schema);
    buf.push('<');
    buf.push_str(tag.as_str());
    if matches!(tag, MarkTag::DataSpan) {
        buf.push_str(" data-native-editor-mark=\"");
        escape_html(mark.mark_type(), buf);
        buf.push('"');
    }
    if let Some(spec) = schema.mark(mark.mark_type()) {
        let mut names = spec.attrs.keys().collect::<Vec<_>>();
        names.sort_unstable();
        for name in names {
            if let Some(value) = mark.attrs().get(name) {
                let rendered = value
                    .as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| json_value_string(value));
                buf.push(' ');
                buf.push_str(name);
                buf.push_str("=\"");
                escape_html(&rendered, buf);
                buf.push('"');
            }
        }
    }
    buf.push('>');
}

fn serialize_mention_node(node: &Node, buf: &mut String) {
    let attrs = node.attrs();
    let label = attrs
        .get("label")
        .and_then(|value| value.as_str())
        .filter(|value| !value.is_empty())
        .unwrap_or("@mention");
    let visible_label = crate::render::mention_label_with_trigger(label, attrs);
    let attrs_json = json_object_string(attrs);

    buf.push_str("<span data-native-editor-mention=\"true\" data-native-editor-mention-attrs=\"");
    escape_html(&attrs_json, buf);
    buf.push_str("\">");
    escape_html(&visible_label, buf);
    buf.push_str("</span>");
}

fn json_value_string(value: &serde_json::Value) -> String {
    String::from_utf8(crate::boundary::serialize_json_value_stack_safe(value, 0))
        .expect("serialized JSON is UTF-8")
}

fn json_object_string(attrs: &std::collections::HashMap<String, serde_json::Value>) -> String {
    let value = crate::boundary::StackSafeJsonValue::new(serde_json::Value::Object(
        attrs
            .iter()
            .map(|(key, value)| {
                (
                    key.clone(),
                    crate::boundary::clone_json_value_stack_safe(value),
                )
            })
            .collect(),
    ));
    json_value_string(value.as_value())
}

/// Serialize an opaque node (unknown tag preserved from parsing) back to HTML.
fn serialize_opaque_node(node: &Node, buf: &mut String) {
    let attrs = node.attrs();
    let tag = attrs
        .get("html_tag")
        .and_then(|v| v.as_str())
        .unwrap_or("span");

    buf.push('<');
    buf.push_str(tag);

    // Restore HTML attributes
    if let Some(html_attrs) = attrs.get("html_attrs") {
        if let Some(obj) = html_attrs.as_object() {
            for (key, val) in obj {
                if let Some(val_str) = val.as_str() {
                    buf.push(' ');
                    buf.push_str(key);
                    buf.push_str("=\"");
                    escape_html(val_str, buf);
                    buf.push('"');
                }
            }
        }
    }

    // Check if this is a void HTML element
    const VOID_HTML_ELEMENTS: &[&str] = &[
        "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param",
        "source", "track", "wbr",
    ];
    if VOID_HTML_ELEMENTS.contains(&tag) {
        buf.push('>');
        return;
    }

    buf.push('>');

    // Emit inner HTML if stored, otherwise use text content
    if let Some(inner_html) = attrs.get("inner_html").and_then(|v| v.as_str()) {
        buf.push_str(inner_html);
    } else if let Some(text) = attrs.get("text_content").and_then(|v| v.as_str()) {
        escape_html(text, buf);
    }

    buf.push_str("</");
    buf.push_str(tag);
    buf.push('>');
}

enum MarkTag<'a> {
    Element(&'a str),
    DataSpan,
}

impl MarkTag<'_> {
    fn as_str(&self) -> &str {
        match self {
            Self::Element(tag) => tag,
            Self::DataSpan => "span",
        }
    }
}

fn mark_tag<'a>(mark: &'a crate::model::Mark, schema: &'a Schema) -> MarkTag<'a> {
    schema
        .mark(mark.mark_type())
        .and_then(|spec| spec.html_tag.as_deref())
        .map(MarkTag::Element)
        .unwrap_or(MarkTag::DataSpan)
}

/// HTML-escape text content: `&`, `<`, `>`, `"`.
fn escape_html(text: &str, buf: &mut String) {
    for ch in text.chars() {
        match ch {
            '&' => buf.push_str("&amp;"),
            '<' => buf.push_str("&lt;"),
            '>' => buf.push_str("&gt;"),
            '"' => buf.push_str("&quot;"),
            _ => buf.push(ch),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::to_html;
    use crate::schema::Schema;
    use crate::serialize::{from_prosemirror_json, UnknownTypeMode};

    #[test]
    fn node_attributes_are_emitted_in_canonical_name_order() {
        let schema = Schema::from_json(&serde_json::json!({
            "nodes": [
                {"name":"doc","content":"media?","role":"doc"},
                {"name":"media","content":"","role":"block","isVoid":true,"htmlTag":"img","attrs":{
                    "zeta":{},"alpha":{},"mu":{},"beta":{},"theta":{},"gamma":{},"eta":{},"delta":{}
                }},
                {"name":"text","content":"","role":"text"}
            ],
            "marks": []
        }))
        .unwrap();
        let document = from_prosemirror_json(
            &serde_json::json!({"type":"doc","content":[{"type":"media","attrs":{
                "zeta":"z","alpha":"a","mu":"m","beta":"b","theta":"t","gamma":"g","eta":"e","delta":"d"
            }}]}),
            &schema,
            UnknownTypeMode::Error,
        )
        .unwrap();

        assert_eq!(
            to_html(&document, &schema),
            "<img alpha=\"a\" beta=\"b\" delta=\"d\" eta=\"e\" gamma=\"g\" mu=\"m\" theta=\"t\" zeta=\"z\">"
        );
    }

    #[test]
    fn mark_attributes_are_emitted_in_canonical_name_order() {
        let schema = Schema::from_json(&serde_json::json!({
            "nodes": [
                {"name":"doc","content":"paragraph","role":"doc"},
                {"name":"paragraph","content":"text*","role":"textBlock","htmlTag":"p"},
                {"name":"text","content":"","role":"text"}
            ],
            "marks": [{"name":"custom","htmlTag":"span","attrs":{
                "zeta":{},"alpha":{},"mu":{},"beta":{},"theta":{},"gamma":{},"eta":{},"delta":{}
            }}]
        }))
        .unwrap();
        let document = from_prosemirror_json(
            &serde_json::json!({"type":"doc","content":[{"type":"paragraph","content":[{
                "type":"text","text":"x","marks":[{"type":"custom","attrs":{
                    "zeta":"z","alpha":"a","mu":"m","beta":"b","theta":"t","gamma":"g","eta":"e","delta":"d"
                }}]
            }]}]}),
            &schema,
            UnknownTypeMode::Error,
        )
        .unwrap();

        assert_eq!(
            to_html(&document, &schema),
            "<p><span alpha=\"a\" beta=\"b\" delta=\"d\" eta=\"e\" gamma=\"g\" mu=\"m\" theta=\"t\" zeta=\"z\">x</span></p>"
        );
    }
}
