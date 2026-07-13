pub fn run_if_selected<T>(
    filter: Option<&str>,
    name: &str,
    group: &str,
    run: impl FnOnce() -> T,
) -> Option<T> {
    let selected = filter.is_none_or(|filter| name.contains(filter) || group.contains(filter));
    selected.then(run)
}
