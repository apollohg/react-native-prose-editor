use super::*;

fn policy_with_filter(pattern: &str) -> SessionPolicy {
    SessionPolicy::from_config(&EditorSessionConfig {
        input_filter: Some(pattern.to_string()),
        ..EditorSessionConfig::local_for_test()
    })
}

#[test]
fn input_filter_regex_compiles_once_and_replays_compile_errors() {
    // Compile-once: repeated borrows return the SAME cached Regex
    // allocation, preserving exact per-character `is_match` semantics.
    let policy = policy_with_filter("^[0-9]$");
    let first = policy.input_filter_regex().unwrap().unwrap();
    assert!(first.is_match("7"));
    assert!(!first.is_match("a"));
    let second = policy.input_filter_regex().unwrap().unwrap();
    assert!(
        std::ptr::eq(first, second),
        "the pattern must compile once and be served from the cache",
    );

    // An invalid pattern caches the compile error and replays the
    // identical message (request-time CONFIG_INVALID semantics kept).
    let policy = policy_with_filter("[unclosed");
    let first = policy.input_filter_regex().unwrap().unwrap_err();
    let second = policy.input_filter_regex().unwrap().unwrap_err();
    assert_eq!(first, second, "identical replay of the compile error");

    // No pattern: nothing compiles, nothing is cached.
    let policy = SessionPolicy::from_config(&EditorSessionConfig::local_for_test());
    assert!(policy.input_filter_regex().is_none());
    assert!(policy.input_filter_regex.get().is_none());
}
