//! Next.js's App Router server module graph (`file.package.serverGraph`): the modules of a
//! package that depends on `next` which Next bundles as Server Components. A server graph starts
//! at the App Router's server file conventions (`page`, `layout`, `template`, `default`, `loading`,
//! `not-found`, `forbidden`, `unauthorized`, `global-not-found`, `route` and the metadata image and
//! route files) under `app/` or `src/app/`, outside private `_` folders. It follows each module's
//! static imports, `export … from`, `import()` and `require()` of a string literal, and stops at a
//! module whose directive prologue holds `"use client"`: that module and what only it imports are
//! client code. Specifiers resolve as Next resolves them in the package: relative paths, then the
//! package's `tsconfig.json` (else `jsconfig.json`) `paths` and `baseUrl`, following relative
//! `extends`; a module of another package (a bare specifier, a file below a nested
//! `package.json`) is no edge.
//!
//! Every ingestion path builds the graph from the same reading (`summarize`, `Config::read`,
//! `server_graph`): file sets (snapshots, editors, workspaces without host facts) from their own
//! entries, hosts and the native repository index from the package on disk (`on_disk`).

use ast_grep_core::{Doc, Node};
use ast_grep_language::LanguageExt;
use indexmap::IndexSet;
use parking_lot::Mutex;
use regex::Regex;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};
use std::time::SystemTime;

use crate::code_rule_dsl::language::adapter::package_manifests;
use crate::code_rule_dsl::language::{dialect_by_label, label_of_path};

/// The package a manifest's `dependsOn` must name for its files to have a server graph.
pub(crate) const NEXT: &str = "next";

/// A package with more source files than this on disk gets no server graph: no file is in one.
const MAX_GRAPH_FILES: usize = 20_000;
/// A source larger than this is not read for imports; it imports nothing.
const MAX_SOURCE_BYTES: u64 = 1024 * 1024;
/// How many `extends` a configuration chain follows.
const MAX_EXTENDS: usize = 8;
/// How many `paths` patterns a configuration maps, the first by pattern text (the order
/// `serde_json` keeps an object's keys in); later ones map nothing.
const MAX_PATHS_PATTERNS: usize = 64;
/// How many targets of one `paths` pattern are tried; later ones are not.
const MAX_PATHS_TARGETS: usize = 16;
/// The extensions an import resolves to, in Next's (webpack's) order.
const EXTENSIONS: [&str; 8] = [".tsx", ".ts", ".jsx", ".js", ".mjs", ".cjs", ".mts", ".cts"];

/// The App Router's server file conventions, relative to the package's folder.
static SERVER_ENTRY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^(?:src/)?app/(?:[^/_][^/]*/)*(?:page|layout|template|default|loading|not-found|forbidden|unauthorized|global-not-found|route|icon|apple-icon|opengraph-image|twitter-image|sitemap|robots|manifest)\.(?:tsx|ts|jsx|js)$",
    )
    .expect("the server entry pattern compiles")
});

/// What the graph reads of one module: whether its directive prologue holds `"use client"`,
/// and the specifiers it imports, each once, in source order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Summary {
    client: bool,
    specifiers: Vec<String>,
}

/// The summary of the ECMA source at `path`; `None` for a path of no ECMA dialect.
pub(crate) fn summarize(path: &str, source: &str) -> Option<Summary> {
    let tree = dialect_by_label(ecma_label(path)?)?.ast_grep(source);
    Some(summary_of(&tree.root()))
}

/// The label of the dialect the registry routes `path` to, where its family reads package
/// manifests (`Adapter::PACKAGE_MANIFESTS`: the files whose imports a package's graph follows).
fn ecma_label(path: &str) -> Option<&'static str> {
    label_of_path(path).filter(|label| package_manifests(label))
}

/// Whether a server graph reads the file at `path`: an ECMA source, or a `tsconfig.json` or
/// `jsconfig.json` (its own, or one it `extends`).
pub(crate) fn read_by_graphs(path: &str) -> bool {
    is_module_config(path) || is_source(path)
}

/// Whether `path` names an ECMA source a server graph summarizes.
pub(crate) fn is_source(path: &str) -> bool {
    ecma_label(path).is_some()
}

fn summary_of<D: Doc>(root: &Node<'_, D>) -> Summary {
    let mut summary = Summary::default();
    let mut specifiers = IndexSet::new();
    let mut prologue = true;
    for statement in root.children() {
        match statement.kind().as_ref() {
            "comment" | "hash_bang_line" => continue,
            "expression_statement" if prologue => match directive(&statement) {
                Some(text) => summary.client |= text == "use client",
                None => prologue = false,
            },
            _ => prologue = false,
        }
        match statement.kind().as_ref() {
            "import_statement" if !has_token(&statement, "type") => {
                let source = statement.field("source").or_else(|| {
                    statement
                        .children()
                        .find(|child| child.kind() == "import_require_clause")
                        .and_then(|clause| clause.field("source"))
                });
                specifiers.extend(source.and_then(|source| literal(&source)));
            }
            "export_statement" if !has_token(&statement, "type") => {
                specifiers.extend(
                    statement
                        .field("source")
                        .and_then(|source| literal(&source)),
                );
            }
            _ => {}
        }
    }
    let text = root.text();
    if text.contains("import") || text.contains("require") {
        for call in root.dfs().filter(|node| node.kind() == "call_expression") {
            let Some(function) = call.field("function") else {
                continue;
            };
            let dynamic = function.kind() == "import";
            if !(dynamic || function.kind() == "identifier" && function.text() == "require") {
                continue;
            }
            let first = call.field("arguments").and_then(|arguments| {
                arguments
                    .children()
                    .find(|argument| argument.is_named() && argument.kind() != "comment")
            });
            specifiers.extend(
                first
                    .filter(|first| first.kind() == "string")
                    .and_then(|first| literal(&first)),
            );
        }
    }
    summary.specifiers = specifiers.into_iter().collect();
    summary
}

/// A directive's text: an expression statement that is only a string literal (comments aside).
fn directive<D: Doc>(statement: &Node<'_, D>) -> Option<String> {
    let mut named = statement
        .children()
        .filter(|child| child.is_named() && child.kind() != "comment");
    let string = named.next().filter(|child| child.kind() == "string")?;
    named.next().is_none().then(|| literal(&string)).flatten()
}

fn has_token<D: Doc>(node: &Node<'_, D>, token: &str) -> bool {
    node.children()
        .any(|child| !child.is_named() && child.kind() == token)
}

/// A string literal's text between its quotes.
fn literal<D: Doc>(string: &Node<'_, D>) -> Option<String> {
    let text = string.text();
    let inner = text.get(1..text.len().checked_sub(1)?)?;
    Some(inner.to_owned())
}

/// A package's TypeScript or JavaScript project configuration, as far as the engine reads it:
/// `compilerOptions.baseUrl`, `paths` and `jsxImportSource`.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Config {
    /// The resolved `baseUrl`, workspace-relative.
    base_url: Option<String>,
    /// `paths` patterns: the text before and after the `*` (`None` for an exact pattern), and
    /// the targets, workspace-relative, each keeping its `*`.
    paths: Vec<(String, Option<String>, Vec<String>)>,
    /// The package the JSX transform imports its runtime from.
    jsx_import_source: Option<String>,
    /// The configuration files read, the package's own first, then its `extends` bases.
    files: Vec<String>,
}

impl Config {
    /// The configuration of the package in `folder`: its `tsconfig.json`, else its
    /// `jsconfig.json`, as `read` gives a workspace-relative file's text. A configuration over
    /// 1 MiB is absent, whichever path reads it, so the same file is chosen everywhere.
    pub(crate) fn read(folder: &str, read: &dyn Fn(&str) -> Option<String>) -> Self {
        let read = |path: &str| read(path).filter(|text| text.len() as u64 <= MAX_SOURCE_BYTES);
        let mut config = Self::default();
        let mut paths_dir = None;
        let Some((path, text)) = ["tsconfig.json", "jsconfig.json"]
            .iter()
            .find_map(|name| read(&join(folder, name)).map(|text| (join(folder, name), text)))
        else {
            return config;
        };
        config.extend(&path, &text, &read, &mut HashSet::new(), &mut paths_dir, 0);
        // Targets resolve against `baseUrl` when one is set, else against the directory of the
        // configuration that sets `paths`.
        if let Some(base) = config.base_url.clone().or(paths_dir) {
            for (_, _, targets) in &mut config.paths {
                for target in targets.iter_mut() {
                    *target = join(&base, target);
                }
            }
        }
        config
    }

    /// Applies the configuration at `path` over its `extends` chain: bases first.
    fn extend(
        &mut self,
        path: &str,
        text: &str,
        read: &dyn Fn(&str) -> Option<String>,
        seen: &mut HashSet<String>,
        paths_dir: &mut Option<String>,
        depth: usize,
    ) {
        if depth > MAX_EXTENDS || !seen.insert(path.to_owned()) {
            return;
        }
        self.files.push(path.to_owned());
        let Some(Value::Object(config)) = parse_jsonc(text) else {
            return;
        };
        let directory = parent(path);
        let bases = match config.get("extends") {
            Some(Value::String(base)) => vec![base.as_str()],
            Some(Value::Array(bases)) => bases.iter().filter_map(Value::as_str).collect(),
            _ => Vec::new(),
        };
        for base in bases {
            // A package's configuration (`@tsconfig/next`) is not read: a module graph holds
            // no file of another package. Nor is a base of another name than a project
            // configuration's (`is_module_config`): every ingestion path holds and routes exactly
            // those files, so all of them read the same chain.
            if !base.starts_with('.') {
                continue;
            }
            let Some(mut base) = normalize(&join(directory, base)) else {
                continue;
            };
            if !base.ends_with(".json") {
                base.push_str(".json");
            }
            if !is_module_config(&base) {
                continue;
            }
            if let Some(text) = read(&base) {
                self.extend(&base, &text, read, seen, paths_dir, depth + 1);
            }
        }
        let Some(Value::Object(options)) = config.get("compilerOptions") else {
            return;
        };
        if let Some(Value::String(base_url)) = options.get("baseUrl") {
            self.base_url = normalize(&join(directory, base_url));
        }
        if let Some(Value::String(source)) = options.get("jsxImportSource") {
            self.jsx_import_source = Some(source.clone());
        }
        if let Some(Value::Object(paths)) = options.get("paths") {
            *paths_dir = Some(directory.to_owned());
            self.paths = paths
                .iter()
                .filter_map(|(pattern, targets)| {
                    let (prefix, suffix) = match pattern.split_once('*') {
                        Some((prefix, suffix)) => (prefix.to_owned(), Some(suffix.to_owned())),
                        None => (pattern.clone(), None),
                    };
                    let targets = targets
                        .as_array()?
                        .iter()
                        .filter_map(Value::as_str)
                        .take(MAX_PATHS_TARGETS)
                        .map(str::to_owned)
                        .collect();
                    Some((prefix, suffix, targets))
                })
                .take(MAX_PATHS_PATTERNS)
                .collect();
        }
    }

    /// `compilerOptions.jsxImportSource`, over the `extends` chain.
    pub(crate) fn jsx_import_source(&self) -> Option<&str> {
        self.jsx_import_source.as_deref()
    }

    /// The configuration files read (workspace-relative): the package's own, then its bases.
    pub(crate) fn files(&self) -> &[String] {
        &self.files
    }

    /// The candidate paths (before extension probing) a non-relative specifier maps to: the
    /// `paths` pattern that matches (an exact one, else the longest prefix), then `baseUrl`.
    fn candidates(&self, specifier: &str) -> Vec<String> {
        let exact = self
            .paths
            .iter()
            .find(|(prefix, suffix, _)| suffix.is_none() && prefix == specifier);
        let pattern = exact.or_else(|| {
            self.paths
                .iter()
                .filter(|(prefix, suffix, _)| {
                    suffix.as_ref().is_some_and(|suffix| {
                        specifier.len() >= prefix.len() + suffix.len()
                            && specifier.starts_with(prefix.as_str())
                            && specifier.ends_with(suffix.as_str())
                    })
                })
                .max_by_key(|(prefix, _, _)| prefix.len())
        });
        let mut candidates = Vec::new();
        if let Some((prefix, suffix, targets)) = pattern {
            let star = suffix
                .as_ref()
                .map(|suffix| &specifier[prefix.len()..specifier.len() - suffix.len()]);
            for target in targets {
                candidates.push(match star {
                    Some(star) => target.replacen('*', star, 1),
                    None => target.clone(),
                });
            }
        }
        if let Some(base_url) = &self.base_url {
            candidates.push(join(base_url, specifier));
        }
        candidates
    }
}

/// The members of the server graph of the package in `folder`, relative to `folder`, over its
/// ECMA `files` (workspace-relative path to summary). Next ignores `src/app` where the package has
/// a root `app` folder (`root_app`), so only one of them holds entries.
pub(crate) fn server_graph(
    folder: &str,
    files: &BTreeMap<String, Summary>,
    config: &Config,
    root_app: bool,
) -> BTreeSet<String> {
    let relative = |path: &str| -> String {
        if folder.is_empty() {
            path.to_owned()
        } else {
            path[folder.len() + 1..].to_owned()
        }
    };
    let mut members = HashSet::new();
    let mut queue = VecDeque::new();
    for (path, summary) in files {
        let relative = relative(path);
        if !summary.client
            && SERVER_ENTRY.is_match(&relative)
            && !(root_app && relative.starts_with("src/"))
        {
            members.insert(path.as_str());
            queue.push_back(path.as_str());
        }
    }
    // A specifier resolves once per importing directory, and a non-relative one once for the
    // package, however many modules import it.
    let mut resolutions = HashMap::new();
    while let Some(path) = queue.pop_front() {
        let directory = parent(path);
        for specifier in &files[path].specifiers {
            let from_importer = specifier.starts_with("./")
                || specifier.starts_with("../")
                || specifier == "."
                || specifier == "..";
            let key = (
                if from_importer { directory } else { "" },
                specifier.as_str(),
            );
            let target = *resolutions.entry(key).or_insert_with(|| {
                let candidates = if from_importer {
                    vec![join(directory, specifier)]
                } else if specifier.starts_with('/') {
                    Vec::new()
                } else {
                    config.candidates(specifier)
                };
                candidates
                    .iter()
                    .filter_map(|candidate| normalize(candidate))
                    .find_map(|candidate| resolve(&candidate, files))
            });
            if let Some((target, summary)) = target
                && !summary.client
                && members.insert(target.as_str())
            {
                queue.push_back(target.as_str());
            }
        }
    }
    members.into_iter().map(relative).collect()
}

/// The file `base` names among `files`: itself, with an extension, a TypeScript source for a
/// JavaScript extension, or its `index` file.
fn resolve<'f>(
    base: &str,
    files: &'f BTreeMap<String, Summary>,
) -> Option<(&'f String, &'f Summary)> {
    let mut candidates = vec![base.to_owned()];
    for (javascript, typescript) in [
        (".js", &[".ts", ".tsx"][..]),
        (".jsx", &[".tsx"][..]),
        (".mjs", &[".mts"][..]),
        (".cjs", &[".cts"][..]),
    ] {
        if let Some(stem) = base.strip_suffix(javascript) {
            candidates.extend(
                typescript
                    .iter()
                    .map(|extension| format!("{stem}{extension}")),
            );
        }
    }
    candidates.extend(
        EXTENSIONS
            .iter()
            .map(|extension| format!("{base}{extension}")),
    );
    candidates.extend(
        EXTENSIONS
            .iter()
            .map(|extension| join(base, &format!("index{extension}"))),
    );
    candidates
        .iter()
        .find_map(|candidate| files.get_key_value(candidate.as_str()))
}

fn parent(path: &str) -> &str {
    path.rfind('/').map_or("", |at| &path[..at])
}

fn join(directory: &str, path: &str) -> String {
    if directory.is_empty() {
        path.to_owned()
    } else {
        format!("{directory}/{path}")
    }
}

/// `path` with `.` and `..` resolved; `None` where it leaves the root, or where a name holds a
/// `\`, `:` or NUL: on Windows such a name is another separator, a drive (`C:`) or a UNC share
/// (`\\host\share`), which `Path::join` would read in place of the workspace.
fn normalize(path: &str) -> Option<String> {
    let mut names = Vec::new();
    for name in path.split('/') {
        match name {
            "" | "." => {}
            ".." => {
                names.pop()?;
            }
            name if name.contains(['\\', ':', '\0']) => return None,
            name => names.push(name),
        }
    }
    Some(names.join("/"))
}

/// JSON with comments and trailing commas, as TypeScript reads a configuration file: comments
/// go first, so a trailing comma before a comment and a closing bracket is still dropped.
fn parse_jsonc(text: &str) -> Option<Value> {
    // TypeScript drops a leading byte-order mark, as editors on Windows write one.
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let without_comments = strip_outside_strings(text, |bytes, at| match bytes[at..] {
        [b'/', b'/', ..] => Some(
            bytes[at..]
                .iter()
                .position(|byte| *byte == b'\n')
                .map_or(bytes.len(), |end| at + end),
        ),
        [b'/', b'*', ..] => Some(
            bytes[at + 2..]
                .windows(2)
                .position(|pair| pair == b"*/")
                .map_or(bytes.len(), |end| at + 2 + end + 2),
        ),
        _ => None,
    });
    let without_trailing_commas = strip_outside_strings(&without_comments, |bytes, at| {
        (bytes[at] == b','
            && matches!(
                bytes[at + 1..]
                    .iter()
                    .find(|byte| !byte.is_ascii_whitespace()),
                Some(b'}' | b']')
            ))
        .then_some(at + 1)
    });
    serde_json::from_str(&without_trailing_commas).ok()
}

/// `text` without the spans `skip` names outside string literals: given the bytes and an offset,
/// it returns where a span to drop starting there ends.
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

/// One summary read from disk, kept while the file's modification time and length stand.
type CachedSummary = (SystemTime, u64, Arc<Summary>);

/// Summaries of files on disk, by absolute path, across calls.
static DISK_SUMMARIES: LazyLock<Mutex<HashMap<PathBuf, CachedSummary>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
/// The disk cache is dropped whole beyond this many entries.
const MAX_CACHED_SUMMARIES: usize = 200_000;

/// The server graph of the package whose folder (workspace-relative) is `folder`, read on disk
/// below `root` with its `config`: its source files outside `node_modules`, hidden folders and
/// nested packages. `overlay` replaces one source's text with a host's (`None`: deleted), for a
/// source being edited that disk does not hold yet. Where a folder or source cannot be read the
/// package has no graph rather than a partial one, as past `MAX_GRAPH_FILES`; the failure is not
/// cached, so the next call reads it again.
pub(crate) fn on_disk(
    root: &Path,
    folder: &str,
    config: &Config,
    overlay: Option<(&str, Option<&str>)>,
) -> BTreeSet<String> {
    let directory = if folder.is_empty() {
        root.to_path_buf()
    } else {
        root.join(folder)
    };
    let mut sources = Vec::new();
    if !walk(&directory, &directory, &mut sources) {
        return BTreeSet::new();
    }
    let mut cache = DISK_SUMMARIES.lock();
    if cache.len() > MAX_CACHED_SUMMARIES {
        cache.clear();
    }
    let mut files = BTreeMap::new();
    for (relative, absolute, metadata) in sources {
        let path = join(folder, &relative);
        // The overlay's text replaces this source's below; disk's is not read.
        if overlay.is_some_and(|(overlaid, _)| overlaid == path) {
            continue;
        }
        let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        let summary = match cache.get(&absolute) {
            Some((time, length, summary)) if *time == modified && *length == metadata.len() => {
                Arc::clone(summary)
            }
            _ => {
                let source = if metadata.len() > MAX_SOURCE_BYTES {
                    String::new()
                } else {
                    match std::fs::read(&absolute) {
                        // As the host decodes a file it reads: invalid UTF-8 becomes
                        // replacement characters.
                        Ok(bytes) => String::from_utf8(bytes).unwrap_or_else(|error| {
                            String::from_utf8_lossy(error.as_bytes()).into_owned()
                        }),
                        Err(error) if gone(&error) => continue,
                        Err(_) => return BTreeSet::new(),
                    }
                };
                let Some(summary) = summarize(&path, &source) else {
                    continue;
                };
                let summary = Arc::new(summary);
                cache.insert(absolute, (modified, metadata.len(), Arc::clone(&summary)));
                summary
            }
        };
        files.insert(path, (*summary).clone());
    }
    drop(cache);
    let mut root_app = directory.join("app").is_dir();
    if let Some((path, source)) = overlay {
        files.remove(path);
        if let Some(relative) = relative_to(folder, path)
            && let Some(source) = source
        {
            // As the walk reads it: not below a nested package, `node_modules` or a hidden folder.
            let mut names = relative.split('/').collect::<Vec<_>>();
            names.pop();
            let walked = !names.iter().enumerate().any(|(depth, name)| {
                *name == "node_modules"
                    || name.starts_with('.')
                    || directory
                        .join(names[..=depth].join("/"))
                        .join("package.json")
                        .exists()
            });
            // A file there makes the folder present whether or not the graph reads it as a source.
            root_app |= relative.starts_with("app/");
            if let Some(summary) = summarize_bounded(path, source).filter(|_| walked) {
                files.insert(path.to_owned(), summary);
            }
        }
    }
    // The walk read one source past the bound, so an overlay's deletion can bring the package
    // back within it; past it, the package has no graph, as a file set has none.
    if files.len() > MAX_GRAPH_FILES {
        return BTreeSet::new();
    }
    server_graph(folder, &files, config, root_app)
}

/// The summary of the ECMA source at `path`, read as a disk walk reads it: a source over 1 MiB
/// imports nothing.
pub(crate) fn summarize_bounded(path: &str, source: &str) -> Option<Summary> {
    summarize(
        path,
        if source.len() as u64 > MAX_SOURCE_BYTES {
            ""
        } else {
            source
        },
    )
}

/// The configuration of the package whose folder (workspace-relative) is `folder`, read on disk
/// below `root` (`Config::read`): regular files of at most 1 MiB.
pub(crate) fn config_on_disk(root: &Path, folder: &str) -> Config {
    let read = |path: &str| {
        // Only a path of plain names below `root`: `join` reads an absolute or prefixed one (a
        // Windows drive or UNC share) in place of the workspace.
        if !Path::new(path)
            .components()
            .all(|component| matches!(component, std::path::Component::Normal(_)))
        {
            return None;
        }
        let file = root.join(path);
        let metadata = std::fs::symlink_metadata(&file).ok()?;
        (metadata.is_file() && metadata.len() <= MAX_SOURCE_BYTES)
            .then(|| std::fs::read_to_string(file).ok())
            .flatten()
    };
    Config::read(folder, &read)
}

/// Collects the ECMA source files below `directory` (paths relative to `top`); `false` where a
/// folder or entry cannot be read (one deleted since its folder was listed is absent), or more
/// than one past `MAX_GRAPH_FILES` (`on_disk` decides the bound once an overlay applies).
fn walk(
    top: &Path,
    directory: &Path,
    sources: &mut Vec<(String, PathBuf, std::fs::Metadata)>,
) -> bool {
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) => return gone(&error),
    };
    let Ok(mut entries) = entries.collect::<Result<Vec<_>, _>>() else {
        return false;
    };
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let kind = match entry.file_type() {
            Ok(kind) => kind,
            Err(error) if gone(&error) => continue,
            Err(_) => return false,
        };
        let path = entry.path();
        if kind.is_dir() {
            if name == "node_modules" || name.starts_with('.') || path.join("package.json").exists()
            {
                continue;
            }
            if !walk(top, &path, sources) {
                return false;
            }
        } else if kind.is_file() && ecma_label(&name).is_some() {
            if sources.len() > MAX_GRAPH_FILES {
                return false;
            }
            let metadata = match entry.metadata() {
                Ok(metadata) => metadata,
                Err(error) if gone(&error) => continue,
                Err(_) => return false,
            };
            let relative = path
                .strip_prefix(top)
                .map(|relative| {
                    relative
                        .components()
                        .map(|component| component.as_os_str().to_string_lossy())
                        .collect::<Vec<_>>()
                        .join("/")
                })
                .unwrap_or_default();
            sources.push((relative, path, metadata));
        }
    }
    true
}

/// Whether a read failed because the entry no longer exists (deleted since its folder was
/// listed): it is absent, not unreadable.
fn gone(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::NotFound
}

/// The server graph of the package in `folder` over a file set's entries: `sources` gives the
/// set's paths and texts (`from_summaries`).
pub(crate) fn in_file_set<'s>(
    folder: &str,
    sources: impl Iterator<Item = (&'s str, &'s str)>,
    manifest: &dyn Fn(&str) -> bool,
    config: &Config,
) -> BTreeSet<String> {
    let entries = sources
        .filter(|(path, _)| relative_to(folder, path).is_some())
        .map(|(path, source)| (path, summarize_bounded(path, source)));
    from_summaries(folder, entries, manifest, config)
}

/// The server graph of the package in `folder` over a file set's `entries`: each path with its
/// summary (`summarize_bounded`; `None` for a file of no ECMA dialect), `manifest` whether a
/// directory holds a `package.json`, `config` the package's configuration. It reads the files a
/// disk walk reads (`walk`): a file below a nested `package.json` belongs to that package, and one
/// in `node_modules` or a hidden folder to none; past `MAX_GRAPH_FILES` sources the package has no
/// graph. Any entry below `app/` marks the root app folder present, as the folder does on disk.
pub(crate) fn from_summaries<'s>(
    folder: &str,
    entries: impl Iterator<Item = (&'s str, Option<Summary>)>,
    manifest: &dyn Fn(&str) -> bool,
    config: &Config,
) -> BTreeSet<String> {
    let walked = |path: &str, relative: &str| {
        let mut directory = parent(path);
        while directory.len() > folder.len() {
            if manifest(directory) {
                return false;
            }
            directory = parent(directory);
        }
        let mut names = relative.split('/').collect::<Vec<_>>();
        names.pop();
        !names
            .iter()
            .any(|name| *name == "node_modules" || name.starts_with('.'))
    };
    let mut files = BTreeMap::new();
    let mut root_app = false;
    for (path, summary) in entries {
        let Some(relative) = relative_to(folder, path) else {
            continue;
        };
        root_app |= relative.starts_with("app/");
        let Some(summary) = summary.filter(|_| walked(path, relative)) else {
            continue;
        };
        if files.len() >= MAX_GRAPH_FILES {
            return BTreeSet::new();
        }
        files.insert(path.to_owned(), summary);
    }
    server_graph(folder, &files, config, root_app)
}

/// `path` relative to `folder`; `None` outside it.
fn relative_to<'p>(folder: &str, path: &'p str) -> Option<&'p str> {
    if folder.is_empty() {
        Some(path)
    } else {
        path.strip_prefix(folder)?.strip_prefix('/')
    }
}

/// Whether `path` names a TypeScript or JavaScript project configuration a package's `Config`
/// may read: a `.json` file whose name starts with `tsconfig` or `jsconfig` (`tsconfig.json`,
/// `tsconfig.base.json`, `jsconfig-shared.json`). Hosts route edits of the same names.
pub(crate) fn is_module_config(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.ends_with(".json") && (name.starts_with("tsconfig") || name.starts_with("jsconfig"))
}

#[cfg(test)]
mod tests;
