//! Node's POSIX `path` functions, as enhanced-resolve and eslint-plugin-n call them on absolute
//! paths: `normalize`, `join`, `resolve`, `dirname`, `basename`, `extname` and `relative`.

/// `path.posix.normalize`'s segment walk: `.` dropped, `..` removing the segment before it, or
/// kept where nothing is left to remove and `above_root` allows it.
fn normalize_segments(path: &str, above_root: bool) -> Vec<&str> {
    let mut segments: Vec<&str> = Vec::new();
    for segment in path.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                if segments.last().is_some_and(|last| *last != "..") {
                    segments.pop();
                } else if above_root {
                    segments.push("..");
                }
            }
            segment => segments.push(segment),
        }
    }
    segments
}

/// `path.posix.normalize`.
pub(crate) fn normalize(path: &str) -> String {
    if path.is_empty() {
        return ".".to_owned();
    }
    let absolute = path.starts_with('/');
    let trailing = path.ends_with('/');
    let mut normalized = normalize_segments(path, !absolute).join("/");
    if normalized.is_empty() {
        if absolute {
            return "/".to_owned();
        }
        return if trailing { "./" } else { "." }.to_owned();
    }
    if trailing {
        normalized.push('/');
    }
    if absolute {
        format!("/{normalized}")
    } else {
        normalized
    }
}

/// `path.posix.join` of two parts.
pub(crate) fn join(base: &str, request: &str) -> String {
    match (base.is_empty(), request.is_empty()) {
        (true, true) => ".".to_owned(),
        (true, false) => normalize(request),
        (false, true) => normalize(base),
        (false, false) => normalize(&format!("{base}/{request}")),
    }
}

/// `path.posix.resolve(base, request)` for an absolute `base`: no trailing separator.
pub(crate) fn resolve(base: &str, request: &str) -> String {
    let joined = if request.starts_with('/') {
        request.to_owned()
    } else {
        format!("{base}/{request}")
    };
    format!("/{}", normalize_segments(&joined, false).join("/"))
}

/// `path.posix.dirname`.
pub(crate) fn dirname(path: &str) -> &str {
    if path.is_empty() {
        return ".";
    }
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        return "/";
    }
    match trimmed.rfind('/') {
        None => ".",
        Some(0) => "/",
        Some(at) => &trimmed[..at],
    }
}

/// `path.posix.basename` without a suffix: the last name, trailing separators aside.
pub(crate) fn basename(path: &str) -> &str {
    let trimmed = path.trim_end_matches('/');
    trimmed.rfind('/').map_or(trimmed, |at| &trimmed[at + 1..])
}

/// `path.posix.extname`: the last name's text from its last `.`, unless that dot opens it (`..`
/// has none).
pub(crate) fn extname(path: &str) -> &str {
    let base = basename(path);
    if base == ".." {
        return "";
    }
    match base.rfind('.') {
        None | Some(0) => "",
        Some(at) => &base[at..],
    }
}

/// `path.posix.basename(path, path.extname(path))`: the last name without its extension.
pub(crate) fn stem(path: &str) -> &str {
    let base = basename(path);
    &base[..base.len() - extname(base).len()]
}

/// `path.posix.relative` between two absolute paths.
pub(crate) fn relative(from: &str, to: &str) -> String {
    let from = normalize_segments(from, false);
    let to = normalize_segments(to, false);
    let common = from
        .iter()
        .zip(&to)
        .take_while(|(left, right)| left == right)
        .count();
    std::iter::repeat_n("..", from.len() - common)
        .chain(to[common..].iter().copied())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The answers `node -p` gives for each (Node 26).
    #[test]
    fn paths_read_as_node_reads_them() {
        assert_eq!(
            [
                extname("..a"),
                extname("a."),
                extname(".a"),
                extname("foo.js/"),
                extname(".."),
                extname("."),
                extname("./"),
                basename("foo/"),
                basename("/"),
                dirname("a"),
                dirname("/a/"),
                dirname("/"),
                extname("a.."),
                extname(".a."),
            ],
            [
                ".a", ".", "", ".js", "", "", "", "foo", "", ".", "/", "/", ".", "."
            ]
        );
        assert_eq!(resolve("/a/b", "./c/"), "/a/b/c");
        assert_eq!(resolve("/a/b", "/x/../y"), "/y");
        assert_eq!(relative("/a/b", "/a"), "..");
        assert_eq!(relative("/a", "/a/b/c"), "b/c");
        assert_eq!(relative("/a", "/a"), "");
        assert_eq!(join("/a", "../.."), "/");
        assert_eq!(normalize("./a/../.."), "..");
        assert_eq!(join("./x", "y/"), "x/y/");
        assert_eq!(dirname("/a/b/c.js"), "/a/b");
        assert_eq!(stem("/a/b.test.js"), "b.test");
    }
}
