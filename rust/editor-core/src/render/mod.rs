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
#[derive(Debug)]
pub struct RenderMark {
    pub mark_type: String,
    pub attrs: std::collections::HashMap<String, serde_json::Value>,
}

impl Clone for RenderMark {
    fn clone(&self) -> Self {
        Self {
            mark_type: self.mark_type.clone(),
            attrs: crate::boundary::clone_json_object_stack_safe(&self.attrs),
        }
    }
}

impl PartialEq for RenderMark {
    fn eq(&self, other: &Self) -> bool {
        self.mark_type == other.mark_type
            && crate::boundary::json_objects_equal_stack_safe(&self.attrs, &other.attrs)
    }
}

impl Eq for RenderMark {}

impl Drop for RenderMark {
    fn drop(&mut self) {
        crate::boundary::drop_json_object_values_stack_safe(&mut self.attrs);
    }
}

/// A flat render element that native platform views consume to build
/// attributed strings (NSAttributedString / SpannableStringBuilder).
#[derive(Debug, PartialEq)]
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
        attrs: std::collections::HashMap<String, serde_json::Value>,
        mention_theme: Option<std::collections::HashMap<String, serde_json::Value>>,
    },
    /// An opaque block atom (unrecognised block void).
    OpaqueBlockAtom {
        node_type: String,
        label: String,
        doc_pos: u32,
        attrs: std::collections::HashMap<String, serde_json::Value>,
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

impl Clone for RenderElement {
    fn clone(&self) -> Self {
        match self {
            Self::TextRun { text, marks } => Self::TextRun {
                text: text.clone(),
                marks: marks.clone(),
            },
            Self::VoidInline {
                node_type,
                doc_pos,
                attrs,
            } => Self::VoidInline {
                node_type: node_type.clone(),
                doc_pos: *doc_pos,
                attrs: crate::boundary::clone_json_object_stack_safe(attrs),
            },
            Self::VoidBlock {
                node_type,
                doc_pos,
                attrs,
            } => Self::VoidBlock {
                node_type: node_type.clone(),
                doc_pos: *doc_pos,
                attrs: crate::boundary::clone_json_object_stack_safe(attrs),
            },
            Self::OpaqueInlineAtom {
                node_type,
                label,
                doc_pos,
                attrs,
                mention_theme,
            } => Self::OpaqueInlineAtom {
                node_type: node_type.clone(),
                label: label.clone(),
                doc_pos: *doc_pos,
                attrs: crate::boundary::clone_json_object_stack_safe(attrs),
                mention_theme: mention_theme
                    .as_ref()
                    .map(crate::boundary::clone_json_object_stack_safe),
            },
            Self::OpaqueBlockAtom {
                node_type,
                label,
                doc_pos,
                attrs,
            } => Self::OpaqueBlockAtom {
                node_type: node_type.clone(),
                label: label.clone(),
                doc_pos: *doc_pos,
                attrs: crate::boundary::clone_json_object_stack_safe(attrs),
            },
            Self::BlockStart {
                node_type,
                depth,
                list_context,
            } => Self::BlockStart {
                node_type: node_type.clone(),
                depth: *depth,
                list_context: list_context.clone(),
            },
            Self::BlockEnd => Self::BlockEnd,
        }
    }
}

impl RenderElement {
    pub(crate) fn drain_json_payloads(&mut self) {
        match self {
            Self::TextRun { marks, .. } => marks.clear(),
            Self::VoidInline { attrs, .. } | Self::VoidBlock { attrs, .. } => {
                crate::boundary::drop_json_object_values_stack_safe(attrs);
            }
            Self::OpaqueInlineAtom {
                attrs,
                mention_theme,
                ..
            } => {
                crate::boundary::drop_json_object_values_stack_safe(attrs);
                if let Some(theme) = mention_theme {
                    crate::boundary::drop_json_object_values_stack_safe(theme);
                }
            }
            Self::OpaqueBlockAtom { attrs, .. } => {
                crate::boundary::drop_json_object_values_stack_safe(attrs)
            }
            Self::BlockStart { .. } | Self::BlockEnd => {}
        }
    }
}

/// Reconstruct the exact flat visible string consumed by scalar editor
/// offsets and the position map.
pub(crate) fn rendered_text(document: &Document, schema: &Schema) -> String {
    #[cfg(test)]
    crate::yrs_engine::observability::record_rendered_text_derivation();
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

    for element in &elements {
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
            RenderElement::TextRun { text: value, .. } => text.push_str(value),
            RenderElement::VoidInline { .. } => text.push('\n'),
            RenderElement::VoidBlock { .. } => {
                begin_block(&mut text, &mut started_block);
                text.push('\u{fffc}');
            }
            RenderElement::OpaqueInlineAtom {
                node_type, label, ..
            } => text.push_str(&opaque_atom_visible_string(node_type, label)),
            RenderElement::OpaqueBlockAtom {
                node_type, label, ..
            } => {
                begin_block(&mut text, &mut started_block);
                text.push_str(&opaque_atom_visible_string(node_type, label));
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
    // Schemas may give every list item a default `checked: false` attribute.
    // Marker semantics belong to the containing list, not to the presence of a
    // child attribute, so ordinary bullet and ordered lists never become task
    // lists merely because their item schema shares that attribute.
    let is_task = list_node_type.to_ascii_lowercase().contains("task");
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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use serde_json::Value;

    use super::task_list_marker_metadata;
    use crate::model::Node;

    fn list_item_with_checked(checked: bool) -> Node {
        Node::void(
            "listItem".to_string(),
            HashMap::from([("checked".to_string(), Value::Bool(checked))]),
        )
    }

    #[test]
    fn default_checked_items_remain_ordinary_in_bullet_and_ordered_lists() {
        for list_type in ["bulletList", "orderedList"] {
            assert_eq!(
                task_list_marker_metadata(list_type, &list_item_with_checked(false)),
                (None, None),
                "{list_type} must not infer taskness from a default checked attribute",
            );
        }
    }

    #[test]
    fn containing_task_list_preserves_item_checked_state() {
        assert_eq!(
            task_list_marker_metadata("taskList", &list_item_with_checked(false)),
            (Some("task".to_string()), Some(false)),
        );
        assert_eq!(
            task_list_marker_metadata("taskList", &list_item_with_checked(true)),
            (Some("task".to_string()), Some(true)),
        );
    }
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

pub(crate) fn opaque_node_is_inline(node: &crate::model::Node, schema: &Schema) -> bool {
    node.attrs()
        .get("opaque_placement")
        .and_then(serde_json::Value::as_str)
        .map_or_else(
            || {
                schema
                    .node(node.node_type())
                    .and_then(|spec| spec.group.as_deref())
                    .is_some_and(|group| group == "inline")
            },
            |placement| placement == "inline",
        )
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
                .map(|(key, value)| {
                    (
                        key.clone(),
                        crate::boundary::clone_json_value_stack_safe(value),
                    )
                })
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
