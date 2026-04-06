use crate::model::{Document, Node};
use crate::schema::{NodeRole, Schema};

/// Serialize a document to an HTML string using the given schema for tag mappings.
///
/// The root "doc" node is not emitted — only its children are serialized.
pub fn to_html(doc: &Document, schema: &Schema) -> String {
    let mut buf = String::new();
    let root = doc.root();
    if let Some(content) = root.content() {
        for child in content.iter() {
            serialize_node(child, schema, &mut buf);
        }
    }
    buf
}

fn serialize_node(node: &Node, schema: &Schema, buf: &mut String) {
    if node.is_text() {
        let text = node.text_str().unwrap_or("");
        // Open mark tags
        for mark in node.marks() {
            serialize_mark_open(mark, buf);
        }
        escape_html(text, buf);
        // Close mark tags in reverse order
        for mark in node.marks().iter().rev() {
            let tag = mark_type_to_tag(mark.mark_type());
            buf.push_str("</");
            buf.push_str(tag);
            buf.push('>');
        }
        return;
    }

    let spec = schema.node(node.node_type());

    // Determine the HTML tag from the schema spec
    let html_tag = spec.and_then(|s| s.html_tag.as_deref());

    if node.is_void() {
        if node.node_type() == "mention" {
            serialize_mention_node(node, buf);
            return;
        }
        // Opaque nodes (unknown tags preserved from parsing)
        if node.node_type() == "__opaque" {
            serialize_opaque_node(node, buf);
            return;
        }
        if let Some(tag) = html_tag {
            buf.push('<');
            buf.push_str(tag);
            if let Some(spec) = spec {
                serialize_node_attrs(node, spec, buf);
            }
            buf.push('>');
        }
        return;
    }

    // Element node
    if let Some(tag) = html_tag {
        buf.push('<');
        buf.push_str(tag);

        if let Some(spec) = spec {
            serialize_node_attrs(node, spec, buf);
        }

        buf.push('>');
    }

    // Serialize children
    if let Some(content) = node.content() {
        for child in content.iter() {
            serialize_node(child, schema, buf);
        }
    }

    if let Some(tag) = html_tag {
        buf.push_str("</");
        buf.push_str(tag);
        buf.push('>');
    }
}

fn serialize_node_attrs(node: &Node, spec: &crate::schema::NodeSpec, buf: &mut String) {
    for key in spec.attrs.keys() {
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

fn serialize_mark_open(mark: &crate::model::Mark, buf: &mut String) {
    let tag = mark_type_to_tag(mark.mark_type());
    buf.push('<');
    buf.push_str(tag);
    if mark.mark_type() == "link" {
        if let Some(href) = mark.attrs().get("href").and_then(|value| value.as_str()) {
            buf.push_str(" href=\"");
            escape_html(href, buf);
            buf.push('"');
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
    let attrs_json = serde_json::to_string(attrs).unwrap_or_else(|_| "{}".to_string());

    buf.push_str("<span data-native-editor-mention=\"true\" data-native-editor-mention-attrs=\"");
    escape_html(&attrs_json, buf);
    buf.push_str("\">");
    escape_html(label, buf);
    buf.push_str("</span>");
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
                    buf.push_str(val_str);
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

/// Map mark type names to their HTML tag equivalents.
fn mark_type_to_tag(mark_type: &str) -> &str {
    match mark_type {
        "bold" => "strong",
        "italic" => "em",
        "underline" => "u",
        "strike" => "s",
        "link" => "a",
        _ => mark_type,
    }
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
