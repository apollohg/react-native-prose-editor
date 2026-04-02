pub mod generate;
pub mod incremental;

/// Context for list items, providing numbering and position metadata.
#[derive(Debug, Clone, PartialEq)]
pub struct ListContext {
    pub ordered: bool,
    pub index: u32,
    pub total: u32,
    pub start: u32,
    pub is_first: bool,
    pub is_last: bool,
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
    TextRun { text: String, marks: Vec<RenderMark> },
    /// An inline void node (e.g. hardBreak).
    VoidInline { node_type: String, doc_pos: u32 },
    /// A block-level void node (e.g. horizontalRule).
    VoidBlock { node_type: String, doc_pos: u32 },
    /// An opaque inline atom (unrecognised inline void).
    OpaqueInlineAtom {
        node_type: String,
        label: String,
        doc_pos: u32,
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
        depth: u8,
        list_context: Option<ListContext>,
    },
    /// End of a block-level container.
    BlockEnd,
}

/// Visible text used for an ordered or unordered list marker.
pub fn list_marker_string(ordered: bool, index: u32) -> String {
    if ordered {
        format!("{index}. ")
    } else {
        "\u{2022} ".to_string()
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

pub fn inline_atom_label(
    node_type: &str,
    attrs: &std::collections::HashMap<String, serde_json::Value>,
) -> String {
    attrs.get("label")
        .and_then(|value| value.as_str())
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| node_type.to_string())
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
    if is_known_rule || matches!(node_type, "horizontalRule" | "horizontal_rule") {
        1
    } else {
        opaque_atom_visible_string(node_type, label.unwrap_or(node_type))
            .chars()
            .count() as u32
    }
}
