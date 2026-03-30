/// A position mapping that tracks how document positions shift after steps.
///
/// Each entry represents a single insertion or deletion:
/// - `(pos, 0, inserted)` — insertion of `inserted` chars at `pos`
/// - `(pos, deleted, 0)` — deletion of `deleted` chars starting at `pos`
/// - `(pos, deleted, inserted)` — replacement
#[derive(Debug, Clone)]
pub struct StepMap {
    /// Each entry is `(pos, deleted_len, inserted_len)`.
    ranges: Vec<(u32, u32, u32)>,
}

impl StepMap {
    /// Create an empty map (identity mapping).
    pub fn empty() -> Self {
        Self { ranges: Vec::new() }
    }

    /// Create a map from a single insertion.
    pub fn from_insert(pos: u32, len: u32) -> Self {
        Self {
            ranges: vec![(pos, 0, len)],
        }
    }

    /// Create a map from a single deletion.
    pub fn from_delete(pos: u32, len: u32) -> Self {
        Self {
            ranges: vec![(pos, len, 0)],
        }
    }

    /// Create a map from a single replacement (delete + insert at same position).
    pub fn from_replace(pos: u32, deleted: u32, inserted: u32) -> Self {
        Self {
            ranges: vec![(pos, deleted, inserted)],
        }
    }

    /// Map an old position through this step map to get the new position.
    ///
    /// Positions before the change are unaffected. Positions inside a deleted
    /// range collapse to the deletion point. Positions after the change are
    /// shifted by the net delta.
    pub fn map_pos(&self, mut pos: u32) -> u32 {
        for &(range_pos, deleted, inserted) in &self.ranges {
            if pos <= range_pos {
                // Position is before (or exactly at the start of) this range.
                // No shift needed from this entry.
                // Actually — if pos < range_pos, no shift. If pos == range_pos
                // and there's an insertion, the convention is to push the
                // position forward (cursor stays after inserted text).
                if pos == range_pos && inserted > 0 {
                    pos += inserted;
                }
            } else if pos <= range_pos + deleted {
                // Position falls inside the deleted range → collapse to
                // the deletion point, then shift by inserted length.
                pos = range_pos + inserted;
            } else {
                // Position is after the deleted range → shift by net delta.
                let delta = inserted as i64 - deleted as i64;
                pos = (pos as i64 + delta) as u32;
            }
        }
        pos
    }

    /// Access the raw range entries.
    pub fn ranges(&self) -> &[(u32, u32, u32)] {
        &self.ranges
    }

    /// Compose two step maps into one that represents applying `self` first,
    /// then `other`.
    pub fn compose(&self, other: &StepMap) -> StepMap {
        let mut combined = self.ranges.clone();
        combined.extend_from_slice(&other.ranges);
        StepMap { ranges: combined }
    }
}
