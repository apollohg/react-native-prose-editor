pub mod build;
pub mod delta_tree;
mod fuzz_tests;
pub mod update;

use smallvec::SmallVec;
use std::collections::HashSet;

use crate::model::node::Node;
use crate::model::resolved_pos::ResolvedPos;
use crate::model::Document;
use crate::render;
use crate::schema::Schema;

use delta_tree::DeltaTree;

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
    pub node_path: SmallVec<[u32; 8]>,
    /// Whether this block maps a block-level void node instead of text content.
    pub is_void_block: bool,
}

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
    hard_break_node_types: HashSet<String>,
}

impl PositionMap {
    /// Build a position map from a document.
    pub fn build(doc: &Document, schema: &Schema) -> Self {
        build::build_position_map(doc, schema)
    }

    /// Create from pre-built block mappings (used by the build module).
    pub(crate) fn from_blocks(blocks: Vec<BlockMapping>, schema: &Schema) -> Self {
        Self {
            blocks,
            prefix_deltas: DeltaTree::empty(),
            hard_break_node_types: schema.hard_break_node_types().map(str::to_owned).collect(),
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

    /// Convert a rendered-text scalar offset to a doc position.
    ///
    /// The scalar offset must be within `0..total_scalars()`. Offsets that
    /// fall on a block break are mapped to the end of the preceding block's
    /// content.
    pub fn scalar_to_doc(&self, scalar_offset: u32, doc: &Document) -> u32 {
        self.scalar_to_doc_metered(scalar_offset, doc, |_| true)
            .map(|(position, _)| position)
            .unwrap_or(0)
    }

    pub(crate) fn scalar_to_doc_metered(
        &self,
        scalar_offset: u32,
        doc: &Document,
        mut consume: impl FnMut(usize) -> bool,
    ) -> Option<(u32, usize)> {
        if self.blocks.is_empty() {
            return Some((0, 0));
        }

        let mut lo = 0usize;
        let mut hi = self.blocks.len();
        while lo < hi {
            if !consume(1) {
                return None;
            }
            let mid = lo + (hi - lo) / 2;
            if self.effective_scalar_start(mid) <= scalar_offset {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        let block_idx = lo.saturating_sub(1);
        let block = &self.blocks[block_idx];
        let eff_scalar_start = self.effective_scalar_start(block_idx);
        let eff_doc_start = self.effective_doc_start(block_idx);

        // Check if this is a void block (doc_start == doc_end).
        if block.is_void_block {
            let visible_len = block.scalar_len;
            let intra_scalar = scalar_offset.saturating_sub(eff_scalar_start);
            return Some((
                if intra_scalar >= visible_len {
                    eff_doc_start.saturating_add(1)
                } else {
                    eff_doc_start
                },
                block_idx,
            ));
        }

        // Intra-block scalar offset.
        let intra_scalar = scalar_offset.saturating_sub(eff_scalar_start);
        if intra_scalar < block.scalar_prefix_len {
            return Some((eff_doc_start, block_idx));
        }
        let intra_scalar = intra_scalar - block.scalar_prefix_len;

        let mut block_node = Some(doc.root());
        for &index in &block.node_path {
            if !consume(1) {
                return None;
            }
            block_node = block_node?.child(usize::try_from(index).ok()?);
        }
        let doc_intra_offset = match block_node {
            Some(node) => scalar_to_doc_intra_block_metered(
                node,
                intra_scalar,
                &self.hard_break_node_types,
                &mut consume,
            )?,
            None => intra_scalar, // fallback: assume 1:1 mapping
        };

        eff_doc_start
            .checked_add(doc_intra_offset)
            .map(|position| (position, block_idx))
    }

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
                    Some(node) => {
                        doc_to_scalar_intra_block(node, intra_doc, &self.hard_break_node_types)
                    }
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

    /// Resolve a doc position to a `ResolvedPos` using the underlying document.
    // Not reachable from production call paths after the Task 16C legacy runtime
    // removal; exercised by crate tests.
    #[allow(dead_code)]
    pub fn resolve(&self, doc_pos: u32, doc: &Document) -> Result<ResolvedPos, String> {
        doc.resolve(doc_pos)
    }

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

        let last_idx = self.blocks.len() - 1;
        let last = &self.blocks[last_idx];
        if last.is_void_block && doc_pos == self.effective_doc_start(last_idx).saturating_add(1) {
            return doc_pos;
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
        self.effective_doc_end(last_idx)
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
    // Not reachable from production call paths after the Task 16C legacy runtime
    // removal; exercised by crate tests.
    #[allow(dead_code)]
    pub fn blocks(&self) -> &[BlockMapping] {
        &self.blocks
    }

    /// Rendered scalar width of an inline void in this map's schema domain.
    /// Localized derived indexes use this while streaming a block once.
    pub(crate) fn inline_void_scalar_len(&self, node: &Node) -> Option<u32> {
        node.is_void()
            .then(|| inline_void_visible_scalar_len(node, &self.hard_break_node_types))
    }

    pub(crate) fn has_effective_stored_bounds(&self) -> bool {
        self.prefix_deltas.is_empty()
    }

    /// Upper bound for heap allocations produced by cloning this map into a
    /// history snapshot. Source capacities bound the corresponding clone
    /// requests; spilled `SmallVec` paths are charged individually.
    pub(crate) fn history_snapshot_clone_retained_bytes(&self) -> Option<usize> {
        let block_bytes = self
            .blocks
            .capacity()
            .checked_mul(std::mem::size_of::<BlockMapping>())?;
        let spilled_path_bytes = self.blocks.iter().try_fold(0usize, |total, block| {
            let path_bytes = if block.node_path.spilled() {
                block
                    .node_path
                    .capacity()
                    .checked_mul(std::mem::size_of::<u32>())?
            } else {
                0
            };
            total.checked_add(path_bytes)
        })?;
        let hard_break_capacity = self.hard_break_node_types.capacity();
        let hard_break_bucket_count_bound = if hard_break_capacity == 0 {
            0
        } else {
            // `HashSet::capacity()` reports the element capacity at its load
            // factor. Twice that capacity plus one bounds both buckets and the
            // trailing control group without relying on hashbrown internals.
            hard_break_capacity.checked_mul(2)?.checked_add(1)?
        };
        // A bucket stores one String plus control metadata. One usize of
        // control charge per bucket also covers the trailing SIMD control
        // group used by the standard hash-table implementation.
        let hard_break_table_bytes = hard_break_bucket_count_bound.checked_mul(
            std::mem::size_of::<String>().checked_add(std::mem::size_of::<usize>())?,
        )?;
        let hard_break_string_bytes = self
            .hard_break_node_types
            .iter()
            .try_fold(0usize, |total, value| total.checked_add(value.capacity()))?;

        block_bytes
            .checked_add(spilled_path_bytes)?
            .checked_add(self.prefix_deltas.history_snapshot_clone_retained_bytes()?)?
            .checked_add(hard_break_table_bytes)?
            .checked_add(hard_break_string_bytes)
    }
}

// Intra-block scalar ↔ doc offset conversion

/// Walk a text block node's content and convert a scalar offset to a doc
/// token offset within the block.
///
/// Text nodes: 1 scalar = 1 doc token (both count Unicode scalars).
/// Void nodes: 1 scalar (placeholder) = 1 doc token.
fn scalar_to_doc_intra_block_metered(
    block_node: &Node,
    scalar_offset: u32,
    hard_break_node_types: &HashSet<String>,
    consume: &mut impl FnMut(usize) -> bool,
) -> Option<u32> {
    let content = block_node.content()?;

    let mut scalars_consumed: u32 = 0;
    let mut doc_offset: u32 = 0;

    for child in content.iter() {
        if !consume(1) {
            return None;
        }
        if child.is_text() {
            if !consume(child.text_str()?.len()) {
                return None;
            }
            let text_scalars = child.node_size();
            if scalars_consumed.checked_add(text_scalars)? > scalar_offset {
                // Position is within this text node.
                let remaining = scalar_offset - scalars_consumed;
                return doc_offset.checked_add(remaining);
            }
            scalars_consumed = scalars_consumed.checked_add(text_scalars)?;
            doc_offset = doc_offset.checked_add(text_scalars)?;
        } else if child.is_void() {
            if !consume(inline_void_render_work(child)?) {
                return None;
            }
            let visible_len = inline_void_visible_scalar_len(child, hard_break_node_types);
            if scalars_consumed.checked_add(visible_len)? > scalar_offset {
                // Position is at this void node.
                return Some(doc_offset);
            }
            scalars_consumed = scalars_consumed.checked_add(visible_len)?;
            doc_offset = doc_offset.checked_add(1)?; // void = 1 doc token
        } else {
            // Nested element inside a "text block" — shouldn't happen normally.
            doc_offset = doc_offset.checked_add(child.node_size())?;
        }
    }

    // At the end of block content.
    Some(doc_offset)
}

fn inline_void_render_work(node: &Node) -> Option<usize> {
    let label_len = node
        .attrs()
        .get("label")
        .and_then(serde_json::Value::as_str)
        .filter(|label| !label.is_empty())
        .map_or(node.node_type().len(), str::len);
    let trigger_len = if node.node_type() == "mention" {
        node.attrs()
            .get("mentionSuggestionChar")
            .and_then(serde_json::Value::as_str)
            .map(str::len)
            .unwrap_or(0)
    } else {
        0
    };
    node.node_type()
        .len()
        .checked_add(label_len)?
        .checked_add(trigger_len)?
        .checked_add(1)
}

/// Walk a text block node's content and convert a doc token offset to a
/// scalar offset within the block.
///
/// Mirrors `scalar_to_doc_intra_block_metered` in reverse.
fn doc_to_scalar_intra_block(
    block_node: &Node,
    doc_offset: u32,
    hard_break_node_types: &HashSet<String>,
) -> u32 {
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
            scalar_offset += inline_void_visible_scalar_len(child, hard_break_node_types);
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

fn inline_void_visible_scalar_len(node: &Node, hard_break_node_types: &HashSet<String>) -> u32 {
    let label = render::inline_atom_label(node.node_type(), node.attrs());
    render::inline_node_visible_scalar_len(
        node.node_type(),
        Some(label.as_str()),
        hard_break_node_types.contains(node.node_type()),
    )
}

#[cfg(test)]
mod retained_size_tests {
    use smallvec::smallvec;

    use super::{BlockMapping, PositionMap};
    use crate::schema::presets::tiptap_schema;

    fn block(path: smallvec::SmallVec<[u32; 8]>) -> BlockMapping {
        BlockMapping {
            doc_start: 0,
            doc_end: 0,
            scalar_start: 0,
            scalar_len: 0,
            scalar_prefix_len: 0,
            rendered_break_after: 0,
            node_path: path,
            is_void_block: false,
        }
    }

    #[test]
    fn history_snapshot_clone_charge_scales_with_summed_spilled_path_depth() {
        let schema = tiptap_schema();
        let shallow =
            PositionMap::from_blocks(vec![block(smallvec![0, 0, 0, 0, 0, 0, 0, 0]); 24], &schema);
        let deep_blocks = (9..=32)
            .map(|depth| block(smallvec::SmallVec::from_vec(vec![0; depth])))
            .collect::<Vec<_>>();
        let summed_spilled_capacity = deep_blocks
            .iter()
            .map(|block| block.node_path.capacity())
            .sum::<usize>();
        let deep = PositionMap::from_blocks(deep_blocks, &schema);

        let shallow_bytes = shallow.history_snapshot_clone_retained_bytes().unwrap();
        let deep_bytes = deep.history_snapshot_clone_retained_bytes().unwrap();
        let spilled_path_bytes = summed_spilled_capacity * std::mem::size_of::<u32>();

        assert!(deep_bytes >= shallow_bytes.saturating_add(spilled_path_bytes));
    }
}
