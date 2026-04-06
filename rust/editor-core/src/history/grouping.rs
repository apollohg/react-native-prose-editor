//! Grouping rules for undo history entries.
//!
//! Determines whether a new transaction should be merged into the most recent
//! undo entry or start a new group.

use std::time::Instant;

use crate::transform::{Source, Step};

/// Maximum time gap (in milliseconds) between two sequential character
/// insertions that can be merged into the same undo group.
const MERGE_WINDOW_MS: u128 = 500;

/// Whether the given step is a "structural" edit that always creates a new
/// undo group (regardless of timing or source).
fn is_structural_step(step: &Step) -> bool {
    matches!(
        step,
        Step::SplitBlock { .. }
            | Step::JoinBlocks { .. }
            | Step::WrapInList { .. }
            | Step::UnwrapFromList { .. }
            | Step::InsertNode { .. }
            | Step::UpdateNodeAttrs { .. }
            | Step::ReplaceRange { .. }
    )
}

/// Whether the given step is a simple text insertion (the kind that can be
/// merged with adjacent insertions).
fn is_text_insert(step: &Step) -> bool {
    matches!(step, Step::InsertText { .. })
}

/// Whether the given step is a simple text deletion (the kind that can be
/// merged with adjacent deletions).
fn is_text_delete(step: &Step) -> bool {
    matches!(step, Step::DeleteRange { .. })
}

/// Determine if a new set of steps from `source` can be merged with the
/// previous history entry.
///
/// Returns `true` when the new steps should be merged (appended) into the
/// most recent undo entry instead of creating a new one.
pub(crate) fn should_merge(
    prev_source: &Source,
    prev_steps: &[Step],
    prev_timestamp: Instant,
    new_source: &Source,
    new_steps: &[Step],
    new_timestamp: Instant,
) -> bool {
    // Reconciliation always creates its own group.
    if *new_source == Source::Reconciliation || *prev_source == Source::Reconciliation {
        return false;
    }

    // Format changes always create a new group.
    if *new_source == Source::Format || *prev_source == Source::Format {
        return false;
    }

    // Both must come from user input to merge.
    if *new_source != Source::Input || *prev_source != Source::Input {
        return false;
    }

    // Any structural step in the new batch forces a new group.
    if new_steps.iter().any(is_structural_step) {
        return false;
    }

    // Any structural step in the previous batch means no merging either.
    if prev_steps.iter().any(is_structural_step) {
        return false;
    }

    // Time window check.
    let elapsed = new_timestamp.duration_since(prev_timestamp).as_millis();
    if elapsed > MERGE_WINDOW_MS {
        return false;
    }

    // Only merge compatible step types: insert+insert or delete+delete.
    let prev_all_insert = prev_steps.iter().all(is_text_insert);
    let new_all_insert = new_steps.iter().all(is_text_insert);
    let prev_all_delete = prev_steps.iter().all(is_text_delete);
    let new_all_delete = new_steps.iter().all(is_text_delete);

    (prev_all_insert && new_all_insert) || (prev_all_delete && new_all_delete)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn instant_plus(base: Instant, ms: u64) -> Instant {
        base + Duration::from_millis(ms)
    }

    fn insert_step(pos: u32, text: &str) -> Step {
        Step::InsertText {
            pos,
            text: text.to_string(),
            marks: vec![],
        }
    }

    fn delete_step(from: u32, to: u32) -> Step {
        Step::DeleteRange { from, to }
    }

    #[test]
    fn test_merge_sequential_inserts_within_window() {
        let now = Instant::now();
        assert!(should_merge(
            &Source::Input,
            &[insert_step(1, "a")],
            now,
            &Source::Input,
            &[insert_step(2, "b")],
            instant_plus(now, 400),
        ));
    }

    #[test]
    fn test_no_merge_inserts_outside_window() {
        let now = Instant::now();
        assert!(!should_merge(
            &Source::Input,
            &[insert_step(1, "a")],
            now,
            &Source::Input,
            &[insert_step(2, "b")],
            instant_plus(now, 600),
        ));
    }

    #[test]
    fn test_no_merge_insert_then_format() {
        let now = Instant::now();
        assert!(!should_merge(
            &Source::Input,
            &[insert_step(1, "a")],
            now,
            &Source::Format,
            &[insert_step(2, "b")],
            instant_plus(now, 100),
        ));
    }

    #[test]
    fn test_no_merge_insert_then_structural() {
        let now = Instant::now();
        let split = Step::SplitBlock {
            pos: 5,
            node_type: "paragraph".to_string(),
            attrs: std::collections::HashMap::new(),
        };
        assert!(!should_merge(
            &Source::Input,
            &[insert_step(1, "a")],
            now,
            &Source::Input,
            &[split],
            instant_plus(now, 100),
        ));
    }

    #[test]
    fn test_no_merge_reconciliation() {
        let now = Instant::now();
        assert!(!should_merge(
            &Source::Input,
            &[insert_step(1, "a")],
            now,
            &Source::Reconciliation,
            &[insert_step(2, "b")],
            instant_plus(now, 100),
        ));
    }

    #[test]
    fn test_no_merge_insert_with_delete() {
        let now = Instant::now();
        assert!(!should_merge(
            &Source::Input,
            &[insert_step(1, "a")],
            now,
            &Source::Input,
            &[delete_step(1, 2)],
            instant_plus(now, 100),
        ));
    }

    #[test]
    fn test_merge_sequential_deletes_within_window() {
        let now = Instant::now();
        assert!(should_merge(
            &Source::Input,
            &[delete_step(5, 6)],
            now,
            &Source::Input,
            &[delete_step(4, 5)],
            instant_plus(now, 200),
        ));
    }

    #[test]
    fn test_no_merge_at_exact_boundary() {
        // At exactly 500ms, should still merge (<=).
        let now = Instant::now();
        assert!(should_merge(
            &Source::Input,
            &[insert_step(1, "a")],
            now,
            &Source::Input,
            &[insert_step(2, "b")],
            instant_plus(now, 500),
        ));
    }

    #[test]
    fn test_no_merge_at_501ms() {
        let now = Instant::now();
        assert!(!should_merge(
            &Source::Input,
            &[insert_step(1, "a")],
            now,
            &Source::Input,
            &[insert_step(2, "b")],
            instant_plus(now, 501),
        ));
    }
}
