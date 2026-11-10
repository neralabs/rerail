//! Parity with real CPython (ignored tests; run on a box): CPython (`oracle.py`, `python -I -S`, which never imports or
//! runs a file) and the engine each give a fact for every file of a corpus; the test prints, per corpus, how many
//! agree and the first differences. CPython runs nowhere else.
//!
//! - `RERAIL_PARITY_PYTHON`: the interpreters, `:`-separated (default `python3`); each is one oracle.
//! - `RERAIL_PARITY_ROOTS`: the corpora, `:`-separated directories (their `.py` files, `.git`, `node_modules` and
//!   virtualenvs left out), and `fixtures`: the files of every recorded event (`fixtures/host/data-layer-port`).
//! - `RERAIL_PARITY_SHOW`: how many differences to print per fact (default 20).
//!
//! `cargo test --profile profiling --lib -- cpython::parity --ignored --nocapture`

use rayon::prelude::*;
use serde_json::Value as Json;
use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const ORACLE: &str = include_str!("oracle.py");

/// A corpus: its name and its files (absolute path, path to show).
pub(crate) struct Corpus {
    pub(crate) name: String,
    pub(crate) files: Vec<(PathBuf, String)>,
}

fn env_list(key: &str, default: &str) -> Vec<String> {
    std::env::var(key).unwrap_or_else(|_| default.to_owned()).split(':').filter(|s| !s.is_empty()).map(str::to_owned).collect()
}

pub(super) fn show() -> usize {
    std::env::var("RERAIL_PARITY_SHOW").ok().and_then(|s| s.parse().ok()).unwrap_or(20)
}

pub(super) fn pythons() -> Vec<String> {
    env_list("RERAIL_PARITY_PYTHON", "python3")
}

pub(crate) fn corpora() -> Vec<Corpus> {
    env_list("RERAIL_PARITY_ROOTS", "fixtures")
        .into_iter()
        .map(|root| if root == "fixtures" { fixtures() } else { directory(Path::new(&root)) })
        .collect()
}

/// A directory's Python files.
fn directory(root: &Path) -> Corpus {
    const SKIP: [&str; 8] = [".git", "node_modules", ".venv", "venv", "__pycache__", "target", ".tox", ".mypy_cache"];
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let Ok(t) = e.file_type() else { continue };
            let p = e.path();
            let name = e.file_name().to_string_lossy().into_owned();
            if t.is_dir() && !SKIP.contains(&name.as_str()) {
                stack.push(p);
            } else if t.is_file() && (name.ends_with(".py") || name.ends_with(".pyi")) {
                let shown = p.strip_prefix(root).unwrap_or(&p).to_string_lossy().into_owned();
                files.push((p, shown));
            }
        }
    }
    files.sort();
    Corpus { name: root.display().to_string(), files }
}

/// The Python files of every recorded event, each distinct (path, source) once, written under a temporary directory.
fn fixtures() -> Corpus {
    let port = Path::new(env!("CARGO_MANIFEST_DIR")).join("../gate/data-layer/fixtures/host/data-layer-port");
    let mut sources: BTreeSet<(String, String)> = BTreeSet::new();
    let mut names: Vec<PathBuf> = std::fs::read_dir(&port).unwrap().flatten().map(|e| e.path()).collect();
    names.sort();
    for p in names {
        let name = p.file_name().unwrap().to_string_lossy().into_owned();
        let bytes = if name.ends_with(".json.gz") {
            let out = Command::new("gzip").arg("-dc").arg(&p).output().unwrap();
            out.stdout
        } else if name.ends_with(".json") {
            std::fs::read(&p).unwrap()
        } else {
            continue;
        };
        let Ok(json) = serde_json::from_slice::<Json>(&bytes) else { continue };
        let by_sha = json.get("sources").cloned().unwrap_or(Json::Null);
        walk(&json, &by_sha, &mut sources);
    }
    let dir = std::env::temp_dir().join(format!("rerail-parity-fixtures-{}", std::process::id()));
    let mut files = Vec::new();
    for (i, (path, source)) in sources.iter().enumerate() {
        let file = dir.join(format!("{i:05}")).join(path.trim_start_matches('/'));
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, source).unwrap();
        files.push((file, format!("{i:05}/{path}")));
    }
    Corpus { name: "fixtures".into(), files }
}

fn walk(v: &Json, by_sha: &Json, out: &mut BTreeSet<(String, String)>) {
    match v {
        Json::Object(o) => {
            if let Some(path) = o.get("path").and_then(Json::as_str).filter(|p| p.ends_with(".py") || p.ends_with(".pyi")) {
                let source = o.get("source").and_then(Json::as_str).or_else(|| o.get("sourceSha256").and_then(Json::as_str).and_then(|s| by_sha[s].as_str()));
                if let Some(source) = source {
                    out.insert((path.to_owned(), source.to_owned()));
                }
            }
            o.values().for_each(|c| walk(c, by_sha, out));
        }
        Json::Array(a) => a.iter().for_each(|c| walk(c, by_sha, out)),
        _ => {}
    }
}

/// The interpreter's version line.
pub(super) fn version(python: &str) -> String {
    let out = Command::new(python).args(["-I", "-S", "-c", "import sys; print(sys.version.split()[0])"]).output().expect("the oracle's python runs");
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

/// The oracle's rows for `files` in `mode`, by path (the absolute path it was given).
pub(super) fn oracle(python: &str, mode: &str, files: &[(PathBuf, String)]) -> BTreeMap<String, Json> {
    let mut child = Command::new(python)
        .args(["-I", "-S", "-c", ORACLE, mode])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("the oracle's python runs");
    let mut stdin = child.stdin.take().unwrap();
    let list: String = files.iter().map(|(p, _)| format!("{}\n", p.display())).collect();
    let writer = std::thread::spawn(move || stdin.write_all(list.as_bytes()).unwrap());
    let out = child.wait_with_output().unwrap();
    writer.join().unwrap();
    assert!(out.status.success(), "the oracle failed");
    let mut rows = BTreeMap::new();
    for line in String::from_utf8(out.stdout).unwrap().lines() {
        let row: Json = serde_json::from_str(line).unwrap();
        rows.insert(row["p"].as_str().unwrap().to_owned(), row);
    }
    rows
}

/// The files the engine reads (UTF-8) and their text, in parallel.
pub(super) fn sources(files: &[(PathBuf, String)]) -> Vec<Option<String>> {
    files.par_iter().map(|(p, _)| std::fs::read(p).ok().and_then(|b| String::from_utf8(b).ok())).collect()
}

/// One count of agreement and its first differences.
#[derive(Default)]
pub(super) struct Tally {
    pub(super) equal: usize,
    pub(super) total: usize,
    pub(super) differences: Vec<String>,
}

impl Tally {
    pub(super) fn add(&mut self, equal: bool, difference: impl FnOnce() -> String) {
        self.total += 1;
        if equal {
            self.equal += 1;
        } else if self.differences.len() < show() {
            self.differences.push(difference());
        }
    }

    pub(super) fn merge(&mut self, other: Tally) {
        self.equal += other.equal;
        self.total += other.total;
        let room = show().saturating_sub(self.differences.len());
        self.differences.extend(other.differences.into_iter().take(room));
    }

    pub(super) fn report(&self, what: &str) {
        println!("  {what}: {} / {} equal", self.equal, self.total);
        for d in &self.differences {
            println!("    {d}");
        }
    }
}

/// A short excerpt of `source` around line `line`.
pub(super) fn excerpt(source: &str, line: usize) -> String {
    let lines: Vec<&str> = source.lines().collect();
    let from = line.saturating_sub(2);
    let to = (line + 1).min(lines.len());
    let text: Vec<String> = (from..to).map(|i| format!("{}| {}", i + 1, lines[i].chars().take(100).collect::<String>())).collect();
    format!("{:?}", text.join("\n"))
}

/// tree-sitter's verdict, the tape's (`syntax_error(d)`): the tree has an error or a missing node.
fn tree_sitter_error(source: &str) -> bool {
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(crate::code_rule_dsl::literal::annotation::python()).unwrap();
    parser.parse(source, None).is_none_or(|t| t.root_node().has_error())
}

/// `syntax_error(d, l, msg)`: the verdict, the line and the message against `ast.parse`'s; and tree-sitter's verdict
/// (the tape's `syntax_error(d)`) against it, for the choice between the two. Another exception `ast.parse` raises
/// is its class at line 1 on both sides (the oracle's `[None, class]`; the engine's `Raised`).
#[test]
#[ignore]
fn syntax_error_parity() {
    for python in pythons() {
        println!("CPython {} ({python})", version(&python));
        for corpus in corpora() {
            let want = oracle(&python, "syntax", &corpus.files);
            let texts = sources(&corpus.files);
            let engine = |s: &str| {
                super::syntax::syntax_error(s).unwrap_or_else(|r| Some(super::syntax::SyntaxError { line: 1, msg: r.class.to_owned() }))
            };
            let got: Vec<Option<(Option<super::syntax::SyntaxError>, bool)>> =
                texts.par_iter().map(|s| s.as_ref().map(|s| (engine(s), tree_sitter_error(s)))).collect();
            let (mut verdict, mut line, mut msg, mut tree) = (Tally::default(), Tally::default(), Tally::default(), Tally::default());
            for (((path, shown), source), got) in corpus.files.iter().zip(&texts).zip(&got) {
                let (Some(source), Some((got, ts)), Some(row)) = (source, got, want.get(&path.display().to_string())) else { continue };
                let cpython = row["e"].as_array().map(|e| (e[0].as_u64().map_or(1, |l| l as usize), e[1].as_str().unwrap_or_default().to_owned()));
                let show_both = || format!("{shown}: CPython {cpython:?}, engine {got:?} [Ruff: {}]", super::syntax::explain(source));
                verdict.add(cpython.is_some() == got.is_some(), || format!("{} {}", show_both(), excerpt(source, cpython.as_ref().or(got.as_ref().map(|g| (g.line, String::new())).as_ref()).map_or(1, |c| c.0))));
                tree.add(cpython.is_some() == *ts, || format!("{shown}: CPython {cpython:?}, tree-sitter error {ts}"));
                if let (Some((l, m)), Some(g)) = (&cpython, got) {
                    line.add(*l == g.line, || format!("{} {}", show_both(), excerpt(source, *l)));
                    msg.add(*m == g.msg, || format!("{} {}", show_both(), excerpt(source, *l)));
                }
            }
            let rejected = want.values().filter(|r| r["e"].is_array()).count();
            println!(" {}: {} files, {} CPython rejects", corpus.name, want.len(), rejected);
            verdict.report("verdict (Ruff)");
            line.report("line, where both reject");
            msg.report("message, where both reject");
            tree.report("verdict (tree-sitter's ERROR/MISSING)");
        }
    }
}

/// The names of an annotation's tree as the oracle's `annotations` mode gives them: each identifier but a keyword
/// argument's name (CPython's `Name` ids and `Attribute` attrs), sorted and distinct. An identifier is its text, as
/// written in the string, under `@nfkc`, as the rules read a name (`bootstrap_str_names`): CPython's parser
/// normalizes it (PEP 3131), the guest's tree holds it as written.
fn annotation_names(tree: &crate::code_rule_dsl::literal::SubTree) -> Vec<String> {
    use crate::code_rule_dsl::literal::annotation::{kind_name, python};
    let name = python().field_id_for_name("name").map_or(0, |f| f.get());
    let nodes: Vec<_> = tree.preorder().collect();
    let mut out = BTreeSet::new();
    for node in &nodes {
        let keyword = node.field == name && node.parent.is_some_and(|p| kind_name(nodes[p as usize].kind) == "keyword_argument");
        if kind_name(node.kind) == "identifier" && node.named && !keyword {
            out.insert(crate::code_rule_dsl::identifier::nfkc(&tree.text()[node.start as usize..node.end as usize]).into_owned());
        }
    }
    out.into_iter().collect()
}

/// `string_annotation(s)` and `parsed(s, "annotation", root)` (`facts::vtab::syntax::annotation`) against CPython: at
/// each string the scope table reads as a type definition in the source (nested ones are in no file's tree), whether
/// `ast.parse(value, mode="eval")` accepts its value where the engine has a tree, and the tree's names (Name ids,
/// Attribute attrs) where both parse; those at an annotation (fields `annotation`, `returns`, and below) counted apart
/// from Ruff's other positions (`cast`, `TypeVar` bounds). Listed: the engine's annotations that are no str Constant
/// (none expected), and the strings at an annotation that the engine does not read as type definitions (a `Literal`
/// argument, `Annotated` metadata).
#[test]
#[ignore]
fn string_annotation_parity() {
    use crate::code_rule_dsl::facts::vtab::syntax::annotation::string_annotations;
    for python in pythons() {
        println!("CPython {} ({python})", version(&python));
        for corpus in corpora() {
            let want = oracle(&python, "annotations", &corpus.files);
            let texts = sources(&corpus.files);
            type Engine = Vec<(usize, usize, bool, Vec<String>)>;
            let got: Vec<Option<Engine>> = corpus
                .files
                .par_iter()
                .zip(&texts)
                .map(|((_, shown), s)| {
                    let s = s.as_ref()?;
                    let file = crate::code_rule_dsl::model::FileInput::new(shown.clone(), "python".to_owned(), std::sync::Arc::new(s.clone()), None, None, "core").ok()?;
                    let (_, tape) = crate::code_rule_dsl::facts::core::build_with_tape(&file).ok()??;
                    let found = string_annotations(&tape).ok()?;
                    Some(found.iter().filter(|a| a.outer.is_none()).map(|a| (a.start, a.end, !a.tree.is_error(), if a.tree.is_error() { Vec::new() } else { annotation_names(&a.tree) })).collect())
                })
                .collect();
            let (mut verdict, mut names, mut at_verdict, mut at_names) = (Tally::default(), Tally::default(), Tally::default(), Tally::default());
            // `unmatched`: the engine's annotations that are no str Constant; `oracle_only`: the oracle's strings at an
            // annotation that the engine does not read as type definitions. Both only listed (equal counts nothing).
            let (mut unmatched, mut oracle_only) = (Tally::default(), Tally::default());
            for (((path, shown), source), got) in corpus.files.iter().zip(&texts).zip(&got) {
                let (Some(source), Some(got), Some(row)) = (source, got, want.get(&path.display().to_string())) else { continue };
                let Some(rows) = row["n"].as_array() else { continue };
                let cpython: BTreeMap<(usize, usize), (bool, Vec<String>, bool)> = rows
                    .iter()
                    .map(|r| {
                        let names = r[3].as_array().map(|a| a.iter().filter_map(|n| n.as_str().map(str::to_owned)).collect()).unwrap_or_default();
                        ((r[0].as_u64().unwrap() as usize, r[1].as_u64().unwrap() as usize), (r[2].as_bool().unwrap(), names, r[4].as_bool().unwrap()))
                    })
                    .collect();
                let engine: BTreeSet<(usize, usize)> = got.iter().map(|g| (g.0, g.1)).collect();
                for ((s, e), _) in cpython.iter().filter(|(k, v)| v.2 && !engine.contains(k)) {
                    oracle_only.add(false, || format!("{shown}@{s}: {:?}", source.get(*s..*e).unwrap_or("?").chars().take(120).collect::<String>()));
                }
                for (s, e, parsed, got_names) in got {
                    let text = || source.get(*s..*e).unwrap_or("?").chars().take(120).collect::<String>();
                    let Some((cp_parsed, cp_names, at)) = cpython.get(&(*s, *e)) else {
                        unmatched.add(false, || format!("{shown}@{s}: {:?}", text()));
                        continue;
                    };
                    let (v, n) = if *at { (&mut at_verdict, &mut at_names) } else { (&mut verdict, &mut names) };
                    v.add(parsed == cp_parsed, || format!("{shown}@{s}: {:?}: CPython parses {cp_parsed}, engine {parsed}", text()));
                    if *parsed && *cp_parsed {
                        n.add(got_names == cp_names, || format!("{shown}@{s}: {:?}: CPython {cp_names:?}, engine {got_names:?}", text()));
                    }
                }
            }
            println!(" {}: {} files; {} string annotations (the engine's, in the source)", corpus.name, want.len(), unmatched.total + verdict.total + at_verdict.total);
            unmatched.report("string annotations that are no str Constant (listed)");
            oracle_only.report("strings at an annotation that are no string annotation (listed)");
            at_verdict.report("at an annotation: parses (ast.parse(value, mode=\"eval\"))");
            at_names.report("at an annotation: names, where both parse");
            verdict.report("elsewhere (cast, TypeVar, …): parses");
            names.report("elsewhere: names, where both parse");
        }
    }
}

/// `@nfkc` (`identifier::nfkc`) against CPython's identifier normalization: each `Name` of each file that parses (the
/// oracle's `names` mode, whose span is the identifier as written), its id against `@nfkc` of its source text. The
/// non-ASCII ones, where normalization can do anything, are counted apart.
#[test]
#[ignore]
fn nfkc_identifier_parity() {
    for python in pythons() {
        println!("CPython {} ({python})", version(&python));
        for corpus in corpora() {
            let want = oracle(&python, "names", &corpus.files);
            let texts = sources(&corpus.files);
            let (mut all, mut non_ascii) = (Tally::default(), Tally::default());
            for ((path, shown), source) in corpus.files.iter().zip(&texts) {
                let (Some(source), Some(row)) = (source, want.get(&path.display().to_string())) else { continue };
                let Some(rows) = row["n"].as_array() else { continue };
                for r in rows.iter().filter(|r| r[2] == "Name") {
                    let (s, e, id) = (r[0].as_u64().unwrap() as usize, r[1].as_u64().unwrap() as usize, r[3].as_str().unwrap());
                    let Some(written) = source.get(s..e) else { continue };
                    let got = crate::code_rule_dsl::identifier::nfkc(written);
                    let difference = || format!("{shown}@{s}: {written:?}: CPython {id:?}, @nfkc {got:?}");
                    all.add(got == id, difference);
                    if !written.is_ascii() {
                        non_ascii.add(got == id, difference);
                    }
                }
            }
            println!(" {}: {} files", corpus.name, want.len());
            all.report("Name ids");
            non_ascii.report("Name ids written with a non-ASCII character");
        }
    }
}

/// `literal_json` (`cpython::literal`) against CPython: at each expression of each file that parses (the oracle's
/// `literals` mode, f-strings' own pieces left out), its `json.dumps(ast.literal_eval(node), sort_keys=True,
/// separators=(",", ":"))`, or none where either raises, against the engine's, by span and class. The expressions with a
/// value are counted apart.
#[test]
#[ignore]
fn literal_json_parity() {
    use super::literal::{Class, literals};
    for python in pythons() {
        println!("CPython {} ({python})", version(&python));
        for corpus in corpora() {
            let want = oracle(&python, "literals", &corpus.files);
            let texts = sources(&corpus.files);
            let got: Vec<Option<Vec<super::literal::Literal>>> = texts.par_iter().map(|s| s.as_deref().map(literals)).collect();
            let (mut all, mut valued) = (Tally::default(), Tally::default());
            for (((path, shown), source), got) in corpus.files.iter().zip(&texts).zip(&got) {
                let (Some(source), Some(got), Some(row)) = (source, got, want.get(&path.display().to_string())) else { continue };
                let Some(rows) = row["n"].as_array() else { continue };
                let engine: BTreeMap<(u32, u32, Class), &str> = got.iter().map(|l| ((l.start, l.end, l.class), l.json.as_str())).collect();
                let mut seen = BTreeSet::new();
                for r in rows {
                    let class = match r[2].as_str().unwrap() {
                        "Constant" => Class::Constant,
                        "Tuple" => Class::Tuple,
                        "List" => Class::List,
                        "Dict" => Class::Dict,
                        "UnaryOp" => Class::UnaryOp,
                        other => {
                            // No other class has a value (a set, a complex sum, `set()` are no JSON).
                            if let Some(v) = r[3].as_str() {
                                valued.add(false, || format!("{shown}: CPython {v:?} for a {other}"));
                            }
                            continue;
                        }
                    };
                    let key = (r[0].as_u64().unwrap() as u32, r[1].as_u64().unwrap() as u32, class);
                    seen.insert(key);
                    let cpython = r[3].as_str();
                    let engine = engine.get(&key).copied();
                    let text = || source.get(key.0 as usize..key.1 as usize).unwrap_or("?").chars().take(160).collect::<String>();
                    let difference = || format!("{shown}@{}: {:?}: CPython {cpython:?}, engine {engine:?}", key.0, text());
                    all.add(cpython == engine, difference);
                    if cpython.is_some() || engine.is_some() {
                        valued.add(cpython == engine, difference);
                    }
                }
                for (key, v) in engine.iter().filter(|(k, _)| !seen.contains(k)) {
                    valued.add(false, || format!("{shown}@{}: engine {v:?} at no CPython expression of its class", key.0));
                }
            }
            println!(" {}: {} files", corpus.name, want.len());
            all.report("expressions of a class with values: literal_json or none");
            valued.report("those with a value on either side");
        }
    }
}

/// `unparse`, `dump_sha256`, `body_sha256` and `signature_sha256` (`cpython::printed`) against CPython: at each located
/// node of each file that parses but an f-string's own pieces (the oracle's `ast` mode), by span and class, the
/// engine's against `ast.unparse` (an expression, a keyword, an arg, an alias; those with a text on either side
/// counted apart, and apart again those whose CPython text holds a lone surrogate, which no UTF-8 text is: the engine
/// gives none), the sha256 of `ast.dump`, and a def's hashes as W's worker made them.
#[test]
#[ignore]
fn printed_parity() {
    use super::printed::{Printed, Want, printed};
    let want_all = Want { unparse: true, dump: false, dump_sha256: true, body_sha256: true, signature_sha256: true };
    for python in pythons() {
        println!("CPython {} ({python})", version(&python));
        for corpus in corpora() {
            let want = oracle(&python, "ast", &corpus.files);
            // nodes, unparse, unparse with a text, unparse with a lone surrogate, dump_sha256, body_sha256, signature_sha256.
            let tallies: Vec<[Tally; 7]> = corpus
                .files
                .par_iter()
                .filter_map(|(path, shown)| {
                    let source = std::fs::read(path).ok().and_then(|b| String::from_utf8(b).ok())?;
                    let rows = want.get(&path.display().to_string())?["n"].as_array()?;
                    let got = printed(&source, want_all);
                    let mut engine: BTreeMap<(u32, u32, &str), &Printed> = BTreeMap::new();
                    for p in &got {
                        engine.entry((p.start, p.end, p.class)).or_insert(p);
                    }
                    let mut t: [Tally; 7] = Default::default();
                    let mut seen = BTreeSet::new();
                    for r in rows {
                        let class = r[2].as_str().unwrap();
                        let key = (r[0].as_u64().unwrap() as u32, r[1].as_u64().unwrap() as u32, class);
                        if !seen.insert(key) {
                            continue;
                        }
                        let text = || source.get(key.0 as usize..key.1 as usize).unwrap_or("?").chars().take(160).collect::<String>();
                        let at = || format!("{shown}@{} {class} {:?}", key.0, text());
                        let e = engine.get(&key).copied();
                        t[0].add(e.is_some(), || format!("{}: no engine node", at()));
                        let Some(e) = e else { continue };
                        let unparsed = matches!(class, "keyword" | "arg" | "alias") || super::unparse::is_expr(class);
                        if unparsed && let Some(surrogates) = r[3].get("surrogates") {
                            let g = e.unparse.as_deref();
                            t[3].add(g.is_none(), || format!("{}: CPython {surrogates}, engine {g:?}", at()));
                        } else if unparsed {
                            let (c, g) = (r[3].as_str(), e.unparse.as_deref());
                            let diff = || format!("{}: CPython {c:?}, engine {g:?}", at());
                            t[1].add(c == g, diff);
                            if c.is_some() || g.is_some() {
                                t[2].add(c == g, || format!("{}: CPython {c:?}, engine {g:?}", at()));
                            }
                        }
                        let (c, g) = (r[4].as_str(), e.dump_sha256.as_deref());
                        t[4].add(c == g, || format!("{}: CPython {c:?}, engine {g:?}", at()));
                        if matches!(class, "FunctionDef" | "AsyncFunctionDef") {
                            let (c, g) = (r[5].as_str(), e.body_sha256.as_deref());
                            t[5].add(c == g, || format!("{}: CPython {c:?}, engine {g:?}", at()));
                            let (c, g) = (r[6].as_str(), e.signature_sha256.as_deref());
                            t[6].add(c == g, || format!("{}: CPython {c:?}, engine {g:?}", at()));
                        }
                    }
                    for key in engine.keys().filter(|k| !seen.contains(*k)) {
                        t[0].add(false, || format!("{shown}@{} {}: engine node at no CPython node", key.0, key.2));
                    }
                    Some(t)
                })
                .collect();
            let mut total: [Tally; 7] = Default::default();
            for t in tallies {
                for (a, b) in total.iter_mut().zip(t) {
                    a.merge(b);
                }
            }
            println!(" {}: {} files", corpus.name, want.len());
            let what = [
                "located nodes (span and class)",
                "unparse of expressions, keywords, args, aliases: text or none",
                "those with a text on either side",
                "those whose CPython text holds a lone surrogate (no UTF-8): none",
                "dump_sha256",
                "body_sha256 of defs",
                "signature_sha256 of defs",
            ];
            for (t, w) in total.iter().zip(what) {
                t.report(w);
            }
        }
    }
}
