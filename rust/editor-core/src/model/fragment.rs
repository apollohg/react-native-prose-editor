use crate::model::node::Node;

/// An ordered sequence of child nodes within a parent node.
///
/// Fragment tracks the aggregate token size of its children, avoiding
/// repeated traversal when computing document positions.
#[derive(Debug, Clone)]
pub struct Fragment {
    children: Vec<Node>,
    /// Cached token size: sum of each child's `node_size()`.
    size: u32,
}

impl Fragment {
    /// Create an empty fragment (no children, size 0).
    pub fn empty() -> Self {
        Self {
            children: Vec::new(),
            size: 0,
        }
    }

    /// Build a fragment from a vec of child nodes.
    pub fn from(children: Vec<Node>) -> Self {
        let size = children.iter().map(|c| c.node_size()).sum();
        Self { children, size }
    }

    /// Total token size of this fragment (sum of children's `node_size()`).
    pub fn size(&self) -> u32 {
        self.size
    }

    /// Number of direct child nodes.
    pub fn child_count(&self) -> usize {
        self.children.len()
    }

    /// Access a child by index, returning `None` if out of bounds.
    pub fn child(&self, index: usize) -> Option<&Node> {
        self.children.get(index)
    }

    /// Iterate over child nodes.
    pub fn iter(&self) -> std::slice::Iter<'_, Node> {
        self.children.iter()
    }

    /// Access the underlying children slice.
    pub fn children(&self) -> &[Node] {
        &self.children
    }
}
