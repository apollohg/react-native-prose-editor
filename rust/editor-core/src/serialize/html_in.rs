use std::collections::HashMap;
use std::fmt;

use ego_tree::iter::Edge;
use scraper::Html;

use crate::boundary::ResourceLimits;
use crate::model::{Document, Fragment, Mark, Node};
use crate::schema::{NodeRole, Schema};

/// Type alias for a node reference in the scraper parse tree.
type SNodeRef<'a> = ego_tree::NodeRef<'a, scraper::Node>;

/// Errors returned by `from_html`.
#[derive(Debug, Clone)]
pub enum ParseError {
    /// An unknown HTML tag was encountered in strict mode.
    UnknownTag(String),
    /// The parsed content does not satisfy the schema's content rules.
    // Not reachable from production call paths after the Task 16C legacy runtime
    // removal; exercised by crate tests.
    #[allow(dead_code)]
    InvalidContent(String),
    ResourceLimit {
        limit: usize,
        actual: usize,
    },
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::UnknownTag(tag) => write!(f, "unknown HTML tag: <{}>", tag),
            ParseError::InvalidContent(msg) => write!(f, "invalid content: {}", msg),
            ParseError::ResourceLimit { limit, actual } => {
                write!(f, "HTML parse work exceeds limit {limit}: {actual}")
            }
        }
    }
}

impl std::error::Error for ParseError {}

/// Options for `from_html`.
#[derive(Debug, Clone, Default)]
pub struct FromHtmlOptions {
    /// If `true`, unknown HTML tags produce an error instead of being preserved
    /// as opaque nodes.
    pub strict: bool,
    /// If `true`, `<img src="data:image/...">` is parsed as a real image node.
    pub allow_base64_images: bool,
}

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

fn mark_from_element(tag: &str, elem: &scraper::node::Element, schema: &Schema) -> Option<Mark> {
    let mark_type = (tag == "span")
        .then(|| element_attr(elem, "data-native-editor-mark"))
        .flatten()
        .filter(|name| schema.mark(name).is_some())
        .or_else(|| tag_to_mark_type(tag).filter(|name| schema.mark(name).is_some()))
        .or_else(|| schema.mark_by_html_tag(tag).map(|spec| spec.name.as_str()))?;
    let spec = schema.mark(mark_type)?;
    let mut attrs = HashMap::new();
    for (name, attr_spec) in &spec.attrs {
        if let Some(value) = element_attr(elem, name) {
            attrs.insert(name.clone(), parse_attr_value(name, attr_spec, value));
        } else if let Some(default) = &attr_spec.default {
            attrs.insert(name.clone(), default.clone());
        }
    }
    Some(Mark::new(mark_type.to_string(), attrs))
}

// Known void HTML elements (self-closing / no content)

const VOID_HTML_ELEMENTS: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];

fn is_void_html_element(tag: &str) -> bool {
    VOID_HTML_ELEMENTS.contains(&tag)
}

fn element_attr<'a>(elem: &'a scraper::node::Element, key: &str) -> Option<&'a str> {
    elem.attrs().find_map(
        |(attr_key, value)| {
            if attr_key == key {
                Some(value)
            } else {
                None
            }
        },
    )
}

fn is_base64_image_source(src: &str) -> bool {
    src.trim_start()
        .get(.."data:image/".len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("data:image/"))
}

pub(crate) fn opaque_html_metadata_remains_opaque(
    tag: &str,
    attrs: &serde_json::Map<String, serde_json::Value>,
    placement: &str,
    schema: &Schema,
) -> bool {
    let attr = |key: &str| attrs.get(key).and_then(serde_json::Value::as_str);
    if tag == "span"
        && (attr("data-native-editor-mark").is_some_and(|name| schema.mark(name).is_some())
            || (schema.node("mention").is_some()
                && attr("data-native-editor-mention") == Some("true")))
    {
        return false;
    }
    if tag_to_mark_type(tag).is_some_and(|name| schema.mark(name).is_some())
        || schema.mark_by_html_tag(tag).is_some()
    {
        return false;
    }
    if placement == "inline" {
        return schema.node_by_html_tag(tag).is_none_or(|spec| {
            !(spec.is_void && matches!(spec.role, NodeRole::HardBreak | NodeRole::Inline))
        });
    }
    if let Some(spec) = schema.node_by_html_tag(tag) {
        return tag == "img"
            && spec.name == "image"
            && attr("src").is_some_and(is_base64_image_source);
    }
    true
}

pub(crate) fn opaque_html_attr_has_canonical_case(tag: &str, attr: &str) -> bool {
    if !attr.bytes().any(|byte| byte.is_ascii_uppercase()) {
        return true;
    }

    // html5ever applies the HTML parsing algorithm's foreign-content case
    // adjustments. Preserve only those exact canonical spellings, and only on
    // tags that can establish the corresponding foreign namespace. This keeps
    // arbitrary mixed-case HTML/private keys rejected without breaking stable
    // SVG/MathML export and reimport.
    const SVG_ADJUSTED_ATTRS: &[&str] = &[
        "attributeName",
        "attributeType",
        "baseFrequency",
        "baseProfile",
        "calcMode",
        "clipPathUnits",
        "diffuseConstant",
        "edgeMode",
        "filterUnits",
        "glyphRef",
        "gradientTransform",
        "gradientUnits",
        "kernelMatrix",
        "kernelUnitLength",
        "keyPoints",
        "keySplines",
        "keyTimes",
        "lengthAdjust",
        "limitingConeAngle",
        "markerHeight",
        "markerUnits",
        "markerWidth",
        "maskContentUnits",
        "maskUnits",
        "numOctaves",
        "pathLength",
        "patternContentUnits",
        "patternTransform",
        "patternUnits",
        "pointsAtX",
        "pointsAtY",
        "pointsAtZ",
        "preserveAlpha",
        "preserveAspectRatio",
        "primitiveUnits",
        "refX",
        "refY",
        "repeatCount",
        "repeatDur",
        "requiredExtensions",
        "requiredFeatures",
        "specularConstant",
        "specularExponent",
        "spreadMethod",
        "startOffset",
        "stdDeviation",
        "stitchTiles",
        "surfaceScale",
        "systemLanguage",
        "tableValues",
        "targetX",
        "targetY",
        "textLength",
        "viewBox",
        "viewTarget",
        "xChannelSelector",
        "yChannelSelector",
        "zoomAndPan",
    ];
    // Opaque foreign subtrees are represented by their namespace-establishing
    // root plus raw inner HTML, so only these roots can carry adjusted metadata
    // without losing their namespace during standalone serialization.
    (tag == "svg" && SVG_ADJUSTED_ATTRS.contains(&attr))
        || (tag == "math" && attr == "definitionURL")
}

fn extract_node_attrs(
    elem: &scraper::node::Element,
    spec: &crate::schema::NodeSpec,
) -> HashMap<String, serde_json::Value> {
    let mut attrs = super::default_node_attrs(spec);

    for (key, attr_spec) in &spec.attrs {
        if let Some(value) = element_attr(elem, key) {
            attrs.insert(key.clone(), parse_attr_value(key, attr_spec, value));
        }
    }

    attrs.retain(|_, value| !value.is_null());
    attrs
}

fn parse_attr_value(key: &str, spec: &crate::schema::AttrSpec, raw: &str) -> serde_json::Value {
    if matches!(spec.default.as_ref(), Some(serde_json::Value::Bool(_))) || key == "checked" {
        let normalized = raw.trim().to_ascii_lowercase();
        if normalized.is_empty() || normalized == "checked" || normalized == "true" {
            return serde_json::Value::Bool(true);
        }
        if normalized == "false" {
            return serde_json::Value::Bool(false);
        }
    }
    if matches!(spec.default.as_ref(), Some(serde_json::Value::Number(_)))
        || matches!(key, "width" | "height")
    {
        let trimmed = raw.trim();
        if let Ok(value) = trimmed.parse::<u64>() {
            return serde_json::Value::Number(value.into());
        }
    }
    serde_json::Value::String(raw.to_string())
}

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

/// Parse an HTML string into a Document tree using the given schema.
///
/// The HTML is parsed as a fragment (no `<html>` wrapper expected). Block-level
/// elements are mapped to their schema node types via `html_tag`. Mark tags
/// (`<strong>`, `<em>`, etc.) are converted to marks on text nodes.
///
/// In non-strict mode (the default), unknown tags are preserved as opaque nodes
/// that round-trip faithfully. In strict mode, unknown tags produce an error.
// Not reachable from production call paths after the Task 16C legacy runtime
// removal; exercised by crate tests.
#[allow(dead_code)]
pub fn from_html(
    html: &str,
    schema: &Schema,
    options: &FromHtmlOptions,
) -> Result<Document, ParseError> {
    from_html_with_limits(html, schema, options, &ResourceLimits::default())
}

pub fn from_html_with_limits(
    html: &str,
    schema: &Schema,
    options: &FromHtmlOptions,
    limits: &ResourceLimits,
) -> Result<Document, ParseError> {
    let fragment = Html::parse_fragment(html);
    let root_el = fragment.root_element();

    // Charge every scraper node once before recursive conversion. This shared
    // admission pass bounds import work independently of schema size and
    // prevents nested helpers from resetting their own allowance.
    let work_limit = limits.max_document_nodes.saturating_mul(4);
    let mut actual = 0usize;
    let mut depth = 0usize;
    for edge in root_el.traverse() {
        match edge {
            Edge::Open(_) => {
                actual = actual.saturating_add(1);
                depth = depth.saturating_add(1);
                if actual > work_limit {
                    return Err(ParseError::ResourceLimit {
                        limit: work_limit,
                        actual,
                    });
                }
                if depth > limits.max_document_depth {
                    return Err(ParseError::ResourceLimit {
                        limit: limits.max_document_depth,
                        actual: depth,
                    });
                }
            }
            Edge::Close(_) => depth = depth.saturating_sub(1),
        }
    }
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
        schema.doc_node_type().to_string(),
        schema
            .node(schema.doc_node_type())
            .map(super::default_node_attrs)
            .unwrap_or_default(),
        Fragment::from(block_children),
    );
    Ok(Document::new(doc_node))
}

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
            let text: &str = text_data;
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
#[allow(clippy::too_many_arguments)]
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
    if let Some(mark) = mark_from_element(tag, elem, schema) {
        let mut new_marks = active_marks.to_vec();
        if !new_marks
            .iter()
            .any(|existing| existing.mark_type() == mark.mark_type())
        {
            new_marks.push(mark);
        }
        return process_children(node_ref, schema, options, &new_marks, block_acc, inline_acc);
    }

    // 1b) Native-editor mention round-trip
    if let Some(mention) = build_mention_node(node_ref, elem, schema) {
        inline_acc.push(mention);
        return Ok(());
    }

    // 2) Check if this matches a known schema node by html_tag
    if let Some(spec) = schema.node_by_html_tag(tag) {
        if tag == "img"
            && spec.name == "image"
            && !options.allow_base64_images
            && element_attr(elem, "src").is_some_and(is_base64_image_source)
        {
            if options.strict {
                return Err(ParseError::UnknownTag(tag.to_string()));
            }
            let opaque = build_opaque_node(node_ref, tag, elem, "block")?;
            flush_inline_acc(inline_acc, schema, block_acc);
            block_acc.push(opaque);
            return Ok(());
        }
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
    let placement = if is_block_html_element(tag) || is_void_html_element(tag) {
        "block"
    } else {
        "inline"
    };
    let opaque = build_opaque_node(node_ref, tag, elem, placement)?;
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
    elem: &scraper::node::Element,
    schema: &Schema,
) -> Option<Node> {
    schema.node("mention")?;
    if elem.name() != "span" {
        return None;
    }

    let is_mention = element_attr(elem, "data-native-editor-mention")
        .map(|value| value == "true")
        .unwrap_or(false);
    if !is_mention {
        return None;
    }

    let mut attrs = HashMap::new();
    if let Some(raw_attrs) = element_attr(elem, "data-native-editor-mention-attrs") {
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
#[allow(clippy::too_many_arguments)]
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
            inline_acc.push(Node::void(
                spec.name.clone(),
                extract_node_attrs(_elem, spec),
            ));
            Ok(())
        }
        NodeRole::Block if spec.is_void => {
            flush_inline_acc(inline_acc, schema, block_acc);
            block_acc.push(Node::void(
                spec.name.clone(),
                extract_node_attrs(_elem, spec),
            ));
            Ok(())
        }
        NodeRole::TextBlock => {
            flush_inline_acc(inline_acc, schema, block_acc);
            let children = collect_inline_children(node_ref, schema, options, active_marks)?;
            let node = Node::element(
                spec.name.clone(),
                extract_node_attrs(_elem, spec),
                Fragment::from(children),
            );
            block_acc.push(node);
            Ok(())
        }
        NodeRole::List { .. } => {
            flush_inline_acc(inline_acc, schema, block_acc);
            let attrs = extract_node_attrs(_elem, spec);
            let list_items = collect_list_items(node_ref, spec, schema, options)?;
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
            let node = Node::element(
                spec.name.clone(),
                extract_node_attrs(_elem, spec),
                Fragment::from(li_blocks),
            );
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
                extract_node_attrs(_elem, spec),
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
            let text: &str = text_data;
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
            if let Some(mark) = mark_from_element(tag, elem, schema) {
                let mut new_marks = active_marks.to_vec();
                if !new_marks
                    .iter()
                    .any(|existing| existing.mark_type() == mark.mark_type())
                {
                    new_marks.push(mark);
                }
                let children = collect_inline_children(child, schema, options, &new_marks)?;
                inline_nodes.extend(children);
                continue;
            }

            // Known void inline node (hardBreak)
            if let Some(spec) = schema.node_by_html_tag(tag) {
                if spec.is_void && matches!(spec.role, NodeRole::HardBreak | NodeRole::Inline) {
                    inline_nodes.push(Node::void(
                        spec.name.clone(),
                        extract_node_attrs(elem, spec),
                    ));
                    continue;
                }
            }

            // Unknown inline tag — opaque or error
            if options.strict {
                return Err(ParseError::UnknownTag(tag.to_string()));
            }
            let opaque = build_opaque_node(child, tag, elem, "inline")?;
            inline_nodes.push(opaque);
        }
    }

    Ok(inline_nodes)
}

/// Collect list items from a list element's children.
fn collect_list_items(
    list_ref: SNodeRef<'_>,
    list_spec: &crate::schema::NodeSpec,
    schema: &Schema,
    options: &FromHtmlOptions,
) -> Result<Vec<Node>, ParseError> {
    let mut items = Vec::new();
    let list_item_name = schema.list_item_type_for(&list_spec.name);
    let fallback_list_item = || schema.fallback_list_item_type().map(str::to_string);

    for child in list_ref.children() {
        let val: &scraper::Node = child.value();
        if let Some(elem) = val.as_element() {
            let tag = elem.name();
            if let Some(spec) = schema.node_by_html_tag(tag) {
                if matches!(spec.role, NodeRole::ListItem) {
                    let item_type = list_item_name
                        .clone()
                        .or_else(fallback_list_item)
                        .unwrap_or_else(|| spec.name.clone());
                    let mut li_blocks = Vec::new();
                    let mut li_inline = Vec::new();
                    process_children(child, schema, options, &[], &mut li_blocks, &mut li_inline)?;
                    flush_inline_acc(&mut li_inline, schema, &mut li_blocks);
                    if li_blocks.is_empty() {
                        li_blocks.push(make_paragraph(schema, vec![]));
                    }
                    let item_spec = schema.node(&item_type).unwrap_or(spec);
                    let node = Node::element(
                        item_type,
                        extract_node_attrs(elem, item_spec),
                        Fragment::from(li_blocks),
                    );
                    items.push(node);
                }
            }
        } else if let Some(text_data) = val.as_text() {
            let text: &str = text_data;
            let text = text.trim();
            if !text.is_empty() {
                if let Some(item_type) = list_item_name.clone().or_else(fallback_list_item) {
                    let para = make_paragraph(schema, vec![Node::text(text.to_string(), vec![])]);
                    let li = Node::element(
                        item_type.clone(),
                        schema
                            .node(&item_type)
                            .map(super::default_node_attrs)
                            .unwrap_or_default(),
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
    placement: &str,
) -> Result<Node, ParseError> {
    let mut attrs = HashMap::new();
    attrs.insert(
        "html_tag".to_string(),
        serde_json::Value::String(tag.to_string()),
    );
    attrs.insert(
        "opaque_placement".to_string(),
        serde_json::Value::String(placement.to_string()),
    );

    // Preserve HTML attributes
    let html_attrs: HashMap<String, String> = elem
        .attrs
        .iter()
        .map(|(name, value)| {
            let local = name.local.as_ref();
            let key = name
                .prefix
                .as_ref()
                .map_or_else(|| local.to_owned(), |prefix| format!("{prefix}:{local}"));
            (key, value.to_string())
        })
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
            buf.push_str(text_data);
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
            escape_html_to(text_data, &mut buf);
        } else if let Some(elem) = val.as_element() {
            let tag = elem.name();
            buf.push('<');
            buf.push_str(tag);
            for (key, val) in elem.attrs() {
                buf.push(' ');
                buf.push_str(key);
                buf.push_str("=\"");
                escape_html_to(val, &mut buf);
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

/// Flush accumulated inline nodes into a paragraph and append to blocks.
fn flush_inline_acc(inline_acc: &mut Vec<Node>, schema: &Schema, block_acc: &mut Vec<Node>) {
    if inline_acc.is_empty() {
        return;
    }
    let children = std::mem::take(inline_acc);
    block_acc.push(make_paragraph(schema, children));
}

/// Create a paragraph node using the schema's paragraph type name.
fn make_paragraph(schema: &Schema, children: Vec<Node>) -> Node {
    let para_name = schema
        .node_by_html_tag("p")
        .or_else(|| schema.node("paragraph"))
        .map(|n| n.name.as_str())
        .or_else(|| schema.preferred_text_block().map(|node| node.name.as_str()))
        .unwrap_or("paragraph");
    let attrs = schema
        .node(para_name)
        .map(super::default_node_attrs)
        .unwrap_or_default();
    Node::element(para_name.to_string(), attrs, Fragment::from(children))
}
