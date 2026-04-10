pub mod fragment;
pub mod mark;
pub mod node;
pub mod resolved_pos;

pub use fragment::Fragment;
pub use mark::Mark;
pub use node::Node;
pub use resolved_pos::ResolvedPos;

use smallvec::SmallVec;

/// A document is a wrapper around a root node (typically "doc") that provides
/// position resolution and tree queries.
#[derive(Debug, Clone, PartialEq)]
pub struct Document {
    root: Node,
}

impl Document {
    /// Create a document from a root node.
    pub fn new(root: Node) -> Self {
        Self { root }
    }

    /// The root node of the document.
    pub fn root(&self) -> &Node {
        &self.root
    }

    /// Total token size of the document including the root node's open and
    /// close tags.
    pub fn doc_size(&self) -> u32 {
        self.root.node_size()
    }

    /// Size of the document's content (excluding the root node's open/close).
    /// Positions in the public API range from `0..=content_size()`.
    pub fn content_size(&self) -> u32 {
        self.root.content_size()
    }

    /// Resolve an integer position to a `ResolvedPos`.
    ///
    /// Positions are relative to the document content (0 = start of root's
    /// content, `content_size()` = end of root's content). The root node's
    /// own open/close tags are not part of the position space.
    pub fn resolve(&self, pos: u32) -> Result<ResolvedPos, String> {
        if pos > self.content_size() {
            return Err(format!(
                "position {} is out of bounds (document content size is {})",
                pos,
                self.content_size()
            ));
        }

        let mut path: SmallVec<[u16; 8]> = SmallVec::new();
        let mut result = resolved_pos::resolve_in_node(&self.root, pos, &mut path)?;

        // Fill in the absolute position.
        result.pos = pos;
        // depth = 1 (doc) + number of path entries
        result.depth = 1 + result.node_path.len();

        Ok(result)
    }

    /// Look up a node by following a path of child indices from the root.
    /// An empty path returns the root node.
    pub fn node_at(&self, path: &[u16]) -> Option<&Node> {
        let mut node = &self.root;
        for &idx in path {
            node = node.child(idx as usize)?;
        }
        Some(node)
    }
}
