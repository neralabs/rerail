//! The typing programs scored against the call oracles the native resolvers are held to (t20 step 0): what the
//! program answers at each call CPython ran, against what it ran. These tests replace the spike's scorers
//! (`score_calls.py`), which were not copied: parity with the spike's outputs (`logic::plan::tests`) says the
//! port derives what the spike derived, not that it is right.

use crate::code_rule_dsl::logic::plan::corpus;
use crate::code_rule_dsl::typing::file_local::Binding;
use serde_json::Value as Json;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// A program's answer at a call.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Answer {
    /// `call_open`: the call may reach something no static name denotes.
    Unknown,
    /// `call_target` rows: the definitions named, as (file, line of the definition).
    Reaches(BTreeSet<(String, u64)>),
}

/// How an answer departs from what CPython ran at a call.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Departure {
    /// Unknown, where CPython ran workspace functions or none.
    Unknown,
    /// No edge, where CPython ran a workspace function.
    NoEdge,
    /// Some of what ran, and nothing else.
    Fewer,
    /// A definition that did not run there: a wrong answer.
    Wrong,
}

/// Sorted relative paths and sources of the `.py` files under `root`.
fn python_files(root: &Path) -> BTreeMap<String, String> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, String>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(root, &path, out);
            } else if path.extension().is_some_and(|e| e == "py") {
                out.insert(path.strip_prefix(root).unwrap().to_string_lossy().into_owned(), std::fs::read_to_string(&path).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

/// 1-based line of a byte offset.
fn line(source: &str, offset: usize) -> u64 {
    source.as_bytes()[..offset].iter().filter(|b| **b == b'\n').count() as u64 + 1
}

/// The spans the scorer reads, added to the program as one more output: each answered call's and each named
/// definition's `(start, end)` bytes. A definition's line is its `def`'s, decorated or not, as the oracle records it.
const SPANS: &str = "
.decl rr_score_at(n: node, s: number, e: number)
rr_score_at(n, s, e) :- call_target(n, _), node(_, n, _, s, e).
rr_score_at(n, s, e) :- call_open(n), node(_, n, _, s, e).
rr_score_at(f, s, e) :- call_target(_, f), node(_, f, _, s, e).
.output rr_score_at
";

/// Runs `program` over the oracle project and gives each answered call's answer, keyed by (file, start, end).
fn answers(program: &str, project: &Path, sources: &BTreeMap<String, String>) -> BTreeMap<(String, usize, usize), Answer> {
    let out = crate::code_rule_dsl::prefilter::tests::temp().with_extension("score");
    let mut src = format!("{program}\n{SPANS}");
    // `RERAIL_PYTHON_CROSS_MEMO`, as the corpus benchmark reads it.
    if let Ok(rels) = std::env::var("RERAIL_PYTHON_CROSS_MEMO") {
        src.push_str(&format!("\n.pragma \"cross-memo\" \"{rels}\"\n"));
    }
    let report = corpus::run(&src, project, None, &out, "program", Binding::Cpu, 1 << 18).unwrap();
    println!("cross-file rounds {:?}", report.rounds);
    assert!(report.rounds.converged || report.rounds.pending, "unconverged with no pending round: {:?}", report.rounds);
    let rows = |rel: &str| -> Vec<Vec<String>> {
        std::fs::read_to_string(out.join(format!("{rel}.csv"))).unwrap().lines().map(|l| l.split('\t').map(str::to_owned).collect()).collect()
    };
    let at: BTreeMap<String, (usize, usize)> = rows("rr_score_at").into_iter().map(|r| (r[0].clone(), (r[1].parse().unwrap(), r[2].parse().unwrap()))).collect();
    let file = |node: &str| node.rsplit_once(':').unwrap().0.to_owned();
    let key = |node: &str| {
        let (s, e) = at[node];
        (file(node), s, e)
    };
    let mut answers = BTreeMap::new();
    for r in rows("call_open") {
        answers.insert(key(&r[0]), Answer::Unknown);
    }
    for r in rows("call_target") {
        let (f, (s, _)) = (file(&r[1]), at[&r[1]]);
        let def = (f.clone(), line(&sources[&f], s));
        match answers.entry(key(&r[0])).or_insert_with(|| Answer::Reaches(BTreeSet::new())) {
            Answer::Reaches(defs) => {
                defs.insert(def);
            }
            // Open and named both: the names are a lower bound, the call is still unknown.
            Answer::Unknown => {}
        }
    }
    let _ = std::fs::remove_dir_all(&out);
    answers
}

/// Each call CPython ran where the program answers otherwise: `file:line `text``, with how.
pub(super) fn score_python(program: &str) -> (usize, BTreeMap<String, Departure>, Vec<String>) {
    let oracle = PathBuf::from(crate::code_rule_dsl::fixtures_dir!()).join("python/calls");
    let expected: Json = serde_json::from_str(&std::fs::read_to_string(oracle.join("python.expected.json")).unwrap()).unwrap();
    let project = oracle.join("project");
    let sources = python_files(&project);
    let answers = answers(program, &project, &sources);
    let (mut agree, mut departures, mut ran_spans) = (0, BTreeMap::new(), BTreeSet::new());
    for (file, calls) in expected["runtime"].as_object().unwrap() {
        for call in calls.as_array().unwrap() {
            let (start, end) = (call["start"].as_u64().unwrap() as usize, call["end"].as_u64().unwrap() as usize);
            ran_spans.insert((file.clone(), start, end));
            let at = format!("{file}:{} `{}`", call["line"], &sources[file][start..end]);
            let callees = call["callees"].as_array().unwrap();
            let ran: BTreeSet<(String, u64)> =
                callees.iter().filter(|c| c[0] == "def").map(|c| (c[1].as_str().unwrap().to_owned(), c[2].as_u64().unwrap())).collect();
            // A lambda is no definition a program can name.
            let lambda = callees.iter().any(|c| c[0] == "lambda");
            let departure = match answers.get(&(file.clone(), start, end)) {
                Some(Answer::Reaches(named)) if *named == ran && !lambda => None,
                None if ran.is_empty() && !lambda => None,
                Some(Answer::Unknown) => Some(Departure::Unknown),
                None => Some(Departure::NoEdge),
                Some(Answer::Reaches(named)) if named.is_subset(&ran) => Some(Departure::Fewer),
                Some(Answer::Reaches(named)) => {
                    if std::env::var_os("RERAIL_SCORE_DETAIL").is_some() {
                        println!("  wrong at {at}: named {named:?}, ran {ran:?}");
                    }
                    Some(Departure::Wrong)
                }
            };
            match departure {
                None => agree += 1,
                Some(d) => {
                    departures.insert(at, d);
                }
            }
        }
    }
    // A definition named at a call CPython never ran is unchecked by the trace, so it is reported: the oracle's
    // main runs every call the workspace names a function for.
    let never_ran = answers
        .iter()
        .filter(|(k, a)| matches!(a, Answer::Reaches(d) if !d.is_empty()) && !ran_spans.contains(*k))
        .map(|((f, s, _), _)| format!("{f}:{}: named, never run", line(&sources[f], *s)))
        .collect();
    (agree, departures, never_ran)
}

/// `RERAIL_PYTHON_PROGRAM`: the calls program the oracle tests score, `calls` unless it names another (`calls_cx`,
/// the closure-local version; O114).
fn calls_name() -> String {
    std::env::var("RERAIL_PYTHON_PROGRAM").unwrap_or_else(|_| "calls".to_owned())
}

fn program(name: &str) -> String {
    crate::code_rule_dsl::tools::program_text(&Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("src/code_rule_dsl/typing/programs/{name}.rr"))).unwrap()
}

/// Where `calls.rr` answers otherwise than CPython ran, and why (2026-11-06: 84 agree, these 9 depart, none
/// wrong, as the spike scored). The native resolver departs at 17 calls on the same oracle; the program resolves `super()`, a local
/// holding an instance, a call's result and an instance's `__call__`, which the native resolver leaves unknown.
const PYTHON_DEPARTURES: &[(&str, Departure, &str)] = &[
    ("pkg/aliases.py:21 `defined_later()`", Departure::Unknown, "a name a star import binds (`from .util import *`)"),
    ("pkg/decorators.py:12 `func(*args, **kwargs)`", Departure::Unknown, "a called parameter"),
    ("pkg/decorators.py:35 `cached(1)`", Departure::Unknown, "`functools.lru_cache`'s wrapper"),
    ("pkg/dynamic.py:18 `HANDLERS[name]()`", Departure::Unknown, "a subscript of a container"),
    ("pkg/dynamic.py:22 `callback()`", Departure::Unknown, "a called parameter"),
    ("pkg/dynamic.py:26 `getattr(util, name)(1)`", Departure::Unknown, "`getattr`'s result"),
    ("pkg/dynamic.py:31 `(lambda: first())()`", Departure::Unknown, "a lambda"),
    ("pkg/dynamic.py:31 `square(3)`", Departure::Unknown, "a lambda bound to a name"),
    ("pkg/dynamic.py:35 `functools.partial(util.helper, 1)()`", Departure::Unknown, "`functools.partial`"),
];

/// `calls.rr` against the CPython runtime oracle (`fixtures/python/calls`): no wrong answer, nothing named that
/// never ran, and the departures as recorded. The native resolver's test
/// (`calls::python_tests::python_calls_agree_with_the_runtime`) holds the same oracle to its own list.
#[test]
fn python_calls_program_agrees_with_the_runtime() {
    let (agree, departures, never_ran) = score_python(&program(&calls_name()));
    println!("calls.rr: {agree} agree, {} depart", departures.len());
    assert!(never_ran.is_empty(), "{never_ran:#?}");
    let listed: BTreeMap<String, Departure> = PYTHON_DEPARTURES.iter().map(|(at, d, _)| ((*at).to_owned(), *d)).collect();
    assert_eq!(departures, listed, "calls where calls.rr answers otherwise than CPython ran");
    assert!(agree >= 84, "{agree} agree");
}

/// 1-based line and code-point column of a byte offset, as rust-analyzer's recording writes them.
fn line_column(source: &str, offset: usize) -> (u64, u64) {
    let start = source[..offset].rfind('\n').map_or(0, |i| i + 1);
    (line(source, offset), source[start..offset].chars().count() as u64 + 1)
}

/// The positions the Rust scorer reads: each answered call's callee name (the last segment of its path, or the
/// method's field) and each named definition's name, as `(node, start byte)`.
const RUST_SPANS: &str = "
.decl rr_score_callee(c: node, x: node)
rr_score_callee(c, x) :- field(c, \"function\", x), kind(x, \"identifier\").
rr_score_callee(c, x) :- field(c, \"function\", p), kind(p, \"scoped_identifier\"), field(p, \"name\", x).
rr_score_callee(c, x) :- field(c, \"function\", p), kind(p, \"field_expression\"), field(p, \"field\", x).
rr_score_callee(c, x) :- field(c, \"function\", g), kind(g, \"generic_function\"), field(g, \"function\", x), kind(x, \"identifier\").
rr_score_callee(c, x) :- field(c, \"function\", g), kind(g, \"generic_function\"), field(g, \"function\", p), kind(p, \"scoped_identifier\"), field(p, \"name\", x).
rr_score_callee(c, x) :- field(c, \"function\", g), kind(g, \"generic_function\"), field(g, \"function\", p), kind(p, \"field_expression\"), field(p, \"field\", x).
.decl rr_score_pos(n: node, s: number)
rr_score_pos(c, s) :- call_target(c, _), rr_score_callee(c, x), node(_, x, _, s, _).
rr_score_pos(c, s) :- call_open(c), rr_score_callee(c, x), node(_, x, _, s, _).
rr_score_pos(f, s) :- call_target(_, f), field(f, \"name\", x), node(_, x, _, s, _).
rr_score_pos(f, s) :- call_target(_, f), !rr_score_named(f), node(_, f, _, s, _).
.decl rr_score_named(f: node)
rr_score_named(f) :- call_target(_, f), field(f, \"name\", _).
.output rr_score_pos
";

/// `calls_rust.rr` against rust-analyzer's resolution of the fixture crate (`fixtures/rust/calls`): each recorded
/// function or method call (macro bodies left out), as `(agree, departures)`. A call into another crate
/// agrees when the program names nothing there; one rust-analyzer resolves in the crate agrees when the program
/// names exactly that definition.
pub(super) fn score_rust(program: &str) -> (usize, BTreeMap<String, Departure>) {
    let oracle = PathBuf::from(crate::code_rule_dsl::fixtures_dir!()).join("rust/calls");
    let recording: Json = serde_json::from_str(&std::fs::read_to_string(oracle.join("calls.expected.json")).unwrap()).unwrap();
    let root = oracle.join("crate");
    let sources: BTreeMap<String, String> = recording["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| (p.as_str().unwrap().to_owned(), std::fs::read_to_string(root.join(p.as_str().unwrap())).unwrap()))
        .collect();
    let out = crate::code_rule_dsl::prefilter::tests::temp().with_extension("score-rust");
    corpus::run(&format!("{program}\n{RUST_SPANS}"), &root, None, &out, "program", Binding::Cpu, 1 << 18).unwrap();
    let rows = |rel: &str| -> Vec<Vec<String>> {
        std::fs::read_to_string(out.join(format!("{rel}.csv"))).unwrap().lines().map(|l| l.split('\t').map(str::to_owned).collect()).collect()
    };
    let at = |node: &str, pos: &BTreeMap<String, usize>| -> (String, u64, u64) {
        let file = node.rsplit_once(':').unwrap().0.to_owned();
        let (l, c) = line_column(&sources[&file], pos[node]);
        (file, l, c)
    };
    let pos: BTreeMap<String, usize> = rows("rr_score_pos").into_iter().map(|r| (r[0].clone(), r[1].parse().unwrap())).collect();
    let mut answers: BTreeMap<(String, u64, u64), Answer> = BTreeMap::new();
    for r in rows("call_open") {
        answers.insert(at(&r[0], &pos), Answer::Unknown);
    }
    for r in rows("call_target") {
        let (f, l, _) = at(&r[1], &pos);
        if let Answer::Reaches(defs) = answers.entry(at(&r[0], &pos)).or_insert_with(|| Answer::Reaches(BTreeSet::new())) {
            defs.insert((f, l));
        }
    }
    let _ = std::fs::remove_dir_all(&out);
    let (mut agree, mut departures) = (0, BTreeMap::new());
    for call in recording["calls"].as_array().unwrap() {
        if call["kind"].as_str().unwrap().starts_with("m:") {
            continue;
        }
        let key = (call["path"].as_str().unwrap().to_owned(), call["line"].as_u64().unwrap(), call["column"].as_u64().unwrap());
        let target = &call["target"];
        let ran: BTreeSet<(String, u64)> =
            target["path"].as_str().map(|p| (p.to_owned(), target["line"].as_u64().unwrap())).into_iter().collect();
        let describe = format!("{}:{}:{} `{}`", key.0, key.1, key.2, call["text"].as_str().unwrap());
        let departure = match answers.get(&key) {
            Some(Answer::Reaches(named)) if *named == ran => None,
            None if ran.is_empty() => None,
            Some(Answer::Unknown) => Some(Departure::Unknown),
            None => Some(Departure::NoEdge),
            Some(Answer::Reaches(named)) if named.is_subset(&ran) => Some(Departure::Fewer),
            Some(Answer::Reaches(named)) => {
                if std::env::var_os("RERAIL_SCORE_DETAIL").is_some() {
                    println!("  wrong at {describe}: named {named:?}, recorded {ran:?}");
                }
                Some(Departure::Wrong)
            }
        };
        match departure {
            None => agree += 1,
            Some(d) => {
                departures.insert(describe, d);
            }
        }
    }
    (agree, departures)
}

/// Where `calls_rust.rr` answers otherwise than rust-analyzer recorded, and why (2026-11-06: 37 agree, these 7
/// depart, as in the spike). `Wrong` here is not wrong: rust-analyzer's definition of a call through a local
/// function pointer is the local; the program follows the local to the function it holds, which is what runs.
const RUST_DEPARTURES: &[(&str, Departure, &str)] = &[
    ("src/lib.rs:45:7 `f`", Departure::Unknown, "a trait method on a generic receiver (`x: T`, `T: Trait`)"),
    ("src/lib.rs:46:7 `defaulted`", Departure::Unknown, "a trait's default method on a generic receiver"),
    ("src/lib.rs:47:19 `f`", Departure::Unknown, "`<T as Trait>::f`"),
    ("src/lib.rs:48:12 `f`", Departure::Unknown, "`Trait::f(&x)`"),
    ("src/lib.rs:50:5 `p`", Departure::Wrong, "through `let p: fn() = helper`: the program names `helper`, rust-analyzer the local"),
    ("src/lib.rs:54:19 `map`", Departure::Unknown, "an iterator adapter on a call's result (`iter()`): no expression type"),
    ("src/lib.rs:54:31 `count`", Departure::Unknown, "an iterator adapter's result"),
];

/// `calls_rust.rr` against rust-analyzer: the departures as recorded.
#[test]
fn rust_calls_program_agrees_with_rust_analyzer() {
    let (agree, departures) = score_rust(&program("calls_rust"));
    println!("calls_rust.rr: {agree} agree, {} depart", departures.len());
    let listed: BTreeMap<String, Departure> = RUST_DEPARTURES.iter().map(|(at, d, _)| ((*at).to_owned(), *d)).collect();
    assert_eq!(departures, listed, "calls where calls_rust.rr answers otherwise than rust-analyzer recorded");
    assert!(agree >= 37, "{agree} agree");
}

/// How the type relations answer at a receiver CPython ran a method call on (`receiver` in the oracle).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum TypeAnswer {
    /// `type_of` names exactly the classes the receiver was.
    Exact,
    /// `type_open`: the receiver may be of a class not listed.
    Unknown,
    /// Closed, and every class it was is listed, with more besides.
    Wider,
    /// A class it was is not listed while closed, a class proven is not in a class's MRO, or a class in an MRO
    /// is not possible: a wrong answer.
    Wrong,
}

/// The spans the types scorer reads: each typed or open node's.
const TYPE_SPANS: &str = "
.decl rr_type_at(n: node, s: number, e: number)
rr_type_at(n, s, e) :- type_of(n, _), node(_, n, _, s, e).
rr_type_at(n, s, e) :- type_open(n), node(_, n, _, s, e).
.output rr_type_at
";

/// What the type relations say at a node: listed classes, open, proven classes, possible classes, possible open.
#[derive(Debug, Default)]
struct Typed {
    of: BTreeSet<String>,
    open: bool,
    proven: BTreeSet<String>,
    possible: BTreeSet<String>,
    possible_open: bool,
}

/// `program`'s type relations at each receiver the CPython oracle records, keyed `file:line `text``.
pub(super) fn score_python_types(program: &str) -> BTreeMap<String, TypeAnswer> {
    let oracle = PathBuf::from(crate::code_rule_dsl::fixtures_dir!()).join("python/calls");
    let expected: Json = serde_json::from_str(&std::fs::read_to_string(oracle.join("python.expected.json")).unwrap()).unwrap();
    let project = oracle.join("project");
    let sources = python_files(&project);
    let out = crate::code_rule_dsl::prefilter::tests::temp().with_extension("score-types");
    corpus::run(&format!("{program}\n{TYPE_SPANS}"), &project, None, &out, "program", Binding::Cpu, 1 << 18).unwrap();
    let rows = |rel: &str| -> Vec<Vec<String>> {
        std::fs::read_to_string(out.join(format!("{rel}.csv"))).unwrap().lines().map(|l| l.split('\t').map(str::to_owned).collect()).collect()
    };
    let at: BTreeMap<String, (String, usize, usize)> = rows("rr_type_at")
        .into_iter()
        .map(|r| (r[0].clone(), (r[0].rsplit_once(':').unwrap().0.to_owned(), r[1].parse().unwrap(), r[2].parse().unwrap())))
        .collect();
    let mut typed: BTreeMap<(String, usize, usize), Typed> = BTreeMap::new();
    for r in rows("type_of") {
        typed.entry(at[&r[0]].clone()).or_default().of.insert(r[1].clone());
    }
    for r in rows("type_open") {
        typed.entry(at[&r[0]].clone()).or_default().open = true;
    }
    for r in rows("proven_type") {
        typed.entry(at[&r[0]].clone()).or_default().proven.insert(r[1].clone());
    }
    for r in rows("possible_type") {
        typed.entry(at[&r[0]].clone()).or_default().possible.insert(r[1].clone());
    }
    for r in rows("possible_open") {
        typed.entry(at[&r[0]].clone()).or_default().possible_open = true;
    }
    let _ = std::fs::remove_dir_all(&out);
    let detail = std::env::var_os("RERAIL_SCORE_DETAIL").is_some();
    let mut answers = BTreeMap::new();
    for (file, calls) in expected["runtime"].as_object().unwrap() {
        for call in calls.as_array().unwrap() {
            let Some(receiver) = call.get("receiver") else { continue };
            let (start, end) = (receiver["start"].as_u64().unwrap() as usize, receiver["end"].as_u64().unwrap() as usize);
            let at = format!("{file}:{} `{}`", call["line"], &sources[file][start..end]);
            let classes: Vec<(String, BTreeSet<String>)> = receiver["classes"]
                .as_array()
                .unwrap()
                .iter()
                .map(|c| (c[0].as_str().unwrap().to_owned(), c[1].as_array().unwrap().iter().map(|m| m.as_str().unwrap().to_owned()).collect()))
                .collect();
            let ran: BTreeSet<String> = classes.iter().map(|(c, _)| c.clone()).collect();
            let none = Typed::default();
            let t = typed.get(&(file.clone(), start, end)).unwrap_or(&none);
            let mut wrong = Vec::new();
            if !t.open && !ran.is_subset(&t.of) {
                wrong.push(format!("closed {:?}, ran {ran:?}", t.of));
            }
            for (c, mro) in &classes {
                wrong.extend(t.proven.iter().filter(|u| !mro.contains(*u)).map(|u| format!("proven {u}, ran {c}")));
                if !t.possible_open {
                    wrong.extend(mro.iter().filter(|u| !t.possible.contains(*u)).map(|u| format!("{u} not possible, ran {c}")));
                }
            }
            let answer = if !wrong.is_empty() {
                if detail {
                    println!("  wrong at {at}: {wrong:?}");
                }
                TypeAnswer::Wrong
            } else if t.open {
                TypeAnswer::Unknown
            } else if t.of == ran {
                TypeAnswer::Exact
            } else {
                TypeAnswer::Wider
            };
            answers.insert(at, answer);
        }
    }
    answers
}

/// `calls.rr`'s type relations against the classes CPython's receivers were (decision 0047, point 3): never wrong,
/// and the receivers not exact as listed.
const PYTHON_TYPE_DEPARTURES: &[(&str, TypeAnswer, &str)] = &[];

#[test]
fn python_type_relations_agree_with_the_runtime() {
    let answers = score_python_types(&program("calls"));
    let exact = answers.values().filter(|a| **a == TypeAnswer::Exact).count();
    println!("calls.rr types: {exact} exact of {} receivers", answers.len());
    for (at, a) in &answers {
        println!("  {a:?}\t{at}");
    }
    let departures: BTreeMap<String, TypeAnswer> = answers.into_iter().filter(|(_, a)| *a != TypeAnswer::Exact).collect();
    let listed: BTreeMap<String, TypeAnswer> = PYTHON_TYPE_DEPARTURES.iter().map(|(at, a, _)| ((*at).to_owned(), *a)).collect();
    assert!(!departures.values().any(|a| *a == TypeAnswer::Wrong), "{departures:#?}");
    assert_eq!(departures, listed, "receivers whose classes calls.rr does not give exactly");
}

/// A rule reads the Python program's answers through `lib/python/calls.rr` and `lib/python/types.rr` (decision 0047, point 2): the engine runs
/// `typing/programs/calls.rr` for it (`corpus::bridge`), and the rule's call edges and proven types, joined back to its
/// own nodes on `core.rr`'s facts, are exactly the program's on T's.
#[test]
fn rules_read_the_python_program_through_lib_calls() {
    let project = PathBuf::from(crate::code_rule_dsl::fixtures_dir!()).join("python/calls/project");
    let keyed = "
.decl edge(cp: symbol, cs: number, fp: symbol, fs: number)
edge(cp, cs, fp, fs) :- call_target(c, f), node(d, c, _, cs, _), doc(d, cp, _), node(g, f, _, fs, _), doc(g, fp, _).
.decl proven(np: symbol, ns: number, ne: number, t: symbol)
proven(np, ns, ne, t) :- proven_type(n, t), node(d, n, _, ns, ne), doc(d, np, _).
.decl unknown(cp: symbol, cs: number)
unknown(cp, cs) :- call_open(c), node(d, c, _, cs, _), doc(d, cp, _).
.output edge, proven, unknown
";
    let rows = |out: &Path, rel: &str| -> BTreeSet<String> {
        std::fs::read_to_string(out.join(format!("{rel}.csv"))).unwrap().lines().map(str::to_owned).collect()
    };
    let direct = crate::code_rule_dsl::prefilter::tests::temp().with_extension("direct");
    corpus::run(&format!("{}\n{keyed}", program("calls")), &project, None, &direct, "program", Binding::Cpu, 1 << 18).unwrap();
    let rule_dir = crate::code_rule_dsl::prefilter::tests::temp().with_extension("rule");
    std::fs::create_dir_all(&rule_dir).unwrap();
    let rule = rule_dir.join("rule.rr");
    std::fs::write(&rule, format!("#include \"python/calls.rr\"\n#include \"python/types.rr\"\n{keyed}")).unwrap();
    let text = crate::code_rule_dsl::logic::rr::print(&crate::code_rule_dsl::logic::rr::parse_file(&rule).unwrap()).unwrap();
    let through = rule_dir.join("out");
    let report = corpus::run(&text, &project, None, &through, "program", Binding::Cpu, 1 << 18).unwrap();
    assert_eq!(report.typing_runs.iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>(), ["calls"], "the program ran once");
    for rel in ["edge", "proven", "unknown"] {
        let (a, b) = (rows(&direct, rel), rows(&through, rel));
        println!("{rel}: {} rows", a.len());
        assert!(!a.is_empty(), "{rel}: the program gives no rows");
        assert_eq!(a, b, "{rel}: the rule's rows are the program's");
    }
    let _ = std::fs::remove_dir_all(&direct);
    let _ = std::fs::remove_dir_all(&rule_dir);
}

/// Quoted annotations (decision 0055's `parsed(s, "annotation", r)`): the string's code is the annotation, its
/// names looked up from the string's scope.
#[test]
fn quoted_annotations_are_read_as_their_code() {
    // fixtures/python/quoted_ann: `-> "Node"`, `Optional["Node"]`, `n: "Node"`, `"list[Node]"`, `"Node | None"`.
    let project = PathBuf::from(crate::code_rule_dsl::fixtures_dir!()).join("python/quoted_ann");
    let rule = "
#include \"python/calls.rr\"
.decl at(p: symbol, s: number, what: symbol)
at(p, s, \"target\") :- call_target(c, _), node(d, c, _, s, _), doc(d, p, _).
at(p, s, \"open\") :- call_open(c), node(d, c, _, s, _), doc(d, p, _).
.output at
";
    let rule_dir = crate::code_rule_dsl::prefilter::tests::temp().with_extension("quoted");
    std::fs::create_dir_all(&rule_dir).unwrap();
    let path = rule_dir.join("rule.rr");
    std::fs::write(&path, rule).unwrap();
    let text = crate::code_rule_dsl::logic::rr::print(&crate::code_rule_dsl::logic::rr::parse_file(&path).unwrap()).unwrap();
    let out = rule_dir.join("out");
    corpus::run(&text, &project, None, &out, "program", Binding::Cpu, 1 << 18).unwrap();
    let mut got: BTreeSet<(String, u64)> = BTreeSet::new();
    for l in std::fs::read_to_string(out.join("at.csv")).unwrap().lines() {
        let r: Vec<&str> = l.split('\t').collect();
        let src = std::fs::read_to_string(project.join(r[0])).unwrap();
        got.insert((r[2].to_owned(), line(&src, r[1].parse().unwrap())));
    }
    // pkg/use.py: n.label() (n: "Node"), make(), m.child() and m.child().label(): every call has its target and
    // none is open.
    let want: BTreeSet<(String, u64)> = [("target", 5), ("target", 9), ("target", 10)].into_iter().map(|(w, l)| (w.to_owned(), l)).collect();
    assert_eq!(got, want);
    let _ = std::fs::remove_dir_all(&rule_dir);
}

#[test]
fn imports_resolve_into_a_library_and_load_its_stubs() {
    // fixtures/python/world_load: ws/main.py imports os.path, json and collections; ws/rerail.env says `lib =
    // ../ts/stdlib`, a stub tree with os/__init__.pyi, os/path.pyi, collections/__init__.pyi and a VERSIONS file. The
    // rules walk it with dir_entry and file_line and load what resolves (t21 E1, E2, E4); json is in no root.
    let project = PathBuf::from(crate::code_rule_dsl::fixtures_dir!()).join("python/world_load/ws");
    let rule = r#"
.decl doc(d: number, path: symbol, module: symbol)
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl child(p: node, c: node, i: number)
.decl field(p: node, f: symbol, c: node)
.decl text(n: node, s: symbol)
.decl op(n: node, s: symbol)
.decl pkg(d: number, p: symbol)
.decl kind(n: node, k: symbol)
.decl docof(n: node, d: number)
.input doc, node, child, field, text, op, pkg, kind, docof
.decl env(key: symbol, value: symbol)
.decl dir_entry(dir: symbol, name: symbol, is_dir: number)
.decl file_line(path: symbol, i: number, text: symbol)
.decl load(path: symbol, module: symbol, grammar: symbol)
.decl loaded(path: symbol, module: symbol, grammar: symbol)
.decl imp(n: node, q: symbol)
imp(n, q) :- kind(n, "import_statement"), field(n, "name", m), kind(m, "dotted_name"), text(m, q).
.decl want(q: symbol)
want(q) :- imp(_, q).
want(m) :- want(q), m = @pkg_up(q, 1), m != "".
.decl modpath(q: symbol, p: symbol)
modpath(q, p) :- want(q), p = @replace_all(q, "\.", "/").
.decl version(m: symbol, v: symbol)
version(m, v) :- env("lib", r), f = cat(r, "/VERSIONS"), file_line(f, _, l), match("(?<m>[a-z_.]+): (?<v>.*)", l).
.decl resolves(q: symbol, f: symbol)
resolves(q, f) :- modpath(q, p), env("lib", r), d = cat(r, "/", p), dir_entry(d, "__init__.pyi", 0), f = cat(d, "/__init__.pyi").
resolves(q, f) :- want(q), split_dot(q, m, x), modpath(m, pm), env("lib", r), d = cat(r, "/", pm), dir_entry(d, n, 0), n = cat(x, ".pyi"), f = cat(d, "/", n).
load(f, q, "python") :- resolves(q, f), version(@head(q), _).
.decl got(m: symbol, p: symbol)
got(m, p) :- doc(d, path, m), pkg(d, p), loaded(path, _, _).
.decl own(c: node)
own(n) :- imp(n, _).
.output got, own
"#;
    let out = crate::code_rule_dsl::prefilter::tests::temp().with_extension("world");
    corpus::run(rule, &project, None, &out, "program", Binding::Cpu, 1 << 18).unwrap();
    let read = |rel: &str| std::fs::read_to_string(out.join(format!("{rel}.csv"))).unwrap();
    assert_eq!(read("got"), "collections\tcollections\nos\tos\nos.path\tos\n");
    // The loaded documents' rows are not the corpus's: main.py's three imports only.
    assert_eq!(read("own").lines().count(), 3, "{}", read("own"));
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn calls_into_the_standard_library_reach_its_stubs() {
    // fixtures/python/world_load/ws: `python.typeshed = ../ts`, a miniature typeshed. calls.rr includes
    // python/modules.rr, which resolves `import os.path` into ts/stdlib/os/path.pyi and loads it: `os.path.join("a",
    // "b")` targets the stub's `join`, and no row is about a stub. The stub defines `join` in both branches of a
    // `sys.version_info` test: only the branch the run's Python (3.13, the default) takes binds it.
    let project = PathBuf::from(crate::code_rule_dsl::fixtures_dir!()).join("python/world_load/ws");
    let out = crate::code_rule_dsl::prefilter::tests::temp().with_extension("stdlib");
    corpus::run(&program("calls"), &project, None, &out, "program", Binding::Cpu, 1 << 18).unwrap();
    let text = std::fs::read_to_string(out.join("call_target.csv")).unwrap();
    let rows: Vec<(&str, &str)> = text.lines().filter_map(|l| l.split_once('\t')).collect();
    // Cells are `path:node`: the one call is main.py's, its target in the stub.
    assert_eq!(rows.len(), 1, "{text}");
    assert!(rows[0].0.starts_with("main.py:"), "{text}");
    assert!(rows[0].1.rsplit_once(':').unwrap().0.ends_with("world_load/ts/stdlib/os/path.pyi"), "{text}");
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn imports_resolve_through_site_packages_stubs_and_declarations_in_pep_561_order() {
    // fixtures/python/world_site/ws: a venv, the user's declarations and a miniature typeshed (rerail.env). Each
    // module of main.py resolves at one layer of python/modules.rr: alpha's stub package before its typed package,
    // beta's typed package from its `.py`, gamma (installed untyped, so not read) from typeshed's stubs/, delta from
    // a directory a `.pth` file names, eps from the declarations before its stub package. zeta, a single-file module
    // in site-packages, is not a typed package: its call has no target. `cv.Var("x")` targets Var's `__new__` only:
    // typeshed's `Generic: type[_Generic]` is a special form as a base, not `_Generic` and its subclasses (else Box,
    // another generic class, would be Var's base and its `__init__` a target).
    let project = PathBuf::from(crate::code_rule_dsl::fixtures_dir!()).join("python/world_site/ws");
    let out = crate::code_rule_dsl::prefilter::tests::temp().with_extension("site");
    corpus::run(&program("calls"), &project, None, &out, "program", Binding::Cpu, 1 << 18).unwrap();
    let text = std::fs::read_to_string(out.join("call_target.csv")).unwrap();
    let mut got: Vec<String> = text
        .lines()
        .filter_map(|l| l.split_once('\t'))
        .map(|(_, t)| t.rsplit_once(':').unwrap().0.split("world_site/").nth(1).unwrap().to_owned())
        .collect();
    got.sort();
    assert_eq!(
        got,
        [
            "decls/eps.pyi",
            "ts/stdlib/cv.pyi",
            "ts/stubs/types-gamma/gamma/__init__.pyi",
            "venv/extra/delta/__init__.pyi",
            "venv/lib/python3.13/site-packages/alpha-stubs/__init__.pyi",
            "venv/lib/python3.13/site-packages/beta/__init__.py",
        ],
        "{text}"
    );
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn declarations_and_installed_sources_type_calls_without_declared_ret_facts() {
    // fixtures/python/world_decl/app: the inputs `typed_corpus_reads_declarations_and_dependency_summaries` gave
    // `typing::deps` and `typing::declare`, as t21 reads them (step 3): the user's declarations are `.pyi` files under
    // `python.declarations`, and the installed package `vendor` (no stubs, no `py.typed`) is read from its sources
    // with `python.untyped = read`. `make`'s declaration comes first (PEP 561's order), over vendor/core.py's int.
    let project = PathBuf::from(crate::code_rule_dsl::fixtures_dir!()).join("python/world_decl/app");
    let out = crate::code_rule_dsl::prefilter::tests::temp().with_extension("decl");
    corpus::run(&program("calls"), &project, None, &out, "program", Binding::Cpu, 1 << 18).unwrap();
    let text = std::fs::read_to_string(out.join("type_of.csv")).unwrap();
    let mut got: Vec<&str> = text.lines().filter_map(|l| l.split_once('\t')).map(|(_, t)| t).filter(|t| *t != "module").collect();
    got.sort();
    // version() and make(): str; other.thing(): bytes | None.
    assert_eq!(got, ["None", "bytes", "str", "str"], "{text}");
    let targets = std::fs::read_to_string(out.join("call_target.csv")).unwrap();
    assert!(targets.contains("world_decl/decls/vendor/core.pyi:"), "{targets}");
    assert!(targets.contains("world_decl/decls/other.pyi:"), "{targets}");
    assert!(targets.contains("site-packages/vendor/__init__.py:"), "{targets}");
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn a_protocols_subclass_is_a_class_and_a_methods_self_annotation_declares_an_attribute() {
    // PEP 544: `Concrete` derives from the protocol `Sized` without listing `Protocol`, so it is an ordinary class: a
    // parameter declared `Concrete` holds a Concrete (or a subclass), and its method call has a target and is not
    // open. `self.names: list[Concrete] = []` in a method declares `names` on the class, as `names: ...` in its body.
    let project = crate::code_rule_dsl::prefilter::tests::temp().with_extension("proto");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(
        project.join("m.py"),
        "from typing import Protocol\n\nclass Sized(Protocol):\n    def size(self) -> int: ...\n\nclass Concrete(Sized):\n    def size(self) -> int:\n        return 1\n\nclass Holder:\n    def __init__(self) -> None:\n        self.names: list[Concrete] = []\n    def first(self) -> Concrete: ...\n\ndef use(c: Concrete, h: Holder) -> None:\n    c.size()\n    h.first().size()\n",
    )
    .unwrap();
    let out = project.join("out");
    corpus::run(&program("calls"), &project, None, &out, "program", Binding::Cpu, 1 << 18).unwrap();
    let targets = std::fs::read_to_string(out.join("call_target.csv")).unwrap();
    let open = std::fs::read_to_string(out.join("call_open.csv")).unwrap();
    // c.size() and h.first().size() reach Concrete.size; h.first() reaches Holder.first.
    assert_eq!(targets.lines().count(), 3, "{targets}");
    assert_eq!(open, "", "{open}");
    let _ = std::fs::remove_dir_all(&project);
}

#[test]
fn a_method_declared_to_return_its_class_parameter_returns_what_the_receivers_generic_binds() {
    // t21 §5.1, first version: `x: M[str, Box]` binds M's `_V` to Box, so `x.get("a")` is a Box and `.m()` targets
    // Box.m; `class D(M[str, Box])` binds it for every D; `pick`'s own `_T` is the argument its parameter is given.
    let project = crate::code_rule_dsl::prefilter::tests::temp().with_extension("generic");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(
        project.join("m.py"),
        "from typing import Generic, TypeVar\n_K = TypeVar(\"_K\")\n_V = TypeVar(\"_V\")\n_T = TypeVar(\"_T\")\nclass Box:\n    def m(self) -> int: ...\nclass M(Generic[_K, _V]):\n    def get(self, k: _K) -> _V: ...\n    def pick(self, k: _K, d: _T) -> _V | _T: ...\nclass D(M[str, Box]): ...\ndef f(x: M[str, Box], y: D) -> None:\n    x.get(\"a\").m()\n    y.get(\"a\").m()\n    x.pick(\"a\", Box()).m()\n",
    )
    .unwrap();
    let out = project.join("out");
    corpus::run(&program("calls"), &project, None, &out, "program", Binding::Cpu, 1 << 18).unwrap();
    let targets = std::fs::read_to_string(out.join("call_target.csv")).unwrap();
    // get, get, pick, and the three `.m()` calls.
    assert_eq!(targets.lines().count(), 6, "{targets}");
    assert_eq!(std::fs::read_to_string(out.join("call_open.csv")).unwrap(), "");
    let _ = std::fs::remove_dir_all(&project);
}

#[test]
fn a_subscript_is_a_call_of_its_receivers_getitem() {
    // `b[0]` is `b.__getitem__(0)`: its value is what B's `__getitem__` returns, so `.run()` targets V.run; a generic's
    // `__getitem__` returns what the receiver's annotation binds (`x: M[str, Box]`). A class's subscript
    // (`M[str, Box]`, a generic alias) is no call of `__getitem__`: calling it stays unknown.
    let project = crate::code_rule_dsl::prefilter::tests::temp().with_extension("subscript");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(
        project.join("m.py"),
        "from typing import Generic, TypeVar\n_K = TypeVar(\"_K\")\n_V = TypeVar(\"_V\")\nclass V:\n    def run(self) -> int: ...\nclass Box:\n    def m(self) -> int: ...\nclass B:\n    def __getitem__(self, i: int) -> V: ...\nclass M(Generic[_K, _V]):\n    def __getitem__(self, k: _K) -> _V: ...\ndef f(b: B, x: M[str, Box]) -> None:\n    b[0].run()\n    x[\"a\"].m()\n    M[str, Box]()\n",
    )
    .unwrap();
    let out = project.join("out");
    corpus::run(&program("calls"), &project, None, &out, "program", Binding::Cpu, 1 << 18).unwrap();
    let targets = std::fs::read_to_string(out.join("call_target.csv")).unwrap();
    assert_eq!(targets.lines().count(), 2, "{targets}");
    let open = std::fs::read_to_string(out.join("call_open.csv")).unwrap();
    assert_eq!(open.lines().count(), 1, "{open}");
    let _ = std::fs::remove_dir_all(&project);
}

#[test]
fn a_loop_target_is_its_iterators_next_value_and_a_with_alias_the_value_entered() {
    // `for x in s`: x is `s.__iter__().__next__()`; `with c as z`: z is `c.__enter__()`. Their `async` forms call other
    // methods (`__aiter__`, `__aenter__`): their targets stay unknown.
    let project = crate::code_rule_dsl::prefilter::tests::temp().with_extension("protocols");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(
        project.join("m.py"),
        "class V:\n    def run(self) -> int: ...\nclass It:\n    def __next__(self) -> V: ...\nclass Seq:\n    def __iter__(self) -> It: ...\nclass Cm:\n    def __enter__(self) -> V: ...\ndef f(s: Seq, c: Cm) -> None:\n    for x in s:\n        x.run()\n    with c as z:\n        z.run()\nasync def h(s: Seq, c: Cm) -> None:\n    async for x in s:\n        x.run()\n    async with c as z:\n        z.run()\n",
    )
    .unwrap();
    let out = project.join("out");
    corpus::run(&program("calls"), &project, None, &out, "program", Binding::Cpu, 1 << 18).unwrap();
    let targets = std::fs::read_to_string(out.join("call_target.csv")).unwrap();
    assert_eq!(targets.lines().count(), 2, "{targets}");
    let open = std::fs::read_to_string(out.join("call_open.csv")).unwrap();
    assert_eq!(open.lines().count(), 2, "{open}");
    let _ = std::fs::remove_dir_all(&project);
}

#[test]
fn a_declared_generic_return_carries_the_receivers_binding() {
    // t21 §5.1: `Seq[V]`'s `__iter__() -> It[_T]` is an It of V, so the loop's `__next__() -> _T` is a V; `Gen`'s
    // `__iter__() -> It[V]` is one as written. The declared generic's class is exact: `Sub`'s `__next__` is typed by
    // It's and does not reach the loop.
    let project = crate::code_rule_dsl::prefilter::tests::temp().with_extension("genret");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(
        project.join("m.py"),
        "from typing import Generic, TypeVar\n_T = TypeVar(\"_T\")\nclass V:\n    def run(self) -> int: ...\nclass W:\n    def run(self) -> int: ...\nclass It(Generic[_T]):\n    def __next__(self) -> _T: ...\nclass Sub(It[W]):\n    def __next__(self) -> W: ...\nclass Seq(Generic[_T]):\n    def __iter__(self) -> It[_T]: ...\nclass Gen:\n    def __iter__(self) -> It[V]: ...\ndef f(s: Seq[V], g: Gen) -> None:\n    for x in s:\n        x.run()\n    for y in g:\n        y.run()\n",
    )
    .unwrap();
    let out = project.join("out");
    corpus::run(&program("calls"), &project, None, &out, "program", Binding::Cpu, 1 << 18).unwrap();
    let targets = std::fs::read_to_string(out.join("call_target.csv")).unwrap();
    assert_eq!(targets.lines().count(), 2, "{targets}");
    // Both calls target one definition (V.run, not W.run).
    let defs: BTreeSet<&str> = targets.lines().filter_map(|l| l.split('\t').nth(1)).collect();
    assert_eq!(defs.len(), 1, "{targets}");
    assert_eq!(std::fs::read_to_string(out.join("call_open.csv")).unwrap(), "");
    let _ = std::fs::remove_dir_all(&project);
}

#[test]
fn a_class_decorator_declared_type_t_to_type_t_keeps_the_class() {
    // `deco(cls: type[T]) -> type[T]` gives back the class it decorates, whatever its body returns (an unknown
    // `wrap(cls)`): A stays A, so B's base is known and `b.m()` targets A.m. Only `wrap(cls)` is unknown.
    let project = crate::code_rule_dsl::prefilter::tests::temp().with_extension("classdec");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(
        project.join("m.py"),
        "from typing import TypeVar\nT = TypeVar(\"T\")\ndef deco(cls: type[T]) -> type[T]:\n    return wrap(cls)\n@deco\nclass A:\n    def m(self) -> int: ...\nclass B(A): ...\ndef f(b: B) -> None:\n    b.m()\n",
    )
    .unwrap();
    let out = project.join("out");
    corpus::run(&program("calls"), &project, None, &out, "program", Binding::Cpu, 1 << 18).unwrap();
    let targets = std::fs::read_to_string(out.join("call_target.csv")).unwrap();
    assert_eq!(targets.lines().count(), 1, "{targets}");
    let open = std::fs::read_to_string(out.join("call_open.csv")).unwrap();
    assert_eq!(open.lines().count(), 1, "{open}");
    let _ = std::fs::remove_dir_all(&project);
}

#[test]
fn a_paramspec_wrapper_call_holds_the_declared_class_bound_by_the_wrapped_return() {
    // `cm(func: Callable[P, It[T]]) -> Callable[P, Cm[T]]` (contextlib's `contextmanager`): `opened()` runs
    // `opened` and holds a Cm of V (`Gen[V]` is an It of V). Cm's parameter is read from its first base with a type
    // variable as an argument (`Enter[T]`, not `Base[It[T]]`), so `with opened() as v` binds v to a V and `v.run()`
    // targets V.run.
    let project = crate::code_rule_dsl::prefilter::tests::temp().with_extension("wrapcm");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(
        project.join("m.py"),
        "from typing import Callable, Generic, ParamSpec, Protocol, TypeVar\nT = TypeVar(\"T\")\nP = ParamSpec(\"P\")\nclass It(Generic[T]): ...\nclass Gen(It[T]): ...\nclass Enter(Protocol[T]):\n    def __enter__(self) -> T: ...\nclass Base(Generic[T]): ...\nclass Cm(Base[It[T]], Enter[T]): ...\ndef cm(func: Callable[P, It[T]]) -> Callable[P, Cm[T]]: ...\nclass V:\n    def run(self) -> int: ...\n@cm\ndef opened() -> Gen[V]: ...\ndef f() -> None:\n    with opened() as v:\n        v.run()\n",
    )
    .unwrap();
    let out = project.join("out");
    corpus::run(&program("calls"), &project, None, &out, "program", Binding::Cpu, 1 << 18).unwrap();
    let targets = std::fs::read_to_string(out.join("call_target.csv")).unwrap();
    assert_eq!(targets.lines().count(), 2, "{targets}");
    let open = std::fs::read_to_string(out.join("call_open.csv")).unwrap_or_default();
    assert_eq!(open.lines().count(), 0, "{open}");
    let types = std::fs::read_to_string(out.join("type_of.csv")).unwrap();
    assert_eq!(types.lines().filter(|l| l.ends_with("\tm.V")).count(), 2, "{types}");
    let _ = std::fs::remove_dir_all(&project);
}

#[test]
fn a_functions_own_type_variable_is_bound_by_its_arguments_generic() {
    // `pick(xs: It[S]) -> S` and `ordered(xs: It[S]) -> Seq[S]` (`min`, `sorted`): S is what the argument's class gives
    // It's parameter, through its generic value (`Seq[V]`) or its class's bases (`class Vs(Seq[V])`). Every `run()`
    // targets V.run.
    let project = crate::code_rule_dsl::prefilter::tests::temp().with_extension("ownvar");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(
        project.join("m.py"),
        "from typing import Generic, TypeVar\nT = TypeVar(\"T\")\nS = TypeVar(\"S\")\nclass It(Generic[T]): ...\nclass Seq(It[T]):\n    def first(self) -> T: ...\nclass V:\n    def run(self) -> int: ...\nclass Vs(Seq[V]): ...\ndef ordered(xs: It[S]) -> Seq[S]: ...\ndef pick(xs: It[S], d: int = 0) -> S: ...\ndef f(xs: Seq[V], ys: Vs) -> None:\n    pick(xs).run()\n    ordered(xs).first().run()\n    pick(ys).run()\n",
    )
    .unwrap();
    let out = project.join("out");
    corpus::run(&program("calls"), &project, None, &out, "program", Binding::Cpu, 1 << 18).unwrap();
    let targets = std::fs::read_to_string(out.join("call_target.csv")).unwrap();
    let open = std::fs::read_to_string(out.join("call_open.csv")).unwrap_or_default();
    eprintln!("T {targets}\nO {open}");
    assert_eq!(open.lines().count(), 0, "{open}");
    assert_eq!(targets.lines().count(), 7, "{targets}");
    let _ = std::fs::remove_dir_all(&project);
}

#[test]
fn a_protocol_parameters_type_variable_is_what_the_arguments_method_returns() {
    // `step(i: HasNext[S]) -> S` (`next`): Feed does not derive from HasNext; S is what `feed.__next__()` returns, a
    // V, so `run()` targets V.run.
    let project = crate::code_rule_dsl::prefilter::tests::temp().with_extension("protovar");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(
        project.join("m.py"),
        "from typing import Protocol, TypeVar\nT = TypeVar(\"T\")\nS = TypeVar(\"S\")\nclass HasNext(Protocol[T]):\n    def __next__(self) -> T: ...\nclass V:\n    def run(self) -> int: ...\nclass Feed:\n    def __next__(self) -> V: ...\ndef step(i: HasNext[S]) -> S: ...\ndef f(feed: Feed) -> None:\n    step(feed).run()\n",
    )
    .unwrap();
    let out = project.join("out");
    corpus::run(&program("calls"), &project, None, &out, "program", Binding::Cpu, 1 << 18).unwrap();
    let targets = std::fs::read_to_string(out.join("call_target.csv")).unwrap();
    let open = std::fs::read_to_string(out.join("call_open.csv")).unwrap_or_default();
    assert_eq!(open.lines().count(), 0, "{open}");
    assert_eq!(targets.lines().count(), 2, "{targets}");
    let _ = std::fs::remove_dir_all(&project);
}

#[test]
fn a_call_is_typed_by_the_overloads_its_positional_arguments_fit() {
    // `get(1)` fits only the first overload (the second requires the keyword z), `get(1, 2, z=3)` only the second (the
    // first takes one positional argument), and `k.m()`, its `self` bound, only the first. Each call is one class, so
    // every `a()` and `b()` is a known method.
    let project = crate::code_rule_dsl::prefilter::tests::temp().with_extension("ovlarity");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(
        project.join("m.py"),
        "from typing import overload\nclass A:\n    def a(self) -> int: ...\nclass B:\n    def b(self) -> int: ...\n@overload\ndef get(x: int, /) -> A: ...\n@overload\ndef get(x: int, y: int = 0, *, z: int) -> B: ...\nclass K:\n    @overload\n    def m(self) -> A: ...\n    @overload\n    def m(self, y: int) -> B: ...\ndef f(k: K) -> None:\n    get(1).a()\n    get(1, 2, z=3).b()\n    k.m().a()\n    k.m(2).b()\n",
    )
    .unwrap();
    let out = project.join("out");
    corpus::run(&program("calls"), &project, None, &out, "program", Binding::Cpu, 1 << 18).unwrap();
    let types = std::fs::read_to_string(out.join("type_of.csv")).unwrap();
    let open = std::fs::read_to_string(out.join("call_open.csv")).unwrap_or_default();
    assert_eq!(open.lines().count(), 0, "{open}");
    assert_eq!(types.lines().filter(|l| l.ends_with("\tm.A") || l.ends_with("\tm.B")).count(), 4, "{types}");
    let _ = std::fs::remove_dir_all(&project);
}

/// A rule that reads types (decision 0047, point 3): `docs/rule-syntax/data-layer/leg-083-typed.rr`, LEG-083 counting
/// a `resolve()` as eager by its receiver's type, not its name, over `fixtures/python/leg083`. A lazy receiver is
/// told by a declared return (`-> LazyResult[...]`), a decorator declared to keep the function (`(fn: F) -> F`), a
/// class attribute's annotation, a quoted annotation and a chain of them; a pathlib path and an untyped parameter
/// are unknown, so not counted; `catalog.resolve(ref)` has an argument.
#[test]
fn leg_083_typed_counts_resolve_by_the_receivers_type() {
    let project = PathBuf::from(crate::code_rule_dsl::fixtures_dir!()).join("python/leg083");
    let rule_src = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../docs/rule-syntax/data-layer/leg-083-typed.rr")).unwrap();
    let spans = "
.decl at(p: symbol, s: number, what: symbol)
at(p, s, \"lazy\") :- lazy_terminal(c), node(d, c, _, s, _), doc(d, p, _).
at(p, s, \"unknown\") :- terminal_unknown(c), node(d, c, _, s, _), doc(d, p, _).
at(p, s, \"finding\") :- LEG_083_TYPED(f), node(d, f, _, s, _), doc(d, p, _).
.output at
";
    let rule_dir = crate::code_rule_dsl::prefilter::tests::temp().with_extension("leg083");
    std::fs::create_dir_all(&rule_dir).unwrap();
    let rule = rule_dir.join("rule.rr");
    std::fs::write(&rule, format!("{rule_src}\n{spans}")).unwrap();
    let text = crate::code_rule_dsl::logic::rr::print(&crate::code_rule_dsl::logic::rr::parse_file(&rule).unwrap()).unwrap();
    let out = rule_dir.join("out");
    let report = corpus::run(&text, &project, None, &out, "program", Binding::Cpu, 1 << 18).unwrap();
    assert_eq!(report.typing_runs.iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>(), ["calls"]);
    let mut got: BTreeSet<(String, u64, String)> = BTreeSet::new();
    for l in std::fs::read_to_string(out.join("at.csv")).unwrap().lines() {
        let r: Vec<&str> = l.split('\t').collect();
        let src = std::fs::read_to_string(project.join(r[0])).unwrap();
        got.insert((r[2].to_owned(), line(&src, r[1].parse().unwrap()), r[0].to_owned()));
    }
    let want: BTreeSet<(String, u64, String)> = [
        ("finding", 6, "pkg/use.py"),  // eager: txn.select_one_lazy(1).resolve()
        ("finding", 10, "pkg/use.py"), // through: calls eager
        ("finding", 14, "pkg/use.py"), // decorated: @lazy keeps count_lazy's declared return
        ("finding", 18, "pkg/use.py"), // attribute: Holder.rows is a LazySeq
        ("finding", 22, "pkg/use.py"), // chained: h.txn.select_one_lazy(2).resolve_async()
        ("finding", 26, "pkg/use.py"), // quoted: query_lazy is declared `-> "LazySeq[int]"`
        ("lazy", 7, "pkg/use.py"),
        ("lazy", 15, "pkg/use.py"),
        ("lazy", 19, "pkg/use.py"),
        ("lazy", 23, "pkg/use.py"),
        ("lazy", 27, "pkg/use.py"),      // a quoted annotation is read as its code (decision 0055)
        ("unknown", 22, "pkg/reads.py"), // path.resolve(): a pathlib class is not typed yet
        ("unknown", 31, "pkg/use.py"),   // p: Path
        ("unknown", 39, "pkg/use.py"),   // x untyped
    ]
    .into_iter()
    .map(|(w, l, p)| (w.to_owned(), l, p.to_owned()))
    .collect();
    assert_eq!(got, want);
    let _ = std::fs::remove_dir_all(&rule_dir);
}

/// Each call's callee class, where calling a class: Jedi's `goto` names the class, the program its `__init__`. The
/// spans are every definition's, not the targets' only: a row about a definition is then not a function of its
/// callers, which a run memoising per import component (decision 0076) needs.
const JEDI_SPANS: &str = "
.decl rr_jedi_cls(c: node, k: node)
rr_jedi_cls(c, k) :- callee(c, e), pv(e, \"cls\", k).
.decl rr_jedi_at(n: node, s: number, e: number)
rr_jedi_at(n, s, e) :- kind(n, \"call\"), node(_, n, _, s, e).
rr_jedi_at(f, s, e) :- kind(f, \"function_definition\"), node(_, f, _, s, e).
rr_jedi_at(f, s, e) :- kind(f, \"class_definition\"), node(_, f, _, s, e).
rr_jedi_at(f, s, e) :- kind(f, \"lambda\"), node(_, f, _, s, e).
.output rr_jedi_cls, rr_jedi_at
";

/// `calls.rr` against Jedi 0.19.2's `goto` over a Python corpus (`RERAIL_PYTHON_CALLS`, written by
/// `fixtures/python/calls/generate_corpus.py`; O88). The program runs over `RERAIL_PYTHON_CALLS_ROOT`, a directory
/// whose tree holds the corpus's paths as the recording names them (`django/...`). Where both decide, the program
/// agrees when Jedi's definition is among those it names, its class among the classes a class call makes, or, for
/// an external target, it names no workspace definition.
#[test]
#[ignore = "Jedi call targets over a Python corpus (minutes)"]
fn benchmark_python_calls_program_corpus() {
    let recording: Json = serde_json::from_str(&std::fs::read_to_string(std::env::var("RERAIL_PYTHON_CALLS").unwrap()).unwrap()).unwrap();
    let root = PathBuf::from(std::env::var("RERAIL_PYTHON_CALLS_ROOT").unwrap());
    // `RERAIL_PYTHON_CALLS_OUT`: write the outputs there and keep them (the program's other `.output`s too, for counting).
    let out = std::env::var_os("RERAIL_PYTHON_CALLS_OUT").map(PathBuf::from).unwrap_or_else(|| crate::code_rule_dsl::prefilter::tests::temp().with_extension("jedi"));
    // `RERAIL_PYTHON_TYPESHED`: a typeshed checkout, so calls.rr loads the standard library's stubs (t21).
    let settings: Vec<String> = std::env::var("RERAIL_PYTHON_TYPESHED").map(|t| vec![format!("python.typeshed={t}")]).unwrap_or_default();
    let mut text = format!("{}\n{JEDI_SPANS}", program(&calls_name()));
    // `RERAIL_PYTHON_ROUNDS`: the rounds' bound k, for measuring what more rounds decide.
    if let Ok(k) = std::env::var("RERAIL_PYTHON_ROUNDS") {
        text = text.replace(".pragma \"cross-rounds\" \"4\"", &format!(".pragma \"cross-rounds\" \"{k}\""));
    }
    // `RERAIL_PYTHON_CROSS_MEMO`: relations memoised per import component (decision 0076), as `.pragma "cross-memo"`.
    if let Ok(rels) = std::env::var("RERAIL_PYTHON_CROSS_MEMO") {
        text.push_str(&format!("\n.pragma \"cross-memo\" \"{rels}\"\n"));
    }
    let report = corpus::run_with_options(&[&text], &root, None, &out, "program", Binding::Cpu, 1 << 18, &corpus::Typing::default(), &serde_json::json!({ "env": settings })).unwrap();
    let cpu: f64 = report.phases.iter().map(|p| p.1 + p.2).sum();
    println!("cross-file memo: {} components found, {} derived", report.cross.0, report.cross.1);
    println!("calls.rr over {} files: rounds {:?}; {cpu:.1} s CPU (main thread), peak RSS {:.0} MB", report.files, report.rounds, report.phases.iter().map(|p| p.3).fold(0.0, f64::max));
    let rows = |rel: &str| -> Vec<Vec<String>> {
        std::fs::read_to_string(out.join(format!("{rel}.csv"))).unwrap().lines().map(|l| l.split('\t').map(str::to_owned).collect()).collect()
    };
    let at: BTreeMap<String, (usize, usize)> = rows("rr_jedi_at").into_iter().map(|r| (r[0].clone(), (r[1].parse().unwrap(), r[2].parse().unwrap()))).collect();
    let file = |node: &str| node.rsplit_once(':').unwrap().0.to_owned();
    let mut sources: BTreeMap<String, String> = BTreeMap::new();
    let mut line_of = |f: &str, s: usize| -> u64 {
        let src = sources.entry(f.to_owned()).or_insert_with(|| std::fs::read_to_string(root.join(f)).unwrap_or_default());
        line(src, s.min(src.len()))
    };
    let key = |n: &str| (file(n), at[n].0, at[n].1);
    // A target in a loaded document (a stub, named by its absolute path) is outside the workspace: Jedi's
    // `external`. Its rows are not written, so it has no span here.
    let library = |f: &str| f.starts_with('/');
    let mut target = |n: &str| -> (String, u64) {
        let f = file(n);
        if library(&f) { ("<library>".to_owned(), 0) } else { let s = at[n].0; let l = line_of(&f, s); (f, l) }
    };
    let open: BTreeSet<(String, usize, usize)> = rows("call_open").iter().map(|r| key(&r[0])).collect();
    let mut named: BTreeMap<(String, usize, usize), BTreeSet<(String, u64)>> = BTreeMap::new();
    for r in rows("call_target") {
        let t = target(&r[1]);
        named.entry(key(&r[0])).or_default().insert(t);
    }
    let mut classes: BTreeMap<(String, usize, usize), BTreeSet<(String, u64)>> = BTreeMap::new();
    for r in rows("rr_jedi_cls") {
        let t = target(&r[1]);
        classes.entry(key(&r[0])).or_default().insert(t);
    }
    if std::env::var_os("RERAIL_PYTHON_CALLS_OUT").is_none() {
        let _ = std::fs::remove_dir_all(&out);
    }
    let mut tally: BTreeMap<&str, usize> = BTreeMap::new();
    let mut examples: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    // `RERAIL_PYTHON_CALLS_DUMP`: each call's category, `path:line<TAB>category<TAB>jedi`, to compare two runs.
    let mut dump = String::new();
    for corpus in recording["corpora"].as_array().unwrap() {
        for (path, records) in corpus["records"].as_object().unwrap() {
            for record in records.as_array().unwrap() {
                let k = (path.clone(), record[0].as_u64().unwrap() as usize, record[1].as_u64().unwrap() as usize);
                let targets = record[3].as_array().unwrap();
                *tally.entry("calls").or_default() += 1;
                let decided: Vec<&Json> = targets.iter().filter(|t| t[0] != "other").collect();
                if decided.is_empty() {
                    *tally.entry("jedi undecided").or_default() += 1;
                    continue;
                }
                if open.contains(&k) {
                    *tally.entry("program unknown, jedi decided").or_default() += 1;
                    dump += &format!("{path}:{}\tunknown\t{decided:?}\n", record[2]);
                    continue;
                }
                *tally.entry("both decided").or_default() += 1;
                let lib = ("<library>".to_owned(), 0);
                let reaches_library = named.get(&k).is_some_and(|d| d.contains(&lib)) || classes.get(&k).is_some_and(|c| c.contains(&lib));
                let mut defs = named.get(&k).cloned().unwrap_or_default();
                let mut cls = classes.get(&k).cloned().unwrap_or_default();
                defs.remove(&lib);
                cls.remove(&lib);
                if reaches_library {
                    *tally.entry("(program reaches a stub)").or_default() += 1;
                }
                let at_of = |t: &Json| (t[1].as_str().unwrap().to_owned(), t[2].as_u64().unwrap());
                let ok = decided.iter().any(|t| match t[0].as_str().unwrap() {
                    "def" => defs.contains(&at_of(t)),
                    "class" => cls.contains(&at_of(t)),
                    _ => defs.is_empty() && cls.is_empty(),
                });
                let what = if ok {
                    "agree"
                } else if defs.is_empty() && cls.is_empty() {
                    "program no edge, jedi a definition"
                } else if decided.iter().all(|t| t[0] == "external") {
                    "program a definition, jedi external"
                } else {
                    "program another definition"
                };
                *tally.entry(what).or_default() += 1;
                dump += &format!("{path}:{}\t{what}\t{decided:?}\n", record[2]);
                if what != "agree" {
                    let list = examples.entry(what).or_default();
                    if list.len() < 12 {
                        list.push(format!("{path}:{} jedi {decided:?} program {defs:?} classes {cls:?}", record[2]));
                    }
                }
            }
        }
    }
    if let Ok(path) = std::env::var("RERAIL_PYTHON_CALLS_DUMP") {
        std::fs::write(path, dump).unwrap();
    }
    for (what, n) in &tally {
        println!("{n:>8}  {what}");
    }
    for (what, list) in &examples {
        println!("{what}:");
        for e in list {
            println!("    {e}");
        }
    }
}

/// O125 (decision 0083, spike 1): delete-and-rederive on calls.rr's cross-file fixpoint. A session (0077) keeps the
/// cross-file part; each edit (`RERAIL_SPIKE_EDITS`: E1 append to exceptions.py, E2 a comment at its top, E3 a
/// statement in an early body of lazy_reads.py) is run through the session, then without it, and the outputs are
/// compared. `RERAIL_SPIKE_DELTA` prints the rows overdeleted and put back per relation.
#[test]
#[ignore = "a spike: minutes over the 84-file corpus with typeshed"]
fn spike_o125_a_session_maintains_calls_rr() {
    use crate::code_rule_dsl::logic::plan::corpus::session;
    let root = PathBuf::from(std::env::var("RERAIL_PYTHON_CALLS_ROOT").unwrap());
    let settings: Vec<String> = std::env::var("RERAIL_PYTHON_TYPESHED").map(|t| vec![format!("python.typeshed={t}")]).unwrap_or_default();
    let text = format!("{}\n{JEDI_SPANS}", program("calls"));
    let base = crate::code_rule_dsl::prefilter::tests::temp().with_extension("o125");
    let out = base.join("out");
    let run = || corpus::run_with_options(&[&text], &root, None, &out, "program", Binding::Cpu, 1 << 18, &corpus::Typing::default(), &serde_json::json!({ "env": settings })).unwrap();
    let dump = || -> BTreeMap<String, Vec<String>> {
        let mut all = BTreeMap::new();
        for e in std::fs::read_dir(&out).unwrap().flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x == "csv") {
                let mut lines: Vec<String> = std::fs::read_to_string(&p).unwrap().lines().map(str::to_owned).collect();
                lines.sort();
                all.insert(p.file_name().unwrap().to_string_lossy().into_owned(), lines);
            }
        }
        all
    };
    let cpu = |r: &corpus::Report, phase: Option<&str>| -> f64 { r.phases.iter().filter(|p| phase.is_none_or(|n| p.0 == n)).map(|p| p.1 + p.2).sum() };
    let rss = |r: &corpus::Report| r.phases.iter().map(|p| p.3).fold(0.0, f64::max);
    let show = |what: &str, r: &corpus::Report| {
        println!("O125 {what}: {:.1} s CPU (main thread), cross {:.1} s, peak RSS {:.0} MB; strata {:?}, maintained {:?}", cpu(r, None), cpu(r, Some("cross")), rss(r), r.cross_strata, r.cross_maintained);
    };
    session::start(&base.join("session")).unwrap();
    let primed = run();
    show("prime (session, cold cross part)", &primed);
    let again = run();
    show("unchanged (session)", &again);
    let edits = std::env::var("RERAIL_SPIKE_EDITS").unwrap_or_else(|_| "E1,E2,E3".to_owned());
    for edit in edits.split(',') {
        let (file, change): (&str, fn(&str) -> String) = match edit {
            "E1" => ("l1100_exceptions/exceptions.py", |s| format!("{s}\n\ndef _o125_probe() -> None:\n    pass\n")),
            "E2" => ("l1100_exceptions/exceptions.py", |s| format!("# o125\n{s}")),
            "E3" => ("l2055_lazy_result/lazy_reads.py", |s| {
                let mut lines: Vec<&str> = s.lines().collect();
                let at = lines.iter().position(|l| l.starts_with("def _lazy_result_to_seq")).unwrap();
                lines.insert(at + 1, "    _o125 = 1");
                lines.join("\n") + "\n"
            }),
            other => panic!("unknown edit {other}"),
        };
        let path = root.join(file);
        let before = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, change(&before)).unwrap();
        let maintained = run();
        show(&format!("{edit} (session, maintained)"), &maintained);
        let kept = dump();
        let full = session::full(run);
        show(&format!("{edit} (no session, cross part derived)"), &full);
        let derived = dump();
        let differ: Vec<&String> = derived.keys().filter(|k| kept.get(*k) != derived.get(*k)).collect();
        println!("O125 {edit}: outputs {}", if differ.is_empty() { "equal".to_owned() } else { format!("DIFFER in {differ:?}") });
        for k in &differ {
            let a: BTreeSet<&String> = kept.get(*k).into_iter().flatten().collect();
            let b: BTreeSet<&String> = derived.get(*k).into_iter().flatten().collect();
            let only_kept: Vec<&&String> = a.difference(&b).collect();
            let only_derived: Vec<&&String> = b.difference(&a).collect();
            println!("O125 {edit} {k}: {} rows, {} only maintained, {} only derived; e.g. {:?} / {:?}", b.len(), only_kept.len(), only_derived.len(), only_kept.iter().take(3).collect::<Vec<_>>(), only_derived.iter().take(3).collect::<Vec<_>>());
        }
    }
    session::stop();
    let _ = std::fs::remove_dir_all(&base);
}
