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
