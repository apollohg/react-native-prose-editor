//! Undo/redo history for the editor.
//!
//! Stores committed transactions as `HistoryEntry` items on an undo stack.
//! Supports grouping of sequential character insertions (within a time window)
//! and always-separate groups for format, structural, and reconciliation edits.

pub(crate) mod grouping;

use std::time::Instant;

use crate::selection::Selection;
use crate::transform::{Source, Step};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Default maximum number of undo entries retained.
const DEFAULT_MAX_DEPTH: usize = 100;

// ---------------------------------------------------------------------------
// HistoryEntry
// ---------------------------------------------------------------------------

/// A single entry on the undo or redo stack.
///
/// Contains both the forward steps (for redo) and the inverse steps (for undo),
/// plus the selection state before the transaction was applied (so undo can
/// restore the cursor position).
#[derive(Debug)]
struct HistoryEntry {
    /// The steps that were applied (for redo).
    steps: Vec<Step>,
    /// The inverse steps (for undo) — applied in reverse order.
    inverse_steps: Vec<Step>,
    /// When this entry was created (or last merged into).
    timestamp: Instant,
    /// Source of the transaction that created (or last merged into) this entry.
    source: Source,
    /// The selection before this transaction was applied (restored on undo).
    selection_before: Selection,
    /// The selection after this transaction was applied (restored on redo).
    selection_after: Selection,
}

// ---------------------------------------------------------------------------
// UndoHistory
// ---------------------------------------------------------------------------

/// Manages undo/redo stacks for an editor instance.
///
/// # Usage
///
/// The `Editor` (Task 13) will compute inverse steps at commit time and pass
/// them to [`push`]. This module handles storage, grouping, and stack
/// management.
///
/// # Grouping rules
///
/// - Sequential `Source::Input` character insertions within 500 ms merge into
///   the current undo group.
/// - `Source::Format` always creates a new group.
/// - Structural edits (SplitBlock, JoinBlocks, etc.) always create a new group.
/// - `Source::Reconciliation` always creates its own group.
/// - A new edit after undo clears the redo stack.
#[derive(Debug)]
pub struct UndoHistory {
    undo_stack: Vec<HistoryEntry>,
    redo_stack: Vec<HistoryEntry>,
    max_depth: usize,
}

impl UndoHistory {
    /// Create a new history with the given maximum depth.
    pub fn new(max_depth: usize) -> Self {
        Self {
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            max_depth,
        }
    }

    /// Create a new history with the default maximum depth (100).
    pub fn with_default_depth() -> Self {
        Self::new(DEFAULT_MAX_DEPTH)
    }

    /// Record a committed transaction.
    ///
    /// May merge with the previous entry based on grouping rules (see module
    /// docs). Clears the redo stack whenever a new edit is pushed.
    ///
    /// `selection_before` is the selection state *before* this transaction was
    /// applied. `selection_after` is the selection *after* the transaction.
    pub fn push(
        &mut self,
        steps: Vec<Step>,
        inverse_steps: Vec<Step>,
        source: Source,
        selection_before: Selection,
        selection_after: Selection,
    ) {
        self.push_at(
            steps,
            inverse_steps,
            source,
            Instant::now(),
            selection_before,
            selection_after,
        );
    }

    /// Push with an explicit timestamp, for deterministic testing and Editor
    /// integration.
    pub fn push_at(
        &mut self,
        steps: Vec<Step>,
        inverse_steps: Vec<Step>,
        source: Source,
        timestamp: Instant,
        selection_before: Selection,
        selection_after: Selection,
    ) {
        // Any new edit clears the redo stack.
        self.clear_redo();

        // Try to merge with the previous entry.
        if let Some(prev) = self.undo_stack.last_mut() {
            if grouping::should_merge(
                &prev.source,
                &prev.steps,
                prev.timestamp,
                &source,
                &steps,
                timestamp,
            ) {
                prev.steps.extend(steps);
                // Inverse steps for a merged group must be prepended so that
                // undoing applies them in the correct (reverse chronological)
                // order. The most recent inverse steps go to the front.
                let mut merged_inverse = inverse_steps;
                merged_inverse.extend(prev.inverse_steps.drain(..));
                prev.inverse_steps = merged_inverse;
                prev.timestamp = timestamp;
                // Keep the original selection_before from the first entry
                // in the group, but update selection_after to the latest.
                prev.selection_after = selection_after;
                return;
            }
        }

        // Create a new entry.
        self.undo_stack.push(HistoryEntry {
            steps,
            inverse_steps,
            timestamp,
            source,
            selection_before,
            selection_after,
        });

        // Enforce max depth.
        if self.undo_stack.len() > self.max_depth {
            let excess = self.undo_stack.len() - self.max_depth;
            self.undo_stack.drain(..excess);
        }
    }

    /// Pop the most recent undo entry and return the inverse steps to apply,
    /// plus the selection that should be restored after undoing.
    ///
    /// The entry is moved to the redo stack so it can be re-applied later.
    /// Returns `None` if there is nothing to undo.
    pub fn undo(&mut self) -> Option<(Vec<Step>, Selection)> {
        let entry = self.undo_stack.pop()?;
        let inverse = entry.inverse_steps.clone();
        let selection = entry.selection_before.clone();
        self.redo_stack.push(entry);
        Some((inverse, selection))
    }

    /// Pop the most recent redo entry and return the original steps to
    /// re-apply, plus the selection that should be restored after redoing.
    ///
    /// The entry is moved back to the undo stack. Returns `None` if there is
    /// nothing to redo.
    pub fn redo(&mut self) -> Option<(Vec<Step>, Selection)> {
        let entry = self.redo_stack.pop()?;
        let steps = entry.steps.clone();
        let selection = entry.selection_after.clone();
        self.undo_stack.push(entry);
        Some((steps, selection))
    }

    /// Whether there are any entries on the undo stack.
    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    /// Whether there are any entries on the redo stack.
    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    /// Number of entries on the undo stack.
    pub fn undo_depth(&self) -> usize {
        self.undo_stack.len()
    }

    /// Number of entries on the redo stack.
    pub fn redo_depth(&self) -> usize {
        self.redo_stack.len()
    }

    /// Clear the redo stack.
    fn clear_redo(&mut self) {
        self.redo_stack.clear();
    }
}

impl Default for UndoHistory {
    fn default() -> Self {
        Self::with_default_depth()
    }
}
