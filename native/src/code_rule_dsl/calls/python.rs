//! Python's own reading of the call graph's files.

/// The dotted name a file's path gives it from the deepest directory whose name is no Python
/// identifier (a checkout's own directory): what `import` finds with that directory on
/// `sys.path`, namespace packages included. `pkg/__init__.py` is `pkg`.
pub(crate) fn path_module(path: &str) -> Option<String> {
    let path = path
        .strip_suffix(".py")
        .or_else(|| path.strip_suffix(".pyi"))?;
    let mut segments = path.split('/').collect::<Vec<_>>();
    if segments.len() > 1 && segments.last() == Some(&"__init__") {
        segments.pop();
    }
    let valid = segments
        .iter()
        .rev()
        .take_while(|s| is_identifier(s))
        .count();
    (valid > 0).then(|| segments[segments.len() - valid..].join("."))
}

/// Whether `segment` is a Python identifier: a letter or `_`, then letters, digits or `_`.
fn is_identifier(segment: &str) -> bool {
    let mut chars = segment.chars();
    chars
        .next()
        .is_some_and(|first| first == '_' || first.is_alphabetic())
        && chars.all(|c| c == '_' || c.is_alphanumeric())
}
