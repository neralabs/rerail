//! Each stage of W's reference graph beside the rules that stand for it (note t50): the same files through W's Rust
//! stage and through `python/references.rerail`, their rows compared stage by stage, and each side's CPU per stage.
//!
//! The stages, W's way and the rules':
//!
//! | stage   | W                                                  | rules                                                 |
//! |---------|----------------------------------------------------|-------------------------------------------------------|
//! | scope   | `ScopeTable::build_in`: the scopes, the loads       | `names.rerail`'s bindings and `nameref`, `bind_only`      |
//! | lookup  | `ScopeTable::resolve_python_index` per load         | `wstart`, `walk`, `look`                              |
//! | summary | `calls::summarize` over the table: defs, references | `references.rerail`'s per-file part: `rf_qn`, `rf_ref`, … |
//! | link    | `Graph::link_references`, its rows                  | `references.rerail`'s cross-file part: `rf_link`, …       |
//!
//! Rows are compared as text, a node as (path, start, end). W's reference names carry its markers
//! (`m.<bare>.x=>q`, `C.<self>.f`, …, `closure::reference_via`); the rules' `rf_ref` forms are spelled into them.
//! W's lookup through a type-parameter scope is the rules' `tp_ref`, which no lookup makes: both are "unbound" here
//! (scope -1, -1). The lookup is the plain walk: W's forward reading of an annotation's names (PEP 563) is the
//! rules' `rf_forward`, compared where it matters, in the summary's references. A `del` target is no load. A name in a string annotation is a reference of W's table at an offset past the source (its
//! `annotations`), with no node: neither side's load and lookup rows hold it.
//!
//! The timing (`reference_stages_timed`, ignored) runs a corpus directory (`RERAIL_STAGE_CORPUS`) and prints each
//! stage's CPU on each side and their ratio. W's stages run on this thread, one file after another; the rules' CPU is
//! their strata's own CPU from the run's profile, every thread's.

use super::write_files;
use crate::code_rule_dsl::calls::{self, closure::Graph, names::Name};
use crate::code_rule_dsl::language::{Family, GrammarDialect};
use crate::code_rule_dsl::model::{FileInput, package_marker_directory, python_module_in};
use crate::code_rule_dsl::scope::{Access, ScopeKind, ScopeTable};
use crate::code_rule_dsl::typing::file_local::Binding;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

/// A stage's rows: each a tuple of texts.
type Rows = BTreeSet<Vec<String>>;

/// The stages, in order.
pub(super) const STAGES: [&str; 4] = ["scope", "lookup", "summary", "link"];

/// What one side gives: each compared relation's rows, and each stage's CPU in seconds.
#[derive(Default)]
pub(super) struct Side {
    pub rows: BTreeMap<&'static str, Rows>,
    pub cpu: BTreeMap<&'static str, f64>,
}

fn row<const N: usize>(cells: [String; N]) -> Vec<String> {
    cells.into()
}

/// W's side over `files` (path, source): every stage, as W's evaluation runs it for the reference graph (the deleted
/// `corpus::references::Files::add_file`, then `relations_of`), with no sibling links or constructor fields.
pub(super) fn w_side(files: &[(String, String)]) -> Side {
    let mut side = Side::default();
    let packages: HashSet<String> = files.iter().filter_map(|(p, _)| package_marker_directory(p)).map(str::to_owned).collect();
    let mut timed = |stage: &'static str, f: &mut dyn FnMut()| {
        let t = Instant::now();
        f();
        *side.cpu.entry(stage).or_default() += t.elapsed().as_secs_f64();
    };
    let mut rows: BTreeMap<&'static str, Rows> = BTreeMap::new();
    let mut summaries = Vec::new();
    for (path, source) in files.iter().filter(|(p, _)| p.ends_with(".py") || p.ends_with(".pyi")) {
        let (tape, _) = crate::code_rule_dsl::fixtures::document(source, GrammarDialect::Python);
        let module = (!packages.is_empty()).then(|| python_module_in(path, |d| packages.contains(d))).flatten();
        let file = FileInput::new(path.clone(), "python".to_owned(), Arc::new(source.clone()), module.as_ref(), None, "references").unwrap();
        let module = file.module.as_deref();
        let has_errors = !crate::code_rule_dsl::soundness::SyntaxErrors::of_doc(&tape).is_empty();
        let root = ast_grep_core::AstGrep::doc(tape.clone());
        let mut table = None;
        timed("scope", &mut || {
            table = ScopeTable::build_in(root.root(), GrammarDialect::Python, calls::MAX_TABLE_NODES, &crate::code_rule_dsl::budget::matcher_budget_exhausted, calls::reading(path), module).ok();
        });
        let mut resolved = Vec::new();
        if let Some(table) = &table {
            timed("lookup", &mut || {
                resolved = table
                    .references
                    .iter()
                    .filter(|r| r.access == Access::Load && r.end <= source.len())
                    // The plain walk: W's forward reading of an annotation (`forward_module`) is the rules'
                    // `rf_forward`, which the summary stage checks.
                    .map(|r| (r.start, r.end, table.resolve_python_index(&crate::code_rule_dsl::scope::Reference { forward: false, ..*r }, &source[r.start..r.end])))
                    .collect();
            });
            for (start, end, found) in &resolved {
                let load = row([path.clone(), start.to_string(), end.to_string()]);
                rows.entry("load").or_default().insert(load.clone());
                let scope = found.map(|(s, _)| &table.scopes[s as usize]).filter(|s| s.kind != ScopeKind::TypeParams);
                let mut looked = load;
                match scope {
                    Some(s) => looked.extend([s.range.start.to_string(), s.range.end.to_string()]),
                    None => looked.extend(["-1".to_owned(), "-1".to_owned()]),
                }
                rows.entry("look").or_default().insert(looked);
            }
        }
        let mut summary = None;
        timed("summary", &mut || {
            summary = Some(calls::summarize(root.root(), GrammarDialect::Python, path, module, has_errors, source.len(), table.as_ref()).0);
        });
        let summary = summary.unwrap();
        for def in &summary.defs {
            let kind = if def.flags & calls::summary::CLASS != 0 { "class_definition" } else { "function_definition" };
            rows.entry("def").or_default().insert(row([path.clone(), def.start.to_string(), def.end.to_string(), summary.name(def.name).to_owned(), kind.to_owned()]));
        }
        for (owner, name) in &summary.references {
            let def = &summary.defs[*owner as usize];
            rows.entry("ref").or_default().insert(row([path.clone(), def.start.to_string(), def.end.to_string(), summary.name(*name).to_owned()]));
        }
        summaries.push((Arc::<str>::from(path.as_str()), Arc::new(summary)));
    }
    summaries.sort_unstable_by(|a, b| a.0.cmp(&b.0));
    let mut graph = None;
    timed("link", &mut || {
        let g = Graph::link_references(0, summaries.iter().map(|(p, s)| (Arc::clone(p), Arc::new(s.as_references(0)))).collect());
        let _ = (g.labelled_reference_rows(), g.reference_open());
        graph = Some(g);
    });
    let graph = graph.unwrap();
    let site = |d: calls::closure::DefRef| {
        let (path, start) = graph.def_site(d);
        (path.to_string(), start.to_string())
    };
    for (edge, via) in graph.labelled_reference_rows() {
        rows.entry("edge").or_default().insert(row([
            edge.path.to_string(),
            edge.source.start.to_string(),
            edge.target_path.to_string(),
            edge.target.start.to_string(),
            via.to_owned(),
        ]));
    }
    for (path, def) in graph.reference_open() {
        rows.entry("open").or_default().insert(row([path.to_string(), def.start.to_string()]));
    }
    // What each `name`-form reference links to, and whether it is open.
    let names: BTreeSet<String> = rows.get("ref").into_iter().flatten().map(|r| r[3].clone()).filter(|n| calls::closure::reference_via(n) == "name").collect();
    for text in names {
        let Some(name) = Name::get(Family::Python, &text) else { continue };
        let (defs, open) = graph.link_of(&name);
        for d in defs.iter() {
            let (p, s) = site(*d);
            rows.entry("link").or_default().insert(row([text.clone(), p, s]));
        }
        if open {
            rows.entry("open_name").or_default().insert(row([text.clone()]));
        }
    }
    side.rows = rows;
    side
}

/// The rules' relations, each at (path, start, end) like W's rows.
const PROGRAM: &str = r#"
.language python
.scope include "**/*.py", "**/*.pyi"
#include "python/references.rerail"
.decl st_pos(n: node, p: symbol, s: number, e: number)
st_pos(n, p, s, e) :- node(d, n, _, s, e), doc(d, p, _).
// A load: a name read in its file's code (not of a string parsed as code: `guest_of`, decision 0431), no `del` target.
.decl st_read(n: node, x: symbol)
st_read(n, x) :- nameref(n, x), !bind_only(n), !rf_deleted(n), !guest_of(n, _).
.decl st_load(p: symbol, s: number, e: number)
st_load(p, s, e) :- st_read(n, _), st_pos(n, p, s, e).
.decl st_look(p: symbol, s: number, e: number, ss: number, se: number)
st_look(p, s, e, ss, se) :- st_read(n, x), look(n, x, c), st_pos(n, p, s, e), st_pos(c, _, ss, se).
st_look(p, s, e, -1, -1) :- st_read(n, x), !look(n, x, _), st_pos(n, p, s, e).
.decl st_def(p: symbol, s: number, e: number, q: symbol, k: symbol)
st_def(p, s, e, q, k) :- rf_qn(f, q), kind(f, k), st_pos(f, p, s, e).
.decl st_ref(p: symbol, s: number, e: number, name: symbol)
st_ref(p, s, e, q) :- rf_ref(t, "name", q, _, _), st_pos(t, p, s, e).
st_ref(p, s, e, @concat(m, ".<bare>.", x, "=>", q)) :- rf_ref(t, "bare", m, x, q), st_pos(t, p, s, e).
st_ref(p, s, e, @concat(c, ".<unqualified>.", x, "=>", m, ".<bare>.", x, "=>", q)) :- rf_ref(t, "unq", c, x, q),
    rf_unq_bare(t, c, x, m), m != "", st_pos(t, p, s, e).
st_ref(p, s, e, @concat(c, ".<unqualified>.", x, "=>", q)) :- rf_ref(t, "unq", c, x, q), rf_unq_bare(t, c, x, ""), st_pos(t, p, s, e).
st_ref(p, s, e, @concat(c, ".<self>.", f)) :- rf_ref(t, "self", c, f, _), st_pos(t, p, s, e).
st_ref(p, s, e, @concat(c, ".<super>.", f)) :- rf_ref(t, "super", c, f, _), st_pos(t, p, s, e).
st_ref(p, s, e, @concat(c, ".<field>.", x, ".", f)) :- rf_ref(t, "field", c, x, f), st_pos(t, p, s, e).
st_ref(p, s, e, @concat(m, ".<type>.", ty, ".", f)) :- rf_ref(t, "type", m, ty, f), st_pos(t, p, s, e).
.decl st_link(q: symbol, p: symbol, s: number)
st_link(q, p, s) :- rf_ref(_, "name", q, _, _), rf_link(q, d), st_pos(d, p, s, _).
.decl st_open_name(q: symbol)
st_open_name(q) :- rf_ref(_, "name", q, _, _), rf_open_name(q).
.decl st_edge(sp: symbol, ss: number, tp: symbol, ts: number, via: symbol)
st_edge(sp, ss, tp, ts, v) :- reference_edge(s, t, v), st_pos(s, sp, ss, _), st_pos(t, tp, ts, _).
.decl st_open(p: symbol, s: number)
st_open(p, s) :- reference_open(n), st_pos(n, p, s, _).
.output st_load, st_look, st_def, st_ref, st_link, st_open_name, st_edge, st_open
"#;

/// The compared relations, by the rules' output names.
pub(super) const RELATIONS: [(&str, &str); 8] = [
    ("load", "st_load"),
    ("look", "st_look"),
    ("def", "st_def"),
    ("ref", "st_ref"),
    ("link", "st_link"),
    ("open_name", "st_open_name"),
    ("edge", "st_edge"),
    ("open", "st_open"),
];

/// The stage a rule's stratum belongs to: `names.rerail`'s lookup rules, the rest of `names.rerail`, `references.rerail` before
/// its "across files" line and after it; the run's generated relations apart.
fn stage_of(rel: &str, names: &HashSet<String>, local: &HashSet<String>, cross: &HashSet<String>) -> &'static str {
    const LOOKUP: [&str; 6] = ["wstart", "walk", "look", "miss", "in_first_iter", "builtin_ref"];
    let base = rel.trim_start_matches("xd_").trim_start_matches("demand_");
    let base = base.split_once('_').filter(|(p, _)| p.starts_with("fl") && p[2..].bytes().all(|b| b.is_ascii_digit())).map_or(base, |(_, r)| r);
    if LOOKUP.contains(&base) {
        "lookup"
    } else if names.contains(base) {
        "scope"
    } else if local.contains(base) {
        "summary"
    } else if cross.contains(base) || base.starts_with("reference_") {
        "link"
    } else if rel.starts_with("st_") {
        "(outputs)"
    } else {
        "(generated)"
    }
}

fn heads(text: &str) -> HashSet<String> {
    text.lines().filter_map(|l| l.split_once('(').map(|(h, _)| h)).filter(|h| !h.is_empty() && h.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')).map(str::to_owned).collect()
}

/// The rules' side over the files under `dir`: the compared rows, and each stage's CPU (its strata's own, from the
/// run's profile), with the run's CPU outside every stratum as `(engine)`.
pub(super) fn rules_side(dir: &std::path::Path) -> Side {
    rules_side_in(dir, 1 << 18)
}

/// `rules_side` with chunks of `chunk_nodes` nodes: the rows are the same at any size.
pub(super) fn rules_side_in(dir: &std::path::Path, chunk_nodes: usize) -> Side {
    let lib = rerail_store::source_tree::native().join("../docs/rule-syntax/lib/python");
    let names = heads(&std::fs::read_to_string(lib.join("names.rerail")).unwrap());
    let refs = std::fs::read_to_string(lib.join("references.rerail")).unwrap();
    let (before, after) = refs.split_once("// ==== across files").expect("references.rerail's across-files line");
    let (local, cross) = (heads(before), heads(after));
    let out = crate::code_rule_dsl::fixtures::temp().with_extension("reference-stages-out");
    let _ = std::fs::remove_dir_all(&out);
    let profile = crate::code_rule_dsl::logic::profile::on();
    crate::code_rule_dsl::logic::profile::set(true);
    crate::code_rule_dsl::logic::profile::reset();
    crate::code_rule_dsl::logic::plan::corpus::run(PROGRAM, dir, None, &out, "program", Binding::Cpu, chunk_nodes).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
    let spans = crate::code_rule_dsl::logic::profile::spans();
    crate::code_rule_dsl::logic::profile::set(profile);
    let mut side = Side::default();
    for (path, t) in &spans {
        let stratum = path.split('/').rev().find_map(|s| s.strip_prefix("stratum ")).map(|s| s.trim_end_matches(" (in a job)"));
        let stage = stratum.map_or("(engine)", |r| stage_of(r, &names, &local, &cross));
        *side.cpu.entry(stage).or_default() += t.own;
    }
    // `RERAIL_STAGE_TOP=<n>`: the n strata of most own CPU, with their stage, and the engine's spans the same way.
    if let Some(n) = std::env::var("RERAIL_STAGE_TOP").ok().and_then(|v| v.parse::<usize>().ok()) {
        let mut by: HashMap<String, f64> = HashMap::new();
        for (path, t) in &spans {
            let stratum = path.split('/').rev().find_map(|s| s.strip_prefix("stratum ")).map(|s| s.trim_end_matches(" (in a job)"));
            let key = match stratum {
                Some(r) => format!("{} {r}", stage_of(r, &names, &local, &cross)),
                None => format!("(engine) {}", path.rsplit('/').next().unwrap_or(path)),
            };
            *by.entry(key).or_default() += t.own;
        }
        let mut by: Vec<(String, f64)> = by.into_iter().collect();
        by.sort_by(|a, b| b.1.total_cmp(&a.1));
        for (k, s) in by.into_iter().take(n) {
            println!("top {s:8.3} {k}");
        }
    }
    // `RERAIL_STAGE_SPANS=<relation>`: the spans under that relation's stratum, by own CPU, the stratum's part of each
    // path cut: where a stratum's time goes besides its joins.
    if let Ok(rel) = std::env::var("RERAIL_STAGE_SPANS") {
        let mut by: HashMap<String, (f64, u64)> = HashMap::new();
        for (path, t) in &spans {
            let parts: Vec<&str> = path.split('/').collect();
            let Some(at) = parts.iter().rposition(|s| s.strip_prefix("stratum ").map(|s| s.trim_end_matches(" (in a job)")) == Some(rel.as_str())) else { continue };
            let e = by.entry(parts[at + 1..].join("/")).or_default();
            e.0 += t.own;
            e.1 += t.calls;
        }
        let mut by: Vec<(String, (f64, u64))> = by.into_iter().collect();
        by.sort_by(|a, b| b.1.0.total_cmp(&a.1.0));
        for (k, (s, calls)) in by.into_iter().take(25) {
            println!("span {s:8.3} {calls:8} {}", if k.is_empty() { "(the stratum itself)" } else { &k });
        }
    }
    for (ours, rel) in RELATIONS {
        let shown = crate::code_rule_dsl::logic::plan::output::shown(&out, rel).unwrap_or_default();
        side.rows.insert(ours, shown.into_iter().collect());
    }
    let _ = std::fs::remove_dir_all(&out);
    side
}

/// The rows where the two sides differ, a few of each way per relation; empty where they agree.
pub(super) fn differences(w: &Side, ours: &Side, shown: usize) -> Vec<String> {
    let mut out = Vec::new();
    for (name, _) in RELATIONS {
        let (a, b) = (w.rows.get(name).cloned().unwrap_or_default(), ours.rows.get(name).cloned().unwrap_or_default());
        if a != b {
            let missing: Vec<_> = a.difference(&b).take(shown).collect();
            let extra: Vec<_> = b.difference(&a).take(shown).collect();
            out.push(format!("{name}: W {} rules {}; only W's {missing:?}; only the rules' {extra:?}", a.len(), b.len()));
        }
    }
    out
}

/// Each stage's CPU on each side, their ratio, and the rows each relation has.
pub(super) fn report(w: &Side, ours: &Side) -> String {
    let mut s = String::from("stage      W s       rules s   rules/W\n");
    for stage in STAGES {
        let (a, b) = (w.cpu.get(stage).copied().unwrap_or_default(), ours.cpu.get(stage).copied().unwrap_or_default());
        s += &format!("{stage:<9} {a:>8.3}  {b:>10.3}  {:>8.1}\n", if a > 0.0 { b / a } else { f64::NAN });
    }
    for (stage, cpu) in ours.cpu.iter().filter(|(k, _)| !STAGES.contains(k)) {
        s += &format!("{stage:<9} {:>8}  {cpu:>10.3}\n", "");
    }
    for (name, _) in RELATIONS {
        s += &format!("rows {name}: W {} rules {}\n", w.rows.get(name).map_or(0, BTreeSet::len), ours.rows.get(name).map_or(0, BTreeSet::len));
    }
    s
}

fn files_under(root: &std::path::Path) -> Vec<(String, String)> {
    let mut files = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|e| e == "py" || e == "pyi")
                && let Ok(source) = std::fs::read_to_string(&path)
            {
                files.push((path.strip_prefix(root).unwrap().to_string_lossy().into_owned(), source));
            }
        }
    }
    files.sort();
    files
}

/// W's calls oracle project and the package fixture: every stage's rows are W's.
#[test]
fn each_stage_of_ws_reference_graph_is_the_rules_on_the_projects() {
    let project = std::path::PathBuf::from(crate::code_rule_dsl::fixtures::fixtures_dir!()).join("python/calls/project");
    let package: Vec<(String, String)> = super::reference_tests::PACKAGE.iter().map(|(p, s)| ((*p).to_owned(), (*s).to_owned())).collect();
    let root = crate::code_rule_dsl::fixtures::temp().with_extension("reference-stages");
    let mut failures = Vec::new();
    for (name, files) in [("oracle-project", files_under(&project)), ("package", package)] {
        let dir = root.join(name);
        let _ = std::fs::remove_dir_all(&dir);
        write_files(&dir, files.iter().map(|(p, s)| (p.as_str(), s.as_str())));
        let w = w_side(&files);
        let ours = rules_side(&dir);
        let _ = std::fs::remove_dir_all(&dir);
        println!("{name}:\n{}", report(&w, &ours));
        failures.extend(differences(&w, &ours, 6).into_iter().map(|d| format!("{name} {d}")));
    }
    assert!(failures.is_empty(), "stages differ from W's:\n{}", failures.join("\n"));
}

/// The rows do not depend on how the files are chunked: the package's files, a chunk or a few each, give the rows one
/// chunk gives. (s1000 in two chunks lost 1,433 `<unqualified>` references once the setting's relations were stated,
/// t50 step 1.)
#[test]
fn each_stage_is_the_same_in_small_chunks() {
    let package: Vec<(String, String)> = super::reference_tests::PACKAGE.iter().map(|(p, s)| ((*p).to_owned(), (*s).to_owned())).collect();
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("reference-stages-chunks");
    let _ = std::fs::remove_dir_all(&dir);
    write_files(&dir, package.iter().map(|(p, s)| (p.as_str(), s.as_str())));
    let (one, small) = (rules_side_in(&dir, 1 << 18), rules_side_in(&dir, 1 << 6));
    let _ = std::fs::remove_dir_all(&dir);
    assert!(one.rows.get("ref").is_some_and(|r| !r.is_empty()), "the package has references");
    let differ = differences(&one, &small, 6);
    assert!(differ.is_empty(), "one chunk (W's side here) and small chunks differ:\n{}", differ.join("\n"));
}

/// The cases the corpus runs found where a stage differed from W's, each a few files: every stage's rows are W's.
const CASES: &[(&str, &[(&str, &str)])] = &[
    // A typed splat's name and a type parameter's name bind; neither is a read (`bind_only`). A subscript's
    // arguments in an annotation (`dict[str, T]`, `type_parameter` nodes too) are reads.
    (
        "splats-and-type-parameters",
        &[(
            "a.py",
            "def f[T](x: T, *args: int, **kwargs: str) -> dict[str, T]:\n    y: list[T] = [x]\n    print(args, kwargs, y)\n    return y\n\n\nclass K[U]:\n    def m(self, *rest: U) -> None:\n        print(rest)\n\n\ndef g():\n    return f(1)\n",
        )],
    ),
    // A class reached by the name its file's path spells (no `__init__.py` between the package and the file): its
    // order, so its bases' methods, are its own name's.
    (
        "a-path-spelled-class",
        &[
            ("impl/__init__.py", ""),
            ("impl/mongo/conn.py", "class Core:\n    def m(self):\n        return 1\n\n\nclass Sync(Core):\n    pass\n"),
            ("impl/mongo/t.py", "def use():\n    from impl.mongo.conn import Sync\n    return Sync.m\n"),
        ],
    ),
    // A name a function declares `global` and binds is, to W's walk, the function's own (Ruff records the binding
    // in the declaring scope too): no reference. Another function's read is the module's.
    (
        "a-global-declared-name",
        &[(
            "a.py",
            "_on = False\n\n\ndef configure():\n    global _on\n    _on = True\n    if _on:\n        print(1)\n\n\ndef read():\n    configure()\n    return _on\n",
        )],
    ),
    // A read in a function that declares a name `global`, before the function binds it (`+=` after) or when it
    // never does, is the module's.
    (
        "global-reads-before-binding",
        &[(
            "a.py",
            "_n = 0\n_m = 0\n\n\ndef bump():\n    global _n\n    if _n >= 3:\n        return\n    _n += 1\n\n\ndef peek():\n    global _m\n    print(_m)\n    bump()\n",
        )],
    ),
    // `nonlocal` names the nearest enclosing function that binds the name, past one that declares it `nonlocal`
    // itself.
    (
        "a-nonlocal-chain",
        &[(
            "a.py",
            "def outer():\n    n = 0\n\n    def mid():\n        nonlocal n\n\n        def inner():\n            nonlocal n\n            n += 1\n            if n == 1:\n                print(n)\n\n        inner()\n\n    mid()\n    return outer\n",
        )],
    ),
    // A class body reads a name it binds only later (`x = x`) from outside: the class's binding is not made yet.
    (
        "a-class-reads-ahead",
        &[(
            "a.py",
            "def f():\n    return 1\n\n\nclass C:\n    f = f\n    g = f\n\n\ndef use():\n    return f(), C.g\n",
        )],
    ),
    // The same, the read in a decorated generator under `from __future__ import annotations`.
    (
        "global-read-in-a-decorated-generator",
        &[(
            "a.py",
            "from __future__ import annotations\n\nimport contextlib\n\n_s = 0\n\n\n@contextlib.contextmanager\ndef cap(phase: str, *, test: str | None = None):\n    global _s\n    if (\n        not phase or _s >= 3\n    ):\n        yield\n        return\n    _s += 1\n    print(f\"{_s}\")\n    yield\n\n\ndef use():\n    return cap(\"a\")\n",
        )],
    ),
    // `type(self).x = ...` is a type alias statement to tree-sitter's grammar (and the tape): its left side
    // declares, and W reads nothing of it.
    (
        "a-plain-call-in-a-target",
        &[(
            "a.py",
            "class A:\n    def m(self):\n        type(self)._x = True\n        return self.n()\n\n    def n(self):\n        return 0\n",
        )],
    ),
    // The same inside an `if` after a `try`.
    (
        "a-call-in-a-target",
        &[(
            "a.py",
            "class A:\n    def m(self, ok):\n        if ok:\n            try:\n                pass\n            except Exception:\n                pass\n            type(self)._x = True\n        return self.n()\n\n    def n(self):\n        return 0\n",
        )],
    ),
    // In a dotted class or value pattern only the first name is read; the rest are attributes.
    (
        "a-dotted-pattern",
        &[(
            "a.py",
            "import os\n\n\ndef f(e):\n    match e:\n        case os.PathLike():\n            return 1\n        case os.sep:\n            return 2\n    return g()\n\n\ndef g():\n    return 0\n",
        )],
    ),
    // A PEP 696 type parameter with a bound and a default declares its name.
    (
        "a-type-parameter-default",
        &[(
            "a.py",
            "class X[T: object = object]:\n    def m(self, v: T) -> T:\n        return v\n\n\ndef use():\n    return X().m(1)\n",
        )],
    ),
    // A parameter's annotation and default, and a lambda's default, are read in the scope around the definition.
    (
        "annotations-and-defaults-outside",
        &[(
            "a.py",
            "import numpy as np\n\npayload = b\"\"\n\n\ndef f(v: np.ndarray, data: bytes = payload) -> np.ndarray:\n    import numpy as np\n    payload = data\n    g = lambda key=payload: key\n    return np.asarray(v), g\n\n\ndef use():\n    return f(1)\n",
        )],
    ),
    // A nested function's parameter it rebinds is its local, not the parameter W's reference graph spells.
    (
        "a-rebound-nested-parameter",
        &[(
            "a.py",
            "def outer(rows):\n    def inner(type_id: int, name: str) -> None:\n        if rows:\n            type_id = rows[0]\n        print(type_id, name)\n    inner(1, \"a\")\n    return outer\n",
        )],
    ),
    // A parameter annotated with a union over lines: its type is the first name, without the line's whitespace.
    (
        "a-union-over-lines",
        &[(
            "a.py",
            "class A:\n    def go(self):\n        return 1\n\n\nclass B:\n    pass\n\n\ndef use(\n    p: A\n    | B,\n) -> None:\n    p.go()\n    print(p.x)\n",
        )],
    ),
    // A method's parenthesized local import, its names called in a loop.
    (
        "a-local-import-in-a-loop",
        &[
            ("pkg/__init__.py", ""),
            ("pkg/helpers.py", "def build(p):\n    return [p]\n\n\ndef name_of(i):\n    return str(i)\n"),
            (
                "pkg/s.py",
                "class S:\n    def run(self, items):\n        from pkg.helpers import (\n            build,\n            name_of,\n        )\n        out = []\n        for i in items:\n            if not i:\n                continue\n            p = name_of(i)\n            out.extend(\n                build(p),\n            )\n        return out\n",
            ),
        ],
    ),
    // A function imports the same name twice, in an `if` and after it, from the same module: each call is the name.
    (
        "a-name-imported-twice-in-a-function",
        &[
            ("pkg/__init__.py", ""),
            ("pkg/helpers.py", "def build(p):\n    return [p]\n"),
            (
                "pkg/s.py",
                "def run(ok, items):\n    out = []\n    if ok:\n        from pkg.helpers import build\n        for i in items:\n            out.extend(build(i))\n    from pkg.helpers import build\n    for i in items:\n        out.extend(build(i))\n    return out\n",
            ),
        ],
    ),
    // A type alias's type parameters bind their names; their bounds and defaults are reads (W's scope table reads
    // them lazily), as a `def`'s and a `class`'s are (`tp_alias_read`; on all, 19 reads of `get_history.py`).
    (
        "a-type-aliass-bounds",
        &[(
            "a.py",
            "class Base:\n    pass\n\n\ntype A[T: Base, R: dict[str, Base] = Base] = list[T]\n\n\ndef f():\n    return g()\n\n\ndef g():\n    return Base()\n",
        )],
    ),
    // Under `from __future__ import annotations`, an annotation in a generic function's body that names a type
    // parameter the module also binds (`T = TypeVar("T")` beside `def a[T]`) reads the module's (W's
    // `forward_module`): a reference, not a local (on all, 199 references of `typed_field.py`, `schema_def.py`, …).
    (
        "a-type-parameter-the-module-binds-read-late",
        &[(
            "pkg/tp.py",
            "from __future__ import annotations\n\nfrom typing import TypeVar\n\nT = TypeVar(\"T\")\n\n\nclass S:\n    def a[T](self, x: T) -> T:\n        def inner() -> T:\n            return x\n        return inner()\n\n    def b[T](self, x: T) -> T:\n        y: T = x\n        return y\n\n\nclass G[U]:\n    def m(self, x: U) -> U:\n        y: U = x\n        return y\n\n\ndef use():\n    return S().a(1), G().m(2)\n",
        )],
    ),
    // W's graph reads Python 3.10's builtins (Ruff's default target): `ExceptionGroup` is a module name there.
    (
        "a-later-builtin",
        &[("a.py", "def f():\n    try:\n        return g()\n    except ExceptionGroup:\n        return len([])\n\n\ndef g():\n    return 0\n")],
    ),
];

#[test]
fn each_stage_of_ws_reference_graph_is_the_rules_on_the_cases() {
    let root = crate::code_rule_dsl::fixtures::temp().with_extension("reference-stage-cases");
    let mut failures = Vec::new();
    for (name, files) in CASES {
        let files: Vec<(String, String)> = files.iter().map(|(p, s)| ((*p).to_owned(), (*s).to_owned())).collect();
        let dir = root.join(name);
        let _ = std::fs::remove_dir_all(&dir);
        write_files(&dir, files.iter().map(|(p, s)| (p.as_str(), s.as_str())));
        let (w, ours) = (w_side(&files), rules_side(&dir));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(w.rows.get("edge").is_some_and(|e| !e.is_empty()), "{name}: W makes edges");
        failures.extend(differences(&w, &ours, 6).into_iter().map(|d| format!("{name} {d}")));
    }
    assert!(failures.is_empty(), "stages differ from W's:\n{}", failures.join("\n"));
}

/// Each stage's CPU on each side over the corpus at `RERAIL_STAGE_CORPUS`, and where their rows differ.
#[test]
#[ignore = "a measurement: RERAIL_STAGE_CORPUS=<dir of Python files>"]
fn reference_stages_timed() {
    let root = std::path::PathBuf::from(std::env::var("RERAIL_STAGE_CORPUS").expect("RERAIL_STAGE_CORPUS"));
    let files = files_under(&root);
    let w = w_side(&files);
    // `RERAIL_STAGE_CHUNK=<nodes>`: the run's chunk size (2^18 by default).
    let chunk = std::env::var("RERAIL_STAGE_CHUNK").ok().and_then(|c| c.parse().ok()).unwrap_or(1 << 18);
    let ours = rules_side_in(&root, chunk);
    println!("{} files\n{}", files.len(), report(&w, &ours));
    for d in differences(&w, &ours, 3) {
        println!("differs: {d}");
    }
    // `RERAIL_STAGE_DIFFS=1`: every differing row, a line each: `diff <relation> <side> <row>`.
    if std::env::var_os("RERAIL_STAGE_DIFFS").is_some() {
        for (name, _) in RELATIONS {
            let (a, b) = (w.rows.get(name).cloned().unwrap_or_default(), ours.rows.get(name).cloned().unwrap_or_default());
            for r in a.difference(&b) {
                println!("diff {name} w {}", r.join("|"));
            }
            for r in b.difference(&a) {
                println!("diff {name} rules {}", r.join("|"));
            }
        }
    }
}
