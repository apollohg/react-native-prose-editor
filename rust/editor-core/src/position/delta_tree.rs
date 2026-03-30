/// Lazy delta tracking for incremental position map updates.
///
/// Instead of rewriting every `BlockMapping` after an edit, we record
/// cumulative (doc_delta, scalar_delta) adjustments keyed by block index.
/// On lookup the caller adds the accumulated delta from this tree.
///
/// Internally this is a sorted `Vec` — sufficient for the number of edits
/// between periodic compactions.
#[derive(Debug, Clone)]
pub struct DeltaTree {
    /// Sorted by block index. Each entry is
    /// `(block_index, cumulative_doc_delta, cumulative_scalar_delta)`.
    deltas: Vec<(usize, i32, i32)>,
}

impl DeltaTree {
    /// Create an empty delta tree (no pending adjustments).
    pub fn empty() -> Self {
        Self { deltas: Vec::new() }
    }

    /// Whether the tree contains any pending deltas.
    pub fn is_empty(&self) -> bool {
        self.deltas.is_empty()
    }

    /// Number of delta entries.
    pub fn len(&self) -> usize {
        self.deltas.len()
    }

    /// Record a delta that applies to all blocks with index >= `from_block`.
    ///
    /// If an entry already exists for `from_block`, the deltas are summed.
    /// Entries for blocks *after* `from_block` are also shifted.
    pub fn insert(&mut self, from_block: usize, doc_delta: i32, scalar_delta: i32) {
        // Find insertion point
        let pos = self
            .deltas
            .binary_search_by_key(&from_block, |&(idx, _, _)| idx);

        match pos {
            Ok(i) => {
                // Existing entry — accumulate
                self.deltas[i].1 += doc_delta;
                self.deltas[i].2 += scalar_delta;
                // Remove if both deltas are zero
                if self.deltas[i].1 == 0 && self.deltas[i].2 == 0 {
                    self.deltas.remove(i);
                }
            }
            Err(i) => {
                self.deltas.insert(i, (from_block, doc_delta, scalar_delta));
            }
        }
    }

    /// Look up the accumulated (doc_delta, scalar_delta) for a given block index.
    ///
    /// This sums all entries with `block_index <= target_block`. The convention
    /// is that an entry at index `k` means "all blocks from k onward are shifted
    /// by this delta", so we accumulate everything up to and including `target_block`.
    pub fn accumulated_delta(&self, target_block: usize) -> (i32, i32) {
        let mut doc_delta = 0i32;
        let mut scalar_delta = 0i32;

        for &(block_idx, dd, sd) in &self.deltas {
            if block_idx > target_block {
                break;
            }
            doc_delta += dd;
            scalar_delta += sd;
        }

        (doc_delta, scalar_delta)
    }

    /// Clear all deltas (after folding them into BlockMappings).
    pub fn clear(&mut self) {
        self.deltas.clear();
    }

    /// Iterate over raw entries (for folding into block mappings).
    pub fn iter(&self) -> impl Iterator<Item = &(usize, i32, i32)> {
        self.deltas.iter()
    }
}
