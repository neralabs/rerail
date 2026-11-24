//! What a project configuration (`tsconfig.json`, `jsconfig.json`) is read with, wherever its files come from: the
//! JSON-with-comments it is written in (`strip_jsonc`) and the configuration an `extends` names (`extends_path`, as
//! get-tsconfig resolves it). The repository reader (`facts::repository`) reads the files on disk below a root,
//! the import resolver (`lang_ecma`'s `import_target::tsconfig`) a workspace's held files over them (decision 0660).

use super::node_path as path;

/// How many `extends` a configuration chain follows.
pub const MAX_EXTENDS_DEPTH: usize = 8;

/// The files a configuration's `extends` is resolved among, by absolute `/`-separated path (`node_path`).
pub trait ConfigFiles {
    /// The absolute path of the root: nothing above it is searched.
    fn base(&self) -> &str;
    fn is_file(&self, path: &str) -> bool;
    fn is_directory(&self, path: &str) -> bool;
    /// The path `path` resolves to through its symlinks, as Node's `realpath` gives a package's folder.
    fn real_path(&self, path: &str) -> String;
    fn text(&self, path: &str) -> Option<String>;
}

/// A JSON-with-comments text as plain JSON, as TypeScript reads a configuration file: without a leading byte-order
/// mark, comments and trailing commas. Comments go first, so a trailing comma before a comment and a closing bracket
/// is still dropped.
pub fn strip_jsonc(text: &str) -> String {
    // TypeScript drops a leading byte-order mark, as editors on Windows write one.
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let without_comments = strip_outside_strings(text, |bytes, at| match bytes[at..] {
        [b'/', b'/', ..] => Some(bytes[at..].iter().position(|byte| *byte == b'\n').map_or(bytes.len(), |end| at + end)),
        [b'/', b'*', ..] => {
            Some(bytes[at + 2..].windows(2).position(|pair| pair == b"*/").map_or(bytes.len(), |end| at + 2 + end + 2))
        }
        _ => None,
    });
    strip_outside_strings(&without_comments, |bytes, at| {
        (bytes[at] == b','
            && matches!(bytes[at + 1..].iter().find(|byte| !byte.is_ascii_whitespace()), Some(b'}' | b']')))
        .then_some(at + 1)
    })
}

/// `text` without the spans `skip` names outside string literals: given the bytes and an offset, it returns where a
/// span to drop starting there ends.
fn strip_outside_strings(text: &str, skip: impl Fn(&[u8], usize) -> Option<usize>) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut kept = 0;
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'"' {
            at += 1;
            while at < bytes.len() && bytes[at] != b'"' {
                at += if bytes[at] == b'\\' { 2 } else { 1 };
            }
            at = (at + 1).min(bytes.len());
        } else if let Some(end) = skip(bytes, at) {
            out.push_str(&text[kept..at]);
            out.push(' ');
            at = end.min(bytes.len());
            kept = at;
        } else {
            at += 1;
        }
    }
    out.push_str(&text[kept..]);
    out
}

/// A configuration's `extends`, one name or a list of them, in order.
pub fn extends_of(config: &serde_json::Value) -> Vec<&str> {
    match config.get("extends") {
        Some(serde_json::Value::String(base)) => vec![base.as_str()],
        Some(serde_json::Value::Array(bases)) => bases.iter().filter_map(serde_json::Value::as_str).collect(),
        _ => Vec::new(),
    }
}

/// get-tsconfig's `resolveExtendsPath`: a relative or absolute path (`.json` appended where it names no file), or a
/// package's configuration in a `node_modules` folder at or above `directory` (its `tsconfig` field, or the subpath
/// it names, else its `tsconfig.json`). An absolute path is one `path.isAbsolute` reads as one where Rerail runs: on
/// Windows a drive or UNC path too (`node_path::platform_absolute`).
pub fn extends_path(files: &dyn ConfigFiles, extends: &str, directory: &str) -> Option<String> {
    let mut target = if extends == ".." { "../tsconfig.json".to_owned() } else { extends.to_owned() };
    if target.starts_with('.') {
        target = path::resolve(directory, &target);
    } else if let Some(absolute) = path::platform_absolute(&target) {
        target = absolute;
    }
    if target.starts_with('/') {
        if files.is_file(&target) {
            return Some(target);
        }
        if !target.ends_with(".json") && files.is_file(&format!("{target}.json")) {
            return Some(format!("{target}.json"));
        }
        return None;
    }
    let mut parts = extends.splitn(2, '/');
    let first = parts.next()?;
    let rest = parts.next().unwrap_or("");
    let (package, subpath) = if first.starts_with('@') {
        let mut scoped = rest.splitn(2, '/');
        let name = scoped.next().unwrap_or("");
        (format!("{first}/{name}"), scoped.next().unwrap_or("").to_owned())
    } else {
        (first.to_owned(), rest.to_owned())
    };
    let mut search = directory.to_owned();
    let folder = loop {
        let candidate = path::join(&search, &format!("node_modules/{package}"));
        if files.is_directory(&candidate) {
            break files.real_path(&candidate);
        }
        if search == files.base() || search == "/" {
            return None;
        }
        search = path::dirname(&search).to_owned();
    };
    let manifest = path::join(&folder, "package.json");
    if subpath.is_empty()
        && let Some(text) = files.text(&manifest)
        && let Ok(json) = serde_json::from_str::<serde_json::Value>(text.trim_start_matches('\u{feff}'))
        && let Some(field) = json.get("tsconfig").and_then(serde_json::Value::as_str)
    {
        let named = path::join(&folder, field);
        if files.is_file(&named) {
            return Some(named);
        }
    }
    let named = path::join(&folder, if subpath.is_empty() { "tsconfig.json" } else { &subpath });
    if !named.ends_with(".json") && files.is_file(&format!("{named}.json")) {
        return Some(format!("{named}.json"));
    }
    if files.is_file(&named) {
        return Some(named);
    }
    let nested = path::join(&named, "tsconfig.json");
    files.is_file(&nested).then_some(nested)
}
