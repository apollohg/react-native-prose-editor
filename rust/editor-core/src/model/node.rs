use std::collections::HashMap;

use crate::model::fragment::Fragment;
use crate::model::mark::Mark;

/// The kind of a node determines how it behaves in the position model.
#[derive(Debug, Clone)]
enum NodeKind {
    /// A text node: carries a string and marks, occupies `text.chars().count()`
    /// tokens (Unicode scalar values). Has no children.
    Text { text: String },
    /// A void/leaf node (e.g. hardBreak, horizontalRule): occupies exactly 1
    /// token. Has no children.
    Void,
    /// A regular element node (e.g. doc, paragraph, list): occupies
    /// `1 (open) + content.size() + 1 (close)` tokens.
    Element { content: Fragment },
}

/// A node in the document tree.
///
/// There are three kinds:
/// - **Text**: inline content with optional marks, measured in Unicode scalars
/// - **Void**: atomic nodes like hard breaks, always 1 token
/// - **Element**: container nodes with a content fragment
#[derive(Debug, Clone)]
pub struct Node {
    node_type: String,
    attrs: HashMap<String, serde_json::Value>,
    marks: Vec<Mark>,
    kind: NodeKind,
}

impl Node {
    /// Create a text node with the given content and marks.
    pub fn text(text: String, marks: Vec<Mark>) -> Self {
        Self {
            node_type: "text".to_string(),
            attrs: HashMap::new(),
            marks,
            kind: NodeKind::Text { text },
        }
    }

    /// Create a void (atomic) node like hardBreak or horizontalRule.
    pub fn void(node_type: String, attrs: HashMap<String, serde_json::Value>) -> Self {
        Self {
            node_type,
            attrs,
            marks: Vec::new(),
            kind: NodeKind::Void,
        }
    }

    /// Create an element (container) node like doc, paragraph, list, etc.
    pub fn element(
        node_type: String,
        attrs: HashMap<String, serde_json::Value>,
        content: Fragment,
    ) -> Self {
        Self {
            node_type,
            attrs,
            marks: Vec::new(),
            kind: NodeKind::Element { content },
        }
    }

    /// The node type name (e.g. "paragraph", "text", "hardBreak").
    pub fn node_type(&self) -> &str {
        &self.node_type
    }

    /// The node's attributes.
    pub fn attrs(&self) -> &HashMap<String, serde_json::Value> {
        &self.attrs
    }

    /// The marks applied to this node (only meaningful for text nodes).
    pub fn marks(&self) -> &[Mark] {
        &self.marks
    }

    /// Whether this is a text node.
    pub fn is_text(&self) -> bool {
        matches!(self.kind, NodeKind::Text { .. })
    }

    /// Whether this is a void (atomic) node.
    pub fn is_void(&self) -> bool {
        matches!(self.kind, NodeKind::Void)
    }

    /// Whether this is an element (container) node.
    pub fn is_element(&self) -> bool {
        matches!(self.kind, NodeKind::Element { .. })
    }

    /// The token size of this node in the ProseMirror position model.
    ///
    /// - Text nodes: number of Unicode scalar values (chars)
    /// - Void nodes: always 1
    /// - Element nodes: 1 (open) + content size + 1 (close)
    pub fn node_size(&self) -> u32 {
        match &self.kind {
            NodeKind::Text { text } => text.chars().count() as u32,
            NodeKind::Void => 1,
            NodeKind::Element { content } => 1 + content.size() + 1,
        }
    }

    /// The size of the node's content (excluding open/close tokens).
    /// For text nodes this equals `node_size()`. For void nodes this is 0.
    /// For element nodes this is the fragment size.
    pub fn content_size(&self) -> u32 {
        match &self.kind {
            NodeKind::Text { text } => text.chars().count() as u32,
            NodeKind::Void => 0,
            NodeKind::Element { content } => content.size(),
        }
    }

    /// Recursively collect all text content from this node and its descendants.
    pub fn text_content(&self) -> String {
        match &self.kind {
            NodeKind::Text { text } => text.clone(),
            NodeKind::Void => String::new(),
            NodeKind::Element { content } => {
                let mut buf = String::new();
                for child in content.iter() {
                    buf.push_str(&child.text_content());
                }
                buf
            }
        }
    }

    /// Number of direct children. Text and void nodes have 0 children.
    pub fn child_count(&self) -> usize {
        match &self.kind {
            NodeKind::Element { content } => content.child_count(),
            _ => 0,
        }
    }

    /// Access a direct child by index.
    pub fn child(&self, index: usize) -> Option<&Node> {
        match &self.kind {
            NodeKind::Element { content } => content.child(index),
            _ => None,
        }
    }

    /// Access the content fragment. Returns `None` for text and void nodes.
    pub fn content(&self) -> Option<&Fragment> {
        match &self.kind {
            NodeKind::Element { content } => Some(content),
            _ => None,
        }
    }

    /// The raw text of a text node. Returns `None` for non-text nodes.
    pub fn text_str(&self) -> Option<&str> {
        match &self.kind {
            NodeKind::Text { text } => Some(text.as_str()),
            _ => None,
        }
    }
}
