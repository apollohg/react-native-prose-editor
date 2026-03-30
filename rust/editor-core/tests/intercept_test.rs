use std::collections::HashMap;

use editor_core::intercept::{
    InputFilter, InterceptorExt, InterceptorPipeline, MaxLength, ReadOnly,
};
use editor_core::model::{Document, Fragment, Node};
use editor_core::transform::{Source, Step, Transaction};

// ---------------------------------------------------------------------------
// Helper builders
// ---------------------------------------------------------------------------

fn text(s: &str) -> Node {
    Node::text(s.to_string(), vec![])
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

/// Build a Document with the given children inside a doc > paragraph structure.
fn make_doc(content: &str) -> Document {
    Document::new(doc_node(vec![paragraph(vec![text(content)])]))
}

// ===========================================================================
// MaxLength tests
// ===========================================================================

#[test]
fn max_length_insert_under_limit_passes() {
    let doc = make_doc("Hi");
    let interceptor = MaxLength::new(10);

    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::InsertText {
        pos: 2,
        text: "!".to_string(),
        marks: vec![],
    });

    let result = interceptor.check(tx, &doc);
    assert!(
        result.is_ok(),
        "inserting 1 char into 2-char doc with max=10 should pass"
    );
}

#[test]
fn max_length_insert_exactly_at_limit_passes() {
    let doc = make_doc("Hi"); // 2 chars
    let interceptor = MaxLength::new(5);

    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::InsertText {
        pos: 2,
        text: "abc".to_string(), // 2 + 3 = 5, exactly at limit
        marks: vec![],
    });

    let result = interceptor.check(tx, &doc);
    assert!(
        result.is_ok(),
        "inserting to exactly the max length should pass"
    );
}

#[test]
fn max_length_insert_exceeding_limit_aborts() {
    let doc = make_doc("Hello"); // 5 chars
    let interceptor = MaxLength::new(6);

    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::InsertText {
        pos: 2,
        text: "XY".to_string(), // 5 + 2 = 7 > 6
        marks: vec![],
    });

    let result = interceptor.check(tx, &doc);
    assert!(
        result.is_err(),
        "inserting 2 chars into 5-char doc with max=6 should abort"
    );
    let err = result.unwrap_err();
    assert!(
        err.message.contains("max length"),
        "error message should mention max length, got: {}",
        err.message
    );
}

#[test]
fn max_length_delete_always_passes() {
    let doc = make_doc("Hello world"); // 11 chars, over limit
    let interceptor = MaxLength::new(5); // limit is 5, doc already exceeds

    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::DeleteRange { from: 2, to: 6 });

    let result = interceptor.check(tx, &doc);
    assert!(
        result.is_ok(),
        "delete should always pass regardless of max length"
    );
}

#[test]
fn max_length_ignores_format_source() {
    let doc = make_doc("Hello"); // 5 chars
    let interceptor = MaxLength::new(5); // already at limit

    // Format source should be ignored by MaxLength
    let mut tx = Transaction::new(Source::Format);
    tx.add_step(Step::InsertText {
        pos: 2,
        text: "extra".to_string(),
        marks: vec![],
    });

    let result = interceptor.check(tx, &doc);
    assert!(
        result.is_ok(),
        "MaxLength should ignore Format source transactions"
    );
}

#[test]
fn max_length_ignores_history_source() {
    let doc = make_doc("Hello"); // 5 chars
    let interceptor = MaxLength::new(5);

    let mut tx = Transaction::new(Source::History);
    tx.add_step(Step::InsertText {
        pos: 2,
        text: "extra".to_string(),
        marks: vec![],
    });

    let result = interceptor.check(tx, &doc);
    assert!(
        result.is_ok(),
        "MaxLength should ignore History source transactions"
    );
}

#[test]
fn max_length_multi_step_tx_second_exceeds_rejects_entire_tx() {
    let doc = make_doc("Hi"); // 2 chars
    let interceptor = MaxLength::new(5);

    let mut tx = Transaction::new(Source::Input);
    // First step: insert "ab" → 4 chars (under limit)
    tx.add_step(Step::InsertText {
        pos: 2,
        text: "ab".to_string(),
        marks: vec![],
    });
    // Second step: insert "cd" → 6 chars (over limit of 5)
    tx.add_step(Step::InsertText {
        pos: 4,
        text: "cd".to_string(),
        marks: vec![],
    });

    let result = interceptor.check(tx, &doc);
    assert!(
        result.is_err(),
        "entire transaction should be rejected when cumulative inserts exceed max length"
    );
}

#[test]
fn max_length_replace_range_accounts_for_removed_text() {
    let doc = make_doc("Hello"); // 5 chars
    let interceptor = MaxLength::new(6);

    let mut tx = Transaction::new(Source::Input);
    // ReplaceRange from 2..4 removes "ll" (2 chars), replaces with content
    // We need a ReplaceRange that adds content. Let's insert a text node.
    tx.add_step(Step::ReplaceRange {
        from: 2,
        to: 4,
        content: Fragment::from(vec![text("xyz")]), // removes 2, adds 3 → net +1 → 6 total
    });

    let result = interceptor.check(tx, &doc);
    assert!(
        result.is_ok(),
        "ReplaceRange that results in exactly max length should pass"
    );
}

#[test]
fn max_length_replace_range_exceeding_limit_aborts() {
    let doc = make_doc("Hello"); // 5 chars
    let interceptor = MaxLength::new(6);

    let mut tx = Transaction::new(Source::Input);
    // Replace 1 char with 3 → net +2 → 7 > 6
    tx.add_step(Step::ReplaceRange {
        from: 2,
        to: 3,
        content: Fragment::from(vec![text("xyz")]),
    });

    let result = interceptor.check(tx, &doc);
    assert!(
        result.is_err(),
        "ReplaceRange exceeding max length should abort"
    );
}

#[test]
fn max_length_applies_to_paste_source() {
    let doc = make_doc("Hi"); // 2 chars
    let interceptor = MaxLength::new(3);

    let mut tx = Transaction::new(Source::Paste);
    tx.add_step(Step::InsertText {
        pos: 2,
        text: "XY".to_string(), // 2 + 2 = 4 > 3
        marks: vec![],
    });

    let result = interceptor.check(tx, &doc);
    assert!(result.is_err(), "MaxLength should apply to Paste source");
}

#[test]
fn max_length_applies_to_api_source() {
    let doc = make_doc("Hi"); // 2 chars
    let interceptor = MaxLength::new(3);

    let mut tx = Transaction::new(Source::Api);
    tx.add_step(Step::InsertText {
        pos: 2,
        text: "XY".to_string(),
        marks: vec![],
    });

    let result = interceptor.check(tx, &doc);
    assert!(result.is_err(), "MaxLength should apply to Api source");
}

#[test]
fn max_length_applies_to_reconciliation_source() {
    let doc = make_doc("Hi"); // 2 chars
    let interceptor = MaxLength::new(3);

    let mut tx = Transaction::new(Source::Reconciliation);
    tx.add_step(Step::InsertText {
        pos: 2,
        text: "XY".to_string(),
        marks: vec![],
    });

    let result = interceptor.check(tx, &doc);
    assert!(
        result.is_err(),
        "MaxLength should apply to Reconciliation source"
    );
}

// ===========================================================================
// ReadOnly tests
// ===========================================================================

#[test]
fn read_only_locked_input_aborts() {
    let doc = make_doc("Hello");
    let interceptor = ReadOnly::new(true);

    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::InsertText {
        pos: 2,
        text: "X".to_string(),
        marks: vec![],
    });

    let result = interceptor.check(tx, &doc);
    assert!(
        result.is_err(),
        "locked ReadOnly should reject Input source"
    );
    let err = result.unwrap_err();
    assert!(
        err.message.contains("read-only") || err.message.contains("read only"),
        "error message should mention read-only, got: {}",
        err.message
    );
}

#[test]
fn read_only_locked_format_aborts() {
    let doc = make_doc("Hello");
    let interceptor = ReadOnly::new(true);

    let mut tx = Transaction::new(Source::Format);
    tx.add_step(Step::AddMark {
        from: 1,
        to: 3,
        mark: editor_core::model::Mark::new("bold".to_string(), HashMap::new()),
    });

    let result = interceptor.check(tx, &doc);
    assert!(
        result.is_err(),
        "locked ReadOnly should reject Format source"
    );
}

#[test]
fn read_only_locked_paste_aborts() {
    let doc = make_doc("Hello");
    let interceptor = ReadOnly::new(true);

    let mut tx = Transaction::new(Source::Paste);
    tx.add_step(Step::InsertText {
        pos: 2,
        text: "X".to_string(),
        marks: vec![],
    });

    let result = interceptor.check(tx, &doc);
    assert!(
        result.is_err(),
        "locked ReadOnly should reject Paste source"
    );
}

#[test]
fn read_only_locked_history_aborts() {
    let doc = make_doc("Hello");
    let interceptor = ReadOnly::new(true);

    let mut tx = Transaction::new(Source::History);
    tx.add_step(Step::InsertText {
        pos: 2,
        text: "X".to_string(),
        marks: vec![],
    });

    let result = interceptor.check(tx, &doc);
    assert!(
        result.is_err(),
        "locked ReadOnly should reject History source"
    );
}

#[test]
fn read_only_locked_reconciliation_aborts() {
    let doc = make_doc("Hello");
    let interceptor = ReadOnly::new(true);

    let mut tx = Transaction::new(Source::Reconciliation);
    tx.add_step(Step::InsertText {
        pos: 2,
        text: "X".to_string(),
        marks: vec![],
    });

    let result = interceptor.check(tx, &doc);
    assert!(
        result.is_err(),
        "locked ReadOnly should reject Reconciliation source"
    );
}

#[test]
fn read_only_locked_api_passes() {
    let doc = make_doc("Hello");
    let interceptor = ReadOnly::new(true);

    let mut tx = Transaction::new(Source::Api);
    tx.add_step(Step::InsertText {
        pos: 2,
        text: "X".to_string(),
        marks: vec![],
    });

    let result = interceptor.check(tx, &doc);
    assert!(result.is_ok(), "locked ReadOnly should allow Api source");
}

#[test]
fn read_only_unlocked_all_pass() {
    let doc = make_doc("Hello");
    let interceptor = ReadOnly::new(false);

    let sources = [
        Source::Input,
        Source::Format,
        Source::Paste,
        Source::History,
        Source::Api,
        Source::Reconciliation,
    ];

    for source in sources {
        let mut tx = Transaction::new(source.clone());
        tx.add_step(Step::InsertText {
            pos: 2,
            text: "X".to_string(),
            marks: vec![],
        });

        let result = interceptor.check(tx, &doc);
        assert!(
            result.is_ok(),
            "unlocked ReadOnly should allow {:?} source",
            source
        );
    }
}

// ===========================================================================
// InputFilter tests
// ===========================================================================

#[test]
fn input_filter_keeps_matching_chars() {
    let doc = make_doc("");
    let interceptor = InputFilter::new(r"[0-9]").expect("valid regex");

    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::InsertText {
        pos: 1,
        text: "abc123def".to_string(),
        marks: vec![],
    });

    let result = interceptor.check(tx, &doc);
    assert!(result.is_ok(), "filter should pass with matching chars");
    let tx = result.unwrap();
    assert_eq!(tx.steps.len(), 1, "should still have 1 step");

    match &tx.steps[0] {
        Step::InsertText { text, .. } => {
            assert_eq!(
                text, "123",
                "filter should keep only digits from 'abc123def'"
            );
        }
        other => panic!("expected InsertText after filter, got: {:?}", other),
    }
}

#[test]
fn input_filter_removes_all_chars_drops_step() {
    let doc = make_doc("");
    let interceptor = InputFilter::new(r"[0-9]").expect("valid regex");

    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::InsertText {
        pos: 1,
        text: "abcdef".to_string(), // no digits
        marks: vec![],
    });

    let result = interceptor.check(tx, &doc);
    assert!(
        result.is_ok(),
        "filter should pass (step dropped, not error)"
    );
    let tx = result.unwrap();
    assert_eq!(
        tx.steps.len(),
        0,
        "step should be dropped when all chars are filtered out"
    );
}

#[test]
fn input_filter_no_effect_on_non_input_source() {
    let doc = make_doc("");
    let interceptor = InputFilter::new(r"[0-9]").expect("valid regex");

    // Api source should not be filtered
    let mut tx = Transaction::new(Source::Api);
    tx.add_step(Step::InsertText {
        pos: 1,
        text: "abcdef".to_string(),
        marks: vec![],
    });

    let result = interceptor.check(tx, &doc);
    assert!(result.is_ok(), "non-Input source should pass through");
    let tx = result.unwrap();
    assert_eq!(tx.steps.len(), 1, "step should not be dropped");

    match &tx.steps[0] {
        Step::InsertText { text, .. } => {
            assert_eq!(
                text, "abcdef",
                "text should be unchanged for non-Input source"
            );
        }
        other => panic!("expected InsertText, got: {:?}", other),
    }
}

#[test]
fn input_filter_applies_to_paste_source() {
    let doc = make_doc("");
    let interceptor = InputFilter::new(r"[a-z]").expect("valid regex");

    let mut tx = Transaction::new(Source::Paste);
    tx.add_step(Step::InsertText {
        pos: 1,
        text: "Hello123World".to_string(),
        marks: vec![],
    });

    let result = interceptor.check(tx, &doc);
    assert!(result.is_ok());
    let tx = result.unwrap();

    match &tx.steps[0] {
        Step::InsertText { text, .. } => {
            assert_eq!(
                text, "elloorld",
                "filter should keep only lowercase letters from Paste source"
            );
        }
        other => panic!("expected InsertText, got: {:?}", other),
    }
}

#[test]
fn input_filter_does_not_affect_delete_steps() {
    let doc = make_doc("Hello");
    let interceptor = InputFilter::new(r"[0-9]").expect("valid regex");

    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::DeleteRange { from: 2, to: 4 });

    let result = interceptor.check(tx, &doc);
    assert!(result.is_ok());
    let tx = result.unwrap();
    assert_eq!(tx.steps.len(), 1, "DeleteRange should pass through filter");
}

#[test]
fn input_filter_multi_step_filters_only_insert_text() {
    let doc = make_doc("Hello");
    let interceptor = InputFilter::new(r"[0-9]").expect("valid regex");

    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::DeleteRange { from: 2, to: 4 });
    tx.add_step(Step::InsertText {
        pos: 2,
        text: "abc123".to_string(),
        marks: vec![],
    });
    tx.add_step(Step::InsertText {
        pos: 5,
        text: "nodigits".to_string(),
        marks: vec![],
    });

    let result = interceptor.check(tx, &doc);
    assert!(result.is_ok());
    let tx = result.unwrap();
    // DeleteRange kept, "abc123" → "123" kept, "nodigits" → all filtered → dropped
    assert_eq!(
        tx.steps.len(),
        2,
        "should have DeleteRange + filtered InsertText, third step dropped"
    );

    match &tx.steps[1] {
        Step::InsertText { text, .. } => {
            assert_eq!(text, "123", "second step should be filtered to '123'");
        }
        other => panic!("expected InsertText, got: {:?}", other),
    }
}

// ===========================================================================
// Pipeline tests
// ===========================================================================

#[test]
fn pipeline_empty_passes_through() {
    let doc = make_doc("Hello");
    let pipeline = InterceptorPipeline::new();

    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::InsertText {
        pos: 2,
        text: "X".to_string(),
        marks: vec![],
    });

    let result = pipeline.run(tx, &doc);
    assert!(result.is_ok(), "empty pipeline should pass through");
    let tx = result.unwrap();
    assert_eq!(tx.steps.len(), 1, "transaction should be unmodified");
}

#[test]
fn pipeline_multiple_interceptors_run_in_order() {
    let doc = make_doc("");
    let mut pipeline = InterceptorPipeline::new();

    // First: filter to digits only
    pipeline.add(Box::new(InputFilter::new(r"[0-9]").expect("valid regex")));
    // Second: max length of 3
    pipeline.add(Box::new(MaxLength::new(3)));

    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::InsertText {
        pos: 1,
        text: "abc12345def".to_string(),
        marks: vec![],
    });

    // After InputFilter: "12345" (5 chars)
    // After MaxLength check: 0 (empty doc) + 5 > 3 → should abort
    let result = pipeline.run(tx, &doc);
    assert!(
        result.is_err(),
        "pipeline should abort when MaxLength is exceeded after filtering"
    );
}

#[test]
fn pipeline_first_interceptor_modifies_tx_for_second() {
    let doc = make_doc("");
    let mut pipeline = InterceptorPipeline::new();

    // First: filter to digits only
    pipeline.add(Box::new(InputFilter::new(r"[0-9]").expect("valid regex")));
    // Second: max length of 5
    pipeline.add(Box::new(MaxLength::new(5)));

    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::InsertText {
        pos: 1,
        text: "abc123def".to_string(), // filtered → "123" (3 chars)
        marks: vec![],
    });

    // After InputFilter: "123" (3 chars)
    // After MaxLength: 0 + 3 = 3 ≤ 5 → passes
    let result = pipeline.run(tx, &doc);
    assert!(
        result.is_ok(),
        "pipeline should pass when filtered text is under max length"
    );
    let tx = result.unwrap();
    match &tx.steps[0] {
        Step::InsertText { text, .. } => {
            assert_eq!(text, "123", "text should be filtered to '123'");
        }
        other => panic!("expected InsertText, got: {:?}", other),
    }
}

#[test]
fn pipeline_err_from_any_interceptor_aborts() {
    let doc = make_doc("Hello");
    let mut pipeline = InterceptorPipeline::new();

    // ReadOnly locked — should abort before MaxLength even runs
    pipeline.add(Box::new(ReadOnly::new(true)));
    pipeline.add(Box::new(MaxLength::new(100)));

    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::InsertText {
        pos: 2,
        text: "X".to_string(),
        marks: vec![],
    });

    let result = pipeline.run(tx, &doc);
    assert!(
        result.is_err(),
        "pipeline should abort when any interceptor returns Err"
    );
}

#[test]
fn pipeline_read_only_allows_api_while_max_length_still_checks() {
    let doc = make_doc("Hello"); // 5 chars
    let mut pipeline = InterceptorPipeline::new();

    pipeline.add(Box::new(ReadOnly::new(true)));
    pipeline.add(Box::new(MaxLength::new(6)));

    // Api source passes ReadOnly, but exceeds MaxLength
    let mut tx = Transaction::new(Source::Api);
    tx.add_step(Step::InsertText {
        pos: 2,
        text: "XY".to_string(), // 5 + 2 = 7 > 6
        marks: vec![],
    });

    let result = pipeline.run(tx, &doc);
    assert!(
        result.is_err(),
        "Api passes ReadOnly but should still be checked by MaxLength"
    );
}

#[test]
fn pipeline_order_matters_filter_then_length() {
    let doc = make_doc(""); // 0 chars
    let mut pipeline = InterceptorPipeline::new();

    // Order: MaxLength first, then InputFilter
    pipeline.add(Box::new(MaxLength::new(3)));
    pipeline.add(Box::new(InputFilter::new(r"[0-9]").expect("valid regex")));

    let mut tx = Transaction::new(Source::Input);
    tx.add_step(Step::InsertText {
        pos: 1,
        text: "abc12345def".to_string(), // 11 chars total before filter
        marks: vec![],
    });

    // MaxLength runs first: 0 + 11 > 3 → aborts (filter never runs)
    let result = pipeline.run(tx, &doc);
    assert!(
        result.is_err(),
        "MaxLength should abort before InputFilter runs when order is reversed"
    );
}
