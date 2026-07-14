pub mod generate;
pub mod incremental;

use crate::model::{Document, Node};
use crate::schema::{NodeRole, Schema};

/// Context for list items, providing numbering and position metadata.
#[derive(Debug, Clone, PartialEq)]
pub struct ListContext {
    pub ordered: bool,
    pub index: u32,
    pub total: u32,
    pub start: u32,
    pub is_first: bool,
    pub is_last: bool,
    pub kind: Option<String>,
    pub checked: Option<bool>,
}

/// A renderable inline mark for native text builders.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderMark {
    pub mark_type: String,
    pub attrs: std::collections::HashMap<String, serde_json::Value>,
}

/// A flat render element that native platform views consume to build
/// attributed strings (NSAttributedString / SpannableStringBuilder).
#[derive(Debug, Clone, PartialEq)]
pub enum RenderElement {
    /// A run of text with applied mark names.
    TextRun {
        text: String,
        marks: Vec<RenderMark>,
    },
    /// An inline void node (e.g. hardBreak).
    VoidInline {
        node_type: String,
        doc_pos: u32,
        attrs: std::collections::HashMap<String, serde_json::Value>,
    },
    /// A block-level void node (e.g. horizontalRule).
    VoidBlock {
        node_type: String,
        doc_pos: u32,
        attrs: std::collections::HashMap<String, serde_json::Value>,
    },
    /// An opaque inline atom (unrecognised inline void).
    OpaqueInlineAtom {
        node_type: String,
        label: String,
        doc_pos: u32,
        mention_theme: Option<std::collections::HashMap<String, serde_json::Value>>,
    },
    /// An opaque block atom (unrecognised block void).
    OpaqueBlockAtom {
        node_type: String,
        label: String,
        doc_pos: u32,
    },
    /// Start of a block-level container (paragraph, listItem, etc.).
    BlockStart {
        node_type: String,
        depth: u16,
        list_context: Option<ListContext>,
    },
    /// End of a block-level container.
    BlockEnd,
}

/// Reconstruct the exact flat visible string consumed by scalar editor
/// offsets and the position map.
pub(crate) fn rendered_text(document: &Document, schema: &Schema) -> String {
    let blocks = incremental::render_blocks(document, schema);
    let elements = incremental::flatten_render_blocks(&blocks);
    let mut text = String::new();
    let mut pending_prefix = String::new();
    let mut started_block = false;

    let begin_block = |text: &mut String, started_block: &mut bool| {
        if *started_block {
            text.push('\n');
        }
        *started_block = true;
    };

    for element in elements {
        match element {
            RenderElement::BlockStart {
                node_type,
                list_context,
                ..
            } => {
                if let Some(context) = list_context {
                    pending_prefix = if context.kind.as_deref() == Some("task") {
                        task_list_marker_string(context.checked.unwrap_or(false))
                    } else {
                        list_marker_string(context.ordered, context.index)
                    };
                }
                if schema
                    .node(&node_type)
                    .is_some_and(|spec| matches!(spec.role, NodeRole::TextBlock))
                {
                    begin_block(&mut text, &mut started_block);
                    text.push_str(&pending_prefix);
                    pending_prefix.clear();
                }
            }
            RenderElement::TextRun { text: value, .. } => text.push_str(&value),
            RenderElement::VoidInline { .. } => text.push('\n'),
            RenderElement::VoidBlock { .. } => {
                begin_block(&mut text, &mut started_block);
                text.push('\u{fffc}');
            }
            RenderElement::OpaqueInlineAtom {
                node_type, label, ..
            } => text.push_str(&opaque_atom_visible_string(&node_type, &label)),
            RenderElement::OpaqueBlockAtom {
                node_type, label, ..
            } => {
                begin_block(&mut text, &mut started_block);
                text.push_str(&opaque_atom_visible_string(&node_type, &label));
            }
            RenderElement::BlockEnd => {}
        }
    }
    text
}

/// Visible text used for an ordered or unordered list marker.
pub fn list_marker_string(ordered: bool, index: u32) -> String {
    if ordered {
        format!("{index}. ")
    } else {
        "\u{2022} ".to_string()
    }
}

/// Visible text used for a task-list marker.
pub fn task_list_marker_string(checked: bool) -> String {
    if checked {
        "\u{2611} ".to_string()
    } else {
        "\u{2610} ".to_string()
    }
}

/// Single source of truth for deciding whether a list item renders with a
/// task (checkbox) marker, and whether it is checked.
///
/// Render generation, incremental patches, AND the position map all derive
/// marker text/length from this function — the marker's scalar length is part
/// of the scalar<->doc position contract, so a divergent copy corrupts
/// position mapping.
pub fn task_list_marker_metadata(
    list_node_type: &str,
    item: &Node,
) -> (Option<String>, Option<bool>) {
    let is_task = list_node_type.to_ascii_lowercase().contains("task")
        || item.node_type().to_ascii_lowercase().contains("task")
        || item.attrs().contains_key("checked");
    if !is_task {
        return (None, None);
    }

    (
        Some("task".to_string()),
        Some(
            item.attrs()
                .get("checked")
                .and_then(|value| value.as_bool())
                .unwrap_or(false),
        ),
    )
}

/// Visible text used for opaque inline and block atoms.
pub fn opaque_atom_string(label: &str) -> String {
    format!("[{label}]")
}

pub fn opaque_atom_visible_string(node_type: &str, label: &str) -> String {
    if node_type == "mention" {
        label.to_string()
    } else {
        opaque_atom_string(label)
    }
}

pub fn mention_label_with_trigger(
    label: &str,
    attrs: &std::collections::HashMap<String, serde_json::Value>,
) -> String {
    let Some(trigger) = attrs
        .get("mentionSuggestionChar")
        .and_then(|value| value.as_str())
        .filter(|value| !value.is_empty())
    else {
        return label.to_string();
    };

    if label.starts_with(trigger) {
        label.to_string()
    } else {
        format!("{trigger}{label}")
    }
}

pub fn inline_atom_label(
    node_type: &str,
    attrs: &std::collections::HashMap<String, serde_json::Value>,
) -> String {
    let label = attrs
        .get("label")
        .and_then(|value| value.as_str())
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| node_type.to_string());

    if node_type == "mention" {
        mention_label_with_trigger(&label, attrs)
    } else {
        label
    }
}

pub fn inline_atom_mention_theme(
    node_type: &str,
    attrs: &std::collections::HashMap<String, serde_json::Value>,
) -> Option<std::collections::HashMap<String, serde_json::Value>> {
    if node_type != "mention" {
        return None;
    }

    attrs
        .get("mentionTheme")
        .and_then(|value| value.as_object())
        .map(|theme| {
            theme
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect()
        })
}

/// Invisible placeholder rendered for empty text blocks so native text views
/// have a real paragraph anchor for caret placement and paragraph styling.
pub fn empty_text_block_placeholder_string() -> String {
    "\u{200B}".to_string()
}

/// Number of visible scalars for an inline node in rendered text.
pub fn inline_node_visible_scalar_len(
    node_type: &str,
    label: Option<&str>,
    is_known_hard_break: bool,
) -> u32 {
    if is_known_hard_break || matches!(node_type, "hardBreak" | "hard_break") {
        1
    } else {
        opaque_atom_visible_string(node_type, label.unwrap_or(node_type))
            .chars()
            .count() as u32
    }
}

/// Number of visible scalars for a block-level void node in rendered text.
pub fn block_node_visible_scalar_len(
    node_type: &str,
    label: Option<&str>,
    is_known_rule: bool,
) -> u32 {
    if is_known_rule || matches!(node_type, "horizontalRule" | "horizontal_rule" | "image") {
        1
    } else {
        opaque_atom_visible_string(node_type, label.unwrap_or(node_type))
            .chars()
            .count() as u32
    }
}
