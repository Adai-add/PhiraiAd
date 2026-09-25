use std::collections::BTreeSet;
/// Explicit AI requests always run; automatic mode adds the remaining catalog.
/// Stable ordering means resuming the app never randomizes the model work queue.
pub fn order(paths: &BTreeSet<String>, priority: &BTreeSet<String>, automatic: bool) -> Vec<String> {
    let mut result = paths
        .union(priority)
        .filter(|p| automatic || priority.contains(*p))
        .cloned()
        .collect::<Vec<_>>();
    result.sort_by_key(|p| !priority.contains(p));
    result
}
pub fn allowed(path: &str, priority: &BTreeSet<String>, automatic: bool) -> bool {
    automatic || priority.contains(path)
}
