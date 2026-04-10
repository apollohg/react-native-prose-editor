use crate::model::Document;
use crate::transform::StepMap;

use super::build::{build_position_map, rebuild_existing_block_mapping};
use super::PositionMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateMode {
    Rebuild,
    MarksOnly,
    InlineTextOnly,
}

impl PositionMap {
    /// Incrementally update the position map after a transaction.
    ///
    /// For the simple implementation: if the edit is a single-range change
    /// that falls entirely within one block, we can update just that block
    /// and shift trailing blocks via the DeltaTree. Otherwise we fall back
    /// to a full rebuild.
    ///
    /// `step_map` is the composed mapping from the transaction.
    /// `new_doc` is the document *after* the transaction has been applied.
    pub fn update(
        &mut self,
        step_map: &StepMap,
        old_doc: &Document,
        new_doc: &Document,
        mode: UpdateMode,
    ) {
        if mode == UpdateMode::MarksOnly {
            return;
        }

        // Try incremental update for simple single-range edits.
        if mode == UpdateMode::InlineTextOnly {
            if let Some(range) = step_map.single_range() {
                if self.try_incremental_update(range, old_doc, new_doc) {
                    return;
                }
            }
        }

        // Fallback: full rebuild
        *self = build_position_map(new_doc);
    }

    /// Attempt an incremental update for a single (pos, deleted, inserted) change.
    ///
    /// Returns `true` if the incremental update succeeded, `false` if we need
    /// a full rebuild.
    fn try_incremental_update(
        &mut self,
        (pos, deleted, inserted): (u32, u32, u32),
        old_doc: &Document,
        new_doc: &Document,
    ) -> bool {
        // Find which block contains the edit position (using current deltas).
        let block_idx = match self.find_block_for_doc_pos(pos) {
            Some(idx) => idx,
            None => return false,
        };

        // Capture what we need from the old block before mutating.
        let old_doc_end = self.effective_doc_end(block_idx);
        let old_scalar_len = self.blocks[block_idx].scalar_len;

        // Check that the entire edit range falls within this one block.
        let edit_end = pos + deleted;
        if edit_end > old_doc_end {
            // Edit spans multiple blocks — fall back to full rebuild.
            return false;
        }

        // Compute the doc delta.
        let doc_delta = inserted as i32 - deleted as i32;
        let old_block = self.blocks[block_idx].clone();

        // Structural edits like split/join shift adjacent block paths. Require
        // the neighboring blocks to remain identical at the same paths before
        // we trust an inline-only update.
        if block_idx > 0 {
            let previous = &self.blocks[block_idx - 1];
            let old_previous = old_doc.node_at(&previous.node_path);
            let new_previous = new_doc.node_at(&previous.node_path);
            if old_previous != new_previous {
                return false;
            }
        }
        if block_idx + 1 < self.blocks.len() {
            let next = &self.blocks[block_idx + 1];
            let old_next = old_doc.node_at(&next.node_path);
            let new_next = new_doc.node_at(&next.node_path);
            if old_next != new_next {
                return false;
            }
        }

        let new_node = match new_doc.node_at(&old_block.node_path) {
            Some(node) => node,
            None => return false,
        };
        let rebuilt_block = match rebuild_existing_block_mapping(new_node, &old_block) {
            Some(block) => block,
            None => return false,
        };
        let rebuilt_doc_delta = rebuilt_block.doc_end as i32 - old_block.doc_end as i32;
        if rebuilt_doc_delta != doc_delta {
            return false;
        }

        let scalar_delta = rebuilt_block.scalar_len as i32 - old_scalar_len as i32;

        // Update the modified block in-place.
        self.blocks[block_idx] = rebuilt_block;

        // Record delta for trailing blocks.
        if block_idx + 1 < self.blocks.len() {
            self.prefix_deltas
                .insert(block_idx + 1, doc_delta, scalar_delta);
        }

        true
    }

    /// Fold all pending deltas from the `DeltaTree` into the `BlockMapping`
    /// values, then clear the tree.
    ///
    /// Call this periodically (e.g. every N transactions) to keep lookups fast.
    pub fn compact(&mut self) {
        if self.prefix_deltas.is_empty() {
            return;
        }

        for i in 0..self.blocks.len() {
            let (dd, sd) = self.prefix_deltas.accumulated_delta(i);
            if dd != 0 || sd != 0 {
                self.blocks[i].doc_start = (self.blocks[i].doc_start as i64 + dd as i64) as u32;
                self.blocks[i].doc_end = (self.blocks[i].doc_end as i64 + dd as i64) as u32;
                self.blocks[i].scalar_start =
                    (self.blocks[i].scalar_start as i64 + sd as i64) as u32;
            }
        }

        self.prefix_deltas.clear();
    }
}

/// Extension trait so we can ask StepMap for a single-range change.
pub(crate) trait StepMapExt {
    fn single_range(&self) -> Option<(u32, u32, u32)>;
}

impl StepMapExt for StepMap {
    fn single_range(&self) -> Option<(u32, u32, u32)> {
        let ranges = self.ranges();
        if ranges.len() == 1 {
            Some(ranges[0])
        } else {
            None
        }
    }
}
