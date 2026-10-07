use super::closure::{Bits, Closure, Graph, Hop, Seed, Status, path_module};
use super::names::Name;
use super::summary::{ASYNC, CLASS, Callee, OPEN, Summary, callee_at};
use super::{CallsView, FileSeeds, SeedScope, Through};
use crate::content_store::HashKey;
use crate::policy_dsl::language::GrammarDialect;
use crate::policy_dsl::module_path::ModulePath;
use crate::policy_dsl::scope::{Reading, ScopeTable};
use ast_grep_core::{Doc, Node};
use ast_grep_language::LanguageExt;
use std::collections::HashMap;
use std::sync::Arc;

fn table<D: Doc>(root: Node<'_, D>, module: &ModulePath) -> ScopeTable {
    ScopeTable::build_in(
        root,
        GrammarDialect::Python,
        usize::MAX,
        &|| false,
        Reading::default(),
        Some(module),
    )
    .expect("table builds")
}

fn summarize(module: &str, source: &str) -> Summary {
    let root = GrammarDialect::Python.ast_grep(source);
    let module = ModulePath::dotted(module).expect("module path");
    let errors = root.root().get_doc().tree.root_node().has_error();
    let table = table(root.root(), &module);
    Summary::extract(&root.root(), Some(&table), Some(&module), errors)
}

/// `(caller, callee)` for every edge.
fn edges(summary: &Summary) -> Vec<(String, String)> {
    let mut edges = summary
        .edges
        .iter()
        .map(|(owner, name)| {
            let caller = summary.name(summary.defs[*owner as usize].name).to_owned();
            (caller, summary.name(*name).to_owned())
        })
        .collect::<Vec<_>>();
    edges.sort();
    edges
}

fn pairs(items: &[(&str, &str)]) -> Vec<(String, String)> {
    let mut pairs = items
        .iter()
        .map(|(a, b)| ((*a).to_owned(), (*b).to_owned()))
        .collect::<Vec<_>>();
    pairs.sort();
    pairs
}

const SERVICE: &str = r#"
import time
import os.path
import requests as r
from .helpers import load
from ..shared import util as u
from pkg.io import *
from pkg.clock import pause as _pause

stop_all = u.stop

async def handler(self_like):
    time.sleep(1)
    os.path.join("a")
    r.get("x")
    load()
    u.run()
    later()
    missing()
    self_like.method()
    print("x")
    (lambda: time.sleep(2))()
    def inner():
        helper()
    inner()

class Service:
    def __init__(self, x=helper()):
        self.start()
    def start(self):
        Service.stop(self)
        if True:
            def nested_in_if(self):
                pass
    @staticmethod
    def stop(x):
        x.start()

def later():
    pass

def helper():
    pass

helper()
"#;

#[test]
fn summaries_name_callees_by_qualified_name() {
    let summary = summarize("pkg.sub.api", SERVICE);
    let defs = summary
        .defs
        .iter()
        .map(|def| (summary.name(def.name).to_owned(), def.flags))
        .collect::<Vec<_>>();
    let m = |name: &str| format!("pkg.sub.api.{name}");
    assert_eq!(
        defs,
        vec![
            // `missing()` is unbound (the star import may bind it), `self_like.method()` a
            // parameter's: the caller is open.
            (m("handler"), ASYNC | OPEN),
            (m("handler.<locals>.inner"), 0),
            (m("Service"), CLASS),
            (m("Service.__init__"), 0),
            (m("Service.start"), 0),
            (m("Service.start.<locals>.nested_in_if"), 0),
            // A static method has no receiver: `x.start()` is a parameter's method.
            (m("Service.stop"), OPEN),
            (m("later"), 0),
            (m("helper"), 0),
        ]
    );
    let (handler, inner) = (m("handler"), m("handler.<locals>.inner"));
    let (init, start, stop) = (m("Service.__init__"), m("Service.start"), m("Service.stop"));
    assert_eq!(
        edges(&summary),
        pairs(&[
            (&handler, "time.sleep"),
            (&handler, "os.path.join"),
            (&handler, "requests.get"),
            (&handler, "pkg.sub.helpers.load"),
            (&handler, "pkg.shared.util.run"),
            // A later module-level definition is visible from a function body.
            (&handler, &m("later")),
            // A builtin is outside every workspace; the lambda's `time.sleep(2)` is the
            // handler's, which runs or passes on the lambda.
            (&handler, &inner),
            (&inner, &m("helper")),
            // A default argument is evaluated where the definition is, not in the method; a
            // method of `self` is late bound, an explicit `Service.stop` is not.
            (&init, &format!("{start}^")),
            (&start, &stop),
        ])
    );
    let targets = summary
        .targets
        .iter()
        .map(|name| summary.name(*name))
        .collect::<std::collections::BTreeSet<_>>();
    // Calls outside any definition are targets too: the lambda's, the default's, the module's.
    assert!(targets.contains(m("helper").as_str()) && targets.contains("time.sleep"));
    assert!(!targets.iter().any(|target| target.starts_with("builtins")));
    assert_eq!(
        summary
            .aliases
            .iter()
            .map(|(a, t)| (summary.name(*a), summary.name(*t)))
            .collect::<Vec<_>>(),
        vec![
            ("pkg.sub.api._pause", "pkg.clock.pause"),
            ("pkg.sub.api.load", "pkg.sub.helpers.load"),
            // `import os.path` binds `os`.
            ("pkg.sub.api.os", "os"),
            ("pkg.sub.api.r", "requests"),
            // An assignment of a name aliases it.
            ("pkg.sub.api.stop_all", "pkg.shared.util.stop"),
            ("pkg.sub.api.time", "time"),
            ("pkg.sub.api.u", "pkg.shared.util"),
        ]
    );
    assert_eq!(
        summary
            .stars
            .iter()
            .map(|s| summary.name(*s))
            .collect::<Vec<_>>(),
        vec!["pkg.io"]
    );
    assert!(!summary.open);
    assert_eq!(Summary::decode(&summary.encode()).as_ref(), Some(&summary));
}

#[test]
fn a_call_site_names_what_extraction_recorded() {
    let root = GrammarDialect::Python.ast_grep(SERVICE);
    let module = ModulePath::dotted("pkg.sub.api").expect("module path");
    let table = table(root.root(), &module);
    let summary = Summary::extract(&root.root(), Some(&table), Some(&module), false);
    let mut named = std::collections::BTreeSet::new();
    let mut open = 0;
    for node in root.root().dfs().filter(|node| node.kind() == "call") {
        match callee_at(&table, Some(&module), &summary, &node) {
            Some(Callee::Name(name)) => {
                named.insert(name);
            }
            Some(Callee::Open) => open += 1,
            None => {}
        }
    }
    let targets = summary
        .targets
        .iter()
        .map(|name| summary.name(*name).to_owned())
        .collect();
    assert_eq!(named, targets);
    // `missing()`, the default `helper()` (a class body sees only earlier bindings), a
    // parameter's method, the lambda called where it is written, and `x.start()`.
    assert_eq!(open, 5);
}

/// The ticket's rule: an async function's un-awaited call of a sync function that reaches blocking
/// I/O, through sync functions, in any file.
pub(crate) fn blocking_rule() -> serde_json::Value {
    serde_json::json!({
        "kind": "function_definition",
        "regex": "^async ",
        "has": {
            "stopBy": { "kind": "function_definition" },
            "kind": "call",
            "not": { "inside": { "kind": "await" } },
            "calls": {
                "through": "sync",
                "has": {
                    "stopBy": { "kind": "function_definition" },
                    "kind": "call",
                    "has": {
                        "field": "function",
                        "resolves": { "import": ["time.sleep", "requests.get", "subprocess.run", "open"] }
                    }
                }
            }
        }
    })
}

pub(crate) fn blocking_policy(rule: serde_json::Value) -> crate::policy_dsl::CompiledPolicyDsl {
    use crate::policy_dsl::test_support::*;
    let document = check_document(serde_json::json!({
        "id": "async-blocking",
        "forbid": { "ast": { "language": "python", "rule": rule } },
        "violation": finding(),
    }));
    let capabilities: serde_json::Value = serde_json::from_str(CAPABILITIES).expect("capabilities");
    compile_document_with(document, &capabilities)
        .unwrap_or_else(|error| panic!("{}", error.reason))
}

fn evaluate_snapshot(
    compiled: &crate::policy_dsl::CompiledPolicyDsl,
    files: &[(&str, &str, &str)],
) -> serde_json::Value {
    let files = files
        .iter()
        .map(|(path, module, source)| {
            serde_json::json!({ "language": "python", "path": path, "module": module, "source": source })
        })
        .collect::<Vec<_>>();
    let snapshot = serde_json::json!({
        "facts": [], "files": files, "format": "rerail.repository-snapshot/v1", "revision": 0,
    });
    let output = compiled
        .evaluate(snapshot.to_string(), "diagnostic".to_owned())
        .expect("evaluation succeeds");
    serde_json::from_str(&output).expect("evaluation is JSON")
}

pub(crate) const CHAIN: [(&str, &str, &str); 5] = [
    (
        "app/api.py",
        "app.api",
        "from app.services import fetch\n\nasync def handler():\n    fetch()\n\nasync def awaited():\n    await fetch()\n\nasync def fine():\n    helper()\n\ndef helper():\n    pass\n",
    ),
    ("app/__init__.py", "app.__init__", ""),
    (
        "app/services/__init__.py",
        "app.services.__init__",
        "from .impl import fetch\n",
    ),
    (
        "app/services/impl.py",
        "app.services.impl",
        "from app.util import pause\n\ndef fetch():\n    pause()\n",
    ),
    (
        "app/util.py",
        "app.util",
        "import time\n\ndef pause():\n    time.sleep(1)\n",
    ),
];

/// A seed query that only asks what a definition's body holds runs at the held nodes and maps
/// each back through the summary: the same definitions as the rule rooted at every definition,
/// through nested functions, classes, lambdas, decorators, defaults and aliases.
#[test]
fn a_seed_query_at_held_nodes_finds_the_definitions_rooted_rules_do() {
    const SOURCE: &str = "import time\nfrom time import sleep as nap\n\ndef outer():\n    time.sleep(1)\n    def inner():\n        nap(2)\n    class Local:\n        x = time.sleep(3)\n        def method(self):\n            time.sleep(4)\n    lam = lambda: time.sleep(5)\n    @decorate(time.sleep(6))\n    def decorated(a=time.sleep(7)):\n        pass\n\ndef clean():\n    print(1)\n\ndef wrapper():\n    def nested():\n        time.sleep(9)\n\nasync def later():\n    time.sleep(8)\n\nclass Top:\n    def m(self):\n        nap()\n    def n(self):\n        def deeper():\n            pass\n";
    let file = [("m.py", "m", SOURCE)];
    let held = serde_json::json!({
        "kind": "call",
        "has": { "field": "function", "resolves": { "import": ["time.sleep"] } }
    });
    let spans = |rule: serde_json::Value| {
        let result = evaluate_snapshot(&blocking_policy(rule), &file);
        assert_eq!(result["failures"], serde_json::json!([]), "{result}");
        result["violations"]
            .as_array()
            .expect("findings")
            .iter()
            .map(|finding| {
                let at = |key: &str| finding["span"][key].as_u64().expect("span") as u32;
                (at("startByte"), at("endByte"))
            })
            .collect::<Vec<_>>()
    };
    let calls = spans(held.clone());
    let witnesses = calls
        .iter()
        .map(|(start, end)| crate::policy_dsl::findings::Witness {
            path: "m.py".into(),
            span: crate::policy_dsl::findings::Span {
                start_byte: *start as usize,
                end_byte: *end as usize,
                ..Default::default()
            },
            captures: Default::default(),
            message: None,
            fix: None,
        })
        .collect::<Vec<_>>();
    let summary = summarize("m", SOURCE);
    for (stop, scope) in [
        (
            serde_json::json!({ "kind": "function_definition" }),
            SeedScope::Innermost,
        ),
        (serde_json::json!("end"), SeedScope::Enclosing),
    ] {
        let mut has = held.clone();
        has["stopBy"] = stop;
        let rooted = spans(
            serde_json::json!({ "all": [{ "kind": "function_definition" }, { "has": has }] }),
        )
        .into_iter()
        .map(|(start, _)| start)
        .collect::<std::collections::BTreeSet<_>>();
        let mapped = super::seed_starts(
            witnesses.iter().map(|witness| &witness.span),
            scope,
            Some(&summary),
        )
        .expect("a summary maps witnesses")
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>();
        assert!(!rooted.is_empty());
        assert_eq!(mapped, rooted, "{scope:?}");
    }
}

#[test]
fn a_stateless_evaluation_follows_calls_across_files() {
    let compiled = blocking_policy(blocking_rule());
    let result = evaluate_snapshot(&compiled, &CHAIN);
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
    let findings = result["violations"].as_array().expect("findings");
    // `handler` only: `awaited` awaits the call, and `fine`'s callee reaches nothing blocking.
    assert_eq!(findings.len(), 1, "{result}");
    assert_eq!(findings[0]["path"], "app/api.py");
    assert_eq!(findings[0]["span"]["startLine"], 2);
    // The sink removed from the last file: the finding in the unchanged first file goes.
    let mut quiet = CHAIN;
    quiet[4].2 = "import time\n\ndef pause():\n    pass\n";
    let result = evaluate_snapshot(&compiled, &quiet);
    assert_eq!(result["violations"], serde_json::json!([]), "{result}");
}

#[test]
fn an_open_chain_is_unknown_so_neither_claim_reports() {
    // The chain passes through a name a star import may bind: whether it reaches is unknown.
    let mut open = CHAIN;
    open[3].2 = "from app.star import *\n\ndef fetch():\n    pause()\n";
    let reach = evaluate_snapshot(&blocking_policy(blocking_rule()), &open);
    assert_eq!(reach["violations"], serde_json::json!([]), "{reach}");
    // "must reach": an async function none of whose calls reaches the sink. Unknown is no finding.
    let absent = serde_json::json!({
        "kind": "function_definition",
        "regex": "^async def handler",
        "not": { "has": {
            "stopBy": { "kind": "function_definition" },
            "kind": "call",
            "calls": { "through": "sync", "has": {
                "stopBy": { "kind": "function_definition" }, "kind": "call",
                "has": { "field": "function", "resolves": { "import": ["time.sleep"] } }
            } }
        } }
    });
    let result = evaluate_snapshot(&blocking_policy(absent.clone()), &open);
    assert_eq!(result["violations"], serde_json::json!([]), "{result}");
    // Closed and absent: the finding holds.
    let mut closed = CHAIN;
    closed[4].2 = "def pause():\n    pass\n";
    let result = evaluate_snapshot(&blocking_policy(absent), &closed);
    assert_eq!(
        result["violations"].as_array().map(Vec::len),
        Some(1),
        "{result}"
    );
}

/// The seed pass's witnesses are no check's: seeds across the snapshot that together pass
/// `maxWitnesses` (each file's within it) leave the check its own witness budget, and its finding.
#[test]
fn seed_witnesses_do_not_spend_the_checks_witness_budget() {
    use crate::policy_dsl::test_support::*;
    let document = check_document(serde_json::json!({
        "id": "async-blocking",
        "forbid": { "ast": { "language": "python", "rule": blocking_rule() } },
        "violation": finding(),
    }));
    let capabilities = capabilities_with(|capabilities| {
        capabilities["limits"]["maxWitnesses"] = serde_json::json!(2);
    });
    let compiled = compile_document_with(document, &capabilities)
        .unwrap_or_else(|error| panic!("{}", error.reason));
    let mut files = CHAIN.to_vec();
    files.push((
        "app/jobs.py",
        "app.jobs",
        "import time\n\ndef first():\n    time.sleep(1)\n\ndef second():\n    time.sleep(2)\n",
    ));
    let result = evaluate_snapshot(&compiled, &files);
    assert_eq!(result["failures"], serde_json::json!([]), "{result}");
    let findings = result["violations"].as_array().expect("findings");
    assert_eq!(findings.len(), 1, "{result}");
    assert_eq!(findings[0]["path"], "app/api.py");
    assert_eq!(findings[0]["span"]["startLine"], 2);
}

#[test]
fn calls_is_refused_where_no_closure_is_read() {
    use crate::policy_dsl::test_support::*;
    let capabilities: serde_json::Value = serde_json::from_str(CAPABILITIES).expect("capabilities");
    let typescript = check_document(serde_json::json!({
        "id": "ts",
        "forbid": { "ast": { "language": "typescript", "rule": { "kind": "call_expression", "calls": { "kind": "identifier" } } } },
        "violation": finding(),
    }));
    assert_eq!(
        rejection(compile_document_with(typescript, &capabilities)),
        "/clauses/0/forbid/ast/rule/calls: `calls` is not available for typescript policies"
    );
    let nested = serde_json::json!({ "kind": "call", "calls": { "has": { "kind": "call", "calls": { "kind": "identifier" } } } });
    let document = check_document(serde_json::json!({
        "id": "nested",
        "forbid": { "ast": { "language": "python", "rule": nested } },
        "violation": finding(),
    }));
    assert!(
        rejection(compile_document_with(document, &capabilities)).contains("cannot contain calls")
    );
    let through = check_document(serde_json::json!({
        "id": "through",
        "forbid": { "ast": { "language": "python", "rule": { "kind": "call", "calls": { "through": "async", "kind": "identifier" } } } },
        "violation": finding(),
    }));
    assert!(rejection(compile_document_with(through, &capabilities)).contains("through"));
}

#[test]
fn a_package_init_is_named_by_its_package() {
    let summary = summarize(
        "pkg.__init__",
        "from .impl import run\n\ndef main():\n    run()\n",
    );
    assert_eq!(summary.name(summary.module.expect("module")), "pkg");
    assert_eq!(edges(&summary), pairs(&[("pkg.main", "pkg.impl.run")]));
    assert_eq!(
        summary
            .aliases
            .iter()
            .map(|(a, t)| (summary.name(*a), summary.name(*t)))
            .collect::<Vec<_>>(),
        vec![("pkg.run", "pkg.impl.run")]
    );
}

#[test]
fn path_modules_start_below_the_checkout() {
    assert_eq!(
        path_module("example-app/impl/a/b.py").as_deref(),
        Some("impl.a.b")
    );
    assert_eq!(path_module("pkg/__init__.py").as_deref(), Some("pkg"));
    assert_eq!(path_module("x/my-scripts/run.py").as_deref(), Some("run"));
    assert_eq!(path_module("README.md"), None);
}

#[test]
fn syntax_errors_open_the_summary() {
    assert!(summarize("pkg.broken", "def f(:\n    g()\n").open);
}

#[test]
fn decoding_rejects_truncated_or_foreign_bytes() {
    let summary = summarize("pkg.m", "def f():\n    g()\n");
    let bytes = summary.encode();
    assert_eq!(Summary::decode(&bytes[..bytes.len() - 1]), None);
    let mut foreign = bytes.clone();
    foreign[0] ^= 0xff;
    assert_eq!(Summary::decode(&foreign), None);
}

/// Term 0: an async caller's chain through sync functions to a blocking call.
const BLOCKING: [&str; 2] = ["time.sleep", "requests.get"];

fn sync_only(flags: u8) -> Bits {
    if flags & (ASYNC | CLASS) != 0 { 0 } else { 1 }
}

fn close(files: &[(&str, &str, &str)]) -> (Graph, Vec<Seed>, Closure) {
    let summaries = files
        .iter()
        .map(|(path, module, source)| (Arc::from(*path), Arc::new(summarize(module, source))))
        .collect::<Vec<_>>();
    let graph = Graph::link(summaries);
    let seeds = seeds_by_direct_call(&graph, &BLOCKING);
    let closure = graph.close(&seeds, sync_only, 1);
    (graph, seeds, closure)
}

/// Seeds for a term whose inner rule is "calls one of `names` directly".
fn seeds_by_direct_call(graph: &Graph, names: &[&str]) -> Vec<Seed> {
    (0..graph.def_count() as u32)
        .map(|def| {
            let (_, summary, local) = graph.def_file(def);
            let matched = summary
                .edges
                .iter()
                .any(|(owner, name)| *owner == local && names.contains(&summary.name(*name)));
            Seed {
                matched: Bits::from(matched),
                unknown: 0,
            }
        })
        .collect()
}

const REACH: Status = Status { reach: 1, open: 0 };
const OPEN_STATUS: Status = Status { reach: 0, open: 1 };
const NONE: Status = Status { reach: 0, open: 0 };

#[test]
fn a_chain_across_files_reaches_the_blocking_call() {
    let (graph, _, closure) = close(&[
        (
            "app/api.py",
            "app.api",
            "from app.services import fetch\n\nasync def handler():\n    fetch()\n",
        ),
        // The package re-exports `fetch` from its implementation module.
        (
            "app/services/__init__.py",
            "app.services.__init__",
            "from .impl import fetch\n",
        ),
        (
            "app/services/impl.py",
            "app.services.impl",
            "from app.util import pause\n\ndef fetch():\n    pause()\n",
        ),
        (
            "app/util.py",
            "app.util",
            "import time\n\ndef pause():\n    time.sleep(1)\n\ndef idle():\n    pass\n",
        ),
    ]);
    assert_eq!(
        graph.status(&closure, &Name::new("app.services.fetch")),
        REACH
    );
    assert_eq!(graph.status(&closure, &Name::new("app.util.idle")), NONE);
    assert_eq!(
        graph.chain(&closure, &Name::new("app.services.fetch"), 0),
        vec!["app.services.impl.fetch", "app.util.pause"]
    );
}

#[test]
fn a_namespace_package_file_is_found_by_its_path() {
    // `tests/` has no `__init__.py`: the package module is the stem, the import is by path.
    let (graph, _, closure) = close(&[
        (
            "app/tests/kit.py",
            "kit",
            "import time\n\ndef make():\n    time.sleep(1)\n",
        ),
        (
            "app/tests/test_x.py",
            "test_x",
            "from app.tests.kit import make\n\ndef case():\n    make()\n",
        ),
    ]);
    assert_eq!(
        graph.status(&closure, &Name::new("app.tests.kit.make")),
        REACH
    );
    assert_eq!(graph.status(&closure, &Name::new("test_x.case")), REACH);
}

#[test]
fn chains_stop_at_definitions_the_term_does_not_pass_through() {
    let (graph, _, closure) = close(&[(
        "m.py",
        "m",
        "import time\n\nasync def slow():\n    time.sleep(1)\n\ndef via():\n    slow()\n",
    )]);
    // `slow` is async: a sync-only term neither starts nor passes there.
    assert_eq!(graph.status(&closure, &Name::new("m.slow")), NONE);
    assert_eq!(graph.status(&closure, &Name::new("m.via")), NONE);
}

#[test]
fn recursion_terminates_and_colours_every_member() {
    let (graph, _, closure) = close(&[
        ("a.py", "a", "import b\n\ndef ping(n):\n    b.pong(n)\n"),
        (
            "b.py",
            "b",
            "import a\nimport time\n\ndef pong(n):\n    a.ping(n)\n    time.sleep(n)\n",
        ),
    ]);
    assert_eq!(graph.status(&closure, &Name::new("a.ping")), REACH);
    assert_eq!(graph.status(&closure, &Name::new("b.pong")), REACH);
}

#[test]
fn unknown_callees_are_open_never_absent() {
    let (graph, _, closure) = close(&[
        (
            "m.py",
            "m",
            "from star import *\n\ndef a():\n    mystery()\n\ndef b():\n    a()\n",
        ),
        ("s.py", "s", "from m import *\n\ndef c():\n    pass\n"),
        ("k.py", "k", "CONSTANT = object()\n"),
        (
            "n.py",
            "n",
            "import s\nimport k\n\ndef d():\n    s.not_defined()\n\ndef e():\n    obj = object()\n    obj.method()\n\ndef f():\n    k.CONSTANT()\n",
        ),
    ]);
    assert_eq!(graph.status(&closure, &Name::new("m.a")), OPEN_STATUS);
    assert_eq!(graph.status(&closure, &Name::new("m.b")), OPEN_STATUS);
    // A member of a module that star-imports, which no star import gives.
    assert_eq!(graph.status(&closure, &Name::new("n.d")), OPEN_STATUS);
    // `obj.method()` (a local's method) and a module's value no name denotes have no static
    // callee: unknown, never absent.
    assert_eq!(graph.status(&closure, &Name::new("n.e")), OPEN_STATUS);
    assert_eq!(graph.status(&closure, &Name::new("n.f")), OPEN_STATUS);
}

#[test]
fn reaching_wins_over_open() {
    let (graph, _, closure) = close(&[(
        "m.py",
        "m",
        "import time\n\ndef f():\n    unknown_name()\n    time.sleep(1)\n",
    )]);
    assert_eq!(graph.status(&closure, &Name::new("m.f")), REACH);
}

#[test]
fn constructing_a_class_calls_its_init() {
    let (graph, _, closure) = close(&[(
        "m.py",
        "m",
        "import time\n\nclass Client:\n    def __init__(self):\n        self.connect()\n    def connect(self):\n        time.sleep(1)\n\ndef make():\n    Client()\n",
    )]);
    assert_eq!(graph.status(&closure, &Name::new("m.make")), REACH);
    assert_eq!(graph.status(&closure, &Name::new("m.Client")), REACH);
}

const CLASSES: &str = "from pkg.util import helper
run_helper = helper

class A:
    def go(self):
        pass

class B(A):
    @classmethod
    def build(cls):
        return cls()

class C(B):
    pass

class D(B):
    def go(self):
        pass

class X:
    def m(self):
        pass

class Y(X):
    def m(self):
        pass

class Z(X):
    def m(self):
        pass

class W(Y, Z):
    pass

def shadowed():
    pass

def shadowed():
    pass

if flag:
    def either():
        pass
else:
    def either():
        pass

def same(f):
    return f

@same
def plain():
    pass

def wrapping(f):
    def wrapper():
        return f()
    return wrapper

@wrapping
def wrapped():
    pass
";

/// The definitions a call of `name` links to, by label and line, and whether it is open.
fn linked(graph: &Graph, sources: &[(&str, &str)], name: &str) -> (Vec<String>, bool) {
    let (defs, open) = graph.link_of(&Name::new(name));
    let labels = defs
        .iter()
        .map(|def| {
            let (path, start) = graph.def_site(*def);
            let source = sources.iter().find(|(p, _)| *p == path.as_ref()).unwrap().1;
            let line = source[..start as usize].matches('\n').count() + 1;
            format!("{}:{line}", graph.def_label(*def))
        })
        .collect();
    (labels, open)
}

/// Python's names as CPython binds them: an alias of a function, inherited class attributes
/// through the C3 order, late-bound methods and `cls()` with every workspace override, a
/// function defined twice (the later one), definitions on either branch, and decorators (one
/// that returns its argument is transparent, a wrapping one is unknown). A class whose base no
/// name denotes may derive from any class: a late-bound method it defines is unknown.
#[test]
fn python_names_follow_aliases_inheritance_and_rebinding() {
    let sources = [
        ("pkg/util.py", "def helper():\n    pass\n"),
        ("m.py", CLASSES),
    ];
    let link = |files: &[(&str, &str)]| {
        Graph::link(
            files
                .iter()
                .map(|(path, source)| {
                    let module = path.strip_suffix(".py").unwrap().replace('/', ".");
                    (Arc::from(*path), Arc::new(summarize(&module, source)))
                })
                .collect(),
        )
    };
    let graph = link(&sources);
    let named = |name: &str, labels: &[&str], open: bool| {
        assert_eq!(
            linked(&graph, &sources, name),
            (labels.iter().map(|l| (*l).to_owned()).collect(), open),
            "{name}"
        );
    };
    named("m.run_helper", &["pkg.util.helper:1"], false);
    named("m.C.build", &["m.B.build:10"], false);
    named("m.C.go", &["m.A.go:5"], false);
    named("m.B.go^", &["m.A.go:5", "m.D.go:17"], false);
    named("m.B.^", &[], false);
    named("m.W.m", &["m.Y.m:25"], false);
    named("m.shadowed", &["m.shadowed:38"], false);
    named("m.either", &["m.either:42", "m.either:45"], false);
    named("m.plain", &["m.plain:52"], false);
    named("m.wrapped", &["m.wrapped:61"], true);
    let orphan = (
        "o.py",
        "from m import B\n\nBase = make()\n\nclass E(Base):\n    def go(self):\n        pass\n",
    );
    let sources = [sources[0], sources[1], orphan];
    let graph = link(&sources);
    assert_eq!(
        linked(&graph, &sources, "m.B.go^"),
        (vec!["m.A.go:5".to_owned(), "m.D.go:17".to_owned()], true)
    );
    assert!(!linked(&graph, &sources, "m.X.m^").1);
}

/// Callees are named as CPython reads identifiers (NFKC): a lookalike import, attribute,
/// definition, receiver and method all name the ASCII ones.
#[test]
fn callees_are_named_under_nfkc() {
    let summary = summarize(
        "m",
        "import 𝐭𝐢𝐦𝐞\n\ndef 𝐰𝐚𝐢𝐭():\n    time.𝐬𝐥𝐞𝐞𝐩(1)\n\nclass 𝒞:\n    def go(𝐬𝐞𝐥𝐟):\n        self.𝐫𝐮𝐧()\n    def run(self):\n        wait()\n",
    );
    let defs = summary
        .defs
        .iter()
        .map(|def| summary.name(def.name).to_owned())
        .collect::<Vec<_>>();
    assert_eq!(defs, ["m.wait", "m.C", "m.C.go", "m.C.run"]);
    assert_eq!(
        edges(&summary),
        pairs(&[
            ("m.wait", "time.sleep"),
            ("m.C.go", "m.C.run^"),
            ("m.C.run", "m.wait"),
        ])
    );
    let (graph, _, closure) = close(&[(
        "m.py",
        "m",
        "import 𝐭𝐢𝐦𝐞\n\ndef 𝐰𝐚𝐢𝐭():\n    time.𝐬𝐥𝐞𝐞𝐩(1)\n\ndef make():\n    wait()\n",
    )]);
    assert_eq!(graph.status(&closure, &Name::new("m.make")), REACH);
}

#[test]
fn a_file_with_syntax_errors_opens_its_definitions() {
    let (graph, _, closure) = close(&[
        ("a.py", "a", "import b\n\ndef f():\n    b.g()\n"),
        ("b.py", "b", "def g(:\n    pass\n"),
    ]);
    assert_eq!(graph.status(&closure, &Name::new("a.f")), OPEN_STATUS);
}

/// Replacing, removing and adding files one at a time answers every call as linking the files
/// afresh does: definitions shifting in a file, re-exports, star imports, a class's `__init__`,
/// a syntax error, and a module leaving and returning.
/// `(path, module, source)`; no source removes the file.
type Step = (&'static str, &'static str, Option<&'static str>);

/// A workspace linked in one batch, then edited one file at a time: definitions shifting in a
/// file, re-exports, star imports, a class's `__init__`, a syntax error, and a module leaving and
/// returning.
fn edit_steps() -> Vec<Vec<Step>> {
    let initial: &[Step] = &[
        (
            "app/api.py",
            "app.api",
            Some(
                "from app.services import fetch\nfrom app import util, klass\nfrom app.star import g\n\nasync def handler():\n    fetch()\n    util.later()\n    klass.make()\n    g()\n",
            ),
        ),
        (
            "app/services/__init__.py",
            "app.services.__init__",
            Some("from .impl import fetch\n"),
        ),
        (
            "app/services/impl.py",
            "app.services.impl",
            Some("from app.util import pause\n\ndef fetch():\n    pause()\n"),
        ),
        (
            "app/util.py",
            "app.util",
            Some("import time\n\ndef pause():\n    time.sleep(1)\n"),
        ),
        (
            "app/star.py",
            "app.star",
            Some("from app.util import *\n\ndef g():\n    later()\n"),
        ),
        (
            "app/klass.py",
            "app.klass",
            Some("class K:\n    def __init__(self):\n        pass\n\ndef make():\n    K()\n"),
        ),
        (
            "app/chain/top.py",
            "app.chain.top",
            Some("from app.chain.mid import b\n\ndef a():\n    b()\n"),
        ),
        (
            "app/chain/mid.py",
            "app.chain.mid",
            Some("from app.chain.low import c1\n\ndef b():\n    c1()\n"),
        ),
        (
            "app/chain/low.py",
            "app.chain.low",
            Some("import time\n\ndef c0():\n    pass\n\ndef c1():\n    time.sleep(1)\n"),
        ),
        (
            "app/chain/user.py",
            "app.chain.user",
            Some("from app.chain.top import a\n\ndef use():\n    a()\n"),
        ),
        (
            "app/shapes.py",
            "app.shapes",
            Some(
                "class Base:\n    def render(self):\n        self.draw()\n    def draw(self):\n        pass\n\ndef show():\n    Base.render(None)\n",
            ),
        ),
        (
            "app/reexport.py",
            "app.reexport",
            Some("from app.shapes import Base\n"),
        ),
    ];
    let edits: &[Step] = &[
        // The end of a chain moves one definition down: every caller's chain changes, though
        // only the last hop does.
        (
            "app/chain/low.py",
            "app.chain.low",
            Some("import time\n\ndef c0():\n    time.sleep(1)\n\ndef c1():\n    c0()\n"),
        ),
        // A caller on the chain stops calling: nothing else changes.
        (
            "app/services/impl.py",
            "app.services.impl",
            Some("def fetch():\n    pass\n"),
        ),
        // And calls again.
        (
            "app/services/impl.py",
            "app.services.impl",
            Some("from app.util import pause\n\ndef fetch():\n    pause()\n"),
        ),
        // A definition inserted before another: every later one moves in the file.
        (
            "app/util.py",
            "app.util",
            Some("import time\n\ndef later():\n    time.sleep(1)\n\ndef pause():\n    pass\n"),
        ),
        (
            "app/services/__init__.py",
            "app.services.__init__",
            Some("from app.util import later as fetch\n"),
        ),
        (
            "app/klass.py",
            "app.klass",
            Some(
                "import time\n\nclass K:\n    def __init__(self):\n        time.sleep(1)\n\ndef make():\n    K()\n",
            ),
        ),
        // The class loses its `__init__`: constructing it runs nothing.
        (
            "app/klass.py",
            "app.klass",
            Some("import time\n\nclass K:\n    pass\n\ndef make():\n    K()\n    time.sleep(1)\n"),
        ),
        (
            "app/services/__init__.py",
            "app.services.__init__",
            Some("from .impl import fetch\n"),
        ),
        ("app/services/impl.py", "app.services.impl", None),
        (
            "app/services/impl.py",
            "app.services.impl",
            Some("import time\n\ndef fetch():\n    time.sleep(1)\n"),
        ),
        ("app/util.py", "app.util", Some("def pause(:\n    pass\n")),
        ("app/star.py", "app.star", Some("def g():\n    pass\n")),
        (
            "app/util.py",
            "app.util",
            Some("import time\n\ndef later():\n    time.sleep(1)\n"),
        ),
        ("app/klass.py", "app.klass", None),
        // A subclass in another file, of the base as a re-export names it, overrides a
        // late-bound method: the base's caller reaches through it.
        (
            "app/circle.py",
            "app.circle",
            Some(
                "import time\nfrom app.reexport import Base\n\nclass Circle(Base):\n    def draw(self):\n        time.sleep(1)\n",
            ),
        ),
        // Its base becomes a value no name denotes: the override may still be one.
        (
            "app/circle.py",
            "app.circle",
            Some(
                "import time\nB = make()\n\nclass Circle(B):\n    def draw(self):\n        time.sleep(1)\n",
            ),
        ),
        ("app/circle.py", "app.circle", None),
        (
            "app/circle.py",
            "app.circle",
            Some(
                "import time\nfrom app.reexport import Base\n\nclass Circle(Base):\n    def draw(self):\n        time.sleep(1)\n",
            ),
        ),
        // The re-export defines its own base: the subclass moves to it.
        (
            "app/reexport.py",
            "app.reexport",
            Some("class Base:\n    def render(self):\n        self.draw()\n"),
        ),
        // A base a star import may bind, from a module not yet in the workspace, then from one
        // that re-exports a workspace class under that name.
        (
            "app/square.py",
            "app.square",
            Some(
                "import time\nfrom app.figures import *\n\nclass Square(Figure):\n    def draw(self):\n        time.sleep(1)\n",
            ),
        ),
        (
            "app/figures.py",
            "app.figures",
            Some("from app.shapes import Base as Figure\n"),
        ),
    ];
    let mut batches = vec![initial.to_vec()];
    batches.extend(edits.iter().map(|edit| vec![*edit]));
    batches
}

fn summary_of((path, module, source): &Step) -> (Arc<str>, Option<Arc<Summary>>) {
    (
        Arc::<str>::from(*path),
        source.map(|source| Arc::new(summarize(module, source))),
    )
}

#[test]
fn an_updated_graph_answers_as_linking_afresh() {
    let mut files = std::collections::BTreeMap::new();
    let mut graph = Graph::default();
    for (step, batch) in edit_steps().iter().enumerate() {
        let changes = batch.iter().map(summary_of).collect::<Vec<_>>();
        for (path, summary) in &changes {
            match summary {
                Some(summary) => files.insert(Arc::clone(path), Arc::clone(summary)),
                None => files.remove(path),
            };
        }
        graph.update(changes);
        let fresh = Graph::link(
            files
                .iter()
                .map(|(p, s)| (Arc::clone(p), Arc::clone(s)))
                .collect(),
        );
        let closure = graph.close(&seeds_by_direct_call(&graph, &BLOCKING), sync_only, 1);
        let expected = fresh.close(&seeds_by_direct_call(&fresh, &BLOCKING), sync_only, 1);
        assert_eq!(graph.stats(), fresh.stats(), "step {step}");
        assert_eq!(
            graph.caller_numbers(),
            fresh.caller_numbers(),
            "step {step}"
        );
        assert_eq!(graph.hierarchy(), fresh.hierarchy(), "step {step}");
        assert_eq!(closure, expected, "step {step}");
        for summary in files.values() {
            for target in &summary.targets {
                let name = summary.interned(*target);
                assert_eq!(
                    (graph.status(&closure, name), graph.chain(&closure, name, 0)),
                    (
                        fresh.status(&expected, name),
                        fresh.chain(&expected, name, 0)
                    ),
                    "step {step}: {name:?}"
                );
            }
        }
    }
    // Something reached, so the comparison is not vacuous.
    let closure = graph.close(&seeds_by_direct_call(&graph, &BLOCKING), sync_only, 1);
    assert_eq!(graph.status(&closure, &Name::new("app.api.handler")), NONE);
    assert_eq!(
        graph.status(&closure, &Name::new("app.services.fetch")),
        REACH
    );
    // The hierarchy kept across edits has a subclass through a re-export, and a class whose base
    // a star import may bind.
    let (subclasses, orphans) = graph.hierarchy();
    assert_eq!(subclasses["app.reexport.Base"], ["app.circle.Circle"]);
    assert_eq!(orphans, ["app.square.Square"]);
}

/// An edit lays out again only the callers of the files whose callers it changes, however many
/// definitions it renumbers: none where the graph keeps its shape.
#[test]
fn an_edit_relays_only_the_callers_it_changes() {
    use super::closure::RELAID_DEFS;
    let file = |path: &str, module: &str, source: &str| {
        (Arc::<str>::from(path), Arc::new(summarize(module, source)))
    };
    let leaf = "def f0():\n    pass\n\ndef f1():\n    f0()\n\ndef f2():\n    f1()\n";
    let mut files = (0..40)
        .map(|i| file(&format!("lib/m{i:02}.py"), &format!("lib.m{i:02}"), leaf))
        .collect::<std::collections::BTreeMap<_, _>>();
    let (path, summary) = file(
        "app/a.py",
        "app.a",
        "from lib.m00 import f0\n\ndef run():\n    f0()\n",
    );
    files.insert(path, summary);
    let mut graph = Graph::link(files.clone().into_iter().collect());
    let mut edit = |(path, summary): (Arc<str>, Arc<Summary>)| {
        files.insert(Arc::clone(&path), Arc::clone(&summary));
        let before = RELAID_DEFS.get();
        let reshaped = graph.update([(path, Some(summary))]).reshaped;
        let relaid = RELAID_DEFS.get() - before;
        let fresh = Graph::link(files.clone().into_iter().collect());
        assert_eq!(graph.caller_numbers(), fresh.caller_numbers());
        assert_eq!(graph.stats(), fresh.stats());
        (relaid, reshaped)
    };
    // Moved definitions calling what they did: the shape is unchanged.
    assert_eq!(
        edit(file(
            "app/a.py",
            "app.a",
            "# moved\nfrom lib.m00 import f0\n\ndef run():\n    f0()\n"
        )),
        (0, false)
    );
    // A new call: only its callee's file is laid out again.
    assert_eq!(
        edit(file(
            "app/a.py",
            "app.a",
            "from lib.m00 import f0\nfrom lib.m01 import f2\n\ndef run():\n    f0()\n    f2()\n"
        )),
        (3, true)
    );
    // A file numbered first shifts every definition's number, and lays out its callee's file.
    assert_eq!(
        edit(file(
            "aaa/first.py",
            "aaa.first",
            "from lib.m05 import f1\n\ndef go():\n    f1()\n"
        )),
        (3, true)
    );
}

/// A term's sparse answer is the dense closure's: the definitions reaching its seeds, and those
/// that may reach something unknown but not it. Its chains are shortest, where the dense
/// closure's are any.
#[test]
fn a_sparse_answer_is_the_dense_closure() {
    let mut files = std::collections::BTreeMap::new();
    let mut reached = 0;
    for (step, batch) in edit_steps().iter().enumerate() {
        for (path, summary) in batch.iter().map(summary_of) {
            match summary {
                Some(summary) => files.insert(path, summary),
                None => files.remove(&path),
            };
        }
        let graph = Graph::link(
            files
                .iter()
                .map(|(p, s)| (Arc::clone(p), Arc::clone(s)))
                .collect(),
        );
        let dense_seeds = seeds_by_direct_call(&graph, &BLOCKING);
        let closure = graph.close(&dense_seeds, sync_only, 1);
        let seeds = (0..graph.def_count() as u32)
            .filter(|def| dense_seeds[*def as usize].matched != 0)
            .map(|def| graph.def_ref(def))
            .collect::<Vec<_>>();
        let allow = |flags| sync_only(flags) != 0;
        let region = graph.reach(&seeds, allow);
        let open = graph.upward(graph.open_defs(), allow);
        for def in 0..graph.def_count() as u32 {
            let at = graph.def_ref(def);
            let label = graph.def_name(def);
            let (reach, open_bit) = closure.bits(def);
            assert_eq!(
                region.contains_key(&at),
                reach != 0,
                "step {step}: reach of {label}"
            );
            assert_eq!(
                open.contains(&at) && !region.contains_key(&at),
                open_bit != 0,
                "step {step}: open of {label}"
            );
            let Some(mut hop) = region.get(&at).copied() else {
                continue;
            };
            // The sparse chain: each hop a callee of the one before, ending at a seed.
            let (mut from, mut length) = (at, 1);
            while let Hop::Via(next) = hop {
                assert!(graph.calls(from, next), "step {step}: {label}");
                (from, hop, length) = (next, region[&next], length + 1);
            }
            assert!(seeds.contains(&from), "step {step}: {label}");
            assert!(
                length <= graph.chain_from(&closure, def, 0).len(),
                "step {step}: {label}"
            );
            reached += 1;
        }
    }
    assert!(reached > 0, "some definition reaches a seed");
}

/// Terms kept across edits answer every call as a view built afresh, and the files an update
/// names are the only ones whose stamp can change: stamps kept for the rest equal fresh ones.
#[test]
fn an_updated_view_restamps_only_what_an_edit_can_change() {
    let terms = vec![
        (Arc::<str>::from("sleep-sync"), Through::Sync),
        (Arc::<str>::from("sleep-any"), Through::Any),
        (Arc::<str>::from("get-any"), Through::Any),
    ];
    let every = [0, 1, 2];
    // Terms 0 and 1 seed definitions calling `time.sleep`, term 2 `requests.get`; term 2's seed
    // query is unknown in `app/klass.py`.
    let seeds_of = |files: &std::collections::BTreeMap<Arc<str>, Arc<Summary>>| {
        let mut seeds = HashMap::<Arc<str>, FileSeeds>::new();
        for (path, summary) in files {
            let file = seeds.entry(Arc::clone(path)).or_default();
            for (owner, callee) in &summary.edges {
                let start = summary.defs[*owner as usize].start;
                match summary.name(*callee) {
                    "time.sleep" => {
                        file.matched(start, 0);
                        file.matched(start, 1);
                    }
                    "requests.get" => file.matched(start, 2),
                    _ => {}
                }
            }
            if path.as_ref() == "app/klass.py" {
                file.unknown.push(2);
            }
        }
        seeds
    };
    let mut files = std::collections::BTreeMap::new();
    let mut view: Option<CallsView> = None;
    let mut stamps = HashMap::<Arc<str>, HashKey>::new();
    let mut restamped = 0;
    for (step, batch) in edit_steps().iter().enumerate() {
        let changes = batch.iter().map(summary_of).collect::<Vec<_>>();
        for (path, summary) in &changes {
            match summary {
                Some(summary) => files.insert(Arc::clone(path), Arc::clone(summary)),
                None => files.remove(path),
            };
        }
        let seeds = seeds_of(&files);
        let all = || {
            files
                .iter()
                .map(|(p, s)| (Arc::clone(p), Arc::clone(s)))
                .collect()
        };
        let affected = match &mut view {
            None => {
                view = Some(CallsView::build(terms.clone(), all(), &seeds));
                files.keys().cloned().collect()
            }
            Some(view) => view.update(changes, &seeds),
        };
        let view = view.as_ref().unwrap();
        stamps.retain(|path, _| files.contains_key(path));
        for path in &affected {
            if let Some(summary) = files.get(path) {
                stamps.insert(Arc::clone(path), view.stamp(summary, &every));
                restamped += 1;
            }
        }
        let fresh = CallsView::build(terms.clone(), all(), &seeds);
        for (path, summary) in &files {
            assert_eq!(
                stamps.get(path),
                Some(&fresh.stamp(summary, &every)),
                "step {step}: {path}"
            );
            for target in &summary.targets {
                let name = summary.interned(*target);
                for term in every {
                    assert_eq!(
                        (view.verdict(name, term), view.chain(name, term)),
                        (fresh.verdict(name, term), fresh.chain(name, term)),
                        "step {step}: {name:?} term {term}"
                    );
                }
            }
        }
        // The open regions kept across edits are those found afresh.
        for through in Through::ALL {
            let sites = |view: &CallsView| {
                let mut sites = view
                    .open(through)
                    .iter()
                    .map(|def| view.graph.def_site(*def))
                    .map(|(path, start)| (path.to_string(), start))
                    .collect::<Vec<_>>();
                sites.sort();
                sites
            };
            assert_eq!(sites(view), sites(&fresh), "step {step}: {through:?}");
        }
    }
    // Some edit left files unstamped: the restamping is selective, not vacuous.
    assert!(restamped < files.len() * edit_steps().len());
}

/// Measures extraction and the closure over a real Python tree:
/// `RERAIL_CALLS_CORPUS=/path/to/repo cargo test --release calls::tests::corpus -- --ignored --nocapture`.
#[test]
#[ignore = "measurement over an external corpus"]
fn corpus() {
    use std::time::Instant;
    let Ok(root) = std::env::var("RERAIL_CALLS_CORPUS") else {
        return;
    };
    let root = std::path::PathBuf::from(root);
    let mut paths = Vec::new();
    let mut pending = vec![root.clone()];
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(&directory)
            .expect("readable directory")
            .flatten()
        {
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if path.is_dir() {
                if !name.starts_with('.') && name != "node_modules" && name != "__pycache__" {
                    pending.push(path);
                }
            } else if name.ends_with(".py") {
                paths.push(path);
            }
        }
    }
    paths.sort();
    let allocated = || {
        let _ = tikv_jemalloc_ctl::epoch::advance();
        tikv_jemalloc_ctl::stats::allocated::read().unwrap_or(0) as f64 / (1 << 20) as f64
    };
    let (mut parse, mut table_time, mut extract, mut bytes) = (0u128, 0u128, 0u128, 0usize);
    let start_mib = allocated();
    let mut files = Vec::new();
    for path in &paths {
        let source = std::fs::read_to_string(path).unwrap_or_default();
        let module = crate::policy_dsl::model::python_module_path_on_disk(path)
            .and_then(|value| match value {
                serde_json::Value::String(dotted) => ModulePath::dotted(&dotted),
                serde_json::Value::Array(names) => {
                    ModulePath::names(&names.iter().filter_map(|n| n.as_str()).collect::<Vec<_>>())
                }
                _ => None,
            })
            .or_else(|| ModulePath::stem(&path.to_string_lossy()));
        let started = Instant::now();
        let tree = GrammarDialect::Python.ast_grep(&source);
        parse += started.elapsed().as_micros();
        let errors = tree.root().get_doc().tree.root_node().has_error();
        let started = Instant::now();
        let table = ScopeTable::build_in(
            tree.root(),
            GrammarDialect::Python,
            5_000_000,
            &|| false,
            Reading::default(),
            module.as_ref(),
        )
        .ok();
        table_time += started.elapsed().as_micros();
        let started = Instant::now();
        let summary = Summary::extract(&tree.root(), table.as_ref(), module.as_ref(), errors);
        extract += started.elapsed().as_micros();
        bytes += summary.encode().len();
        let relative = path.strip_prefix(&root).unwrap_or(path).to_string_lossy();
        files.push((Arc::<str>::from(relative.as_ref()), Arc::new(summary)));
    }
    let blocking = ["time.sleep", "requests.get", "subprocess.run"];
    let summaries_mib = allocated() - start_mib;
    let started = Instant::now();
    let graph = Graph::link(files.clone());
    let link = started.elapsed();
    let graph_mib = allocated() - start_mib - summaries_mib;
    let started = Instant::now();
    let components = graph.components();
    let tarjan = started.elapsed();
    let seeds = seeds_by_direct_call(&graph, &blocking);
    let started = Instant::now();
    let closure = graph.close(&seeds, sync_only, 1);
    let close_time = started.elapsed();
    // Ten terms: the ticket's "closure time for about 10 summaries".
    let wide = seeds
        .iter()
        .map(|seed| Seed {
            matched: if seed.matched != 0 { 0x3ff } else { 0 },
            unknown: 0,
        })
        .collect::<Vec<_>>();
    let started = Instant::now();
    let _ = graph.close(
        &wide,
        |flags| {
            if flags & (ASYNC | CLASS) != 0 {
                0
            } else {
                0x3ff
            }
        },
        10,
    );
    let close_ten = started.elapsed();
    // The sparse answer: one backward walk from the seeds, and one per mode from open definitions.
    let seed_defs = (0..graph.def_count() as u32)
        .filter(|def| seeds[*def as usize].matched != 0)
        .map(|def| graph.def_ref(def))
        .collect::<Vec<_>>();
    let started = Instant::now();
    let region = graph.reach(&seed_defs, |flags| sync_only(flags) != 0);
    let reach_time = started.elapsed();
    let started = Instant::now();
    let open_sync = graph.upward(graph.open_defs(), |flags| sync_only(flags) != 0);
    let open_sync_time = started.elapsed();
    let open_any = graph.upward(graph.open_defs(), |flags| flags & CLASS == 0);
    let stats = graph.stats();
    let nontrivial = components
        .iter()
        .filter(|c| c.len() > 1)
        .collect::<Vec<_>>();
    // Findings: an async definition's call of a sync definition that reaches a blocking call.
    let mut findings = Vec::new();
    let mut open_calls = 0;
    for (path, summary) in &files {
        for (owner, name) in &summary.edges {
            let def = summary.defs[*owner as usize];
            if !def.is_async() {
                continue;
            }
            let status = graph.status(&closure, summary.interned(*name));
            if status.reach & 1 != 0 {
                findings.push(format!(
                    "{path} {} -> {}",
                    summary.name(def.name),
                    graph
                        .chain(&closure, summary.interned(*name), 0)
                        .join(" -> ")
                ));
            } else if status.open & 1 != 0 {
                open_calls += 1;
            }
        }
    }
    let mut open_names = std::collections::HashMap::<String, usize>::new();
    for (_, summary) in &files {
        for name in &summary.targets {
            let name = summary.name(*name);
            if graph.link_is_open(name) {
                *open_names.entry(name.to_owned()).or_default() += 1;
            }
        }
    }
    let mut open_names = open_names.into_iter().collect::<Vec<_>>();
    open_names.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let closure_mib = allocated() - start_mib - summaries_mib - graph_mib;
    // An edit: re-link and re-close after one file changes.
    let mut edits = Vec::new();
    let mut graph = graph;
    for index in (0..files.len()).step_by((files.len() / 20).max(1)).take(20) {
        let (path, summary) = &files[index];
        let mut changed = Summary::clone(summary);
        changed.edges.pop();
        let started = Instant::now();
        graph.update([(Arc::clone(path), Some(Arc::new(changed)))]);
        let _ = graph.reach(&seed_defs, |flags| sync_only(flags) != 0);
        let _ = graph.upward(graph.open_defs(), |flags| sync_only(flags) != 0);
        edits.push(started.elapsed().as_micros());
    }
    edits.sort_unstable();
    println!(
        "{}",
        serde_json::json!({
            "files": paths.len(),
            "parse_ms": parse / 1000,
            "table_ms": table_time / 1000,
            "extract_ms": extract / 1000,
            "summary_bytes": bytes,
            "summaries_mib": summaries_mib,
            "graph_mib": graph_mib,
            "closure_and_scratch_mib": closure_mib,
            "stats": format!("{stats:?}"),
            "components": components.len(),
            "nontrivial_components": nontrivial.len(),
            "largest_component": nontrivial.iter().map(|c| c.len()).max().unwrap_or(1),
            "link_ms": link.as_secs_f64() * 1000.0,
            "tarjan_ms": tarjan.as_secs_f64() * 1000.0,
            "close_ms": close_time.as_secs_f64() * 1000.0,
            "close_ten_terms_ms": close_ten.as_secs_f64() * 1000.0,
            "seed_defs": seed_defs.len(),
            "reach_region": region.len(),
            "reach_us": reach_time.as_micros(),
            "open_sync_region": open_sync.len(),
            "open_sync_us": open_sync_time.as_micros(),
            "open_any_region": open_any.len(),
            "relink_sparse_us_p50": edits.get(edits.len() / 2),
            "relink_sparse_us_p95": edits.get(edits.len() * 95 / 100),
            "findings": findings.len(),
            "open_async_calls": open_calls,
            "sample": findings.iter().take(10).collect::<Vec<_>>(),
            "open_names": open_names.iter().take(15).collect::<Vec<_>>(),
        })
    );
}

#[test]
fn a_reaching_call_binds_its_chain_for_the_message() {
    use crate::policy_dsl::test_support::*;
    let mut rule = blocking_rule();
    rule["has"]["calls"]["chain"] = serde_json::json!("CHAIN");
    let document = check_document(serde_json::json!({
        "id": "async-blocking",
        "forbid": { "ast": { "language": "python", "rule": rule } },
        "violation": { "anchor": "witness.span", "code": "X", "message": "blocks via $CHAIN" },
    }));
    let capabilities: serde_json::Value = serde_json::from_str(CAPABILITIES).expect("capabilities");
    let compiled = compile_document_with(document, &capabilities)
        .unwrap_or_else(|error| panic!("{}", error.reason));
    let result = evaluate_snapshot(&compiled, &CHAIN);
    let findings = result["violations"].as_array().expect("findings");
    assert_eq!(findings.len(), 1, "{result}");
    assert_eq!(
        findings[0]["message"],
        "blocks via app.services.impl.fetch -> app.util.pause"
    );
}

#[test]
fn a_utility_may_hold_calls_but_no_seed_may_reach_one() {
    use crate::policy_dsl::test_support::*;
    let capabilities: serde_json::Value = serde_json::from_str(CAPABILITIES).expect("capabilities");
    let compile = |rule: serde_json::Value, utils: serde_json::Value| {
        compile_document_with(
            check_document(serde_json::json!({
                "id": "utils",
                "forbid": { "ast": { "language": "python", "rule": rule, "utils": utils } },
                "violation": finding(),
            })),
            &capabilities,
        )
    };
    let sleeps = serde_json::json!({ "has": { "stopBy": { "kind": "function_definition" }, "kind": "call",
        "has": { "field": "function", "resolves": { "import": ["time.sleep"] } } } });
    // A utility holding calls, which the seed of its own term leaves out.
    let compiled = compile(
        serde_json::json!({ "kind": "call", "matches": "blocks" }),
        serde_json::json!({ "blocks": { "calls": sleeps.clone() }, "helper": { "kind": "call" } }),
    )
    .unwrap_or_else(|error| panic!("{}", error.reason));
    let result = evaluate_snapshot(&compiled, &CHAIN);
    // Both calls of `fetch` in `api.py`, and `pause()` in `impl.py`.
    assert_eq!(
        result["violations"].as_array().map(Vec::len),
        Some(3),
        "{result}"
    );
    // An inner rule reaching that utility would nest calls.
    let nested = compile(
        serde_json::json!({ "kind": "call", "calls": { "matches": "blocks" } }),
        serde_json::json!({ "blocks": { "calls": sleeps } }),
    );
    assert!(rejection(nested).contains("cannot contain calls"));
}
