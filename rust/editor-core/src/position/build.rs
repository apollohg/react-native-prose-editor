use smallvec::SmallVec;

use crate::model::node::Node;
use crate::model::Document;
use crate::render;

use super::{BlockMapping, PositionMap};

/// Placeholder character used for void block-level nodes (e.g. horizontalRule)
/// in the rendered text. U+FFFC OBJECT REPLACEMENT CHARACTER.
pub const VOID_BLOCK_PLACEHOLDER: char = '\u{FFFC}';

/// Placeholder character used for inline void nodes (e.g. hardBreak) in the
/// rendered text.
pub const HARD_BREAK_PLACEHOLDER: char = '\n';

/// Number of rendered scalars for a block separator (newline between blocks).
pub const BLOCK_BREAK_SCALARS: u32 = 1;

/// Build a `PositionMap` by walking the document tree.
///
/// The walk identifies every "text block" (a leaf-level element that directly
/// contains inline content, e.g. `paragraph`) and every block-level void node
/// (e.g. `horizontalRule`). Each one becomes a `BlockMapping`.
pub fn build_position_map(doc: &Document) -> PositionMap {
    let mut blocks: Vec<BlockMapping> = Vec::new();
    let mut scalar_cursor: u32 = 0;
    let path: SmallVec<[u16; 8]> = SmallVec::new();

    // The root node ("doc") is an element. We walk its direct and indirect
    // children looking for text blocks and block-level void nodes.
    walk_node(
        doc.root(),
        &path,
        0, // doc_offset: content starts at position 0 inside the doc
        &mut blocks,
        &mut scalar_cursor,
        0,
    );

    // Fix up rendered_break_after: every block except the last gets a break
    // scalar. The last block (terminal) gets 0.
    let block_count = blocks.len();
    for (i, block) in blocks.iter_mut().enumerate() {
        if i + 1 < block_count {
            block.rendered_break_after = BLOCK_BREAK_SCALARS;
            // scalar_start of subsequent blocks is recalculated below.
        } else {
            block.rendered_break_after = 0;
        }
    }

    // Now fix scalar_start values to account for inter-block breaks.
    // We rebuild them: the first block starts at 0, each subsequent block
    // starts at prev.scalar_start + prev.scalar_len + prev.rendered_break_after.
    if !blocks.is_empty() {
        blocks[0].scalar_start = 0;
        for i in 1..blocks.len() {
            let prev_end = blocks[i - 1].scalar_start
                + blocks[i - 1].scalar_prefix_len
                + blocks[i - 1].scalar_len
                + blocks[i - 1].rendered_break_after;
            blocks[i].scalar_start = prev_end;
        }
    }

    PositionMap::from_blocks(blocks)
}

/// Recursively walk a node to find text blocks and block-level void nodes.
///
/// `doc_offset` is the doc position at the start of `node`'s content
/// (i.e. just after the open tag for element nodes, or the position of a
/// void/text node).
fn walk_node(
    node: &Node,
    path: &SmallVec<[u16; 8]>,
    doc_offset: u32,
    blocks: &mut Vec<BlockMapping>,
    scalar_cursor: &mut u32,
    mut pending_prefix_len: u32,
) {
    if node.is_text() {
        // Text nodes are inline content — handled by their parent block.
        return;
    }

    if node.is_void() {
        // Block-level void node (e.g. horizontalRule).
        // Rendered as a placeholder or opaque label.
        blocks.push(BlockMapping {
            doc_start: doc_offset,
            doc_end: doc_offset, // void has no "content range", just a position
            scalar_start: *scalar_cursor, // will be recalculated
            scalar_len: block_visible_scalar_len(node),
            scalar_prefix_len: std::mem::take(&mut pending_prefix_len),
            rendered_break_after: 0, // will be fixed up
            node_path: path.clone(),
            is_void_block: true,
        });
        *scalar_cursor +=
            blocks.last().unwrap().scalar_prefix_len + blocks.last().unwrap().scalar_len;
        return;
    }

    // Element node — check if it's a text block (contains only inline content)
    // or a container (contains other elements).
    let content = node.content().expect("element nodes have content");

    if is_text_block(node) {
        // This is a text block. Compute the scalar length from its inline content.
        let scalar_len = compute_inline_scalars(node);

        blocks.push(BlockMapping {
            doc_start: doc_offset,
            doc_end: doc_offset + content.size(),
            scalar_start: *scalar_cursor, // will be recalculated
            scalar_len,
            scalar_prefix_len: std::mem::take(&mut pending_prefix_len),
            rendered_break_after: 0, // will be fixed up
            node_path: path.clone(),
            is_void_block: false,
        });
        *scalar_cursor += blocks.last().unwrap().scalar_prefix_len + scalar_len;
        return;
    }

    // Container node — recurse into children.
    let mut child_doc_offset = doc_offset;
    for (child_idx, child) in content.iter().enumerate() {
        let mut child_path = path.clone();
        child_path.push(child_idx as u16);
        let mut child_prefix_len = pending_prefix_len;
        pending_prefix_len = 0;

        if is_list_node(node) && is_list_item_node(child) {
            child_prefix_len += list_marker_len(node, child_idx);
        }

        if child.is_element() {
            // Skip the open tag to get to the child's content start
            walk_node(
                child,
                &child_path,
                child_doc_offset + 1, // +1 for open tag
                blocks,
                scalar_cursor,
                child_prefix_len,
            );
        } else if child.is_void() {
            // Block-level void (e.g. hr at doc level).
            // The doc position of the void node is child_doc_offset.
            walk_node(
                child,
                &child_path,
                child_doc_offset,
                blocks,
                scalar_cursor,
                child_prefix_len,
            );
        } else {
            pending_prefix_len = child_prefix_len;
        }
        // Text nodes at this level would be unusual (text directly in doc)
        // but we skip them — they'd only appear in text blocks which we
        // handle above.

        child_doc_offset += child.node_size();
    }
}

/// Determine if a node is a "text block" — an element that contains only
/// inline content (text nodes and inline void nodes like hardBreak).
///
/// An element with no children (empty paragraph) is also a text block.
fn is_text_block(node: &Node) -> bool {
    let content = match node.content() {
        Some(c) => c,
        None => return false,
    };

    // Empty element counts as a text block (e.g. empty paragraph)
    if content.child_count() == 0 {
        return true;
    }

    // All children must be inline (text or void)
    content
        .iter()
        .all(|child| child.is_text() || child.is_void())
}

/// Count the number of rendered scalars in a text block's inline content.
///
/// - Text nodes contribute their Unicode scalar count.
/// - Inline void nodes (hardBreak) contribute 1 scalar each.
fn compute_inline_scalars(node: &Node) -> u32 {
    let content = match node.content() {
        Some(c) => c,
        None => return 0,
    };

    if content.child_count() == 0 {
        return 1;
    }

    let mut count: u32 = 0;
    for child in content.iter() {
        if child.is_text() {
            count += child.node_size(); // node_size for text = scalar count
        } else if child.is_void() {
            count += inline_visible_scalar_len(child);
        }
        // Element children inside a text block shouldn't exist, but if they
        // do we skip them (defensive).
    }
    count
}

fn is_list_node(node: &Node) -> bool {
    matches!(
        node.node_type(),
        "bulletList" | "bullet_list" | "orderedList" | "ordered_list"
    )
}

fn is_ordered_list_node(node: &Node) -> bool {
    matches!(node.node_type(), "orderedList" | "ordered_list")
}

fn is_list_item_node(node: &Node) -> bool {
    matches!(node.node_type(), "listItem" | "list_item")
}

fn list_marker_len(list_node: &Node, child_index: usize) -> u32 {
    let ordered = is_ordered_list_node(list_node);
    let start = list_node
        .attrs()
        .get("start")
        .and_then(|value| value.as_u64())
        .unwrap_or(1) as u32;
    let index = if ordered {
        start + child_index as u32
    } else {
        child_index as u32 + 1
    };
    render::list_marker_string(ordered, index).chars().count() as u32
}

fn inline_visible_scalar_len(node: &Node) -> u32 {
    render::inline_node_visible_scalar_len(
        node.node_type(),
        node.attrs().get("label").and_then(|value| value.as_str()),
        matches!(node.node_type(), "hardBreak" | "hard_break"),
    )
}

fn block_visible_scalar_len(node: &Node) -> u32 {
    render::block_node_visible_scalar_len(
        node.node_type(),
        node.attrs().get("label").and_then(|value| value.as_str()),
        matches!(node.node_type(), "horizontalRule" | "horizontal_rule"),
    )
}
