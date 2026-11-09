//! Repository policy files become typed facts once, before document partitioning.
//! Readers cache bytes and decoded documents across selectors; a failed selector publishes no prefix.
use super::{Error, Result, front};
use front::Value;
use serde_json::Value as Json;
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

#[derive(Clone, Debug)]
struct Failure {
    kind: &'static str,
    detail: String,
}
type ReadResult<T> = std::result::Result<T, Failure>;
fn malformed(detail: impl Into<String>) -> Failure { Failure { kind: "malformed", detail: detail.into() } }

pub(in crate::code_rule_dsl) fn parameters(text: &str) -> Result<BTreeMap<String, String>> {
    let mut parts = Vec::new();
    let (mut start, mut quoted, mut escaped) = (0, false, false);
    for (i, c) in text.char_indices() {
        if escaped { escaped = false; continue; }
        match c {
            '\\' if quoted => escaped = true,
            '"' => quoted = !quoted,
            ',' if !quoted => { parts.push(&text[start..i]); start = i + 1; }
            _ => {}
        }
    }
    parts.push(&text[start..]);
    let mut out = BTreeMap::new();
    for part in parts {
        let Some((key, value)) = part.split_once('=') else { return Err(Error::Plan(format!("invalid input parameter: {part}"))); };
        let (key, value) = (key.trim(), value.trim());
        if !["IO", "filename", "dir", "path", "records", "at", "artifact", "query"].contains(&key) {
            return Err(Error::Plan(format!("unsupported input parameter: {key}")));
        }
        let value = if value.starts_with('"') {
            serde_json::from_str::<String>(value).map_err(|e| Error::Plan(format!("invalid input parameter {key}: {e}")))?
        } else { value.to_owned() };
        if out.insert(key.to_owned(), value).is_some() { return Err(Error::Plan(format!("duplicate input parameter: {key}"))); }
    }
    Ok(out)
}

/// JSON pointer selectors retain the key or index of each wildcard as a leading column.
fn select<'a>(value: &'a Json, steps: &[&str], prefix: Vec<Json>, out: &mut Vec<(Vec<Json>, &'a Json)>) {
    let Some((step, rest)) = steps.split_first() else { out.push((prefix, value)); return; };
    if *step == "*" {
        let members: Vec<(Json, &Json)> = match value {
            Json::Object(map) => map.iter().map(|(k, v)| (Json::String(k.clone()), v)).collect(),
            Json::Array(items) => items.iter().enumerate().map(|(i, v)| (Json::from(i), v)).collect(),
            _ => Vec::new(),
        };
        for (key, child) in members {
            let mut next = prefix.clone(); next.push(key);
            select(child, rest, next, out);
        }
    } else {
        let key = step.replace("~1", "/").replace("~0", "~");
        if let Some(child) = match value {
            Json::Object(map) => map.get(&key),
            Json::Array(items) => key.parse::<usize>().ok().and_then(|i| items.get(i)),
            _ => None,
        } { select(child, rest, prefix, out); }
    }
}

fn scalar(value: &Json, ty: &str, text: bool) -> ReadResult<Value> {
    match (ty, value) {
        ("sym", Json::String(s)) => Ok(Value::Str(s.clone())),
        ("int", Json::Number(n)) => n.as_i64().map(Value::Int).ok_or_else(|| malformed("number is outside the signed integer domain")),
        ("int", Json::Bool(b)) => Ok(Value::Int(i64::from(*b))),
        ("int", Json::String(s)) if text => s.parse().map(Value::Int).map_err(|_| malformed("invalid integer field")),
        _ => Err(malformed(format!("expected a {ty} scalar, found {value}"))),
    }
}

fn typed(row: Vec<Json>, columns: &[(String, String)], text: bool) -> ReadResult<Vec<Value>> {
    if row.len() != columns.len() { return Err(malformed(format!("expected {} columns, found {}", columns.len(), row.len()))); }
    row.iter().zip(columns).map(|(v, (_, ty))| scalar(v, ty, text)).collect()
}

fn json_rows(value: &Json, pointer: &str, records: bool, prefix: Vec<Json>, columns: &[(String, String)]) -> ReadResult<Vec<Vec<Value>>> {
    if !pointer.is_empty() && !pointer.starts_with('/') { return Err(malformed("path must be a JSON pointer")); }
    let steps: Vec<_> = if pointer.is_empty() { Vec::new() } else { pointer[1..].split('/').collect() };
    let mut selected = Vec::new();
    select(value, &steps, prefix, &mut selected);
    let mut out = Vec::new();
    for (prefix, value) in selected {
        let rows: Vec<Vec<Json>> = if records {
            let Json::Array(items) = value else { return Err(malformed("records=true requires a list of objects")); };
            items.iter().map(|item| {
                let Json::Object(map) = item else { return Err(malformed("record is not an object")); };
                columns[prefix.len().min(columns.len())..].iter().map(|(key, _)|
                    map.get(key).cloned().ok_or_else(|| malformed(format!("record is missing {key}")))).collect()
            }).collect::<ReadResult<_>>()?
        } else { match value {
            Json::Object(map) => map.iter().map(|(k, v)| vec![Json::String(k.clone()), v.clone()]).collect(),
            Json::Array(items) => items.iter().map(|v| match v { Json::Array(tuple) => tuple.clone(), _ => vec![v.clone()] }).collect(),
            scalar => vec![vec![scalar.clone()]],
        }};
        for row in rows { out.push(typed(prefix.iter().cloned().chain(row).collect(), columns, false)?); }
    }
    Ok(out)
}

#[derive(Default)]
struct Readers {
    bytes: BTreeMap<String, ReadResult<String>>,
    documents: BTreeMap<(String, String), ReadResult<Json>>,
}
impl Readers {
    fn read<'a>(&'a mut self, path: &str, read: &mut impl FnMut(&str) -> ReadResult<String>) -> ReadResult<&'a str> {
        self.bytes.entry(path.to_owned()).or_insert_with(|| read(path)).as_ref().map(String::as_str).map_err(Clone::clone)
    }
    fn rows(&mut self, path: &str, io: &str, pointer: &str, records: bool, prefix: Vec<Json>, columns: &[(String, String)], read: &mut impl FnMut(&str) -> ReadResult<String>) -> ReadResult<Vec<Vec<Value>>> {
        if matches!(io, "json" | "yaml") {
            let key = (path.to_owned(), io.to_owned());
            if !self.documents.contains_key(&key) {
                let decoded = self.read(path, read).and_then(|s| if io == "json" {
                    serde_json::from_str(s).map_err(|e| malformed(e.to_string()))
                } else { serde_yaml_ng::from_str::<Json>(s).map_err(|e| malformed(e.to_string())) });
                self.documents.insert(key.clone(), decoded);
            }
            let document = self.documents[&key].as_ref().map_err(Clone::clone)?;
            return json_rows(document, pointer, records, prefix, columns);
        }
        let source = self.read(path, read)?;
        // Repository text readers use universal newlines. No line is manufactured after a final separator.
        let source = source.replace("\r\n", "\n").replace('\r', "\n");
        let mut out = Vec::new();
        for (i, line) in source.split_terminator('\n').enumerate() {
            let mut row = prefix.clone();
            if matches!(io, "lines" | "jsonl") { row.push(Json::from(i + 1)); }
            if io == "jsonl" {
                let value = serde_json::from_str(line).map_err(|e| malformed(format!("line {}: {e}", i + 1)))?;
                out.extend(json_rows(&value, pointer, records, row, columns)?);
            } else {
                row.extend(if io == "lines" { vec![Json::String(line.to_owned())] }
                           else { line.split('\t').map(|s| Json::String(s.to_owned())).collect() });
                out.push(typed(row, columns, true)?);
            }
        }
        Ok(out)
    }
}

/// Where `artifact=` inputs are read: `--facts DIR` (`set_facts_dir`), else `<root>/.rerail/facts`
/// (data-layer/RUN-FACTS.md section 2). Never the repository tree itself.
static FACTS_DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// `logic --facts DIR`: the run's facts directory, for the process.
pub(in crate::code_rule_dsl) fn set_facts_dir(dir: PathBuf) -> std::result::Result<(), String> {
    FACTS_DIR.set(dir).map_err(|_| "the facts directory is already set".to_owned())
}

fn facts_dir(root: &Path) -> PathBuf {
    FACTS_DIR.get().cloned().unwrap_or_else(|| root.join(".rerail").join("facts"))
}

/// `logic --require-facts` (data-layer/RUN-FACTS.md section 5, the CI column): an artifact that is missing,
/// stale or malformed fails the run, and so does a failed one unless the policy reports it.
static REQUIRE_FACTS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub(in crate::code_rule_dsl) fn require_facts() {
    REQUIRE_FACTS.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// The revision `git_rev("base", …)` names, which `at="base"` inputs are read at: `logic --base REV`, a commit id
/// or a reference name (`origin/main`). The merge base of a change is the caller's to compute until the git
/// provider gives `git_rev` (decision 0094, order of work).
static BASE: std::sync::OnceLock<String> = std::sync::OnceLock::new();

pub(in crate::code_rule_dsl) fn set_base(rev: String) -> std::result::Result<(), String> {
    BASE.set(rev).map_err(|_| "the base revision is already set".to_owned())
}

fn unreadable(detail: impl std::fmt::Display) -> Failure { Failure { kind: "unreadable", detail: detail.to_string() } }

/// The tree of `rev` in the repository that holds `root`, and `root`'s path inside it.
fn base_tree(root: &Path, rev: &str) -> ReadResult<(gix::Repository, gix::ObjectId, PathBuf)> {
    let repo = gix::discover(root).map_err(|e| unreadable(format!("no repository holds {}: {e}", root.display())))?;
    let resolve = || -> std::result::Result<gix::ObjectId, Box<dyn std::error::Error + Send + Sync>> {
        let commit = match gix::ObjectId::from_hex(rev.as_bytes()) {
            Ok(id) => id,
            Err(_) => repo.find_reference(rev)?.peel_to_id()?.detach(),
        };
        Ok(repo.find_object(commit)?.peel_to_tree()?.id)
    };
    let tree = resolve().map_err(|e| unreadable(format!("base revision {rev}: {e}")))?;
    let workdir = repo.workdir().ok_or_else(|| unreadable("the repository has no work tree"))?;
    let canonical = |p: &Path| p.canonicalize().map_err(|e| unreadable(format!("{}: {e}", p.display())));
    let prefix = canonical(root)?.strip_prefix(canonical(workdir)?).map(Path::to_path_buf).map_err(unreadable)?;
    Ok((repo, tree, prefix))
}

/// Reads repository-relative files as they are at the base revision, from the object store; the repository is
/// opened on the first read. A path the base tree lacks is `missing`, as a file the tree lacks is.
fn base_reader<'a>(root: &'a Path, rev: Option<&'a str>) -> impl FnMut(&str) -> ReadResult<String> + 'a {
    let mut tree: Option<ReadResult<(gix::Repository, gix::ObjectId, PathBuf)>> = None;
    move |path| {
        let rev = rev.ok_or_else(|| Failure { kind: "missing", detail: "no base revision: run with --base REV".into() })?;
        let (repo, id, prefix) = tree.get_or_insert_with(|| base_tree(root, rev)).as_ref().map_err(Clone::clone)?;
        let entry = repo.find_tree(*id).map_err(unreadable)?.lookup_entry_by_path(prefix.join(path)).map_err(unreadable)?
            .ok_or_else(|| Failure { kind: "missing", detail: format!("{path} is not in {rev}") })?;
        let blob = entry.object().map_err(unreadable)?;
        String::from_utf8(blob.data.clone()).map_err(|e| malformed(format!("{path} at {rev}: {e}")))
    }
}

/// The format of a stamped artifact (RUN-FACTS.md section 3).
pub(in crate::code_rule_dsl) const FORMAT: &str = "rerail.facts/v1";

/// `.artifact "name" inputs "glob", …`: the name and its input globs.
pub(in crate::code_rule_dsl) fn artifact_declaration(text: &str) -> Result<(String, Vec<String>)> {
    let bad = || Error::Plan(format!(".artifact {text}: expected .artifact \"name\" inputs \"glob\", …"));
    let text = text.trim();
    let (name, rest) = quoted(text).ok_or_else(bad)?;
    let rest = rest.trim_start().strip_prefix("inputs").ok_or_else(bad)?;
    let mut globs = Vec::new();
    let mut rest = rest.trim_start();
    loop {
        let (glob, after) = quoted(rest).ok_or_else(bad)?;
        globs.push(glob);
        rest = after.trim_start();
        if rest.is_empty() {
            break;
        }
        rest = rest.strip_prefix(',').ok_or_else(bad)?.trim_start();
    }
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return Err(Error::Plan(format!(".artifact {name:?}: a name is letters, digits, `_` and `-`")));
    }
    Ok((name, globs))
}

/// A leading JSON string literal and the text after it.
fn quoted(text: &str) -> Option<(String, &str)> {
    let body = text.strip_prefix('"')?;
    let mut escaped = false;
    for (i, c) in body.char_indices() {
        match c {
            _ if escaped => escaped = false,
            '\\' => escaped = true,
            '"' => return serde_json::from_str::<String>(&text[..i + 2]).ok().map(|s| (s, &body[i + 1..])),
            _ => {}
        }
    }
    None
}

/// Every artifact the program declares. Two declarations of one name must agree (RUN-FACTS.md section 2).
pub(in crate::code_rule_dsl) fn artifacts(settings: &[(String, String)]) -> Result<BTreeMap<String, Vec<String>>> {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (_, text) in settings.iter().filter(|(k, _)| k == "artifact") {
        let (name, globs) = artifact_declaration(text)?;
        match out.get(&name) {
            Some(other) if *other != globs => {
                return Err(Error::Plan(format!(".artifact {name:?} is declared twice with different inputs: {other:?} and {globs:?}")));
            }
            _ => {
                out.insert(name, globs);
            }
        }
    }
    Ok(out)
}

/// Git's blob id of `bytes`: SHA-1 over `blob <len>\0` and the bytes.
fn blob_id(bytes: &[u8]) -> String {
    use sha1::Digest as _;
    let mut hash = sha1::Sha1::new();
    hash.update(format!("blob {}\0", bytes.len()).as_bytes());
    hash.update(bytes);
    hash.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// The files of `root` the corpus walk sees (`corpus_files`: no hidden directory, no `target`), relative and sorted.
fn walk(root: &Path) -> std::io::Result<Vec<String>> {
    let mut out = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir)? {
            let path = entry?.path();
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if path.is_dir() {
                if !name.starts_with('.') && name != "target" {
                    pending.push(path);
                }
            } else if let Ok(relative) = path.strip_prefix(root) {
                out.push(relative.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    out.sort();
    Ok(out)
}

/// The digest of an artifact's inputs on the tree at `root` (RUN-FACTS.md section 4): SHA-256 over the sorted lines
/// `path NUL blob-id LF`, one per file the walk sees that a glob matches.
pub(in crate::code_rule_dsl) fn inputs_digest(root: &Path, globs: &[String]) -> std::result::Result<String, String> {
    use sha2::Digest as _;
    let mut set = globset::GlobSetBuilder::new();
    for glob in globs {
        set.add(globset::GlobBuilder::new(glob).literal_separator(true).build().map_err(|e| format!("input glob {glob:?}: {e}"))?);
    }
    let set = set.build().map_err(|e| e.to_string())?;
    let mut hash = sha2::Sha256::new();
    for path in walk(root).map_err(|e| format!("{}: {e}", root.display()))? {
        if !set.is_match(&path) {
            continue;
        }
        let bytes = std::fs::read(root.join(&path)).map_err(|e| format!("{path}: {e}"))?;
        hash.update(path.as_bytes());
        hash.update([0]);
        hash.update(blob_id(&bytes).as_bytes());
        hash.update([b'\n']);
    }
    Ok(format!("sha256:{}", hash.finalize().iter().map(|b| format!("{b:02x}")).collect::<String>()))
}

/// A stamped artifact's `data`, if it is there, well formed, ok and fresh against `digest`.
fn artifact_data(name: &str, text: &str, digest: &str) -> ReadResult<Json> {
    let envelope: Json = serde_json::from_str(text).map_err(|e| malformed(e.to_string()))?;
    let field = |k: &str| envelope.get(k).and_then(Json::as_str);
    if field("format") != Some(FORMAT) {
        return Err(malformed(format!("not a {FORMAT} file")));
    }
    if field("artifact") != Some(name) {
        return Err(malformed(format!("stamped for artifact {:?}", field("artifact").unwrap_or(""))));
    }
    let Some(stamped) = field("inputs") else { return Err(malformed("no inputs digest")) };
    if stamped != digest {
        return Err(Failure { kind: "stale", detail: format!("its inputs changed since it was stamped ({stamped}, now {digest})") });
    }
    match field("status") {
        Some("ok") => envelope.get("data").cloned().ok_or_else(|| malformed("status ok with no data")),
        Some("failed") => Err(Failure { kind: "failed", detail: field("error").unwrap_or("the producer failed").to_owned() }),
        other => Err(malformed(format!("status {other:?}"))),
    }
}

/// `rerail facts stamp RAW --out DIR` (RUN-FACTS.md sections 1 and 3): each `NAME.json` a producer wrote in `raw`
/// wrapped with the digest of the inputs `declared` names for it, as `DIR/NAME.json`. A `NAME.status` (the
/// producer's exit status) makes the artifact failed, with the first 4 KB of `NAME.err`; so does output that is not
/// JSON. A raw file no policy declares is an error. Returns one line per artifact.
pub(in crate::code_rule_dsl) fn stamp(raw: &Path, out: &Path, root: &Path, declared: &BTreeMap<String, Vec<String>>) -> std::result::Result<String, String> {
    let mut names: Vec<String> = std::fs::read_dir(raw)
        .map_err(|e| format!("{}: {e}", raw.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "json" || x == "status"))
        .filter_map(|p| p.file_stem().and_then(|s| s.to_str()).map(str::to_owned))
        .collect();
    names.sort();
    names.dedup();
    std::fs::create_dir_all(out).map_err(|e| format!("{}: {e}", out.display()))?;
    let mut report = String::new();
    for name in names {
        let globs = declared.get(&name).ok_or_else(|| format!("{name}: no policy declares .artifact {name:?}"))?;
        let inputs = inputs_digest(root, globs)?;
        let read = |ext: &str| std::fs::read_to_string(raw.join(format!("{name}.{ext}"))).ok();
        let failure = |why: String| {
            let err = read("err").unwrap_or_default();
            let err: String = err.chars().take(4096).collect();
            if err.is_empty() { why } else { format!("{why}: {err}") }
        };
        let mut envelope = serde_json::json!({ "format": FORMAT, "artifact": name, "inputs": inputs });
        let status = match (read("status"), read("json")) {
            (Some(code), _) => Err(failure(format!("the producer exited with status {}", code.trim()))),
            (None, None) => Err(failure("the producer wrote nothing".to_owned())),
            (None, Some(text)) => serde_json::from_str::<Json>(&text).map_err(|e| failure(format!("the producer's output is not JSON: {e}"))),
        };
        match status {
            Ok(data) => {
                envelope["status"] = "ok".into();
                envelope["data"] = data;
            }
            Err(error) => {
                envelope["status"] = "failed".into();
                envelope["error"] = error.into();
            }
        }
        let text = serde_json::to_string_pretty(&envelope).map_err(|e| e.to_string())?;
        std::fs::write(out.join(format!("{name}.json")), text + "\n").map_err(|e| format!("{name}: {e}"))?;
        report.push_str(&format!("{name}: {} ({inputs})\n", envelope["status"].as_str().unwrap_or("")));
    }
    Ok(report)
}

/// What the readers of the last `populate` read, as the whole-run cutoff records it (`Report::consulted`, `reuse`):
/// `("input", path, digest)` for a repository file, `("input_dir", dir, listing)`, `("artifact", name, digest of
/// the stamped file and of its inputs)` and `("base", "REV\tpath", digest)`. `still` checks each again.
static CONSULTED: std::sync::Mutex<Vec<(String, String, String)>> = std::sync::Mutex::new(Vec::new());

pub(super) fn take_consulted() -> Vec<(String, String, String)> {
    std::mem::take(&mut *CONSULTED.lock().unwrap_or_else(|e| e.into_inner()))
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest as _;
    sha2::Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

fn of_result(result: &ReadResult<String>) -> String {
    match result { Ok(text) => sha256_hex(text.as_bytes()), Err(f) => format!("{}", f.kind) }
}

fn read_repository_file(root: &Path, name: &str) -> ReadResult<String> {
    std::fs::read_to_string(root.join(name)).map_err(|e| Failure {
        kind: if e.kind() == std::io::ErrorKind::NotFound { "missing" } else { "unreadable" }, detail: e.to_string(),
    })
}

/// An artifact's reading: the stamped file's text, and the digest of its declared inputs now.
fn read_artifact(root: &Path, declared: &BTreeMap<String, Vec<String>>, name: &str) -> (ReadResult<String>, ReadResult<String>) {
    let text = std::fs::read_to_string(facts_dir(root).join(format!("{name}.json"))).map_err(|e| Failure {
        kind: if e.kind() == std::io::ErrorKind::NotFound { "missing" } else { "malformed" },
        detail: format!("{}: {e}", facts_dir(root).join(format!("{name}.json")).display()),
    });
    let digest = declared.get(name).ok_or_else(|| Failure { kind: "malformed", detail: format!("no .artifact {name:?} declares its inputs") })
        .and_then(|globs| inputs_digest(root, globs).map_err(|detail| Failure { kind: "malformed", detail }));
    (text, digest)
}

fn artifact_entry(text: &ReadResult<String>, digest: &ReadResult<String>) -> String {
    format!("{}:{}", of_result(text), digest.as_ref().map_or_else(|f| f.kind.to_owned(), Clone::clone))
}

pub(super) fn populate(program: &mut front::Program, root: &Path) -> Result<()> {
    let declared = artifacts(&program.settings)?;
    let base = BASE.get().map(String::as_str);
    let mut consulted = Vec::new();
    let mut base_read = base_reader(root, base);
    let seen = std::cell::RefCell::new(Vec::new());
    populate_with(
        program,
        root,
        &mut |name| {
            let result = read_repository_file(root, name);
            seen.borrow_mut().push(("input".to_owned(), name.to_owned(), of_result(&result)));
            result
        },
        &mut |name| {
            let (text, digest) = read_artifact(root, &declared, name);
            seen.borrow_mut().push(("artifact".to_owned(), name.to_owned(), artifact_entry(&text, &digest)));
            artifact_data(name, &text?, &digest?)
        },
        &mut |path| {
            let result = base_read(path);
            seen.borrow_mut().push(("base".to_owned(), format!("{}\t{path}", base.unwrap_or("")), of_result(&result)));
            result
        },
        &mut consulted,
    )?;
    consulted.extend(seen.into_inner());
    CONSULTED.lock().unwrap_or_else(|e| e.into_inner()).extend(consulted);
    if REQUIRE_FACTS.load(std::sync::atomic::Ordering::Relaxed) {
        required(program)?;
    }
    Ok(())
}

/// Whether the reads `populate` recorded (`take_consulted`) would read the same now, for `program`'s artifact
/// declarations. A base read names its revision as it was given (a reference may move), and is read again.
pub(super) fn still(program: &front::Program, root: &Path, consulted: &[(String, String, String)]) -> bool {
    let Ok(declared) = artifacts(&program.settings) else { return false };
    consulted.iter().all(|(what, path, digest)| {
        let now = match what.as_str() {
            "input" => of_result(&read_repository_file(root, path)),
            "input_dir" => listing(root, path),
            "artifact" => {
                let (text, inputs) = read_artifact(root, &declared, path);
                artifact_entry(&text, &inputs)
            }
            "base" => {
                let Some((rev, file)) = path.split_once('\t') else { return false };
                of_result(&base_reader(root, (!rev.is_empty()).then_some(rev))(file))
            }
            _ => return false,
        };
        now == *digest
    })
}

/// The file names of a `dir=` input's directory, as its reader lists them.
fn listing(root: &Path, dir: &str) -> String {
    let mut names: Vec<String> = std::fs::read_dir(root.join(dir)).into_iter().flatten().flatten()
        .filter(|e| e.path().is_file()).map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    sha256_hex(names.join("\n").as_bytes())
}

/// `--require-facts`: the artifact errors that fail a CI run. A `failed` artifact is the policy's to report when one
/// of its rules reads `input_error` (RUN-FACTS.md section 5); the other kinds always fail.
fn required(program: &front::Program) -> Result<()> {
    let reports = program.rules.iter().any(|r| r.body.iter().filter_map(front::Literal::atom).any(|a| !a.neg && a.rel == "input_error"));
    let failing: Vec<String> = program.facts.get("input_error").into_iter().flatten()
        .filter_map(|row| match row.as_slice() {
            [Value::Str(path), Value::Str(kind), Value::Str(detail)] if path.starts_with("artifact:") && !(reports && kind == "failed") =>
                Some(format!("{path}: {kind}: {detail}")),
            _ => None,
        })
        .collect();
    if failing.is_empty() { Ok(()) } else { Err(Error::Plan(format!("--require-facts: {}", failing.join("; ")))) }
}

fn populate_with(
    program: &mut front::Program,
    root: &Path,
    read: &mut impl FnMut(&str) -> ReadResult<String>,
    artifact: &mut impl FnMut(&str) -> ReadResult<Json>,
    base: &mut impl FnMut(&str) -> ReadResult<String>,
    consulted: &mut Vec<(String, String, String)>,
) -> Result<()> {
    let settings: Vec<_> = program.settings.iter().filter(|(k, _)| k == "input").map(|(_, s)| s.clone()).collect();
    let mut readers = Readers::default();
    let mut base_readers = Readers::default();
    let mut artifacts: BTreeMap<String, ReadResult<Json>> = BTreeMap::new();
    let mut errors: Vec<Vec<Value>> = Vec::new();
    // Relations an artifact left without rows, and why (FINDINGS-LAYER.md section 2).
    let mut incomplete: BTreeMap<String, String> = BTreeMap::new();
    for setting in settings {
        let Some((rel, params)) = setting.strip_suffix(')').and_then(|s| s.split_once('(')) else { return Err(Error::Plan(format!("invalid input setting: {setting}"))); };
        let params = parameters(params)?;
        // `at="base"`: the file as it is at `git_rev("base", …)`. Its readers compare it with the tree, mostly through
        // `!` (a burn-down list that may only shrink), so a base that cannot be read withholds them, as an artifact
        // does.
        let at_base = match params.get("at").map(String::as_str) {
            None => false,
            Some("base") => true,
            Some(other) => return Err(Error::Plan(format!("{rel}: at={other:?}: only \"base\" is a revision an input is read at"))),
        };
        if at_base && (params.contains_key("artifact") || params.contains_key("dir")) {
            return Err(Error::Plan(format!("{rel}: at=\"base\" reads one repository file (filename=)")));
        }
        let io = params.get("IO").map_or("file", String::as_str);
        if !["file", "lines", "json", "yaml", "jsonl"].contains(&io) { return Err(Error::Plan(format!("{rel}: unsupported input reader {io}"))); }
        let records = match params.get("records").map(String::as_str) {
            None | Some("false") => false, Some("true") => true,
            _ => return Err(Error::Plan(format!("{rel}: records must be true or false"))),
        };
        let pointer = params.get("path").map_or("", String::as_str);
        let columns = program.decls.get(rel).ok_or_else(|| Error::Plan(format!("{rel}: input has no declaration")))?;
        // `artifact="name"`: a stamped file of the facts directory, read as IO=json reads its `data`.
        if let Some(name) = params.get("artifact") {
            if params.contains_key("filename") || params.contains_key("dir") {
                return Err(Error::Plan(format!("{rel}: artifact= replaces filename= and dir=")));
            }
            if io != "json" {
                return Err(Error::Plan(format!("{rel}: an artifact is read with IO=json")));
            }
            let data = artifacts.entry(name.clone()).or_insert_with(|| artifact(name));
            let result = data.as_ref().map_err(Clone::clone).and_then(|data| json_rows(data, pointer, records, Vec::new(), columns));
            program.inputs.retain(|n| n != rel);
            match result {
                Ok(rows) => program.facts.entry(rel.to_owned()).or_default().extend(rows),
                Err(error) => {
                    let row = vec![Value::Str(format!("artifact:{name}")), Value::Str(error.kind.to_owned()), Value::Str(error.detail.clone())];
                    if !errors.contains(&row) {
                        errors.push(row);
                    }
                    incomplete.insert(rel.to_owned(), format!("artifact:{name} {}", error.kind));
                }
            }
            continue;
        }
        let filename = params.get("filename");
        let directory = params.get("dir");
        if filename.is_some() == directory.is_some() { return Err(Error::Plan(format!("{rel}: supply exactly one filename or dir"))); }
        let path = filename.or(directory).unwrap();
        if Path::new(path).components().any(|c| matches!(c, Component::ParentDir | Component::RootDir | Component::Prefix(_))) {
            return Err(Error::Plan(format!("{rel}: input path must be repository-relative")));
        }
        let result = if directory.is_some() {
            if io != "lines" { return Err(Error::Plan(format!("{rel}: dir is supported only by IO=lines"))); }
            consulted.push(("input_dir".to_owned(), path.clone(), listing(root, path)));
            match std::fs::read_dir(root.join(path)) {
                Err(e) => Err(Failure { kind: if e.kind() == std::io::ErrorKind::NotFound { "missing" } else { "unreadable" }, detail: e.to_string() }),
                Ok(entries) => {
                    let mut paths = entries.map(|entry| entry.map(|e| e.path())).collect::<std::io::Result<Vec<_>>>()
                        .map_err(|e| Error::Plan(format!("{rel}: listing {path}: {e}")))?;
                    paths.sort();
                    let mut rows = Vec::new();
                    let mut failure = None;
                    for file in paths {
                        if !file.is_file() { continue; }
                        let name = file.strip_prefix(root).unwrap().to_string_lossy().into_owned();
                        match readers.rows(&name, io, pointer, records, vec![Json::String(name.clone())], columns, read) {
                            Ok(part) => rows.extend(part), Err(e) => { failure = Some(e); break; }
                        }
                    }
                    failure.map_or(Ok(rows), Err)
                }
            }
        } else if at_base {
            base_readers.rows(path, io, pointer, records, Vec::new(), columns, base)
        } else { readers.rows(path, io, pointer, records, Vec::new(), columns, read) };
        program.inputs.retain(|name| name != rel);
        match result {
            Ok(rows) => program.facts.entry(rel.to_owned()).or_default().extend(rows),
            Err(error) if at_base => {
                let row = vec![Value::Str(format!("{path}@base")), Value::Str(error.kind.to_owned()), Value::Str(error.detail.clone())];
                if !errors.contains(&row) {
                    errors.push(row);
                }
                incomplete.insert(rel.to_owned(), format!("{path}@base {}", error.kind));
            }
            Err(error) => errors.push(vec![Value::Str(path.clone()), Value::Str(error.kind.to_owned()), Value::Str(error.detail)]),
        }
    }
    if !errors.is_empty() {
        program.inputs.retain(|name| name != "input_error");
        program.facts.entry("input_error".to_owned()).or_default().extend(errors);
    }
    if !incomplete.is_empty() {
        let withheld = withhold(program, &incomplete);
        for (output, why) in withheld.unknown {
            crate::code_rule_dsl::logic::profile::decision("incomplete inputs (FINDINGS-LAYER section 2)", || format!("{output} is Unknown: {why}"));
            eprintln!("unknown: {} ({why}): rows may be missing", output.strip_prefix("rr_place_").unwrap_or(&output));
        }
        for (rel, rules, why) in withheld.rules {
            crate::code_rule_dsl::logic::profile::decision("incomplete inputs (FINDINGS-LAYER section 2)", || format!("{rel}: {rules} rules withheld ({why})"));
        }
    }
    Ok(())
}

/// What `withhold` did: the rules it removed, per head, and the outputs whose rows may now be missing.
#[derive(Debug, Default)]
struct Withheld {
    rules: Vec<(String, usize, String)>,
    unknown: Vec<(String, String)>,
}

/// FINDINGS-LAYER.md section 2, by rule: a relation that reads an incomplete one, in either polarity, is incomplete
/// itself. A rule that reads an incomplete relation through `!` or an aggregate is withheld: with fewer rows its
/// rows could be wrong. A rule that reads it positively only is kept: it derives fewer rows, never a wrong one.
/// Withholding a rule makes its head incomplete, so this runs to a fixpoint.
fn withhold(program: &mut front::Program, incomplete: &BTreeMap<String, String>) -> Withheld {
    use front::Literal;
    let mut why = incomplete.clone();
    let mut removed: BTreeMap<String, (usize, String)> = BTreeMap::new();
    loop {
        let mut grew = false;
        for rule in &program.rules {
            if why.contains_key(&rule.head.rel) {
                continue;
            }
            if let Some(w) = rule.body.iter().filter_map(Literal::atom).find_map(|a| why.get(&a.rel)) {
                let w = w.clone();
                why.insert(rule.head.rel.clone(), w);
                grew = true;
            }
        }
        let mut dropped = Vec::new();
        program.rules.retain(|rule| {
            let reason = rule.body.iter().find_map(|l| match l {
                Literal::Atom(a) if a.neg => why.get(&a.rel),
                Literal::Agg { atom, .. } => why.get(&atom.rel),
                _ => None,
            });
            match reason {
                Some(w) => {
                    dropped.push((rule.head.rel.clone(), w.clone()));
                    false
                }
                None => true,
            }
        });
        for (head, w) in dropped {
            let entry = removed.entry(head.clone()).or_insert((0, w.clone()));
            entry.0 += 1;
            why.entry(head).or_insert(w);
            grew = true;
        }
        if !grew {
            break;
        }
    }
    // The relations findings come from: the outputs, and what `rr_diagnostic` (lowering's findings) reads. Each
    // incomplete one is named with how many of its rules read an incomplete relation (the granularity is the rule).
    let mut reported: Vec<String> = program.outputs.clone();
    for rule in program.rules.iter().filter(|r| r.head.rel == "rr_diagnostic") {
        reported.extend(rule.body.iter().filter_map(Literal::atom).filter(|a| !a.neg).map(|a| a.rel.clone()));
    }
    let mut seen = std::collections::BTreeSet::new();
    reported.retain(|r| seen.insert(r.clone()));
    let unknown = reported
        .into_iter()
        .filter_map(|rel| {
            let w = why.get(&rel)?;
            let rules: Vec<&front::Rule> = program.rules.iter().filter(|r| r.head.rel == rel).collect();
            let reading = rules.iter().filter(|r| r.body.iter().filter_map(Literal::atom).any(|a| why.contains_key(&a.rel))).count();
            let gone = removed.get(&rel).map_or(0, |(n, _)| *n);
            Some((rel, format!("{w}; {} of its {} rules", reading + gone, rules.len() + gone)))
        })
        .collect();
    Withheld { rules: removed.into_iter().map(|(rel, (n, w))| (rel, n, w)).collect(), unknown }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn rows(io: &str, source: &str, pointer: &str, records: bool, columns: &[(&str, &str)]) -> ReadResult<Vec<Vec<Value>>> {
        let cols = columns.iter().map(|(name, ty)| ((*name).to_owned(), (*ty).to_owned())).collect::<Vec<_>>();
        Readers::default().rows("fixture", io, pointer, records, Vec::new(), &cols, &mut |_| Ok(source.to_owned()))
    }

    #[test]
    fn line_and_tab_readers_preserve_empty_fields_and_universal_newlines() {
        assert_eq!(rows("lines", "first\r\n\rthird\n", "", false, &[("line", "int"), ("text", "sym")]).unwrap(), vec![
            vec![Value::Int(1), Value::Str("first".into())], vec![Value::Int(2), Value::Str("".into())],
            vec![Value::Int(3), Value::Str("third".into())]]);
        assert!(rows("lines", "", "", false, &[("line", "int"), ("text", "sym")]).unwrap().is_empty());
        assert_eq!(rows("file", "\t7\n", "", false, &[("name", "sym"), ("limit", "int")]).unwrap(),
            vec![vec![Value::Str("".into()), Value::Int(7)]]);
    }

    #[test]
    fn yaml_and_json_lines_share_strict_typed_projection() {
        assert_eq!(rows("yaml", "counts:\n  first: 7\n", "/counts", false, &[("key", "sym"), ("n", "int")]).unwrap(),
            vec![vec![Value::Str("first".into()), Value::Int(7)]]);
        assert_eq!(rows("jsonl", "{\"n\":7}\n{\"n\":8}\n", "/n", false, &[("line", "int"), ("n", "int")]).unwrap(),
            vec![vec![Value::Int(1), Value::Int(7)], vec![Value::Int(2), Value::Int(8)]]);
        assert!(rows("jsonl", "7\nnot json\n", "", false, &[("line", "int"), ("n", "int")]).is_err());
        assert!(rows("json", "[7,\"8\"]", "", false, &[("n", "int")]).is_err());
        assert!(rows("json", "[7,9223372036854775808]", "", false, &[("n", "int")]).is_err());
    }

    #[test]
    fn selectors_decode_pointer_escapes_and_reject_incomplete_records() {
        assert_eq!(rows("json", r#"{"a/b":{"~":["one","two"]}}"#, "/a~1b/~0/*", false, &[("index", "int"), ("name", "sym")]).unwrap(),
            vec![vec![Value::Int(0), Value::Str("one".into())], vec![Value::Int(1), Value::Str("two".into())]]);
        assert!(rows("json", r#"[{"name":"one"},{}]"#, "", true, &[("name", "sym")]).is_err());
        assert!(parameters(r#"IO=json, filename="a,b.json", filename="other.json""#).is_err());
        assert_eq!(parameters(r#"IO=json, filename = "a,b\".json""#).unwrap()["filename"], "a,b\".json");
    }

    fn none(_: &str) -> ReadResult<Json> { panic!("no artifact is read") }

    fn no_base(_: &str) -> ReadResult<String> { panic!("no file is read at the base revision") }

    /// A burn-down list that may only shrink: its entries at the base revision and in the tree.
    const BURN_DOWN: &str = r#"
.decl now(n: number, e: symbol)
.input now(IO=lines, filename="allow.txt")
.decl was(n: number, e: symbol)
.input was(IO=lines, filename="allow.txt", at="base")
.decl now_entry(e: symbol)
now_entry(e) :- now(_, e).
.decl grew(e: symbol)
grew(e) :- now_entry(e), !was(_, e).
.output grew
"#;

    fn burn_down(base: &mut impl FnMut(&str) -> ReadResult<String>) -> front::Program {
        let mut program = crate::code_rule_dsl::logic::rr::parse(BURN_DOWN).unwrap();
        populate_with(&mut program, Path::new("."), &mut |_| Ok("a\nb\nc\n".to_owned()), &mut none, base, &mut Vec::new()).unwrap();
        program
    }

    #[test]
    fn a_base_input_reads_the_file_at_the_base_revision() {
        let mut reads = Vec::new();
        let program = burn_down(&mut |path| { reads.push(path.to_owned()); Ok("a\nb\n".to_owned()) });
        assert_eq!(reads, ["allow.txt"]);
        assert_eq!(program.facts["was"], vec![vec![Value::Int(1), Value::Str("a".into())], vec![Value::Int(2), Value::Str("b".into())]]);
        assert_eq!(program.facts["now"].len(), 3, "the tree's reading is its own");
        assert!(program.rules.iter().any(|r| r.head.rel == "grew"));
    }

    /// Without a base, the comparison is withheld rather than reporting every entry as new.
    #[test]
    fn an_unreadable_base_gives_an_input_error_and_withholds_the_comparison() {
        let program = burn_down(&mut |_| Err(Failure { kind: "missing", detail: "no base revision".into() }));
        assert_eq!(error_kinds(&program), [r#"Str("allow.txt@base") Str("missing")"#]);
        assert!(!program.rules.iter().any(|r| r.head.rel == "grew"), "a rule reading the base through ! is withheld");
        assert!(program.rules.iter().any(|r| r.head.rel == "now_entry"));
    }

    #[test]
    fn only_the_base_revision_is_read_and_only_from_a_file() {
        for input in [r#"at="head", filename="a.json""#, r#"at="base", dir="allow/""#, r#"at="base", artifact="x""#] {
            let mut program = crate::code_rule_dsl::logic::rr::parse(&format!(".decl a(n: number)\n.input a(IO=json, {input})\n")).unwrap();
            assert!(populate_with(&mut program, Path::new("."), &mut |_| panic!("not read"), &mut none, &mut no_base, &mut Vec::new()).is_err(), "{input}");
        }
    }

    /// The base reader over a real repository: a commit, then an edit in the work tree. The root is a
    /// subdirectory, so paths are read under its prefix.
    #[test]
    fn the_base_reader_reads_the_object_store() {
        let dir = std::env::temp_dir().join(format!("rerail-base-{}", std::process::id()));
        let root = dir.join("sub");
        std::fs::create_dir_all(&root).unwrap();
        let git = |args: &[&str]| assert!(std::process::Command::new("git").arg("-C").arg(&dir).args(args).output().unwrap().status.success(), "git {args:?}");
        git(&["init", "-q"]);
        std::fs::write(root.join("allow.txt"), "a\nb\n").unwrap();
        git(&["add", "sub/allow.txt"]);
        git(&["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", "base"]);
        std::fs::write(root.join("allow.txt"), "a\nb\nc\n").unwrap();
        let head = String::from_utf8(std::process::Command::new("git").arg("-C").arg(&dir).args(["rev-parse", "HEAD"]).output().unwrap().stdout).unwrap();
        for rev in [head.trim(), "HEAD"] {
            let mut read = base_reader(&root, Some(rev));
            assert_eq!(read("allow.txt").unwrap(), "a\nb\n", "{rev}");
            assert_eq!(read("absent.txt").unwrap_err().kind, "missing");
        }
        assert_eq!(base_reader(&root, None)("allow.txt").unwrap_err().kind, "missing");
        assert_eq!(base_reader(&root, Some("no-such-ref"))("allow.txt").unwrap_err().kind, "unreadable");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// `--require-facts`: a missing artifact fails; a failed one fails unless a rule reads `input_error`.
    #[test]
    fn required_facts_fail_the_run_unless_the_policy_reports_a_failure() {
        assert!(required(&budget_with(None)).is_err());
        assert!(required(&budget_with(Some(stamped("ok", "sha256:now", r#"{"total": 12}"#)))).is_ok());
        assert!(required(&budget_with(Some(stamped("failed", "sha256:now", "null")))).is_err());
        let mut reporting = budget_with(Some(stamped("failed", "sha256:now", "null")));
        reporting.rules.extend(crate::code_rule_dsl::logic::rr::parse(".decl input_error(p: symbol, k: symbol, d: symbol)\n.decl e(p: symbol)\ne(p) :- input_error(p, _, _).\n").unwrap().rules);
        assert!(required(&reporting).is_ok());
    }

    /// The blob id is git's: `printf 'hello\n' | git hash-object --stdin`.
    #[test]
    fn blob_ids_are_gits() {
        assert_eq!(blob_id(b"hello\n"), "ce013625030ba8dba906f756967f9e9ca394464a");
        assert_eq!(blob_id(b""), "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391");
    }

    #[test]
    fn artifact_declarations_are_parsed_and_must_agree() {
        let program = crate::code_rule_dsl::logic::rr::parse(".artifact \"test_collection\" inputs \"**/*.py\", \"pyproject.toml\"\n").unwrap();
        let declared = artifacts(&program.settings).unwrap();
        assert_eq!(declared["test_collection"], vec!["**/*.py".to_owned(), "pyproject.toml".to_owned()]);
        let twice = vec![
            ("artifact".to_owned(), "\"a\" inputs \"*.py\"".to_owned()),
            ("artifact".to_owned(), "\"a\" inputs \"*.toml\"".to_owned()),
        ];
        assert!(artifacts(&twice).is_err());
        assert!(artifact_declaration("\"a\" inputs").is_err());
    }

    /// The digest covers the files the globs match, by content: a change to an unmatched file keeps it.
    #[test]
    fn an_artifacts_digest_moves_only_with_its_inputs() {
        let root = std::env::temp_dir().join(format!("rerail-artifact-digest-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("pkg")).unwrap();
        std::fs::create_dir_all(root.join(".rerail/facts")).unwrap();
        std::fs::write(root.join("pkg/a.py"), "x = 1\n").unwrap();
        std::fs::write(root.join("README.md"), "one\n").unwrap();
        let globs = vec!["**/*.py".to_owned()];
        let first = inputs_digest(&root, &globs).unwrap();
        std::fs::write(root.join("README.md"), "two\n").unwrap();
        std::fs::write(root.join(".rerail/facts/x.py"), "hidden\n").unwrap();
        assert_eq!(inputs_digest(&root, &globs).unwrap(), first);
        std::fs::write(root.join("pkg/a.py"), "x = 2\n").unwrap();
        assert_ne!(inputs_digest(&root, &globs).unwrap(), first);
        let _ = std::fs::remove_dir_all(&root);
    }

    const BUDGET: &str = r#"
.artifact "collection" inputs "**/*.py"
.decl collected(n: number)
.input collected(IO=json, artifact="collection", path="/total")
.decl ceiling(n: number)
ceiling(10).
.decl over(n: number)
.output over
over(n) :- collected(n), ceiling(c), n > c.
.decl none_collected()
.output none_collected
none_collected() :- ceiling(_), count : { collected(_) } = 0.
.decl quiet()
.output quiet
quiet() :- ceiling(_), !over(_).
"#;

    fn stamped(status: &str, inputs: &str, data: &str) -> String {
        format!(r#"{{"format": "{FORMAT}", "artifact": "collection", "inputs": "{inputs}", "status": "{status}", "error": "exit 1", "data": {data}}}"#)
    }

    fn budget_with(text: Option<String>) -> front::Program {
        let mut program = crate::code_rule_dsl::logic::rr::parse(BUDGET).unwrap();
        populate_with(&mut program, Path::new("."), &mut |_| panic!("no repository file is read"), &mut |name| {
            assert_eq!(name, "collection");
            match &text {
                None => Err(Failure { kind: "missing", detail: "collection.json".into() }),
                Some(t) => artifact_data(name, t, "sha256:now"),
            }
        }, &mut no_base, &mut Vec::new())
        .unwrap();
        program
    }

    fn error_kinds(program: &front::Program) -> Vec<String> {
        program.facts.get("input_error").into_iter().flatten().map(|row| format!("{:?} {:?}", row[0], row[1])).collect()
    }

    #[test]
    fn a_fresh_artifact_gives_its_rows() {
        let program = budget_with(Some(stamped("ok", "sha256:now", r#"{"total": 12}"#)));
        assert_eq!(program.facts["collected"], vec![vec![Value::Int(12)]]);
        assert!(error_kinds(&program).is_empty());
        let heads: Vec<&str> = program.rules.iter().map(|r| r.head.rel.as_str()).collect();
        for kept in ["over", "none_collected", "quiet"] {
            assert!(heads.contains(&kept), "{kept} is withheld: {heads:?}");
        }
    }

    /// Missing, stale, failed and malformed artifacts give no rows and one input_error each. The rules that read the
    /// artifact through an aggregate (`none_collected`) or through a relation derived from it under `!` (`quiet`)
    /// are withheld; `over`, positive in it, is kept.
    #[test]
    fn an_incomplete_artifact_gives_an_input_error_and_withholds_its_negative_readers() {
        for (text, kind) in [
            (None, "missing"),
            (Some(stamped("ok", "sha256:then", r#"{"total": 12}"#)), "stale"),
            (Some(stamped("failed", "sha256:now", "null")), "failed"),
            (Some("{".to_owned()), "malformed"),
        ] {
            let program = budget_with(text);
            assert!(!program.facts.contains_key("collected"), "{kind}");
            assert_eq!(error_kinds(&program), vec![format!("Str(\"artifact:collection\") Str(\"{kind}\")")]);
            let heads: Vec<&str> = program.rules.iter().map(|r| r.head.rel.as_str()).collect();
            assert!(heads.contains(&"over"), "{kind}: {heads:?}");
            assert!(!heads.contains(&"none_collected") && !heads.contains(&"quiet"), "{kind}: {heads:?}");
        }
    }

    #[test]
    fn multiple_selectors_read_and_decode_one_document_once() {
        let mut program = crate::code_rule_dsl::logic::rr::parse(r#"
.decl a(n: number)
.input a(IO=json, filename="shared.json", path="/a")
.decl b(s: symbol)
.input b(IO=json, filename="shared.json", path="/b")
"#).unwrap();
        let mut reads = 0;
        populate_with(&mut program, Path::new("."), &mut |_| { reads += 1; Ok(r#"{"a":7,"b":"kept"}"#.to_owned()) }, &mut none, &mut no_base, &mut Vec::new()).unwrap();
        assert_eq!(reads, 1);
        assert_eq!(program.facts["a"], vec![vec![Value::Int(7)]]);
        assert_eq!(program.facts["b"], vec![vec![Value::Str("kept".into())]]);
    }

    /// Both noqa-sec-ratchet and test-budget name their different baseline signature input `base_sig`.
    #[test]
    fn combined_policies_keep_distinct_named_file_inputs_private() {
        let first = ".decl base_sig(s: symbol)\n.input base_sig(IO=json, filename=\"noqa.json\", path=\"/signature\")\n.decl result(s: symbol)\nresult(s) :- base_sig(s).\n.output result\n";
        let second = first.replace("noqa.json", "budget.json");
        let mut program = super::super::combine(&[first, &second], "").unwrap();
        populate_with(&mut program, Path::new("."), &mut |name| Ok(format!("{{\"signature\":\"{name}\"}}")), &mut none, &mut no_base, &mut Vec::new()).unwrap();
        assert_eq!(program.facts.get("p0_base_sig"), Some(&vec![vec![Value::Str("noqa.json".into())]]));
        assert_eq!(program.facts.get("p1_base_sig"), Some(&vec![vec![Value::Str("budget.json".into())]]));
        assert!(!program.facts.contains_key("base_sig"));
    }

    /// A stamped artifact reads back fresh, goes stale when an input changes, and a failed producer is recorded.
    #[test]
    fn a_stamp_reads_back_until_an_input_changes() {
        let root = std::env::temp_dir().join(format!("rerail-artifact-stamp-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (raw, facts) = (root.join(".rerail/raw"), root.join(".rerail/facts"));
        std::fs::create_dir_all(&raw).unwrap();
        std::fs::write(root.join("a.py"), "x = 1\n").unwrap();
        std::fs::write(raw.join("collection.json"), "{\"total\": 12}\n").unwrap();
        std::fs::write(raw.join("probes.status"), "1\n").unwrap();
        std::fs::write(raw.join("probes.err"), "ImportError\n").unwrap();
        let declared: BTreeMap<String, Vec<String>> =
            [("collection".to_owned(), vec!["**/*.py".to_owned()]), ("probes".to_owned(), vec!["**/*.py".to_owned()])].into();
        let report = stamp(&raw, &facts, &root, &declared).unwrap();
        assert!(report.contains("collection: ok") && report.contains("probes: failed"), "{report}");
        let read = |name: &str| {
            let text = std::fs::read_to_string(facts.join(format!("{name}.json"))).unwrap();
            artifact_data(name, &text, &inputs_digest(&root, &declared[name]).unwrap())
        };
        assert_eq!(read("collection").unwrap()["total"], 12);
        let failed = read("probes").unwrap_err();
        assert_eq!(failed.kind, "failed");
        assert!(failed.detail.contains("status 1") && failed.detail.contains("ImportError"), "{}", failed.detail);
        std::fs::write(root.join("a.py"), "x = 2\n").unwrap();
        assert_eq!(read("collection").unwrap_err().kind, "stale");
        assert!(stamp(&raw, &facts, &root, &BTreeMap::new()).is_err(), "an undeclared artifact is refused");
        let _ = std::fs::remove_dir_all(&root);
    }
}
