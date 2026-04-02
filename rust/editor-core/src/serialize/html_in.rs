use std::collections::HashMap;
use std::fmt;

use scraper::Html;

use crate::model::{Document, Fragment, Mark, Node};
use crate::schema::{NodeRole, Schema};

/// Type alias for a node reference in the scraper parse tree.
type SNodeRef<'a> = ego_tree::NodeRef<'a, scraper::Node>;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors returned by `from_html`.
#[derive(Debug, Clone)]
pub enum ParseError {
    /// An unknown HTML tag was encountered in strict mode.
    UnknownTag(String),
    /// The parsed content does not satisfy the schema's content rules.
    InvalidContent(String),
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::UnknownTag(tag) => write!(f, "unknown HTML tag: <{}>", tag),
            ParseError::InvalidContent(msg) => write!(f, "invalid content: {}", msg),
        }
    }
}

impl std::error::Error for ParseError {}

// ---------------------------------------------------------------------------
// Options
// ---------------------------------------------------------------------------

/// Options for `from_html`.
#[derive(Debug, Clone, Default)]
pub struct FromHtmlOptions {
    /// If `true`, unknown HTML tags produce an error instead of being preserved
    /// as opaque nodes.
    pub strict: bool,
}

// ---------------------------------------------------------------------------
// Tag-to-mark mapping
// ---------------------------------------------------------------------------

/// Map HTML tag names to mark type names.
fn tag_to_mark_type(tag: &str) -> Option<&'static str> {
    match tag {
        "strong" | "b" => Some("bold"),
        "em" | "i" => Some("italic"),
        "u" => Some("underline"),
        "s" | "del" | "strike" => Some("strike"),
        "a" => Some("link"),
        _ => None,
    }
}

/// Check if a tag is a known inline mark tag.
fn is_mark_tag(tag: &str) -> bool {
    tag_to_mark_type(tag).is_some()
}

// ---------------------------------------------------------------------------
// Known void HTML elements (self-closing / no content)
// ---------------------------------------------------------------------------

const VOID_HTML_ELEMENTS: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];

fn is_void_html_element(tag: &str) -> bool {
    VOID_HTML_ELEMENTS.contains(&tag)
}

fn element_attr<'a>(elem: &'a scraper::node::Element, key: &str) -> Option<&'a str> {
    elem.attrs().find_map(|(attr_key, value)| {
        if attr_key.to_string() == key {
            Some(value)
        } else {
            None
        }
    })
}

// ---------------------------------------------------------------------------
// Block-level HTML elements
// ---------------------------------------------------------------------------

const BLOCK_HTML_ELEMENTS: &[&str] = &[
    "address",
    "article",
    "aside",
    "blockquote",
    "details",
    "dialog",
    "dd",
    "div",
    "dl",
    "dt",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "form",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "hgroup",
    "hr",
    "li",
    "main",
    "nav",
    "ol",
    "p",
    "pre",
    "section",
    "table",
    "ul",
];

fn is_block_html_element(tag: &str) -> bool {
    BLOCK_HTML_ELEMENTS.contains(&tag)
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Parse an HTML string into a Document tree using the given schema.
///
/// The HTML is parsed as a fragment (no `<html>` wrapper expected). Block-level
/// elements are mapped to their schema node types via `html_tag`. Mark tags
/// (`<strong>`, `<em>`, etc.) are converted to marks on text nodes.
///
/// In non-strict mode (the default), unknown tags are preserved as opaque nodes
/// that round-trip faithfully. In strict mode, unknown tags produce an error.
pub fn from_html(
    html: &str,
    schema: &Schema,
    options: &FromHtmlOptions,
) -> Result<Document, ParseError> {
    let fragment = Html::parse_fragment(html);
    let root_el = fragment.root_element();

    let mut block_children = Vec::new();
    let mut inline_acc: Vec<Node> = Vec::new();

    // The root_element in scraper is the fragment root (an <html> wrapper).
    // Its children are the top-level nodes. We walk them, collecting blocks
    // and auto-wrapping bare inline content into paragraphs.
    process_children(
        *root_el,
        schema,
        options,
        &[],
        &mut block_children,
        &mut inline_acc,
    )?;

    // Flush any remaining inline content
    flush_inline_acc(&mut inline_acc, schema, &mut block_children);

    // If no blocks were produced, create an empty paragraph
    if block_children.is_empty() {
        block_children.push(make_paragraph(schema, vec![]));
    }

    let doc_node = Node::element(
        "doc".to_string(),
        HashMap::new(),
        Fragment::from(block_children),
    );
    Ok(Document::new(doc_node))
}

// ---------------------------------------------------------------------------
// Internal processing
// ---------------------------------------------------------------------------

/// Process children of a scraper node, dispatching to block or inline handling.
fn process_children(
    parent: SNodeRef<'_>,
    schema: &Schema,
    options: &FromHtmlOptions,
    active_marks: &[Mark],
    block_acc: &mut Vec<Node>,
    inline_acc: &mut Vec<Node>,
) -> Result<(), ParseError> {
    for child in parent.children() {
        let val: &scraper::Node = child.value();
        if let Some(text_data) = val.as_text() {
            let text: &str = &*text_data;
            if !text.is_empty() {
                inline_acc.push(Node::text(text.to_string(), active_marks.to_vec()));
            }
        } else if let Some(elem) = val.as_element() {
            let tag = elem.name();
            process_element(
                child,
                tag,
                elem,
                schema,
                options,
                active_marks,
                block_acc,
                inline_acc,
            )?;
        }
        // Comments, processing instructions, etc. — skip
    }
    Ok(())
}

/// Process a single HTML element node.
fn process_element(
    node_ref: SNodeRef<'_>,
    tag: &str,
    elem: &scraper::node::Element,
    schema: &Schema,
    options: &FromHtmlOptions,
    active_marks: &[Mark],
    block_acc: &mut Vec<Node>,
    inline_acc: &mut Vec<Node>,
) -> Result<(), ParseError> {
    // 1) Check if this is a mark tag
    if is_mark_tag(tag) {
        let mark_type = tag_to_mark_type(tag).unwrap();
        if schema.mark(mark_type).is_some() {
            let mut new_marks = active_marks.to_vec();
            if !new_marks.iter().any(|m| m.mark_type() == mark_type) {
                let mut attrs = HashMap::new();
                if mark_type == "link" {
                    if let Some(href) = element_attr(elem, "href") {
                        attrs.insert(
                            "href".to_string(),
                            serde_json::Value::String(href.to_string()),
                        );
                    }
                }
                new_marks.push(Mark::new(mark_type.to_string(), attrs));
            }
            return process_children(node_ref, schema, options, &new_marks, block_acc, inline_acc);
        }
    }

    // 1b) Native-editor mention round-trip
    if let Some(mention) = build_mention_node(node_ref, elem, schema) {
        inline_acc.push(mention);
        return Ok(());
    }

    // 2) Check if this matches a known schema node by html_tag
    if let Some(spec) = schema.node_by_html_tag(tag) {
        return process_schema_node(
            node_ref,
            elem,
            spec,
            schema,
            options,
            active_marks,
            block_acc,
            inline_acc,
        );
    }

    // 3) Unknown tag
    if options.strict {
        return Err(ParseError::UnknownTag(tag.to_string()));
    }

    // Preserve as opaque node
    let opaque = build_opaque_node(node_ref, tag, elem)?;
    if is_block_html_element(tag) || is_void_html_element(tag) {
        flush_inline_acc(inline_acc, schema, block_acc);
        block_acc.push(opaque);
    } else {
        inline_acc.push(opaque);
    }
    Ok(())
}

fn build_mention_node(
    node_ref: SNodeRef<'_>,
    _elem: &scraper::node::Element,
    schema: &Schema,
) -> Option<Node> {
    if schema.node("mention").is_none() {
        return None;
    }

    let is_mention = element_attr(_elem, "data-native-editor-mention")
        .map(|value| value == "true")
        .unwrap_or(false);
    if !is_mention {
        return None;
    }

    let mut attrs = HashMap::new();
    if let Some(raw_attrs) = element_attr(_elem, "data-native-editor-mention-attrs") {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(raw_attrs) {
            if let Some(map) = value.as_object() {
                for (key, value) in map {
                    attrs.insert(key.clone(), value.clone());
                }
            }
        }
    }

    if !attrs.contains_key("label") {
        let label = node_ref
            .children()
            .filter_map(|child| child.value().as_text().map(|text| text.to_string()))
            .collect::<String>();
        if !label.is_empty() {
            attrs.insert("label".to_string(), serde_json::Value::String(label));
        }
    }

    Some(Node::void("mention".to_string(), attrs))
}

/// Process an element that maps to a known schema node spec.
fn process_schema_node(
    node_ref: SNodeRef<'_>,
    _elem: &scraper::node::Element,
    spec: &crate::schema::NodeSpec,
    schema: &Schema,
    options: &FromHtmlOptions,
    active_marks: &[Mark],
    block_acc: &mut Vec<Node>,
    inline_acc: &mut Vec<Node>,
) -> Result<(), ParseError> {
    match &spec.role {
        NodeRole::HardBreak => {
            inline_acc.push(Node::void(spec.name.clone(), HashMap::new()));
            Ok(())
        }
        NodeRole::Block if spec.is_void => {
            flush_inline_acc(inline_acc, schema, block_acc);
            block_acc.push(Node::void(spec.name.clone(), HashMap::new()));
            Ok(())
        }
        NodeRole::TextBlock => {
            flush_inline_acc(inline_acc, schema, block_acc);
            let children = collect_inline_children(node_ref, schema, options, active_marks)?;
            let node = Node::element(spec.name.clone(), HashMap::new(), Fragment::from(children));
            block_acc.push(node);
            Ok(())
        }
        NodeRole::List { ordered } => {
            flush_inline_acc(inline_acc, schema, block_acc);
            let mut attrs = HashMap::new();
            if *ordered {
                let start_val = element_attr(_elem, "start")
                    .and_then(|s| s.parse::<u64>().ok())
                    .unwrap_or(1);
                attrs.insert(
                    "start".to_string(),
                    serde_json::Value::Number(start_val.into()),
                );
            }
            let list_items = collect_list_items(node_ref, schema, options)?;
            let node = Node::element(spec.name.clone(), attrs, Fragment::from(list_items));
            block_acc.push(node);
            Ok(())
        }
        NodeRole::ListItem => {
            flush_inline_acc(inline_acc, schema, block_acc);
            let mut li_blocks = Vec::new();
            let mut li_inline = Vec::new();
            process_children(
                node_ref,
                schema,
                options,
                &[],
                &mut li_blocks,
                &mut li_inline,
            )?;
            flush_inline_acc(&mut li_inline, schema, &mut li_blocks);
            if li_blocks.is_empty() {
                li_blocks.push(make_paragraph(schema, vec![]));
            }
            let node = Node::element(spec.name.clone(), HashMap::new(), Fragment::from(li_blocks));
            block_acc.push(node);
            Ok(())
        }
        NodeRole::Block => {
            flush_inline_acc(inline_acc, schema, block_acc);
            let mut child_blocks = Vec::new();
            let mut child_inline = Vec::new();
            process_children(
                node_ref,
                schema,
                options,
                active_marks,
                &mut child_blocks,
                &mut child_inline,
            )?;
            flush_inline_acc(&mut child_inline, schema, &mut child_blocks);
            if child_blocks.is_empty() {
                child_blocks.push(make_paragraph(schema, vec![]));
            }
            let node = Node::element(
                spec.name.clone(),
                HashMap::new(),
                Fragment::from(child_blocks),
            );
            block_acc.push(node);
            Ok(())
        }
        NodeRole::Doc | NodeRole::Text | NodeRole::Inline => {
            flush_inline_acc(inline_acc, schema, block_acc);
            process_children(
                node_ref,
                schema,
                options,
                active_marks,
                block_acc,
                inline_acc,
            )
        }
    }
}

/// Collect inline children of an element (for paragraph-like nodes).
fn collect_inline_children(
    parent: SNodeRef<'_>,
    schema: &Schema,
    options: &FromHtmlOptions,
    active_marks: &[Mark],
) -> Result<Vec<Node>, ParseError> {
    let mut inline_nodes = Vec::new();

    for child in parent.children() {
        let val: &scraper::Node = child.value();
        if let Some(text_data) = val.as_text() {
            let text: &str = &*text_data;
            if !text.is_empty() {
                inline_nodes.push(Node::text(text.to_string(), active_marks.to_vec()));
            }
        } else if let Some(elem) = val.as_element() {
            let tag = elem.name();

            if let Some(mention) = build_mention_node(child, elem, schema) {
                inline_nodes.push(mention);
                continue;
            }

            // Mark tag — recurse with added mark
            if let Some(mark_type) = tag_to_mark_type(tag) {
                if schema.mark(mark_type).is_some() {
                    let mut new_marks = active_marks.to_vec();
                    if !new_marks.iter().any(|m| m.mark_type() == mark_type) {
                        let mut attrs = HashMap::new();
                        if mark_type == "link" {
                            if let Some(href) = element_attr(elem, "href") {
                                attrs.insert(
                                    "href".to_string(),
                                    serde_json::Value::String(href.to_string()),
                                );
                            }
                        }
                        new_marks.push(Mark::new(mark_type.to_string(), attrs));
                    }
                    let children = collect_inline_children(child, schema, options, &new_marks)?;
                    inline_nodes.extend(children);
                    continue;
                }
            }

            // Known void inline node (hardBreak)
            if let Some(spec) = schema.node_by_html_tag(tag) {
                if spec.is_void && matches!(spec.role, NodeRole::HardBreak | NodeRole::Inline) {
                    inline_nodes.push(Node::void(spec.name.clone(), HashMap::new()));
                    continue;
                }
            }

            // Unknown inline tag — opaque or error
            if options.strict {
                return Err(ParseError::UnknownTag(tag.to_string()));
            }
            let opaque = build_opaque_node(child, tag, elem)?;
            inline_nodes.push(opaque);
        }
    }

    Ok(inline_nodes)
}

/// Collect list items from a list element's children.
fn collect_list_items(
    list_ref: SNodeRef<'_>,
    schema: &Schema,
    options: &FromHtmlOptions,
) -> Result<Vec<Node>, ParseError> {
    let mut items = Vec::new();

    for child in list_ref.children() {
        let val: &scraper::Node = child.value();
        if let Some(elem) = val.as_element() {
            let tag = elem.name();
            if let Some(spec) = schema.node_by_html_tag(tag) {
                if matches!(spec.role, NodeRole::ListItem) {
                    let mut li_blocks = Vec::new();
                    let mut li_inline = Vec::new();
                    process_children(child, schema, options, &[], &mut li_blocks, &mut li_inline)?;
                    flush_inline_acc(&mut li_inline, schema, &mut li_blocks);
                    if li_blocks.is_empty() {
                        li_blocks.push(make_paragraph(schema, vec![]));
                    }
                    let node =
                        Node::element(spec.name.clone(), HashMap::new(), Fragment::from(li_blocks));
                    items.push(node);
                }
            }
        } else if let Some(text_data) = val.as_text() {
            let text: &str = &*text_data;
            let text = text.trim();
            if !text.is_empty() {
                let li_spec = schema
                    .all_nodes()
                    .find(|n| matches!(n.role, NodeRole::ListItem));
                if let Some(spec) = li_spec {
                    let para = make_paragraph(schema, vec![Node::text(text.to_string(), vec![])]);
                    let li = Node::element(
                        spec.name.clone(),
                        HashMap::new(),
                        Fragment::from(vec![para]),
                    );
                    items.push(li);
                }
            }
        }
    }

    Ok(items)
}

/// Build an opaque node for an unknown HTML tag (non-strict mode).
///
/// Opaque nodes preserve the tag name and attributes so they can survive
/// round-trips. They are stored as void nodes with metadata in attrs.
fn build_opaque_node(
    node_ref: SNodeRef<'_>,
    tag: &str,
    elem: &scraper::node::Element,
) -> Result<Node, ParseError> {
    let mut attrs = HashMap::new();
    attrs.insert(
        "html_tag".to_string(),
        serde_json::Value::String(tag.to_string()),
    );

    // Preserve HTML attributes
    let html_attrs: HashMap<String, String> = elem
        .attrs()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    if !html_attrs.is_empty() {
        attrs.insert("html_attrs".to_string(), serde_json::json!(html_attrs));
    }

    // Collect text content for display
    let text_content = collect_text_content(node_ref);
    if !text_content.is_empty() {
        attrs.insert(
            "text_content".to_string(),
            serde_json::Value::String(text_content),
        );
    }

    // Store the original inner HTML for faithful round-trip
    let inner_html = collect_inner_html(node_ref);
    if !inner_html.is_empty() {
        attrs.insert(
            "inner_html".to_string(),
            serde_json::Value::String(inner_html),
        );
    }

    Ok(Node::void("__opaque".to_string(), attrs))
}

/// Recursively collect all text content from a scraper node.
fn collect_text_content(node_ref: SNodeRef<'_>) -> String {
    let mut buf = String::new();
    for child in node_ref.children() {
        let val: &scraper::Node = child.value();
        if let Some(text_data) = val.as_text() {
            buf.push_str(&*text_data);
        } else if val.is_element() {
            buf.push_str(&collect_text_content(child));
        }
    }
    buf
}

/// Collect inner HTML (serialized) from a scraper node for faithful round-trip.
fn collect_inner_html(node_ref: SNodeRef<'_>) -> String {
    let mut buf = String::new();
    for child in node_ref.children() {
        let val: &scraper::Node = child.value();
        if let Some(text_data) = val.as_text() {
            escape_html_to(&*text_data, &mut buf);
        } else if let Some(elem) = val.as_element() {
            let tag = elem.name();
            buf.push('<');
            buf.push_str(tag);
            for (key, val) in elem.attrs() {
                buf.push(' ');
                buf.push_str(key);
                buf.push_str("=\"");
                buf.push_str(val);
                buf.push('"');
            }
            buf.push('>');
            if !is_void_html_element(tag) {
                buf.push_str(&collect_inner_html(child));
                buf.push_str("</");
                buf.push_str(tag);
                buf.push('>');
            }
        }
    }
    buf
}

/// Escape text for safe HTML embedding.
fn escape_html_to(text: &str, buf: &mut String) {
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

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Flush accumulated inline nodes into a paragraph and append to blocks.
fn flush_inline_acc(inline_acc: &mut Vec<Node>, schema: &Schema, block_acc: &mut Vec<Node>) {
    if inline_acc.is_empty() {
        return;
    }
    let children: Vec<Node> = inline_acc.drain(..).collect();
    block_acc.push(make_paragraph(schema, children));
}

/// Create a paragraph node using the schema's paragraph type name.
fn make_paragraph(schema: &Schema, children: Vec<Node>) -> Node {
    let para_name = schema
        .all_nodes()
        .find(|n| matches!(n.role, NodeRole::TextBlock))
        .map(|n| n.name.as_str())
        .unwrap_or("paragraph");
    Node::element(
        para_name.to_string(),
        HashMap::new(),
        Fragment::from(children),
    )
}
