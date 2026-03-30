use smallvec::SmallVec;

use crate::model::node::Node;

/// A resolved document position.
///
/// In ProseMirror's position model, a document is a flat token stream where:
/// - Each element node open/close tag = 1 token
/// - Each Unicode scalar in a text node = 1 token
/// - Each void node = 1 token
///
/// `ResolvedPos` maps an integer position in this token stream back to a
/// concrete location in the document tree.
#[derive(Debug, Clone)]
pub struct ResolvedPos {
    /// The original document position.
    pub pos: u32,
    /// Nesting depth (1 = inside doc, 2 = inside a child of doc, etc.).
    pub depth: usize,
    /// Index chain from root: `node_path[i]` is the child index at depth `i+1`.
    /// Length = `depth - 1` (the root doc is implicit).
    pub node_path: SmallVec<[u16; 8]>,
    /// Offset within the innermost parent node's content (in tokens).
    pub parent_offset: u32,
}

impl ResolvedPos {
    /// Return a reference to the parent node at the resolved position.
    ///
    /// Follows `node_path` from the document root to reach the parent.
    pub fn parent<'a>(&self, doc: &'a super::Document) -> &'a Node {
        let mut node = doc.root();
        for &idx in &self.node_path {
            node = node
                .child(idx as usize)
                .expect("node_path should contain valid child indices");
        }
        node
    }
}

/// Walk the document tree and resolve an integer position to a `ResolvedPos`.
///
/// `pos` is relative to the start of `node`'s content (i.e. after the node's
/// open tag). `base_pos` is the absolute position of the start of `node`'s
/// content in the full document.
///
/// Returns `Err` if `pos` is out of bounds for the node's content.
pub(crate) fn resolve_in_node(
    node: &Node,
    pos: u32,
    path: &mut SmallVec<[u16; 8]>,
) -> Result<ResolvedPos, String> {
    let content = match node.content() {
        Some(c) => c,
        None => {
            // Text and void nodes have no content to resolve into.
            // This shouldn't be reached from the public API because we only
            // call resolve_in_node on element nodes.
            return Err(format!(
                "cannot resolve position inside non-element node '{}'",
                node.node_type()
            ));
        }
    };

    if pos > content.size() {
        return Err(format!(
            "position {} is out of bounds for node '{}' with content size {}",
            pos,
            node.node_type(),
            content.size()
        ));
    }

    // Walk children, accumulating token offsets to find which child (if any)
    // the position falls inside.
    let mut offset: u32 = 0;

    for (child_idx, child) in content.iter().enumerate() {
        let child_size = child.node_size();

        if child.is_text() {
            // Text nodes are flat — positions within them don't increase depth.
            // A position falls "in" this text if offset <= pos < offset + child_size.
            // But if pos == offset + child_size, we've moved past this text node.
            if pos < offset + child_size {
                // Position is inside this text node (or at its start).
                // Depth stays at the current level, parent_offset = pos.
                return Ok(ResolvedPos {
                    pos: 0, // will be overwritten by caller
                    depth: 0,
                    node_path: path.clone(),
                    parent_offset: pos,
                });
            }
            offset += child_size;
        } else if child.is_void() {
            // Void nodes occupy exactly 1 token. If pos == offset, the
            // position is "at" this void node (in the parent's content).
            if pos < offset + 1 {
                return Ok(ResolvedPos {
                    pos: 0,
                    depth: 0,
                    node_path: path.clone(),
                    parent_offset: pos,
                });
            }
            offset += 1;
        } else {
            // Element node: open tag (1) + content + close tag (1)
            // If pos == offset, we're at this element's position in the
            // parent (before its open tag, from the parent's perspective).
            if pos == offset {
                return Ok(ResolvedPos {
                    pos: 0,
                    depth: 0,
                    node_path: path.clone(),
                    parent_offset: pos,
                });
            }

            // If pos falls within the element's span (between open and close tags)
            let inner_start = offset + 1; // after open tag
            let inner_end = offset + child_size - 1; // before close tag

            if pos >= inner_start && pos <= inner_end {
                // Position is inside this child element
                let inner_pos = pos - inner_start;
                path.push(child_idx as u16);
                return resolve_in_node(child, inner_pos, path);
            }

            // pos is at or after the close tag — continue to next child
            offset += child_size;
        }
    }

    // If we've consumed all children and pos == offset, the position is at
    // the end of the parent's content (before the close tag).
    if pos == offset {
        return Ok(ResolvedPos {
            pos: 0,
            depth: 0,
            node_path: path.clone(),
            parent_offset: pos,
        });
    }

    Err(format!(
        "position {} not found in node '{}' content (offset reached {})",
        pos,
        node.node_type(),
        offset
    ))
}
