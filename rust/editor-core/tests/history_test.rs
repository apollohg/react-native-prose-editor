use std::collections::HashMap;
use std::time::{Duration, Instant};

use editor_core::history::UndoHistory;
use editor_core::model::{Document, Fragment, Mark, Node};
use editor_core::schema::presets::tiptap_schema;
use editor_core::schema::Schema;
use editor_core::selection::Selection;
use editor_core::transform::{Source, Step, Transaction};

// ---------------------------------------------------------------------------
// Helper builders
// ---------------------------------------------------------------------------

fn text(s: &str) -> Node {
    Node::text(s.to_string(), vec![])
}

fn bold() -> Mark {
    Mark::new("bold".to_string(), HashMap::new())
}

fn paragraph(children: Vec<Node>) -> Node {
    Node::element(
        "paragraph".to_string(),
        HashMap::new(),
        Fragment::from(children),
    )
}

fn doc_node(children: Vec<Node>) -> Node {
    Node::element("doc".to_string(), HashMap::new(), Fragment::from(children))
}

fn insert_step(pos: u32, s: &str) -> Step {
    Step::InsertText {
        pos,
        text: s.to_string(),
        marks: vec![],
    }
}

fn delete_step(from: u32, to: u32) -> Step {
    Step::DeleteRange { from, to }
}

fn instant_plus(base: Instant, ms: u64) -> Instant {
    base + Duration::from_millis(ms)
}

/// Default selection for test history entries.
fn sel(pos: u32) -> Selection {
    Selection::cursor(pos)
}

/// Apply a single step to a document via a Transaction, computing the inverse
/// step from the pre-step document state.
///
/// Returns (new_doc, forward_step, inverse_step).
fn apply_step_with_inverse(
    doc: &Document,
    step: Step,
    source: Source,
    schema: &Schema,
) -> (Document, Step, Step) {
    let inverse = invert_step(&step, doc);
    let mut tx = Transaction::new(source);
    tx.add_step(step.clone());
    let (new_doc, _map) = tx
        .apply(doc, schema)
        .expect("step application should succeed");
    (new_doc, step, inverse)
}

/// Apply a sequence of steps (inverse or redo) to a document.
fn apply_steps(doc: &Document, steps: &[Step], schema: &Schema) -> Document {
    let mut current = doc.clone();
    for step in steps {
        let mut tx = Transaction::new(Source::History);
        tx.add_step(step.clone());
        let (new_doc, _map) = tx
            .apply(&current, schema)
            .expect("step application should succeed during undo/redo");
        current = new_doc;
    }
    current
}

/// Compute the inverse of a single step given the document state before
/// the step was applied.
fn invert_step(step: &Step, doc: &Document) -> Step {
    match step {
        Step::InsertText { pos, text, .. } => {
            let len = text.chars().count() as u32;
            Step::DeleteRange {
                from: *pos,
                to: *pos + len,
            }
        }
        Step::DeleteRange { from, to } => {
            let deleted_text = extract_text_range(doc, *from, *to);
            Step::InsertText {
                pos: *from,
                text: deleted_text,
                marks: vec![],
            }
        }
        Step::AddMark { from, to, mark } => Step::RemoveMark {
            from: *from,
            to: *to,
            mark_type: mark.mark_type().to_string(),
        },
        Step::RemoveMark {
            from,
            to,
            mark_type,
        } => Step::AddMark {
            from: *from,
            to: *to,
            mark: Mark::new(mark_type.to_string(), HashMap::new()),
        },
        Step::SplitBlock { pos, .. } => Step::JoinBlocks { pos: *pos },
        Step::JoinBlocks { pos } => Step::SplitBlock {
            pos: *pos,
            node_type: "paragraph".to_string(),
            attrs: HashMap::new(),
        },
        _ => step.clone(),
    }
}

/// Extract text content from a document range within a single parent node.
fn extract_text_range(doc: &Document, from: u32, to: u32) -> String {
    let resolved_from = doc.resolve(from).unwrap();
    let parent = resolved_from.parent(doc);
    let parent_text = parent.text_content();
    let from_offset = resolved_from.parent_offset as usize;
    let len = (to - from) as usize;

    let chars: Vec<char> = parent_text.chars().collect();
    if from_offset + len <= chars.len() {
        chars[from_offset..from_offset + len].iter().collect()
    } else {
        parent_text
    }
}

// ===========================================================================
// Basic undo/redo tests
// ===========================================================================

#[test]
fn test_undo_single_insert_restores_original() {
    let schema = tiptap_schema();
    let original = Document::new(doc_node(vec![paragraph(vec![text("Hello")])]));

    let (after_insert, fwd, inv) =
        apply_step_with_inverse(&original, insert_step(6, " World"), Source::Input, &schema);
    assert_eq!(after_insert.root().text_content(), "Hello World");

    let mut history = UndoHistory::new(100);
    history.push(vec![fwd], vec![inv], Source::Input, sel(6), sel(12));

    assert!(history.can_undo());
    assert!(!history.can_redo());

    let (undo_steps, _sel) = history.undo().unwrap();
    let restored = apply_steps(&after_insert, &undo_steps, &schema);
    assert_eq!(
        restored.root().text_content(),
        "Hello",
        "After undo, document should match original"
    );

    assert!(!history.can_undo());
    assert!(history.can_redo());
}

#[test]
fn test_redo_restores_post_transaction_state() {
    let schema = tiptap_schema();
    let original = Document::new(doc_node(vec![paragraph(vec![text("Hello")])]));

    let (after_insert, fwd, inv) =
        apply_step_with_inverse(&original, insert_step(6, " World"), Source::Input, &schema);

    let mut history = UndoHistory::new(100);
    history.push(vec![fwd], vec![inv], Source::Input, sel(6), sel(12));

    // Undo.
    let (undo_steps, _sel) = history.undo().unwrap();
    let after_undo = apply_steps(&after_insert, &undo_steps, &schema);
    assert_eq!(after_undo.root().text_content(), "Hello");

    // Redo.
    let (redo_steps, _sel) = history.redo().unwrap();
    let after_redo = apply_steps(&after_undo, &redo_steps, &schema);
    assert_eq!(
        after_redo.root().text_content(),
        "Hello World",
        "After redo, document should match post-transaction state"
    );
}

#[test]
fn test_undo_on_empty_history_returns_none() {
    let history = UndoHistory::new(100);
    assert!(!history.can_undo());
}

#[test]
fn test_undo_returns_none_when_empty() {
    let mut history = UndoHistory::new(100);
    assert!(history.undo().is_none());
}

#[test]
fn test_redo_returns_none_when_empty() {
    let mut history = UndoHistory::new(100);
    assert!(history.redo().is_none());
}

#[test]
fn test_redo_returns_none_when_no_undo_performed() {
    let mut history = UndoHistory::new(100);
    history.push(
        vec![insert_step(1, "a")],
        vec![delete_step(1, 2)],
        Source::Input,
        sel(1),
        sel(2),
    );
    assert!(!history.can_redo());
    assert!(history.redo().is_none());
}

// ===========================================================================
// Grouping tests
// ===========================================================================

#[test]
fn test_two_inserts_within_500ms_grouped_single_undo() {
    let schema = tiptap_schema();
    let original = Document::new(doc_node(vec![paragraph(vec![])]));

    // Insert "a" at pos 1.
    let (after_a, fwd1, inv1) =
        apply_step_with_inverse(&original, insert_step(1, "a"), Source::Input, &schema);
    assert_eq!(after_a.root().text_content(), "a");

    // Insert "b" at pos 2.
    let (after_ab, fwd2, inv2) =
        apply_step_with_inverse(&after_a, insert_step(2, "b"), Source::Input, &schema);
    assert_eq!(after_ab.root().text_content(), "ab");

    let mut history = UndoHistory::new(100);
    let now = Instant::now();

    history.push_at(vec![fwd1], vec![inv1], Source::Input, now, sel(1), sel(2));
    history.push_at(
        vec![fwd2],
        vec![inv2],
        Source::Input,
        instant_plus(now, 400),
        sel(2),
        sel(3),
    );

    assert_eq!(
        history.undo_depth(),
        1,
        "Two inserts within 500ms should merge into one undo entry"
    );

    // Single undo should revert both.
    let (undo_steps, _sel) = history.undo().unwrap();
    let restored = apply_steps(&after_ab, &undo_steps, &schema);
    assert_eq!(
        restored.root().text_content(),
        "",
        "Single undo should revert both grouped inserts"
    );
}

#[test]
fn test_two_inserts_600ms_apart_separate_groups() {
    let mut history = UndoHistory::new(100);
    let now = Instant::now();

    history.push_at(
        vec![insert_step(1, "a")],
        vec![delete_step(1, 2)],
        Source::Input,
        now,
        sel(1),
        sel(2),
    );
    history.push_at(
        vec![insert_step(2, "b")],
        vec![delete_step(2, 3)],
        Source::Input,
        instant_plus(now, 600),
        sel(2),
        sel(3),
    );

    assert_eq!(
        history.undo_depth(),
        2,
        "Two inserts 600ms apart should be separate undo entries"
    );
}

#[test]
fn test_insert_then_format_separate_groups() {
    let mut history = UndoHistory::new(100);
    let now = Instant::now();

    history.push_at(
        vec![insert_step(1, "a")],
        vec![delete_step(1, 2)],
        Source::Input,
        now,
        sel(1),
        sel(2),
    );
    history.push_at(
        vec![Step::AddMark {
            from: 1,
            to: 2,
            mark: bold(),
        }],
        vec![Step::RemoveMark {
            from: 1,
            to: 2,
            mark_type: "bold".to_string(),
        }],
        Source::Format,
        instant_plus(now, 100),
        sel(1),
        sel(2),
    );

    assert_eq!(
        history.undo_depth(),
        2,
        "Insert + Format should be separate groups regardless of timing"
    );
}

#[test]
fn test_insert_then_split_block_separate_groups() {
    let mut history = UndoHistory::new(100);
    let now = Instant::now();

    history.push_at(
        vec![insert_step(1, "a")],
        vec![delete_step(1, 2)],
        Source::Input,
        now,
        sel(1),
        sel(2),
    );
    history.push_at(
        vec![Step::SplitBlock {
            pos: 2,
            node_type: "paragraph".to_string(),
            attrs: HashMap::new(),
        }],
        vec![Step::JoinBlocks { pos: 2 }],
        Source::Input,
        instant_plus(now, 100),
        sel(2),
        sel(3),
    );

    assert_eq!(
        history.undo_depth(),
        2,
        "Insert + SplitBlock should be separate groups"
    );
}

#[test]
fn test_reconciliation_always_own_group() {
    let mut history = UndoHistory::new(100);
    let now = Instant::now();

    history.push_at(
        vec![insert_step(1, "a")],
        vec![delete_step(1, 2)],
        Source::Input,
        now,
        sel(1),
        sel(2),
    );
    history.push_at(
        vec![insert_step(2, "b")],
        vec![delete_step(2, 3)],
        Source::Reconciliation,
        instant_plus(now, 100),
        sel(2),
        sel(3),
    );

    assert_eq!(
        history.undo_depth(),
        2,
        "Reconciliation should always be its own group"
    );
}

// ===========================================================================
// Redo clearing tests
// ===========================================================================

#[test]
fn test_edit_after_undo_clears_redo() {
    let mut history = UndoHistory::new(100);
    let now = Instant::now();

    history.push_at(
        vec![insert_step(1, "a")],
        vec![delete_step(1, 2)],
        Source::Input,
        now,
        sel(1),
        sel(2),
    );
    history.push_at(
        vec![insert_step(2, "b")],
        vec![delete_step(2, 3)],
        Source::Input,
        instant_plus(now, 600),
        sel(2),
        sel(3),
    );

    history.undo();
    assert!(history.can_redo(), "Redo should be available after undo");

    // New edit after undo should clear redo.
    history.push(
        vec![insert_step(2, "c")],
        vec![delete_step(2, 3)],
        Source::Input,
        sel(2),
        sel(3),
    );
    assert!(
        !history.can_redo(),
        "Redo should be cleared after a new edit"
    );
}

// ===========================================================================
// Max depth tests
// ===========================================================================

#[test]
fn test_max_depth_drops_oldest_entries() {
    let max = 5;
    let mut history = UndoHistory::new(max);
    let now = Instant::now();

    for i in 0..10u32 {
        history.push_at(
            vec![insert_step(i + 1, "x")],
            vec![delete_step(i + 1, i + 2)],
            Source::Input,
            // Space 600ms apart so they don't merge.
            instant_plus(now, (i as u64) * 600),
            sel(i + 1),
            sel(i + 2),
        );
    }

    assert_eq!(
        history.undo_depth(),
        max,
        "History should be capped at max_depth ({})",
        max
    );

    // Verify we can undo exactly max_depth times.
    for _ in 0..max {
        assert!(history.undo().is_some());
    }
    assert!(history.undo().is_none());
}

// ===========================================================================
// 100-step round-trip test (Phase 1 exit criterion)
// ===========================================================================

#[test]
fn test_100_step_round_trip_undo_all_then_redo_all() {
    let schema = tiptap_schema();
    let original = Document::new(doc_node(vec![paragraph(vec![])]));

    let mut history = UndoHistory::new(200);
    let mut current = original.clone();
    let now = Instant::now();
    let alphabet: Vec<char> = "abcdefghijklmnopqrstuvwxyz".chars().collect();

    // Apply 100 single-character insertions, each 600ms apart to avoid merging.
    for i in 0..100u32 {
        let c = alphabet[(i as usize) % alphabet.len()];
        let pos = i + 1;

        let (new_doc, fwd, inv) = apply_step_with_inverse(
            &current,
            insert_step(pos, &c.to_string()),
            Source::Input,
            &schema,
        );
        history.push_at(
            vec![fwd],
            vec![inv],
            Source::Input,
            instant_plus(now, (i as u64) * 600),
            sel(pos),
            sel(pos + 1),
        );
        current = new_doc;
    }

    let final_text = current.root().text_content();
    assert_eq!(final_text.len(), 100, "Should have 100 characters inserted");
    let expected_final = final_text.clone();

    assert_eq!(history.undo_depth(), 100);

    // Undo all 100.
    for i in 0..100 {
        let (undo_steps, _sel) = history
            .undo()
            .unwrap_or_else(|| panic!("undo #{} should succeed", i + 1));
        current = apply_steps(&current, &undo_steps, &schema);
    }

    assert_eq!(
        current.root().text_content(),
        "",
        "After undoing all 100 steps, document should be empty"
    );
    assert!(!history.can_undo());
    assert!(history.can_redo());

    // Redo all 100.
    for i in 0..100 {
        let (redo_steps, _sel) = history
            .redo()
            .unwrap_or_else(|| panic!("redo #{} should succeed", i + 1));
        current = apply_steps(&current, &redo_steps, &schema);
    }

    assert_eq!(
        current.root().text_content(),
        expected_final,
        "After redoing all 100 steps, document should match the final state"
    );
    assert!(history.can_undo());
    assert!(!history.can_redo());
}

// ===========================================================================
// Delete undo/redo round-trip
// ===========================================================================

#[test]
fn test_undo_delete_restores_deleted_text() {
    let schema = tiptap_schema();
    let original = Document::new(doc_node(vec![paragraph(vec![text("Hello")])]));

    let (after_delete, fwd, inv) =
        apply_step_with_inverse(&original, delete_step(2, 5), Source::Input, &schema);
    assert_eq!(after_delete.root().text_content(), "Ho");

    let mut history = UndoHistory::new(100);
    history.push(vec![fwd], vec![inv], Source::Input, sel(2), sel(2));

    let (undo_steps, _sel) = history.undo().unwrap();
    let restored = apply_steps(&after_delete, &undo_steps, &schema);
    assert_eq!(
        restored.root().text_content(),
        "Hello",
        "Undo of delete should restore original text"
    );
}

// ===========================================================================
// Multiple undo/redo interleaving
// ===========================================================================

#[test]
fn test_multiple_undo_redo_interleaving() {
    let schema = tiptap_schema();
    let original = Document::new(doc_node(vec![paragraph(vec![])]));
    let mut current = original.clone();
    let mut history = UndoHistory::new(100);
    let now = Instant::now();

    // Insert "a".
    let (new_doc, fwd, inv) =
        apply_step_with_inverse(&current, insert_step(1, "a"), Source::Input, &schema);
    history.push_at(vec![fwd], vec![inv], Source::Input, now, sel(1), sel(2));
    current = new_doc;
    assert_eq!(current.root().text_content(), "a");

    // Insert "b".
    let (new_doc, fwd, inv) =
        apply_step_with_inverse(&current, insert_step(2, "b"), Source::Input, &schema);
    history.push_at(
        vec![fwd],
        vec![inv],
        Source::Input,
        instant_plus(now, 600),
        sel(2),
        sel(3),
    );
    current = new_doc;
    assert_eq!(current.root().text_content(), "ab");

    // Insert "c".
    let (new_doc, fwd, inv) =
        apply_step_with_inverse(&current, insert_step(3, "c"), Source::Input, &schema);
    history.push_at(
        vec![fwd],
        vec![inv],
        Source::Input,
        instant_plus(now, 1200),
        sel(3),
        sel(4),
    );
    current = new_doc;
    assert_eq!(current.root().text_content(), "abc");

    // Undo "c".
    let (undo_steps, _sel) = history.undo().unwrap();
    current = apply_steps(&current, &undo_steps, &schema);
    assert_eq!(current.root().text_content(), "ab");

    // Undo "b".
    let (undo_steps, _sel) = history.undo().unwrap();
    current = apply_steps(&current, &undo_steps, &schema);
    assert_eq!(current.root().text_content(), "a");

    // Redo "b".
    let (redo_steps, _sel) = history.redo().unwrap();
    current = apply_steps(&current, &redo_steps, &schema);
    assert_eq!(current.root().text_content(), "ab");

    // Push new edit "d" — should clear redo for "c".
    let (new_doc, fwd, inv) =
        apply_step_with_inverse(&current, insert_step(3, "d"), Source::Input, &schema);
    history.push_at(
        vec![fwd],
        vec![inv],
        Source::Input,
        instant_plus(now, 1800),
        sel(3),
        sel(4),
    );
    current = new_doc;
    assert_eq!(current.root().text_content(), "abd");

    assert!(!history.can_redo(), "Redo should be cleared after new edit");
    assert_eq!(history.undo_depth(), 3); // "a", "b", "d"
}

// ===========================================================================
// Default constructor
// ===========================================================================

#[test]
fn test_default_constructor() {
    let history = UndoHistory::default();
    assert!(!history.can_undo());
    assert!(!history.can_redo());
    assert_eq!(history.undo_depth(), 0);
    assert_eq!(history.redo_depth(), 0);
}

// ===========================================================================
// Grouped undo correctness: inverse order
// ===========================================================================

#[test]
fn test_grouped_undo_applies_inverses_in_correct_order() {
    let schema = tiptap_schema();
    let original = Document::new(doc_node(vec![paragraph(vec![text("X")])]));
    let mut current = original.clone();
    let mut history = UndoHistory::new(100);
    let now = Instant::now();

    // Insert "a" at pos 1 (before "X") -> "aX".
    let (after_a, fwd1, inv1) =
        apply_step_with_inverse(&current, insert_step(1, "a"), Source::Input, &schema);
    current = after_a;
    assert_eq!(current.root().text_content(), "aX");

    // Insert "b" at pos 2 (between "a" and "X") -> "abX".
    let (after_ab, fwd2, inv2) =
        apply_step_with_inverse(&current, insert_step(2, "b"), Source::Input, &schema);
    current = after_ab;
    assert_eq!(current.root().text_content(), "abX");

    // Both within 400ms -> should merge.
    history.push_at(vec![fwd1], vec![inv1], Source::Input, now, sel(1), sel(2));
    history.push_at(
        vec![fwd2],
        vec![inv2],
        Source::Input,
        instant_plus(now, 400),
        sel(2),
        sel(3),
    );

    assert_eq!(history.undo_depth(), 1, "Should be merged into one group");

    // Undo the merged group.
    let (undo_steps, _sel) = history.undo().unwrap();
    current = apply_steps(&current, &undo_steps, &schema);
    assert_eq!(
        current.root().text_content(),
        "X",
        "Grouped undo should restore original text"
    );
}

// ===========================================================================
// Depth reporting
// ===========================================================================

#[test]
fn test_undo_redo_depth_tracking() {
    let mut history = UndoHistory::new(100);
    let now = Instant::now();

    assert_eq!(history.undo_depth(), 0);
    assert_eq!(history.redo_depth(), 0);

    history.push_at(
        vec![insert_step(1, "a")],
        vec![delete_step(1, 2)],
        Source::Input,
        now,
        sel(1),
        sel(2),
    );
    assert_eq!(history.undo_depth(), 1);
    assert_eq!(history.redo_depth(), 0);

    history.push_at(
        vec![insert_step(2, "b")],
        vec![delete_step(2, 3)],
        Source::Input,
        instant_plus(now, 600),
        sel(2),
        sel(3),
    );
    assert_eq!(history.undo_depth(), 2);
    assert_eq!(history.redo_depth(), 0);

    history.undo();
    assert_eq!(history.undo_depth(), 1);
    assert_eq!(history.redo_depth(), 1);

    history.undo();
    assert_eq!(history.undo_depth(), 0);
    assert_eq!(history.redo_depth(), 2);

    history.redo();
    assert_eq!(history.undo_depth(), 1);
    assert_eq!(history.redo_depth(), 1);
}

// ===========================================================================
// Three inserts within window: all merge into one group
// ===========================================================================

#[test]
fn test_three_sequential_inserts_all_merge() {
    let mut history = UndoHistory::new(100);
    let now = Instant::now();

    history.push_at(
        vec![insert_step(1, "a")],
        vec![delete_step(1, 2)],
        Source::Input,
        now,
        sel(1),
        sel(2),
    );
    history.push_at(
        vec![insert_step(2, "b")],
        vec![delete_step(2, 3)],
        Source::Input,
        instant_plus(now, 200),
        sel(2),
        sel(3),
    );
    history.push_at(
        vec![insert_step(3, "c")],
        vec![delete_step(3, 4)],
        Source::Input,
        instant_plus(now, 400),
        sel(3),
        sel(4),
    );

    assert_eq!(
        history.undo_depth(),
        1,
        "Three sequential inserts within 500ms windows should all merge"
    );
}

// ===========================================================================
// Undo/redo with delete operations
// ===========================================================================

#[test]
fn test_insert_then_delete_then_undo_both() {
    let schema = tiptap_schema();
    let original = Document::new(doc_node(vec![paragraph(vec![text("Hello")])]));
    let mut current = original.clone();
    let mut history = UndoHistory::new(100);
    let now = Instant::now();

    // Insert " World" at pos 6.
    let (new_doc, fwd, inv) =
        apply_step_with_inverse(&current, insert_step(6, " World"), Source::Input, &schema);
    history.push_at(vec![fwd], vec![inv], Source::Input, now, sel(6), sel(12));
    current = new_doc;
    assert_eq!(current.root().text_content(), "Hello World");

    // Delete "World" (pos 7..12).
    let (new_doc, fwd, inv) =
        apply_step_with_inverse(&current, delete_step(7, 12), Source::Input, &schema);
    history.push_at(
        vec![fwd],
        vec![inv],
        Source::Input,
        instant_plus(now, 1000),
        sel(7),
        sel(7),
    );
    current = new_doc;
    assert_eq!(current.root().text_content(), "Hello ");

    // Undo delete.
    let (undo_steps, _sel) = history.undo().unwrap();
    current = apply_steps(&current, &undo_steps, &schema);
    assert_eq!(current.root().text_content(), "Hello World");

    // Undo insert.
    let (undo_steps, _sel) = history.undo().unwrap();
    current = apply_steps(&current, &undo_steps, &schema);
    assert_eq!(current.root().text_content(), "Hello");
}
