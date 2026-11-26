//! `core.rerail`'s repository inputs (decision 0046): facts about the repository that are not source code, one reader
//! per kind of file, made once a run (not per batch) and bound beside the batches' relations.
//!
//! - `option(name, v)` and `option_item(name, i, v)`: the policy's `options` (`rerail.policy/v1`), as rrlab reads
//!   them: a list gives one `option` row per element and `option_item` rows with positions; booleans are `"true"` and
//!   `"false"`; nested objects are flattened with `/`. An ESLint-style array (`["all", {…}]`) gives `option`,
//!   `option2`, … for its strings, and merges its objects.
//! - `file(path)`: every file under the root, the host's ignored directories (`git_workspace::ignored_directories`:
//!   `.git`, `node_modules`, `target`, …) skipped.
//! - `package(dir, name)`, `package_field(dir, key, v)`: each `package.json`'s name and string-valued members, keys
//!   as paths from the root (`engines/node`); `package_for(path, dir)`: a document's nearest `package.json`.
//! - `tsconfig(dir, base_url)`, `tsconfig_path(dir, pattern, target)`: each `tsconfig.json` or `jsconfig.json`'s
//!   `compilerOptions`, its `extends` chain merged beneath it (a path, relative or from the root, `.json` added where
//!   it names no file); `baseUrl` from the root (the configuration's folder where none is set), each `paths` target
//!   as written. `tsconfig_for(path, dir)`: a document's nearest `tsconfig.json`, else `jsconfig.json`. The files
//!   are JSONC: comments and trailing commas are dropped before they are read.
//!
//! `git_*` and `signed_list` are not read yet (open item O80). `input_error`'s `missing` rows are made
//! where the `.input` files are (`logic::plan::corpus::external_inputs`).

use rerail_blocks::vector::Vector;
use super::vtab::Relation;
use crate::content_store::{FactStore, StoreResult};
use rerail_lang::code_rule_dsl::tsconfig;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// At most this many files are listed; a larger tree's `file` rows are cut there.
const MAX_FILES: usize = 200_000;

/// The rows, as strings, before they are interned.
#[derive(Default, Debug, PartialEq)]
pub struct Rows {
    pub option: Vec<(String, String)>,
    pub option_item: Vec<(String, i64, String)>,
    pub(crate) file: Vec<String>,
    pub(crate) package: Vec<(String, String)>,
    pub(crate) package_field: Vec<(String, String, String)>,
    pub(crate) package_for: Vec<(String, String)>,
    pub(crate) tsconfig: Vec<(String, String)>,
    pub(crate) tsconfig_for: Vec<(String, String)>,
    pub(crate) tsconfig_path: Vec<(String, String, String)>,
}

fn scalar(v: &Value) -> String {
    match v {
        Value::Bool(b) => b.to_string(),
        Value::Null => "null".to_owned(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// The run's options over the defaults its programs declare (`.options {"name": {"default": …}, …}`): an option the
/// run does not set has its declared default, so a rule's option reads the same whether or not its caller passes
/// options. A run option that is not an object (none given) leaves only the defaults.
pub fn with_declared(settings: &[(String, String)], opts: &Value) -> Value {
    let mut merged = serde_json::Map::new();
    for (_, text) in settings.iter().filter(|(k, _)| k == "options") {
        let Ok(Value::Object(decl)) = serde_json::from_str::<Value>(text) else { continue };
        for (name, spec) in decl {
            if let Some(d) = spec.get("default") {
                merged.entry(name).or_insert_with(|| d.clone());
            }
        }
    }
    if merged.is_empty() {
        return opts.clone();
    }
    if let Value::Object(given) = opts {
        merged.extend(given.clone());
    }
    Value::Object(merged)
}

/// `option` and `option_item` of a policy's `options`.
pub fn options(opts: &Value, rows: &mut Rows) {
    let merged;
    let opts = match opts {
        Value::Array(list) => {
            let mut m = serde_json::Map::new();
            let mut k = 0;
            for x in list {
                match x {
                    Value::Object(o) => m.extend(o.clone()),
                    other => {
                        m.insert(if k == 0 { "option".to_owned() } else { format!("option{}", k + 1) }, other.clone());
                        k += 1;
                    }
                }
            }
            merged = Value::Object(m);
            &merged
        }
        other => other,
    };
    fn walk(prefix: &str, v: &Value, rows: &mut Rows) {
        match v {
            Value::Object(o) => {
                for (k, x) in o {
                    walk(&if prefix.is_empty() { k.clone() } else { format!("{prefix}/{k}") }, x, rows);
                }
            }
            Value::Array(list) => {
                for (i, x) in list.iter().enumerate() {
                    rows.option.push((prefix.to_owned(), scalar(x)));
                    rows.option_item.push((prefix.to_owned(), i as i64, scalar(x)));
                }
            }
            other => rows.option.push((prefix.to_owned(), scalar(other))),
        }
    }
    if !opts.is_null() {
        walk("", opts, rows);
    }
}

/// Every string member of `v`, with its path from the root (JSON pointer tokens: `~` as `~0`, `/` as `~1`).
fn strings(prefix: &str, v: &Value, out: &mut Vec<(String, String)>) {
    let key = |k: &str| {
        let k = k.replace('~', "~0").replace('/', "~1");
        if prefix.is_empty() { k } else { format!("{prefix}/{k}") }
    };
    match v {
        Value::Object(o) => o.iter().for_each(|(k, x)| strings(&key(k), x, out)),
        Value::Array(list) => list.iter().enumerate().for_each(|(i, x)| strings(&key(&i.to_string()), x, out)),
        Value::String(s) => out.push((prefix.to_owned(), s.clone())),
        _ => {}
    }
}

/// `file`, `package`, `package_field`, `package_for` and the `tsconfig*` rows of the tree under `root`, for documents
/// `docs` (paths relative to it), each configuration file read (`files`).
pub(crate) fn tree(root: &Path, docs: &[String], rows: &mut Rows, files: &mut dyn Files) {
    let (packages, configs) = walk(root, docs, rows);
    read_settings(&Disk::new(root, files), &packages, &configs, rows);
}

/// The tree under `root`: its `file` rows, and the `package.json` and project configuration of each folder that has
/// one (by folder, their paths), with `package_for` and `tsconfig_for` of documents `docs`. No file is read.
fn walk(root: &Path, docs: &[String], rows: &mut Rows) -> (BTreeMap<String, String>, BTreeMap<String, String>) {
    let mut packages: BTreeMap<String, String> = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in sorted_entries(&dir) {
            tree_entry(root, &entry, &mut stack, &mut packages, rows);
        }
    }
    rows.file.sort();
    let configs = tsconfig_files(rows);
    package_for_rows(docs, &packages, rows);
    tsconfig_for_rows(docs, &configs, rows);
    (packages, configs)
}

/// The configuration files' rows (`package`, `package_field`, `tsconfig`, `tsconfig_path`), read through `disk`.
fn read_settings(disk: &Disk<'_>, packages: &BTreeMap<String, String>, configs: &BTreeMap<String, String>, rows: &mut Rows) {
    package_rows(disk, packages, rows);
    tsconfig_rows(disk, configs, rows);
}

/// What a run reads of the configuration files (decision 0665): a file's digest, which the observer reuses while the
/// file's stat is the one kept with it (no read), and its bytes. `Plain` reads the disk each time.
pub trait Files {
    fn digest(&mut self, path: &Path) -> Option<[u8; 32]>;
    fn read(&mut self, path: &Path) -> Option<Vec<u8>>;
}

/// The disk, read each time.
pub struct Plain;

impl Files for Plain {
    fn digest(&mut self, path: &Path) -> Option<[u8; 32]> {
        use sha2::Digest as _;
        std::fs::read(path).ok().filter(|_| path.is_file()).map(|b| sha2::Sha256::digest(&b).into())
    }
    fn read(&mut self, path: &Path) -> Option<Vec<u8>> {
        std::fs::read(path).ok().filter(|_| path.is_file())
    }
}

impl Files for crate::content_store::observe::Observer {
    fn digest(&mut self, path: &Path) -> Option<[u8; 32]> {
        crate::content_store::observe::Observer::digest(self, path)
    }
    fn read(&mut self, path: &Path) -> Option<Vec<u8>> {
        crate::content_store::observe::Observer::read(self, path).map(|(bytes, _)| bytes)
    }
}

/// One question the reading of the configuration files asked of the disk, with its answer: what a later run asks
/// again to know the kept rows still hold.
#[derive(Clone, Debug, PartialEq)]
enum Probe {
    /// A file's digest (`None`: not a readable file).
    Text(String, Option<[u8; 32]>),
    IsFile(String, bool),
    IsDirectory(String, bool),
    RealPath(String, String),
}

impl Probe {
    /// The probe as a kept entry's read (`KeptPut::reads`): its question, a tag and the path, and its answer's word.
    fn read(&self) -> (String, u64) {
        use sha2::Digest as _;
        let word = |d: &[u8]| u64::from_le_bytes(d[..8].try_into().expect("8 bytes"));
        match self {
            Probe::Text(p, d) => (format!("t\0{p}"), d.as_ref().map_or(u64::MAX, |d| word(d))),
            Probe::IsFile(p, b) => (format!("f\0{p}"), u64::from(*b)),
            Probe::IsDirectory(p, b) => (format!("d\0{p}"), u64::from(*b)),
            Probe::RealPath(p, r) => (format!("r\0{p}"), word(&sha2::Sha256::digest(r.as_bytes()))),
        }
    }

    /// Whether the disk answers kept read `(question, answer)` (`read`) as it did, asked through `disk`'s files: a
    /// file's digest without reading it, where the observer keeps its stat.
    fn holds((question, answer): (&str, u64), disk: &Disk<'_>) -> bool {
        let Some((tag, path)) = question.split_once('\0') else { return false };
        let now = match tag {
            "t" => Probe::Text(path.to_owned(), disk.files.borrow_mut().digest(&disk.on_disk(path))),
            "f" => Probe::IsFile(path.to_owned(), disk.on_disk(path).is_file()),
            "d" => Probe::IsDirectory(path.to_owned(), disk.on_disk(path).is_dir()),
            "r" => Probe::RealPath(path.to_owned(), disk.real_path_unrecorded(path)),
            _ => return false,
        };
        now.read().1 == answer
    }
}

/// The settings relations kept (decision 0665), by name, with their widths: what `read_settings` reads of the
/// configuration files.
const KEPT_SETTINGS: [(&str, usize); 4] = [("package", 2), ("package_field", 3), ("tsconfig", 2), ("tsconfig_path", 3)];

/// The fact space the settings of the tree at `root` are kept in, and the identity of its configuration files
/// (`packages` and `configs`, by folder): their entries' key.
fn settings_key(root: &Path, packages: &BTreeMap<String, String>, configs: &BTreeMap<String, String>) -> ([u8; 32], [u8; 32]) {
    use sha2::Digest as _;
    let mut space = sha2::Sha256::new();
    space.update(b"rerail.space/v1\0settings\0");
    space.update(root.to_string_lossy().as_bytes());
    let mut key = sha2::Sha256::new();
    key.update(b"rerail.settings-kept/v1\0");
    for (dir, file) in packages.iter().chain([(&String::new(), &String::new())]).chain(configs) {
        key.update([0]);
        key.update(dir.as_bytes());
        key.update([0]);
        key.update(file.as_bytes());
    }
    (space.finalize().into(), key.finalize().into())
}

/// The kept settings rows (`KEPT_SETTINGS`, atoms) under `key` in `space`, where the disk answers each question their
/// reading asked (`Probe`) as it did: then no configuration file is read.
fn kept_settings(store: &dyn FactStore, space: &[u8; 32], key: &[u8; 32], disk: &Disk<'_>) -> StoreResult<Option<Vec<Vec<Vec<i64>>>>> {
    let mut out: Option<Vec<Vec<Vec<i64>>>> = None;
    store.kept().read(space, &mut |chain| {
        let Some(chain) = chain else { return };
        let mut rows = Vec::new();
        for (k, (rel, _)) in KEPT_SETTINGS.iter().enumerate() {
            let Some(entry) = chain.get(&rerail_blocks::kept::FactKey { rel, ident: key, part: 0 }) else { return };
            if k == 0 && !entry.reads().all(|r| Probe::holds(r, disk)) {
                return;
            }
            rows.push((0..entry.rows.len()).map(|i| entry.row(i)).collect());
        }
        out = Some(rows);
    });
    Ok(out)
}

/// Keeps `rows` (`KEPT_SETTINGS`' relations, atoms) under `key` in `space`, with the questions their reading asked.
fn keep_settings(store: &dyn FactStore, space: [u8; 32], key: [u8; 32], rows: &[Vec<Vec<i64>>], probes: &[Probe]) {
    use crate::content_store::kept_facts::base::{BlockFlat, KeptPut, KeptWhat};
    use sha2::Digest as _;
    let reads: Vec<(String, u64)> = probes.iter().map(Probe::read).collect();
    let puts = KEPT_SETTINGS
        .iter()
        .zip(rows)
        .map(|(&(rel, width), rows)| {
            let mut sorted = rows.clone();
            sorted.sort_unstable();
            sorted.dedup();
            let mut flat = BlockFlat::new(sorted.len(), width, true);
            let mut hash = sha2::Sha256::new();
            for (r, row) in sorted.iter().enumerate() {
                for (c, &v) in row.iter().enumerate() {
                    flat.put(r, c, v);
                    hash.update(v.to_le_bytes());
                }
            }
            let digest = u64::from_le_bytes(hash.finalize()[..8].try_into().expect("8 bytes"));
            KeptPut { rel: rel.to_owned(), ident: key, part: 0, digest, reads: reads.clone(), types: vec![b's'; width], what: KeptWhat::Picked(Box::new(flat)) }
        })
        .collect();
    store.kept().queue(space, puts, "the settings kept");
}

/// The names a project configuration has, the first found in a folder governing it.
const TSCONFIG_NAMES: [&str; 2] = ["tsconfig.json", "jsconfig.json"];
/// `a/b/../c` as `a/c`: `.` and `..` segments resolved, never above the root.
fn normalize(path: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                out.pop();
            }
            s => out.push(s),
        }
    }
    out.join("/")
}

/// `dir` joined with `rel` (`rel` from the root where it starts with `/`), normalized.
fn join(dir: &str, rel: &str) -> String {
    if rel.starts_with('/') { normalize(rel) } else { normalize(&format!("{dir}/{rel}")) }
}

/// The files on disk below a root, by absolute `/`-separated path, as the shared `extends` resolver reads them
/// (`tsconfig::ConfigFiles`, decision 0670).
struct Disk<'f> {
    base: String,
    /// The root with its symlinks resolved, as `real_path` resolves a path below it.
    canonical: Option<PathBuf>,
    files: std::cell::RefCell<&'f mut dyn Files>,
    /// Each question asked, with its answer (`settings_rows`).
    probes: std::cell::RefCell<Vec<Probe>>,
}

impl<'f> Disk<'f> {
    fn new(root: &Path, files: &'f mut dyn Files) -> Self {
        let base = root.to_string_lossy().replace('\\', "/");
        let base = if base.starts_with('/') { base } else { format!("/{base}") };
        Disk { base: base.trim_end_matches('/').to_owned(), canonical: std::fs::canonicalize(root).ok(), files: std::cell::RefCell::new(files), probes: Default::default() }
    }

    /// The path on disk of an absolute path here (`/C:/a` on Windows is `C:/a`).
    fn on_disk(&self, path: &str) -> PathBuf {
        PathBuf::from(if cfg!(windows) { path.trim_start_matches('/') } else { path })
    }

    /// The root-relative path of an absolute one, where it lies below the root.
    fn relative<'p>(&self, path: &'p str) -> Option<&'p str> {
        path.strip_prefix(self.base.as_str())?.strip_prefix('/')
    }
}

impl tsconfig::ConfigFiles for Disk<'_> {
    fn base(&self) -> &str {
        &self.base
    }
    fn is_file(&self, path: &str) -> bool {
        let got = self.on_disk(path).is_file();
        self.probes.borrow_mut().push(Probe::IsFile(path.to_owned(), got));
        got
    }
    fn is_directory(&self, path: &str) -> bool {
        let got = self.on_disk(path).is_dir();
        self.probes.borrow_mut().push(Probe::IsDirectory(path.to_owned(), got));
        got
    }
    fn real_path(&self, path: &str) -> String {
        let got = self.real_path_unrecorded(path);
        self.probes.borrow_mut().push(Probe::RealPath(path.to_owned(), got.clone()));
        got
    }
    fn text(&self, path: &str) -> Option<String> {
        use sha2::Digest as _;
        let bytes = self.files.borrow_mut().read(&self.on_disk(path));
        self.probes.borrow_mut().push(Probe::Text(path.to_owned(), bytes.as_ref().map(|b| sha2::Sha256::digest(b).into())));
        String::from_utf8(bytes?).ok()
    }
}

impl Disk<'_> {
    fn real_path_unrecorded(&self, path: &str) -> String {
        // A path below the root is spelled from the root; one linked from outside it keeps its own spelling, which
        // `relative` refuses: nothing above the root is read.
        let Ok(real) = std::fs::canonicalize(self.on_disk(path)) else { return path.to_owned() };
        match self.canonical.as_ref().and_then(|c| real.strip_prefix(c).ok()) {
            Some(rest) if rest.as_os_str().is_empty() => self.base.clone(),
            Some(rest) => format!("{}/{}", self.base, rest.to_string_lossy().replace('\\', "/")),
            None => format!("/{}", real.to_string_lossy().replace('\\', "/").trim_start_matches('/')),
        }
    }
}

/// The `compilerOptions` of the configuration at `file` (from the root), the chain it `extends` merged beneath its
/// own (later bases over earlier ones), with `baseUrl` resolved from the root where each configuration of the chain
/// sets it. A base is a relative or absolute path, or a package's configuration in a `node_modules` folder at or
/// above it (`tsconfig::extends_path`); one outside the root is not read. `None` where it cannot be read.
fn compiler_options(disk: &Disk<'_>, file: &str, depth: usize) -> Option<serde_json::Map<String, Value>> {
    use tsconfig::ConfigFiles;
    let text = disk.text(&format!("{}/{file}", disk.base))?;
    let json: Value = serde_json::from_str(&tsconfig::strip_jsonc(&text)).ok()?;
    let dir = file.rsplit_once('/').map_or("", |(d, _)| d);
    let mut merged = serde_json::Map::new();
    if depth < tsconfig::MAX_EXTENDS_DEPTH {
        let directory = if dir.is_empty() { disk.base.clone() } else { format!("{}/{dir}", disk.base) };
        for e in tsconfig::extends_of(&json) {
            let base = tsconfig::extends_path(disk, e, &directory);
            if let Some(opts) = base.as_deref().and_then(|b| disk.relative(b)).and_then(|b| compiler_options(disk, b, depth + 1)) {
                merged.extend(opts);
            }
        }
    }
    if let Some(Value::Object(own)) = json.get("compilerOptions") {
        for (k, v) in own {
            let v = match (k.as_str(), v) {
                ("baseUrl", Value::String(u)) => Value::String(join(dir, u)),
                _ => v.clone(),
            };
            merged.insert(k.clone(), v);
        }
    }
    Some(merged)
}

/// The project configuration of each folder that has one, by folder: the first of `TSCONFIG_NAMES` in it.
fn tsconfig_files(rows: &Rows) -> BTreeMap<String, String> {
    let name = |f: &str| f.rsplit_once('/').map_or(f, |(_, n)| n).to_owned();
    let mut configs: BTreeMap<String, String> = BTreeMap::new();
    for f in &rows.file {
        let n = name(f);
        if let Some(rank) = TSCONFIG_NAMES.iter().position(|c| *c == n) {
            let dir = f.rsplit_once('/').map_or("", |(d, _)| d).to_owned();
            let keep = configs.get(&dir).is_none_or(|have| TSCONFIG_NAMES.iter().position(|c| *c == name(have)) > Some(rank));
            if keep {
                configs.insert(dir, f.clone());
            }
        }
    }
    configs
}

/// `tsconfig`, `tsconfig_path` of each project configuration (`configs`).
fn tsconfig_rows(disk: &Disk<'_>, configs: &BTreeMap<String, String>, rows: &mut Rows) {
    for (dir, file) in configs {
        let Some(opts) = compiler_options(disk, file, 0) else { continue };
        let base = opts.get("baseUrl").and_then(Value::as_str).map_or_else(|| dir.clone(), str::to_owned);
        rows.tsconfig.push((dir.clone(), base));
        if let Some(Value::Object(paths)) = opts.get("paths") {
            for (pattern, targets) in paths {
                for t in targets.as_array().into_iter().flatten().filter_map(Value::as_str) {
                    rows.tsconfig_path.push((dir.clone(), pattern.clone(), t.to_owned()));
                }
            }
        }
    }
}

/// `tsconfig_for` of each document: the nearest folder above it with a project configuration (`configs`).
fn tsconfig_for_rows(docs: &[String], configs: &BTreeMap<String, String>, rows: &mut Rows) {
    for path in docs {
        let mut d = path.rsplit_once('/').map_or("", |(d, _)| d);
        loop {
            if configs.contains_key(d) {
                rows.tsconfig_for.push((path.clone(), d.to_owned()));
                break;
            }
            if d.is_empty() {
                break;
            }
            d = d.rsplit_once('/').map_or("", |(p, _)| p);
        }
    }
}

/// A folder's entries in name order; none where it cannot be read.
fn sorted_entries(dir: &Path) -> Vec<std::fs::DirEntry> {
    let mut entries: Vec<_> = std::fs::read_dir(dir).into_iter().flatten().filter_map(Result::ok).collect();
    entries.sort_by_key(std::fs::DirEntry::file_name);
    entries
}

/// One entry of the tree: a folder the host's walk enters goes on `stack`; a file is a `file` row (up to
/// `MAX_FILES`), and a `package.json`'s path goes in `packages`, by its folder (`settings_rows` reads it).
fn tree_entry(root: &Path, entry: &std::fs::DirEntry, stack: &mut Vec<PathBuf>, packages: &mut BTreeMap<String, String>, rows: &mut Rows) {
    let name = entry.file_name().to_string_lossy().into_owned();
    let path = entry.path();
    let Ok(kind) = entry.file_type() else { return };
    if kind.is_dir() {
        stack.extend((!crate::git_workspace::ignored_directory(&name)).then_some(path));
        return;
    }
    if rows.file.len() >= MAX_FILES {
        return;
    }
    let rel = path.strip_prefix(root).unwrap_or(&path).to_string_lossy().replace('\\', "/");
    if name == "package.json" {
        let dir = rel.rsplit_once('/').map_or("", |(d, _)| d).to_owned();
        packages.insert(dir, rel.clone());
    }
    rows.file.push(rel);
}

/// `package` and `package_field` of each `package.json` (`packages`, by folder) that parses to an object.
fn package_rows(disk: &Disk<'_>, packages: &BTreeMap<String, String>, rows: &mut Rows) {
    use tsconfig::ConfigFiles;
    for (dir, file) in packages {
        let json: Option<Value> = disk.text(&format!("{}/{file}", disk.base)).and_then(|t| serde_json::from_str(&t).ok());
        let Some(json @ Value::Object(o)) = &json else { continue };
        if let Some(Value::String(name)) = o.get("name") {
            rows.package.push((dir.clone(), name.clone()));
        }
        let mut fields = Vec::new();
        strings("", json, &mut fields);
        rows.package_field.extend(fields.into_iter().map(|(k, v)| (dir.clone(), k, v)));
    }
}

/// `package_for` of each document: the nearest folder above it that holds a `package.json`.
fn package_for_rows(docs: &[String], packages: &BTreeMap<String, String>, rows: &mut Rows) {
    for path in docs {
        let mut d = path.rsplit_once('/').map_or("", |(d, _)| d);
        loop {
            if packages.contains_key(d) {
                rows.package_for.push((path.clone(), d.to_owned()));
                break;
            }
            if d.is_empty() {
                break;
            }
            d = d.rsplit_once('/').map_or("", |(p, _)| p);
        }
    }
}

/// The repository inputs a program reads (`reads`), interned in `store`, by relation.
pub fn relations(store: &dyn FactStore, root: &Path, docs: &[String], opts: &Value, reads: &dyn Fn(&str) -> bool, files: &mut dyn Files) -> StoreResult<Vec<(&'static str, Relation)>> {
    let mut rows = Rows::default();
    if reads("option") || reads("option_item") {
        options(opts, &mut rows);
    }
    // The configuration files' rows kept from an earlier run where every question their reading asked of the disk is
    // answered as it was (decision 0665): none of them is read. Else read, and kept.
    let mut kept: Option<Vec<Vec<Vec<i64>>>> = None;
    let mut keep: Option<([u8; 32], [u8; 32], Vec<Probe>)> = None;
    if ["file", "package", "package_field", "package_for", "tsconfig", "tsconfig_for", "tsconfig_path"].iter().any(|r| reads(r)) {
        let (packages, configs) = walk(root, docs, &mut rows);
        let (space, key) = settings_key(root, &packages, &configs);
        let disk = Disk::new(root, files);
        kept = kept_settings(store, &space, &key, &disk)?;
        if kept.is_none() {
            read_settings(&disk, &packages, &configs, &mut rows);
            keep = Some((space, key, disk.probes.take()));
        }
    }
    let mut texts: Vec<&str> = Vec::new();
    rows.option.iter().for_each(|(a, b)| texts.extend([a.as_str(), b.as_str()]));
    rows.option_item.iter().for_each(|(a, _, b)| texts.extend([a.as_str(), b.as_str()]));
    texts.extend(rows.file.iter().map(String::as_str));
    rows.package.iter().for_each(|(a, b)| texts.extend([a.as_str(), b.as_str()]));
    rows.package_field.iter().for_each(|(a, b, c)| texts.extend([a.as_str(), b.as_str(), c.as_str()]));
    rows.package_for.iter().for_each(|(a, b)| texts.extend([a.as_str(), b.as_str()]));
    rows.tsconfig.iter().chain(&rows.tsconfig_for).for_each(|(a, b)| texts.extend([a.as_str(), b.as_str()]));
    rows.tsconfig_path.iter().for_each(|(a, b, c)| texts.extend([a.as_str(), b.as_str(), c.as_str()]));
    let atoms = store.intern_atoms(&texts)?;
    let mut next = atoms.iter().map(|&a| i64::from(a));
    let mut sym = || next.next().expect("an atom per string");
    let mut out = Vec::new();
    let mut make = |name: &'static str, columns: &[&str], rows: Vec<Vector<i64>>| {
        let mut r = Relation::new(columns);
        rows.iter().for_each(|row| r.push(row));
        out.push((name, r));
    };
    let option: Vec<Vector<i64>> = rows.option.iter().map(|_| rerail_blocks::vector![sym(), sym()]).collect();
    make("option", &["name", "v"], option);
    let item = rows.option_item.iter().map(|(_, i, _)| {
        let name = sym();
        rerail_blocks::vector![name, *i, sym()]
    });
    let item: Vec<Vector<i64>> = item.collect();
    make("option_item", &["name", "i", "v"], item);
    let file: Vec<Vector<i64>> = rows.file.iter().map(|_| rerail_blocks::vector![sym()]).collect();
    make("file", &["path"], file);
    let package: Vec<Vec<i64>> = rows.package.iter().map(|_| vec![sym(), sym()]).collect();
    let field: Vec<Vec<i64>> = rows.package_field.iter().map(|_| vec![sym(), sym(), sym()]).collect();
    let pfor: Vec<Vec<i64>> = rows.package_for.iter().map(|_| vec![sym(), sym()]).collect();
    let tsconfig: Vec<Vec<i64>> = rows.tsconfig.iter().chain(&rows.tsconfig_for).map(|_| vec![sym(), sym()]).collect();
    let (tsconfig, tfor) = {
        let mut tsconfig = tsconfig;
        let tfor = tsconfig.split_off(rows.tsconfig.len());
        (tsconfig, tfor)
    };
    let tpath: Vec<Vec<i64>> = rows.tsconfig_path.iter().map(|_| vec![sym(), sym(), sym()]).collect();
    // `KEPT_SETTINGS`' order.
    let mut settings = vec![package, field, tsconfig, tpath];
    if let Some(k) = kept {
        settings = k;
    } else if let Some((space, key, probes)) = keep {
        keep_settings(store, space, key, &settings, &probes);
    }
    let [package, field, tsconfig, tpath]: [Vec<Vec<i64>>; 4] = settings.try_into().expect("the four settings relations");
    let rows_of = |r: Vec<Vec<i64>>| -> Vec<Vector<i64>> { r.into_iter().map(|row| row.into_iter().collect()).collect() };
    make("package", &["dir", "name"], rows_of(package));
    make("package_field", &["dir", "key", "v"], rows_of(field));
    make("package_for", &["path", "dir"], rows_of(pfor));
    make("tsconfig", &["dir", "base_url"], rows_of(tsconfig));
    make("tsconfig_for", &["path", "dir"], rows_of(tfor));
    make("tsconfig_path", &["dir", "pattern", "target"], rows_of(tpath));
    Ok(out)
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::content_store::ContentStore;

    #[test]
    fn options_flatten_as_rrlab_reads_them() {
        let mut rows = Rows::default();
        options(&serde_json::json!({"max": 3, "allow": ["a", "b"], "o": {"x": true}}), &mut rows);
        assert_eq!(rows.option, [("allow", "a"), ("allow", "b"), ("max", "3"), ("o/x", "true")].map(|(a, b)| (a.to_owned(), b.to_owned())));
        assert_eq!(rows.option_item, [("allow".to_owned(), 0, "a".to_owned()), ("allow".to_owned(), 1, "b".to_owned())]);
        let mut rows = Rows::default();
        options(&serde_json::json!(["always", {"ignoreCase": false}]), &mut rows);
        assert_eq!(rows.option, [("ignoreCase", "false"), ("option", "always")].map(|(a, b)| (a.to_owned(), b.to_owned())));
    }

    #[test]
    fn the_tree_gives_files_and_packages() {
        let root = std::env::temp_dir().join(format!("rerail-repo-{}-{:?}", std::process::id(), std::thread::current().id()));
        for (p, t) in [("web/package.json", r#"{"name": "@a/web", "engines": {"node": ">=18"}, "n": 1}"#), ("web/src/a.ts", ""), ("node_modules/x/b.js", ""), ("vendor/v.php", ""), ("c.py", "")] {
            let p = root.join(p);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, t).unwrap();
        }
        let mut rows = Rows::default();
        tree(&root, &["web/src/a.ts".to_owned(), "c.py".to_owned()], &mut rows, &mut Plain);
        assert_eq!(rows.file, ["c.py", "web/package.json", "web/src/a.ts"]);
        assert_eq!(rows.package, [("web".to_owned(), "@a/web".to_owned())]);
        assert!(rows.package_field.contains(&("web".to_owned(), "engines/node".to_owned(), ">=18".to_owned())));
        assert!(!rows.package_field.iter().any(|(_, k, _)| k == "n"));
        assert_eq!(rows.package_for, [("web/src/a.ts".to_owned(), "web".to_owned())]);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The configuration files' rows are kept (decision 0665): a run whose configuration files, and every file their
    /// reading looked at, are as the last run read them reads none of them; an edit of one, or a base its chain
    /// `extends` appearing, is read again, and the rows are a fresh reading's.
    #[test]
    fn kept_settings_are_not_read_again_while_their_files_are_unchanged() {
        struct Counted(usize);
        impl Files for Counted {
            fn digest(&mut self, path: &Path) -> Option<[u8; 32]> {
                Plain.digest(path)
            }
            fn read(&mut self, path: &Path) -> Option<Vec<u8>> {
                self.0 += 1;
                Plain.read(path)
            }
        }
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("repo");
        let write = |rel: &str, text: &str| {
            let path = root.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        };
        write("web/package.json", r#"{"name": "@a/web", "engines": {"node": ">=18"}}"#);
        write("web/tsconfig.json", r#"{"extends": "./base", "compilerOptions": {"paths": {"x": ["y"]}}}"#);
        write("web/src/a.ts", "");
        // The store's folder is the user's alone, whatever the umask (decision 0574).
        rerail_blocks::mapped::make_folder(&tmp.path().join("store")).unwrap();
        let store = ContentStore::open(&tmp.path().join("store").join("store.db")).unwrap();
        let docs = ["web/src/a.ts".to_owned()];
        let all = |_: &str| true;
        // The settings relations a run binds, by name, each as its sorted rows.
        let run = |counted: &mut Counted| -> BTreeMap<&'static str, Vec<Vec<i64>>> {
            use crate::code_rule_dsl::facts::vtab::Source;
            relations(&store, &root, &docs, &Value::Null, &all, counted)
                .unwrap()
                .into_iter()
                .filter(|(name, _)| KEPT_SETTINGS.iter().any(|(k, _)| k == name))
                .map(|(name, r)| {
                    let mut rows: Vec<Vec<i64>> = (0..r.len()).map(|i| (0..r.columns().len()).map(|c| r.value(i, c)).collect()).collect();
                    rows.sort_unstable();
                    (name, rows)
                })
                .collect()
        };
        // A fresh reading's rows, interned in the same store.
        let fresh = || -> BTreeMap<&'static str, Vec<Vec<i64>>> {
            let mut rows = Rows::default();
            tree(&root, &docs, &mut rows, &mut Plain);
            let atoms = |texts: Vec<&str>| -> Vec<i64> { store.intern_atoms(&texts).unwrap().into_iter().map(i64::from).collect() };
            let mut out = BTreeMap::new();
            let mut put = |name: &'static str, got: Vec<Vec<i64>>| {
                let mut got = got;
                got.sort_unstable();
                out.insert(name, got);
            };
            put("package", rows.package.iter().map(|(a, b)| atoms(vec![a, b])).collect());
            put("package_field", rows.package_field.iter().map(|(a, b, c)| atoms(vec![a, b, c])).collect());
            put("tsconfig", rows.tsconfig.iter().map(|(a, b)| atoms(vec![a, b])).collect());
            put("tsconfig_path", rows.tsconfig_path.iter().map(|(a, b, c)| atoms(vec![a, b, c])).collect());
            out
        };
        let mut first = Counted(0);
        let rows = run(&mut first);
        assert_eq!(first.0, 2, "the first run reads both");
        assert_eq!(rows, fresh());
        let mut again = Counted(0);
        assert_eq!(run(&mut again), rows);
        assert_eq!(again.0, 0, "an unchanged run reads no configuration file");
        // A source edit reads none either.
        write("web/src/a.ts", "export const a = 1;\n");
        let mut edited = Counted(0);
        assert_eq!(run(&mut edited), rows);
        assert_eq!(edited.0, 0);
        // The package's edit is read.
        write("web/package.json", r#"{"name": "@a/web2"}"#);
        let mut package = Counted(0);
        let got = run(&mut package);
        assert!(package.0 > 0);
        assert_eq!(got, fresh());
        assert_ne!(got["package"], rows["package"]);
        // The base the configuration extends appears: it was looked for, so it is read now.
        write("web/base.json", r#"{"compilerOptions": {"baseUrl": "src"}}"#);
        let mut base = Counted(0);
        let got = run(&mut base);
        assert!(base.0 > 0);
        assert_eq!(got, fresh());
        assert_ne!(got["tsconfig"], rows["tsconfig"]);
    }

    /// A project configuration is read as TypeScript reads it (decision 0670): JSONC, a chain of `extends` merged
    /// beneath its own options (a relative base without `.json`, a package's configuration in `node_modules` by its
    /// `tsconfig` field), and each `baseUrl` from the configuration that sets it.
    #[test]
    fn a_project_configuration_is_read_with_the_chain_it_extends() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let write = |rel: &str, text: &str| {
            let path = root.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        };
        write("app/tsconfig.json", "\u{feff}{\n  // the app\n  \"extends\": [\"@org/cfg\", \"./base\"],\n  \"compilerOptions\": {\n    \"paths\": { \"~/*\": [\"src/*\",], },\n  },\n}\n");
        write("app/base.json", "{ /* shared */ \"compilerOptions\": { \"baseUrl\": \"./src\" } }");
        write("node_modules/@org/cfg/package.json", "{ \"name\": \"@org/cfg\", \"tsconfig\": \"strict.json\" }");
        write("node_modules/@org/cfg/strict.json", "{ \"compilerOptions\": { \"baseUrl\": \"lib\", \"paths\": { \"x\": [\"y\"] } } }");
        write("app/src/a.ts", "export const a = 1;\n");
        write("b.ts", "export const b = 1;\n");
        let mut rows = Rows::default();
        tree(root, &["app/src/a.ts".to_owned(), "b.ts".to_owned()], &mut rows, &mut Plain);
        // `./base` is merged over `@org/cfg` (later over earlier), the app's own `paths` over both.
        assert_eq!(rows.tsconfig, vec![("app".to_owned(), "app/src".to_owned())]);
        assert_eq!(rows.tsconfig_path, vec![("app".to_owned(), "~/*".to_owned(), "src/*".to_owned())]);
        assert_eq!(rows.tsconfig_for, vec![("app/src/a.ts".to_owned(), "app".to_owned())]);
        // The package's configuration alone: its `baseUrl` from its own folder.
        let mut plain = Plain;
        let disk = Disk::new(root, &mut plain);
        let base = compiler_options(&disk, "node_modules/@org/cfg/strict.json", 0).unwrap();
        assert_eq!(base.get("baseUrl"), Some(&Value::String("node_modules/@org/cfg/lib".to_owned())));
    }
}
