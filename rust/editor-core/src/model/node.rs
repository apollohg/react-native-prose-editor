use std::collections::HashMap;
use std::sync::Arc;

use crate::model::fragment::Fragment;
use crate::model::mark::Mark;

#[cfg(test)]
thread_local! {
    static DEEP_NODE_PAYLOAD_CLONES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(crate) fn reset_deep_node_payload_clones_for_test() {
    DEEP_NODE_PAYLOAD_CLONES.set(0);
}

#[cfg(test)]
pub(crate) fn take_deep_node_payload_clones_for_test() -> usize {
    DEEP_NODE_PAYLOAD_CLONES.replace(0)
}

/// The kind of a node determines how it behaves in the position model.
#[derive(Debug, Clone, PartialEq)]
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
#[derive(Debug)]
struct NodeData {
    node_type: String,
    attrs: HashMap<String, serde_json::Value>,
    marks: Vec<Mark>,
    kind: NodeKind,
}

impl Clone for NodeData {
    fn clone(&self) -> Self {
        #[cfg(test)]
        DEEP_NODE_PAYLOAD_CLONES.set(DEEP_NODE_PAYLOAD_CLONES.get().saturating_add(1));
        Self {
            node_type: self.node_type.clone(),
            attrs: crate::boundary::clone_json_object_stack_safe(&self.attrs),
            marks: self.marks.clone(),
            kind: self.kind.clone(),
        }
    }
}

impl NodeData {
    fn take_recursive_payloads(&mut self, pending_nodes: &mut Vec<Node>) {
        for value in self.attrs.values_mut() {
            crate::boundary::drop_json_value_stack_safe(std::mem::take(value));
        }
        self.marks.clear();
        if let NodeKind::Element { content } = &mut self.kind {
            pending_nodes.extend(content.take_children_for_drop());
        }
    }
}

impl Drop for NodeData {
    fn drop(&mut self) {
        let mut pending_nodes = Vec::new();
        self.take_recursive_payloads(&mut pending_nodes);
        while let Some(mut node) = pending_nodes.pop() {
            if let Some(data) = Arc::get_mut(&mut node.data) {
                data.take_recursive_payloads(&mut pending_nodes);
            }
            // Unique payloads were emptied above before their Arc destructor
            // runs. Shared payloads are drained by whichever owner is last.
            drop(node);
        }
    }
}

#[derive(Debug, Clone)]
pub struct Node {
    data: Arc<NodeData>,
}

impl PartialEq for Node {
    fn eq(&self, other: &Self) -> bool {
        if self.shares_storage_with(other) {
            return true;
        }
        let mut pending = vec![(self, other)];
        while let Some((left, right)) = pending.pop() {
            if left.shares_storage_with(right) {
                continue;
            }
            if left.node_type() != right.node_type()
                || !crate::boundary::json_objects_equal_stack_safe(left.attrs(), right.attrs())
                || left.marks() != right.marks()
            {
                return false;
            }
            match (&left.data.kind, &right.data.kind) {
                (NodeKind::Text { text: left }, NodeKind::Text { text: right })
                    if left == right => {}
                (NodeKind::Void, NodeKind::Void) => {}
                (NodeKind::Element { content: left }, NodeKind::Element { content: right })
                    if left.child_count() == right.child_count() =>
                {
                    pending.extend(left.iter().zip(right.iter()));
                }
                _ => return false,
            }
        }
        true
    }
}

impl Eq for Node {}

impl Node {
    /// Create a text node with the given content and marks.
    pub fn text(text: String, marks: Vec<Mark>) -> Self {
        Self {
            data: Arc::new(NodeData {
                node_type: "text".to_string(),
                attrs: HashMap::new(),
                marks,
                kind: NodeKind::Text { text },
            }),
        }
    }

    /// Create a void (atomic) node like hardBreak or horizontalRule.
    pub fn void(node_type: String, attrs: HashMap<String, serde_json::Value>) -> Self {
        Self {
            data: Arc::new(NodeData {
                node_type,
                attrs,
                marks: Vec::new(),
                kind: NodeKind::Void,
            }),
        }
    }

    /// Create an element (container) node like doc, paragraph, list, etc.
    pub fn element(
        node_type: String,
        attrs: HashMap<String, serde_json::Value>,
        content: Fragment,
    ) -> Self {
        Self {
            data: Arc::new(NodeData {
                node_type,
                attrs,
                marks: Vec::new(),
                kind: NodeKind::Element { content },
            }),
        }
    }

    /// The node type name (e.g. "paragraph", "text", "hardBreak").
    pub fn node_type(&self) -> &str {
        &self.data.node_type
    }

    /// The node's attributes.
    pub fn attrs(&self) -> &HashMap<String, serde_json::Value> {
        &self.data.attrs
    }

    /// The marks applied to this node (only meaningful for text nodes).
    pub fn marks(&self) -> &[Mark] {
        &self.data.marks
    }

    /// Whether this is a text node.
    pub fn is_text(&self) -> bool {
        matches!(&self.data.kind, NodeKind::Text { .. })
    }

    /// Whether this is a void (atomic) node.
    pub fn is_void(&self) -> bool {
        matches!(&self.data.kind, NodeKind::Void)
    }

    /// Whether this is an element (container) node.
    pub fn is_element(&self) -> bool {
        matches!(&self.data.kind, NodeKind::Element { .. })
    }

    /// The token size of this node in the ProseMirror position model.
    ///
    /// - Text nodes: number of Unicode scalar values (chars)
    /// - Void nodes: always 1
    /// - Element nodes: 1 (open) + content size + 1 (close)
    pub fn node_size(&self) -> u32 {
        match &self.data.kind {
            NodeKind::Text { text } => text.chars().count() as u32,
            NodeKind::Void => 1,
            NodeKind::Element { content } => 1 + content.size() + 1,
        }
    }

    /// The size of the node's content (excluding open/close tokens).
    /// For text nodes this equals `node_size()`. For void nodes this is 0.
    /// For element nodes this is the fragment size.
    pub fn content_size(&self) -> u32 {
        match &self.data.kind {
            NodeKind::Text { text } => text.chars().count() as u32,
            NodeKind::Void => 0,
            NodeKind::Element { content } => content.size(),
        }
    }

    /// Collect all text content from this node and its descendants.
    pub fn text_content(&self) -> String {
        let mut output = String::new();
        let mut pending = vec![self];
        while let Some(node) = pending.pop() {
            match &node.data.kind {
                NodeKind::Text { text } => output.push_str(text),
                NodeKind::Void => {}
                NodeKind::Element { content } => {
                    pending.extend(content.iter().rev());
                }
            }
        }
        output
    }

    /// Number of direct children. Text and void nodes have 0 children.
    pub fn child_count(&self) -> usize {
        match &self.data.kind {
            NodeKind::Element { content } => content.child_count(),
            _ => 0,
        }
    }

    /// Access a direct child by index.
    pub fn child(&self, index: usize) -> Option<&Node> {
        match &self.data.kind {
            NodeKind::Element { content } => content.child(index),
            _ => None,
        }
    }

    /// Access the content fragment. Returns `None` for text and void nodes.
    pub fn content(&self) -> Option<&Fragment> {
        match &self.data.kind {
            NodeKind::Element { content } => Some(content),
            _ => None,
        }
    }

    /// The raw text of a text node. Returns `None` for non-text nodes.
    pub fn text_str(&self) -> Option<&str> {
        match &self.data.kind {
            NodeKind::Text { text } => Some(text.as_str()),
            _ => None,
        }
    }

    pub(crate) fn shares_storage_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.data, &other.data)
    }

    pub(crate) fn history_snapshot_retained_bytes(&self) -> Option<usize> {
        let mut total = 0usize;
        let mut pending = vec![self];
        while let Some(node) = pending.pop() {
            total = total
                .checked_add(crate::model::arc_allocation_retained_bytes(
                    std::mem::size_of::<NodeData>(),
                )?)?
                .checked_add(node.data.node_type.capacity())?;
            total = total.checked_add(crate::model::hash_table_retained_bytes::<
                String,
                serde_json::Value,
            >(node.data.attrs.capacity())?)?;
            for (key, value) in &node.data.attrs {
                total = total
                    .checked_add(key.capacity())?
                    .checked_add(crate::model::json_value_retained_bytes(value)?)?;
            }
            total = total.checked_add(
                node.data
                    .marks
                    .capacity()
                    .checked_mul(std::mem::size_of::<Mark>())?,
            )?;
            for mark in &node.data.marks {
                total = total.checked_add(mark.history_snapshot_clone_retained_bytes()?)?;
            }
            match &node.data.kind {
                NodeKind::Text { text } => total = total.checked_add(text.capacity())?,
                NodeKind::Void => {}
                NodeKind::Element { content } => {
                    total = total.checked_add(
                        content
                            .children_capacity()
                            .checked_mul(std::mem::size_of::<Node>())?,
                    )?;
                    pending.extend(content.iter());
                }
            }
        }
        Some(total)
    }
}

#[cfg(test)]
mod shared_storage_tests {
    use super::*;
    use crate::model::Document;
    use crate::schema::presets::tiptap_schema;
    use crate::serialize::to_prosemirror_json;
    use crate::transform::{apply_step, Step};

    fn document() -> Document {
        Document::new(Node::element(
            "doc".into(),
            HashMap::new(),
            Fragment::from(vec![Node::element(
                "paragraph".into(),
                HashMap::from([("class".into(), serde_json::json!("lead"))]),
                Fragment::from(vec![Node::text("abc".into(), Vec::new())]),
            )]),
        ))
    }

    #[test]
    fn node_and_document_clones_share_immutable_payload_and_preserve_equality() {
        let original = document();
        let cloned = original.clone();

        assert!(original.root().shares_storage_with(cloned.root()));
        assert_eq!(original, cloned);
        assert_eq!(cloned.root().node_type(), "doc");
        assert_eq!(
            cloned.root().child(0).unwrap().attrs()["class"],
            serde_json::json!("lead")
        );
        assert_eq!(
            to_prosemirror_json(&original, &tiptap_schema()),
            to_prosemirror_json(&cloned, &tiptap_schema())
        );
    }

    #[test]
    fn transform_rebuilds_new_path_without_mutating_shared_original() {
        let schema = tiptap_schema();
        let original = document();
        let shared_original = original.clone();
        let (changed, _) = apply_step(
            &original,
            &Step::InsertText {
                pos: 2,
                text: "x".into(),
                marks: Vec::new(),
            },
            &schema,
        )
        .unwrap();

        assert!(original.root().shares_storage_with(shared_original.root()));
        assert_eq!(original, shared_original);
        assert_ne!(changed, original);
        assert_eq!(
            original
                .root()
                .child(0)
                .unwrap()
                .child(0)
                .unwrap()
                .text_str(),
            Some("abc")
        );
        assert_eq!(
            changed
                .root()
                .child(0)
                .unwrap()
                .child(0)
                .unwrap()
                .text_str(),
            Some("axbc")
        );
    }
}
