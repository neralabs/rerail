//! `reference_edge` and `reference_open` (`logic::plan::corpus::references`) against W's reference graph on the
//! replay's fixtures: for every event of the policies that read the graph (`internal-dead-code`'s relational
//! `references`, `imperative-private-functions`' `calls { graph: references }`), the corpus runner's rows under the
//! converter's `.references` setting equal the graph W's evaluation of the policy installs (`eval::stateless_calls`)
//! for the event's files and scope.

use super::{Adjust, Failure, convert, events, rule_file, write_files};
use crate::code_rule_dsl::calls::{self, contracts};
use crate::code_rule_dsl::test_support::CAPABILITIES;
use crate::code_rule_dsl::typing::file_local::Binding;
use crate::code_rule_dsl::{CodeRuleDslRuntime, eval, model::Metrics};
use serde_json::{Value as Json, json};
use std::collections::{BTreeMap, BTreeSet};

/// The checks whose policies read the reference graph.
const CHECKS: [&str; 2] = ["internal_dead_code", "imperative_private_functions"];

/// An edge as (source path, start, end, target path, start, end), and a definition as (path, start, end).
type Edge = (String, u32, u32, String, u32, u32);
type Def = (String, u32, u32);

/// What one side gives: the edges with their labels, and the open definitions.
#[derive(Debug, Default, PartialEq)]
struct Graph {
    edges: BTreeSet<(Edge, String)>,
    open: BTreeSet<Def>,
}

impl Graph {
    fn pairs(&self) -> BTreeSet<Edge> {
        self.edges.iter().map(|(e, _)| e.clone()).collect()
    }
}

fn edge(e: &calls::closure::ReferenceEdge) -> Edge {
    (e.path.to_string(), e.source.start, e.source.end, e.target_path.to_string(), e.target.start, e.target.end)
}

/// W's graph for the policy `document` over the Python `files`: its key, its `reference_rows` edges, and the graph
/// itself read as the corpus runner reads it (`labelled_reference_rows`, `reference_open`).
fn w(document: &Json, files: &[(String, String)]) -> (contracts::Key, BTreeSet<Edge>, Graph) {
    let compiled = CodeRuleDslRuntime::compile(document.to_string(), "json".to_owned(), CAPABILITIES.to_owned()).unwrap_or_else(|e| panic!("{}", e.reason));
    let rule = &compiled.inner;
    let keys: BTreeSet<contracts::Key> = rule.clauses.iter().filter_map(|c| c.kind.calls_query()).flat_map(|q| q.calls()).filter(|t| t.through.references).map(|t| t.through.links).collect();
    assert_eq!(keys.len(), 1, "{}: one reference graph", document["id"]);
    let key = *keys.first().unwrap();
    let snapshot = json!({"files": files.iter().map(|(path, source)| json!({"path": path, "language": "python", "source": source})).collect::<Vec<_>>()});
    let (snapshot, _) = eval::parse_snapshot(&snapshot.to_string(), false).unwrap_or_else(|e| panic!("{}", e.reason));
    let mut metrics = Metrics::default();
    let _slot = eval::stateless_calls(rule, &snapshot, &rule.clauses, &mut metrics).unwrap_or_else(|e| panic!("{}", e.reason)).expect("the policy's terms install a graph");
    let rows = calls::reference_rows(key).expect("W linked the policy's key").edges.iter().map(edge).collect();
    let graph = calls::with_reference_graph(key, |g| Graph {
        edges: g.labelled_reference_rows().iter().map(|(e, via)| (edge(e), (*via).to_owned())).collect(),
        open: g.reference_open().into_iter().map(|(path, def)| (path.to_string(), def.start, def.end)).collect(),
    })
    .unwrap();
    (key, rows, graph)
}

/// The corpus runner's graph over the files under `dir` under the `.references` setting `setting`.
fn ours(dir: &std::path::Path, setting: &str) -> Graph {
    let program = format!(
        "{setting}
.decl doc(d: number, path: symbol, module: symbol)
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl reference_edge(src: node, dst: node, via: symbol)
.decl reference_open(n: node)
.input doc, node, reference_edge, reference_open
.decl edge(sp: symbol, ss: number, se: number, tp: symbol, ts: number, te: number, via: symbol)
edge(sp, ss, se, tp, ts, te, v) :- reference_edge(s, t, v), node(d, s, _, ss, se), doc(d, sp, _), node(e, t, _, ts, te), doc(e, tp, _).
.decl open(p: symbol, s: number, e: number)
open(p, s, e) :- reference_open(n), node(d, n, _, s, e), doc(d, p, _).
.output edge, open
"
    );
    let out = dir.join(".out");
    crate::code_rule_dsl::logic::plan::corpus::run(&program, dir, None, &out, "program", Binding::Cpu, 1 << 18).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
    let read = |rel: &str| std::fs::read_to_string(out.join(format!("{rel}.csv"))).unwrap_or_default();
    let num = |s: &str| s.parse::<u32>().unwrap();
    Graph {
        edges: read("edge")
            .lines()
            .map(|l| {
                let f: Vec<&str> = l.split('\t').collect();
                ((f[0].to_owned(), num(f[1]), num(f[2]), f[3].to_owned(), num(f[4]), num(f[5])), f[6].to_owned())
            })
            .collect(),
        open: read("open")
            .lines()
            .map(|l| {
                let f: Vec<&str> = l.split('\t').collect();
                (f[0].to_owned(), num(f[1]), num(f[2]))
            })
            .collect(),
    }
}

/// What the comparisons saw: runs, W's edges, open definitions and labels.
#[derive(Default)]
struct Seen {
    compared: usize,
    edges: usize,
    open: usize,
    vias: BTreeMap<String, usize>,
    failures: Vec<String>,
}

/// `rule` run as `adjust` configures it over `files` in `dir`: the converter's `.references` setting names W's key and
/// scope, and the corpus runner's rows are W's graph's (`w`). A difference is a failure in `seen`, named `what`;
/// `None` where the run's rule cannot be configured so.
fn compare(rule: &std::path::Path, adjust: &Adjust, files: &[(String, String)], dir: &std::path::Path, what: &str, seen: &mut Seen) -> Option<()> {
    let converted = match convert(rule, adjust) {
        Ok(c) => c,
        Err(Failure::Config(_)) => return None,
        Err(e) => panic!("{what}: {e}"),
    };
    let setting = converted.program.lines().find(|l| l.starts_with(".references ")).unwrap_or_else(|| panic!("{what}: no .references setting")).to_owned();
    let config: Json = serde_json::from_str(&setting[".references ".len()..]).unwrap();
    // The policy as W runs it: edited as the converter edits it, over the run's scope.
    let mut document: Json = serde_json::from_str(&std::fs::read_to_string(rule).unwrap()).unwrap();
    if let Some(edit) = &adjust.policy {
        edit(&mut document).unwrap();
    }
    if let Some((include, exclude)) = &adjust.scope {
        document["scope"] = json!({"include": include, "exclude": exclude});
    }
    assert_eq!(config["scope"]["include"], document["scope"]["include"], "{what}: the setting's scope");
    assert_eq!(config["scope"]["exclude"], document["scope"]["exclude"].as_array().map_or(json!([]), |e| json!(e)), "{what}: the setting's scope");
    let files: Vec<(String, String)> = files.iter().filter(|(p, _)| p.ends_with(".py")).cloned().collect();
    let _ = std::fs::remove_dir_all(dir);
    write_files(dir, files.iter().map(|(p, s)| (p.as_str(), s.as_str())));
    let got = ours(dir, &setting);
    let _ = std::fs::remove_dir_all(dir);
    let (rows, want) = if config["scope"]["include"].as_array().is_some_and(Vec::is_empty) {
        (BTreeSet::new(), Graph::default())
    } else {
        let (key, rows, want) = w(&document, &files);
        let ours = contracts::compile(config.get("siblingLinks"), config.get("constructorFields"), "/references", usize::MAX).unwrap();
        assert_eq!(ours, key, "{what}: the setting's key is W's");
        assert_eq!(want.pairs(), rows, "{what}: W's labelled rows are its reference rows");
        (rows, want)
    };
    seen.compared += 1;
    seen.edges += rows.len();
    seen.open += want.open.len();
    for (_, via) in &want.edges {
        *seen.vias.entry(via.clone()).or_default() += 1;
    }
    if got.pairs() != rows || got != want {
        let missing: Vec<_> = want.edges.difference(&got.edges).take(5).collect();
        let extra: Vec<_> = got.edges.difference(&want.edges).take(5).collect();
        let open_missing: Vec<_> = want.open.difference(&got.open).take(5).collect();
        let open_extra: Vec<_> = got.open.difference(&want.open).take(5).collect();
        seen.failures.push(format!("{what}: edges missing {missing:?} extra {extra:?}; open missing {open_missing:?} extra {open_extra:?}"));
    }
    Some(())
}

/// Every event of the two policies: the corpus runner's `reference_edge` rows (with their labels) and
/// `reference_open` rows are W's graph's for the event's files and scope; the labelled rows' pairs are W's
/// `reference_rows`, which its relational stages read.
#[test]
fn reference_edge_is_ws_reference_graph_on_the_fixtures() {
    let root = crate::code_rule_dsl::fixtures::temp().with_extension("references");
    let mut seen = Seen::default();
    let mut skipped = 0;
    for (k, event) in events().into_iter().enumerate().filter(|(_, e)| CHECKS.contains(&e.check.as_str())) {
        let rule = rule_file(&event.check).expect("the policy's file");
        let what = format!("{} {}", event.check, event.test);
        let compared = event.adjust.as_ref().ok().and_then(|adjust| compare(&rule, adjust, &event.files, &root.join(format!("e{k}")), &what, &mut seen));
        skipped += usize::from(compared.is_none());
    }
    let _ = std::fs::remove_dir_all(&root);
    println!("compared {} events ({skipped} not configurable): {} edges, {} open definitions; edges by via {:?}", seen.compared, seen.edges, seen.open, seen.vias);
    assert!(seen.compared > 0 && seen.edges > 0, "the fixtures exercise the graph");
    assert!(seen.failures.is_empty(), "{} events differ from W:\n{}", seen.failures.len(), seen.failures.join("\n"));
}

/// A package exercising every way an edge is made and a definition is open: re-exports through `__init__`, a bare
/// name and a class's own, `self`, `cls`, `super()`, a constructor field (`_conn` from parameter `conn: Conn`), a
/// parameter's declared type, both sibling links, a star import, a module member no definition gives, and a file
/// that does not parse.
const PACKAGE: [(&str, &str); 8] = [
    ("pkg/__init__.py", "from .base import Base\nfrom .store import Store\n__all__ = [\"Base\", \"Store\"]\n"),
    ("pkg/conn.py", "class Conn:\n    def execute(self, q):\n        return q\n\n    def close(self):\n        pass\n"),
    ("pkg/base.py", "class Base:\n    def setup(self):\n        return self.prepare()\n\n    def prepare(self):\n        return 1\n"),
    (
        "pkg/store.py",
        r#"from pkg.base import Base
from pkg.conn import Conn
from . import conn as conn_mod


def legacy_sync(f):
    return f


def legacy_async(f):
    return f


class Store(Base):
    def __init__(self, conn: Conn):
        self._conn = conn

    def run(self, q):
        return self._conn.execute(q)

    def use(self, c: Conn):
        c.close()
        return super().setup()

    @legacy_sync
    def fetch(self):
        return 1

    def fetch_lazy(self):
        return 2

    @legacy_async
    def load_async(self):
        return 3

    def load_lazy(self):
        return helper()

    @classmethod
    def make(cls):
        return cls(None).run("x")

    def again(self):
        return fetch_lazy


def helper():
    return conn_mod.Conn()


def unused():
    return Store.make()
"#,
    ),
    ("pkg/star.py", "from pkg.store import *\n\n\ndef h():\n    return helper()\n\n\ndef k():\n    return missing()\n"),
    ("pkg/dyn.py", "import pkg.conn\n\n\ndef d(name):\n    return pkg.conn.nothing_here(name)\n\n\ndef e():\n    return pkg.base.Base().setup()\n"),
    ("pkg/broken.py", "def f(:\n    pass\n\n\ndef g():\n    return helper()\n"),
    ("app.py", "from pkg import Store\nfrom pkg.store import helper as aid\n\n\ndef main():\n    aid()\n    return Store(None).run(\"q\")\n"),
];

/// W's calls oracle project (`fixtures/python/calls/project`) and `PACKAGE`, under each policy over every file: the
/// corpus runner's rows are W's graph's, and between them every label but `late` and an open definition occur.
#[test]
fn reference_edge_is_ws_reference_graph_on_projects() {
    let project = std::path::PathBuf::from(crate::code_rule_dsl::fixtures::fixtures_dir!()).join("python/calls/project");
    let mut oracle = Vec::new();
    let mut pending = vec![project.clone()];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|e| e == "py") {
                oracle.push((path.strip_prefix(&project).unwrap().to_string_lossy().into_owned(), std::fs::read_to_string(&path).unwrap()));
            }
        }
    }
    let package: Vec<(String, String)> = PACKAGE.iter().map(|(p, s)| ((*p).to_owned(), (*s).to_owned())).collect();
    let root = crate::code_rule_dsl::fixtures::temp().with_extension("reference-projects");
    let mut seen = Seen::default();
    for check in CHECKS {
        let rule = rule_file(check).expect("the policy's file");
        for (name, files) in [("oracle-project", &oracle), ("package", &package)] {
            let adjust = Adjust { scope: Some((vec!["**".to_owned()], Vec::new())), ..Adjust::default() };
            compare(&rule, &adjust, files, &root.join(format!("{check}-{name}")), &format!("{check} {name}"), &mut seen).expect("configurable");
        }
    }
    println!("compared {} runs: {} edges, {} open definitions; edges by via {:?}", seen.compared, seen.edges, seen.open, seen.vias);
    assert!(seen.failures.is_empty(), "{} runs differ from W:\n{}", seen.failures.len(), seen.failures.join("\n"));
    for via in ["name", "bare", "unqualified", "self", "super", "field", "param_type", "sibling"] {
        assert!(seen.vias.contains_key(via), "no edge {via}: {:?}", seen.vias);
    }
    assert!(seen.open > 0, "no open definition");
}
