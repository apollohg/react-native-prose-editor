pub mod build;
pub mod delta_tree;
mod fuzz_tests;
pub mod update;

use smallvec::SmallVec;

use crate::model::node::Node;
use crate::model::resolved_pos::ResolvedPos;
use crate::model::Document;
use crate::render;

use delta_tree::DeltaTree;

// ---------------------------------------------------------------------------
// BlockMapping
// ---------------------------------------------------------------------------

/// Maps one "rendered block" between doc positions and scalar offsets.
///
/// A block is either:
/// - A text block (e.g. paragraph) that directly contains inline content
/// - A block-level void node (e.g. horizontalRule) rendered as a placeholder
#[derive(Debug, Clone)]
pub struct BlockMapping {
    /// Doc position at the start of this block's content (after the open tag).
    /// For void blocks, this is the position of the void node itself.
    pub doc_start: u32,
    /// Doc position at the end of this block's content (before the close tag).
    /// For void blocks, this equals `doc_start`.
    pub doc_end: u32,
    /// Rendered-text scalar offset where this block begins.
    pub scalar_start: u32,
    /// Number of rendered scalars in this block's content.
    pub scalar_len: u32,
    /// Number of rendered scalars prepended ahead of the block content
    /// (for example list markers like "1. " or "• ").
    pub scalar_prefix_len: u32,
    /// Number of scalars for the separator after this block (0 for terminal).
    pub rendered_break_after: u32,
    /// Path from doc root to this block's node (child indices at each level).
    pub node_path: SmallVec<[u16; 8]>,
    /// Whether this block maps a block-level void node instead of text content.
    pub is_void_block: bool,
}

// ---------------------------------------------------------------------------
// PositionMap
// ---------------------------------------------------------------------------

/// Bidirectional index for converting between doc positions and rendered-text
/// scalar offsets.
///
/// Doc positions are ProseMirror-style token offsets (including structural
/// open/close tokens). Rendered-text scalar offsets are the flat visible text
/// shown in the native text view.
#[derive(Debug, Clone)]
pub struct PositionMap {
    blocks: Vec<BlockMapping>,
    prefix_deltas: DeltaTree,
}

impl PositionMap {
    /// Build a position map from a document.
    pub fn build(doc: &Document) -> Self {
        build::build_position_map(doc)
    }

    /// Create from pre-built block mappings (used by the build module).
    pub(crate) fn from_blocks(blocks: Vec<BlockMapping>) -> Self {
        Self {
            blocks,
            prefix_deltas: DeltaTree::empty(),
        }
    }

    /// Number of blocks in the map.
    pub fn block_count(&self) -> usize {
        self.blocks.len()
    }

    /// Access a block mapping by index.
    pub fn block(&self, index: usize) -> Option<&BlockMapping> {
        self.blocks.get(index)
    }

    /// Total rendered scalar count (sum of all block scalars + inter-block breaks).
    pub fn total_scalars(&self) -> u32 {
        if self.blocks.is_empty() {
            return 0;
        }
        let last = &self.blocks[self.blocks.len() - 1];
        let (_, sd) = self.prefix_deltas.accumulated_delta(self.blocks.len() - 1);
        let last_scalar_start = (last.scalar_start as i64 + sd as i64) as u32;
        last_scalar_start + last.scalar_prefix_len + last.scalar_len + last.rendered_break_after
    }

    /// Get the effective doc_start for a block, accounting for pending deltas.
    fn effective_doc_start(&self, block_idx: usize) -> u32 {
        let block = &self.blocks[block_idx];
        let (dd, _) = self.prefix_deltas.accumulated_delta(block_idx);
        (block.doc_start as i64 + dd as i64) as u32
    }

    /// Get the effective doc_end for a block, accounting for pending deltas.
    fn effective_doc_end(&self, block_idx: usize) -> u32 {
        let block = &self.blocks[block_idx];
        let (dd, _) = self.prefix_deltas.accumulated_delta(block_idx);
        (block.doc_end as i64 + dd as i64) as u32
    }

    /// Get the effective scalar_start for a block, accounting for pending deltas.
    fn effective_scalar_start(&self, block_idx: usize) -> u32 {
        let block = &self.blocks[block_idx];
        let (_, sd) = self.prefix_deltas.accumulated_delta(block_idx);
        (block.scalar_start as i64 + sd as i64) as u32
    }

    // -----------------------------------------------------------------------
    // scalar_to_doc
    // -----------------------------------------------------------------------

    /// Convert a rendered-text scalar offset to a doc position.
    ///
    /// The scalar offset must be within `0..total_scalars()`. Offsets that
    /// fall on a block break are mapped to the end of the preceding block's
    /// content.
    pub fn scalar_to_doc(&self, scalar_offset: u32, doc: &Document) -> u32 {
        if self.blocks.is_empty() {
            return 0;
        }

        // Find the block containing this scalar offset.
        let block_idx = self.find_block_for_scalar(scalar_offset);
        let block = &self.blocks[block_idx];
        let eff_scalar_start = self.effective_scalar_start(block_idx);
        let eff_doc_start = self.effective_doc_start(block_idx);

        // Check if this is a void block (doc_start == doc_end).
        if block.is_void_block {
            let visible_len = block.scalar_len;
            let intra_scalar = scalar_offset.saturating_sub(eff_scalar_start);
            return if intra_scalar >= visible_len {
                eff_doc_start.saturating_add(1)
            } else {
                eff_doc_start
            };
        }

        // Intra-block scalar offset.
        let intra_scalar = scalar_offset.saturating_sub(eff_scalar_start);
        if intra_scalar < block.scalar_prefix_len {
            return eff_doc_start;
        }
        let intra_scalar = intra_scalar - block.scalar_prefix_len;

        // Walk the block's content in the document to map scalar offset to
        // doc offset.
        let block_node = doc.node_at(&block.node_path);
        let doc_intra_offset = match block_node {
            Some(node) => scalar_to_doc_intra_block(node, intra_scalar),
            None => intra_scalar, // fallback: assume 1:1 mapping
        };

        eff_doc_start + doc_intra_offset
    }

    // -----------------------------------------------------------------------
    // doc_to_scalar
    // -----------------------------------------------------------------------

    /// Convert a doc position to a rendered-text scalar offset.
    ///
    /// If the position falls on a structural token (between blocks), it is
    /// snapped to the nearest cursorable position.
    pub fn doc_to_scalar(&self, doc_pos: u32, doc: &Document) -> u32 {
        if self.blocks.is_empty() {
            return 0;
        }

        // Find which block contains (or is nearest to) this doc position.
        match self.find_block_for_doc_pos(doc_pos) {
            Some(block_idx) => {
                let eff_doc_start = self.effective_doc_start(block_idx);
                let eff_doc_end = self.effective_doc_end(block_idx);
                let eff_scalar_start = self.effective_scalar_start(block_idx);
                let block = &self.blocks[block_idx];

                // Void block: return the block's scalar start.
                if block.is_void_block {
                    if doc_pos <= eff_doc_start {
                        return eff_scalar_start;
                    }
                    return eff_scalar_start + block.scalar_len;
                }

                if block.doc_start == block.doc_end && block.scalar_len > 0 {
                    if doc_pos < eff_doc_start {
                        return eff_scalar_start + block.scalar_prefix_len;
                    }
                    return eff_scalar_start + block.scalar_prefix_len + block.scalar_len;
                }

                if doc_pos < eff_doc_start {
                    // Before this block's content — snap to start.
                    return eff_scalar_start + block.scalar_prefix_len;
                }

                if doc_pos > eff_doc_end {
                    // After this block's content — snap to end.
                    return eff_scalar_start + block.scalar_prefix_len + block.scalar_len;
                }

                // Inside the block — compute intra-block offset.
                let intra_doc = doc_pos - eff_doc_start;
                let block_node = doc.node_at(&block.node_path);
                let intra_scalar = match block_node {
                    Some(node) => doc_to_scalar_intra_block(node, intra_doc),
                    None => intra_doc, // fallback
                };

                eff_scalar_start + block.scalar_prefix_len + intra_scalar
            }
            None => {
                // Position is beyond all blocks — return total scalars.
                self.total_scalars()
            }
        }
    }

    // -----------------------------------------------------------------------
    // resolve
    // -----------------------------------------------------------------------

    /// Resolve a doc position to a `ResolvedPos` using the underlying document.
    pub fn resolve(&self, doc_pos: u32, doc: &Document) -> Result<ResolvedPos, String> {
        doc.resolve(doc_pos)
    }

    // -----------------------------------------------------------------------
    // normalize_cursor_pos
    // -----------------------------------------------------------------------

    /// Snap a doc position to the nearest cursorable position.
    ///
    /// - If inside text content: already cursorable, return as-is.
    /// - If on a structural token (node open/close): snap to nearest content.
    ///   - Node open tag: snap to first content position inside the node.
    ///   - Node close tag: snap to last content position inside the node.
    ///   - Between blocks: snap to start of next block's content, or end of
    ///     previous block's content.
    pub fn normalize_cursor_pos(&self, doc_pos: u32, _doc: &Document) -> u32 {
        if self.blocks.is_empty() {
            return 0;
        }

        // Check if the position is inside a block.
        if let Some(block_idx) = self.find_block_for_doc_pos(doc_pos) {
            let eff_doc_start = self.effective_doc_start(block_idx);
            let eff_doc_end = self.effective_doc_end(block_idx);
            let block = &self.blocks[block_idx];

            // Void block
            if block.doc_start == block.doc_end {
                return eff_doc_start;
            }

            if doc_pos >= eff_doc_start && doc_pos <= eff_doc_end {
                // Inside block content — already cursorable.
                return doc_pos;
            }

            // Position is on a structural token near this block.
            if doc_pos < eff_doc_start {
                // Before block content (on open tag) — snap to start.
                return eff_doc_start;
            }

            // After block content (on close tag) — snap to end.
            return eff_doc_end;
        }

        // Position is beyond all blocks — snap to the end of the last block.
        let last_idx = self.blocks.len() - 1;
        self.effective_doc_end(last_idx)
    }

    // -----------------------------------------------------------------------
    // Block lookup helpers
    // -----------------------------------------------------------------------

    /// Find the block index that contains the given scalar offset.
    ///
    /// Uses binary search on effective scalar ranges.
    fn find_block_for_scalar(&self, scalar_offset: u32) -> usize {
        if self.blocks.is_empty() {
            return 0;
        }

        // Binary search: find the last block whose effective scalar_start <= scalar_offset.
        let mut lo = 0usize;
        let mut hi = self.blocks.len();

        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let eff_start = self.effective_scalar_start(mid);
            if eff_start <= scalar_offset {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }

        // lo is now the first block whose effective_scalar_start > scalar_offset.
        // The containing block is lo - 1 (or 0 if lo == 0).
        if lo == 0 {
            0
        } else {
            lo - 1
        }
    }

    /// Find the block index that contains or is nearest to the given doc position.
    ///
    /// Returns `None` if the position is beyond all blocks.
    pub(crate) fn find_block_for_doc_pos(&self, doc_pos: u32) -> Option<usize> {
        if self.blocks.is_empty() {
            return None;
        }

        // For each block, the "coverage" is from some position before doc_start
        // (the open tag) to some position after doc_end (the close tag).
        // For precise matching we need to account for structural tokens.
        //
        // Strategy: find the block with the closest doc_start that is <= doc_pos.
        // If doc_pos is past that block's doc_end, check if it's on the close
        // tag or between blocks.

        let mut best_idx: Option<usize> = None;

        for i in 0..self.blocks.len() {
            let eff_start = self.effective_doc_start(i);
            let eff_end = self.effective_doc_end(i);
            let block = &self.blocks[i];

            // For void blocks, the coverage is exactly at doc_start.
            if block.doc_start == block.doc_end {
                // Void block: position is at or near the void's position.
                // The void node occupies 1 doc token at doc_start.
                // But doc_start here was set to the content position (after open tag
                // of parent), and the void occupies that position.
                if doc_pos == eff_start {
                    return Some(i);
                }
                if doc_pos < eff_start {
                    // Position is before this void block — use previous block or this one.
                    break;
                }
                best_idx = Some(i);
                continue;
            }

            if doc_pos >= eff_start && doc_pos <= eff_end {
                return Some(i);
            }

            if doc_pos < eff_start {
                // Position is before this block (on a structural token).
                // Snap to this block or the previous one.
                if let Some(prev) = best_idx {
                    // Between two blocks: snap to whichever is closer.
                    let prev_end = self.effective_doc_end(prev);
                    let dist_to_prev = doc_pos - prev_end;
                    let dist_to_next = eff_start - doc_pos;
                    if dist_to_prev <= dist_to_next {
                        return Some(prev);
                    } else {
                        return Some(i);
                    }
                }
                return Some(i);
            }

            best_idx = Some(i);
        }

        best_idx
    }

    /// Access the internal blocks slice (for testing / debugging).
    pub fn blocks(&self) -> &[BlockMapping] {
        &self.blocks
    }
}

// ---------------------------------------------------------------------------
// Intra-block scalar ↔ doc offset conversion
// ---------------------------------------------------------------------------

/// Walk a text block node's content and convert a scalar offset to a doc
/// token offset within the block.
///
/// Text nodes: 1 scalar = 1 doc token (both count Unicode scalars).
/// Void nodes: 1 scalar (placeholder) = 1 doc token.
fn scalar_to_doc_intra_block(block_node: &Node, scalar_offset: u32) -> u32 {
    let content = match block_node.content() {
        Some(c) => c,
        None => return scalar_offset,
    };

    let mut scalars_consumed: u32 = 0;
    let mut doc_offset: u32 = 0;

    for child in content.iter() {
        if child.is_text() {
            let text_scalars = child.node_size();
            if scalars_consumed + text_scalars > scalar_offset {
                // Position is within this text node.
                let remaining = scalar_offset - scalars_consumed;
                return doc_offset + remaining;
            }
            scalars_consumed += text_scalars;
            doc_offset += text_scalars;
        } else if child.is_void() {
            let visible_len = inline_void_visible_scalar_len(child);
            if scalars_consumed + visible_len > scalar_offset {
                // Position is at this void node.
                return doc_offset;
            }
            scalars_consumed += visible_len;
            doc_offset += 1; // void = 1 doc token
        } else {
            // Nested element inside a "text block" — shouldn't happen normally.
            doc_offset += child.node_size();
        }
    }

    // At the end of block content.
    doc_offset
}

/// Walk a text block node's content and convert a doc token offset to a
/// scalar offset within the block.
///
/// Mirrors `scalar_to_doc_intra_block` in reverse.
fn doc_to_scalar_intra_block(block_node: &Node, doc_offset: u32) -> u32 {
    let content = match block_node.content() {
        Some(c) => c,
        None => return doc_offset,
    };

    let mut doc_consumed: u32 = 0;
    let mut scalar_offset: u32 = 0;

    for child in content.iter() {
        if child.is_text() {
            let text_size = child.node_size();
            if doc_consumed + text_size > doc_offset {
                let remaining = doc_offset - doc_consumed;
                return scalar_offset + remaining;
            }
            doc_consumed += text_size;
            scalar_offset += text_size;
        } else if child.is_void() {
            if doc_consumed + 1 > doc_offset {
                return scalar_offset;
            }
            doc_consumed += 1;
            scalar_offset += inline_void_visible_scalar_len(child);
        } else {
            let node_size = child.node_size();
            if doc_consumed + node_size > doc_offset {
                // Position is inside a nested element in a text block.
                // This shouldn't happen in well-formed documents, but
                // snap to the scalar position before it.
                return scalar_offset;
            }
            doc_consumed += node_size;
            // Nested elements don't contribute scalars in a text block.
        }
    }

    scalar_offset
}

fn inline_void_visible_scalar_len(node: &Node) -> u32 {
    render::inline_node_visible_scalar_len(
        node.node_type(),
        node.attrs().get("label").and_then(|value| value.as_str()),
        matches!(node.node_type(), "hardBreak" | "hard_break"),
    )
}
