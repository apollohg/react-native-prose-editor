#[path = "../benches/support/benchmark_filter.rs"]
mod benchmark_filter;

use std::cell::Cell;
use std::sync::OnceLock;

#[test]
fn misses_are_lazy_while_name_and_group_matches_execute() {
    for (filter, name, group, expected_calls) in [
        (Some("other"), "case", "group", 0),
        (Some("case"), "case", "group", 1),
        (Some("group"), "case", "group", 1),
        (None, "case", "group", 1),
    ] {
        let calls = Cell::new(0);
        let result = benchmark_filter::run_if_selected(filter, name, group, || {
            calls.set(calls.get() + 1);
            "constructed"
        });

        assert_eq!(calls.get(), expected_calls);
        assert_eq!(result.is_some() as usize, expected_calls);
    }
}

#[test]
fn filtered_out_case_does_not_initialize_its_case_fixture() {
    let fixture_calls = Cell::new(0);
    let fixture = OnceLock::new();

    let result = benchmark_filter::run_if_selected(
        Some("selected-case"),
        "filtered-case",
        "filtered-group",
        || {
            fixture.get_or_init(|| {
                fixture_calls.set(fixture_calls.get() + 1);
                "constructed"
            })
        },
    );

    assert!(result.is_none());
    assert_eq!(fixture_calls.get(), 0);
    assert!(fixture.get().is_none());
}

#[test]
fn yrs_editing_semantics_cases_are_declared_exactly_once() {
    let source = include_str!("../benches/perf_suite.rs");
    // Task 16C (user directive 2026-07-20): the legacy runtime and its
    // reference benchmark cases/fixtures were deleted; only the Yrs cases
    // remain, verified after timing against v2-native expected derivations.
    let expected = [
        "yrs.edit.insert_char.article.1x",
        "yrs.edit.typing_burst.article.1x",
        "yrs.state.selection_light.article.1x",
        "yrs.command.toggle_mark.article.1x",
        "yrs.command.wrap_list.article.1x",
        "yrs.history.undo.article.1x",
        "yrs.history.redo.article.1x",
        "yrs.edit.insert_char.article.2x",
        "yrs.state.selection_light.article.2x",
        "yrs.command.wrap_list.article.2x",
    ];

    for name in expected {
        assert_eq!(
            source.matches(&format!("\"{name}\"")).count(),
            1,
            "benchmark case {name} must be declared exactly once",
        );
    }
    assert_eq!(
        source.matches("\"yrs-editing\"").count(),
        expected.len(),
        "yrs-editing group must contain exactly the approved cases",
    );
    assert_eq!(
        source
            .matches("verified_bench_case(\n            \"")
            .count(),
        expected.len(),
        "every yrs-editing case must use untimed semantic verification",
    );
    assert_eq!(source.matches("assert_legacy_editing_output(").count(), 0);
    assert_eq!(source.matches("assert_legacy_selection_output(").count(), 0);
    assert_eq!(source.matches("assert_yrs_editing_output(").count(), 11);
    let verified_runner = source
        .split_once("fn verified_bench_case")
        .expect("verified benchmark runner must exist")
        .1
        .split_once("fn build_result")
        .expect("verified runner must precede result construction")
        .0;
    let elapsed = verified_runner
        .find("let elapsed = started_at.elapsed();")
        .expect("elapsed time must be captured explicitly");
    let verification = verified_runner[elapsed..]
        .find("verify(&state, &output);")
        .expect("semantic verification must follow timing")
        + elapsed;
    let sample_push = verified_runner[verification..]
        .find("samples.push(elapsed);")
        .expect("captured time must be recorded after verification")
        + verification;
    assert!(elapsed < verification && verification < sample_push);
    assert!(
        source.contains("\"editingTypingBurst\": EDITING_TYPING_BURST"),
        "JSON evidence must identify the exact editing burst independently",
    );
    assert!(
        source.contains("\"typingBurst\": profile.typing_burst"),
        "the phase-1 profile field must remain backward compatible",
    );
}

#[test]
fn yrs_editing_semantics_verifiers_require_exact_expected_documents() {
    let source = include_str!("../benches/perf_suite.rs");

    assert!(
        source.contains("fn build_pure_bold_document("),
        "the selected bold range must have an independent pure expected document",
    );
    assert!(
        source.contains("fn build_yrs_expected_document("),
        "pure semantic fixtures must be canonicalized by the Yrs engine outside timing",
    );
    assert!(
        source.contains("assert_eq!(&actual_document, expected_document)"),
        "Yrs verification must compare its raw canonical JSON directly",
    );
    assert!(
        !source.contains("build_expected_canonical_document(&actual_document)"),
        "Yrs output must not pass through the lossy legacy parser",
    );
    let pure_helpers = source
        .split_once("fn build_pure_insert_document")
        .expect("pure insert fixture helper must exist")
        .1
        .split_once("fn build_yrs_expected_document")
        .expect("pure helpers must precede Yrs canonicalization")
        .0;
    assert!(
        !pure_helpers.contains("insert_text_scalar")
            && !pure_helpers.contains("toggle_mark")
            && !pure_helpers.contains("wrap_in_list"),
        "expected mutations must not call any command API measured by the benchmark",
    );
    assert!(
        !source.contains("text_content().contains"),
        "global text existence is not an exact edit-location assertion",
    );
    assert!(
        !source.contains("fn json_contains_node_type"),
        "global node-type existence is not an exact structural assertion",
    );
}

#[test]
fn yrs_editing_semantics_verifies_returned_outputs_after_timing() {
    let source = include_str!("../benches/perf_suite.rs");
    let verified_runner = source
        .split_once("fn verified_bench_case")
        .expect("verified benchmark runner must exist")
        .1
        .split_once("fn build_result")
        .expect("verified runner must precede result construction")
        .0;

    assert!(verified_runner.contains("Verify: FnMut(&S, &Output)"));
    assert!(verified_runner.contains("let output = black_box(run(&mut state));"));
    let elapsed = verified_runner
        .find("let elapsed = started_at.elapsed();")
        .expect("elapsed time must be captured before verification");
    let verification = verified_runner[elapsed..]
        .find("verify(&state, &output);")
        .expect("state and returned output must be verified after timing")
        + elapsed;
    assert!(elapsed < verification);
    assert!(!source.contains("fn assert_legacy_editing_output("));
    assert!(source.contains("fn assert_yrs_editing_output("));
    assert!(source.contains("resolved_anchor.document"));
    assert!(source.contains("resolved_anchor.scalar"));
    assert!(source.contains("resolved_anchor.utf16"));
    assert!(source.contains("resolved_head.document"));
    assert!(source.contains("resolved_head.scalar"));
    assert!(source.contains("resolved_head.utf16"));
}
