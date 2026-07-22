pub mod fragment;
pub mod mark;
pub mod node;
pub mod resolved_pos;

pub use fragment::Fragment;
pub use mark::Mark;
pub use node::Node;
pub use resolved_pos::ResolvedPos;

use smallvec::SmallVec;

pub(crate) fn arc_allocation_retained_bytes(payload_bytes: usize) -> Option<usize> {
    payload_bytes.checked_add(std::mem::size_of::<[usize; 3]>())
}

pub(crate) fn hash_table_retained_bytes<K, V>(capacity: usize) -> Option<usize> {
    if capacity == 0 {
        return Some(0);
    }
    let buckets_and_controls = capacity.checked_mul(2)?.checked_add(1)?;
    buckets_and_controls
        .checked_mul(std::mem::size_of::<(K, V)>().checked_add(std::mem::size_of::<usize>())?)
}

pub(crate) fn json_value_retained_bytes(value: &serde_json::Value) -> Option<usize> {
    let mut total = 0usize;
    let mut pending = vec![value];
    while let Some(value) = pending.pop() {
        match value {
            serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_) => {
            }
            serde_json::Value::String(value) => total = total.checked_add(value.capacity())?,
            serde_json::Value::Array(values) => {
                total = total.checked_add(
                    values
                        .capacity()
                        .checked_mul(std::mem::size_of::<serde_json::Value>())?,
                )?;
                pending.extend(values);
            }
            serde_json::Value::Object(values) => {
                // With serde_json's default BTreeMap backing, even a one-entry map
                // allocates a node with capacity for 11 key/value pairs and 12
                // child edges. Charging one full node per populated entry safely
                // overbounds every possible tree shape, including sparse nodes.
                let btree_node = std::mem::size_of::<(String, serde_json::Value)>()
                    .checked_mul(11)?
                    .checked_add(std::mem::size_of::<usize>().checked_mul(12)?)?
                    .checked_add(std::mem::size_of::<[usize; 3]>())?;
                total = total.checked_add(values.len().checked_mul(btree_node)?)?;
                for (key, value) in values {
                    total = total.checked_add(key.capacity())?;
                    pending.push(value);
                }
            }
        }
    }
    Some(total)
}

/// A document is a wrapper around a root node (typically "doc") that provides
/// position resolution and tree queries.
#[derive(Debug, Clone, PartialEq)]
pub struct Document {
    root: Node,
}

#[cfg(test)]
thread_local! {
    static HISTORY_SNAPSHOT_RETAINED_BYTES_TRAVERSALS: std::cell::Cell<usize> =
        const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(crate) fn reset_history_snapshot_retained_bytes_traversals_for_test() {
    HISTORY_SNAPSHOT_RETAINED_BYTES_TRAVERSALS.set(0);
}

#[cfg(test)]
pub(crate) fn take_history_snapshot_retained_bytes_traversals_for_test() -> usize {
    HISTORY_SNAPSHOT_RETAINED_BYTES_TRAVERSALS.replace(0)
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

    pub(crate) fn shares_root_storage_with(&self, other: &Self) -> bool {
        self.root.shares_storage_with(&other.root)
    }

    pub(crate) fn history_snapshot_retained_bytes(&self) -> Option<usize> {
        #[cfg(test)]
        HISTORY_SNAPSHOT_RETAINED_BYTES_TRAVERSALS.set(
            HISTORY_SNAPSHOT_RETAINED_BYTES_TRAVERSALS
                .get()
                .saturating_add(1),
        );
        self.root.history_snapshot_retained_bytes()
    }

    /// Total token size of the document including the root node's open and
    /// close tags.
    // Not reachable from production call paths after the Task 16C legacy runtime
    // removal; exercised by crate tests.
    #[allow(dead_code)]
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

        let mut path: SmallVec<[u32; 8]> = SmallVec::new();
        let mut result = resolved_pos::resolve_in_node(&self.root, pos, &mut path)?;

        // Fill in the absolute position.
        result.pos = pos;
        // depth = 1 (doc) + number of path entries
        result.depth = 1 + result.node_path.len();

        Ok(result)
    }

    /// Look up a node by following a path of child indices from the root.
    /// An empty path returns the root node.
    pub fn node_at(&self, path: &[u32]) -> Option<&Node> {
        let mut node = &self.root;
        for &idx in path {
            node = node.child(idx as usize)?;
        }
        Some(node)
    }
}
