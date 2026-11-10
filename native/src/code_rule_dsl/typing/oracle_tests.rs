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
.decl oracle_score_at(n: node, s: number, e: number)
oracle_score_at(n, s, e) :- call_target(n, _), node(_, n, _, s, e).
oracle_score_at(n, s, e) :- call_open(n), node(_, n, _, s, e).
oracle_score_at(f, s, e) :- call_target(_, f), node(_, f, _, s, e).
.output oracle_score_at
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
    let at: BTreeMap<String, (usize, usize)> = rows("oracle_score_at").into_iter().map(|r| (r[0].clone(), (r[1].parse().unwrap(), r[2].parse().unwrap()))).collect();
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
.decl oracle_type_at(n: node, s: number, e: number)
oracle_type_at(n, s, e) :- type_of(n, _), node(_, n, _, s, e).
oracle_type_at(n, s, e) :- type_open(n), node(_, n, _, s, e).
.output oracle_type_at
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
    let at: BTreeMap<String, (String, usize, usize)> = rows("oracle_type_at")
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
fn the_analysis_phase_binds_a_library_generics_type_variables_fits_overloads_and_leaves_async_calls_open() {
    // calls_p3 over a miniature typeshed: `box.Box[V]`'s `get()` (the overload without a default) returns its `_T`,
    // bound by the annotation to V; a loop over it calls `__iter__() -> It[_T]`, whose `__next__() -> _T` is then a V;
    // an `async def`'s call holds a coroutine, so a method called on it is open.
    let dir = crate::code_rule_dsl::prefilter::tests::temp().with_extension("p3-generics");
    let _ = std::fs::remove_dir_all(&dir);
    let (ws, lib) = (dir.join("ws"), dir.join("ts/stdlib"));
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::create_dir_all(&lib).unwrap();
    std::fs::write(lib.join("VERSIONS"), "builtins: 3.0-\ntyping: 3.0-\nbox: 3.0-\n").unwrap();
    std::fs::write(lib.join("builtins.pyi"), "class object: ...\nclass int:\n    def bit_length(self) -> int: ...\nclass str: ...\n").unwrap();
    std::fs::write(lib.join("typing.pyi"), "class _SpecialForm: ...\nGeneric: _SpecialForm\ndef TypeVar(name: str) -> object: ...\ndef overload(f: object) -> object: ...\n").unwrap();
    std::fs::write(
        lib.join("box.pyi"),
        "from typing import Generic, TypeVar, overload\n_T = TypeVar(\"_T\")\n_D = TypeVar(\"_D\")\nclass It(Generic[_T]):\n    def __next__(self) -> _T: ...\nclass Box(Generic[_T]):\n    def __iter__(self) -> It[_T]: ...\n    @overload\n    def get(self) -> _T: ...\n    @overload\n    def get(self, default: _D) -> _T | _D: ...\nasync def fetch() -> int: ...\n",
    )
    .unwrap();
    std::fs::write(ws.join("rerail.env"), "python.typeshed = ../ts\n").unwrap();
    std::fs::write(
        ws.join("main.py"),
        "from box import Box, fetch\nclass V:\n    def run(self) -> int: ...\ndef f(b: Box[V]) -> None:\n    b.get().run()\n    for x in b:\n        x.run()\n    fetch().bit_length()\n",
    )
    .unwrap();
    let out = dir.join("out");
    let _no_session = corpus::SESSION_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    corpus::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(dir.join("store.sqlite")));
    let text = format!("{}\n{JEDI_SPANS}", program("calls_p3"));
    corpus::run(&text, &ws, None, &out, "program", Binding::Cpu, 1 << 18).unwrap();
    let read = |rel: &str| std::fs::read_to_string(out.join(format!("{rel}.csv"))).unwrap();
    let src = std::fs::read_to_string(ws.join("main.py")).unwrap();
    let starts: std::collections::HashMap<String, usize> =
        read("oracle_jedi_at").lines().map(|l| { let v: Vec<&str> = l.split('\t').collect(); (v[0].to_owned(), v[1].parse().unwrap()) }).collect();
    let at_line = |cell: &str| line(&src, starts[cell]);
    let targets = read("call_target");
    let mut to_v: Vec<u64> = targets.lines().filter_map(|l| l.split_once('\t')).map(|(c, _)| at_line(c)).collect();
    to_v.sort();
    assert_eq!(to_v, [5, 7], "V.run, through get()'s _T and the loop's __next__: {targets}");
    let open: Vec<u64> = read("call_open").lines().map(|l| at_line(l.split('\t').next().unwrap())).collect();
    assert_eq!(open, [8], "the method of an async call's coroutine: {}", read("call_open"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_analysis_phase_fits_a_library_overload_by_its_keywords_and_its_literal_arguments_types() {
    // calls_p3 over a miniature typeshed: `e.get("k", default=1)` is the overload with a `default` parameter, not the
    // one that has room for its one positional argument; `conv(1)` is the overload declaring an `int`, `conv("s")`
    // the one declaring a `str`; `wide(1)` fits `x: float`.
    let dir = crate::code_rule_dsl::prefilter::tests::temp().with_extension("p3-overloads");
    let _ = std::fs::remove_dir_all(&dir);
    let (ws, lib) = (dir.join("ws"), dir.join("ts/stdlib"));
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::create_dir_all(&lib).unwrap();
    std::fs::write(lib.join("VERSIONS"), "builtins: 3.0-\ntyping: 3.0-\nbox: 3.0-\n").unwrap();
    std::fs::write(lib.join("builtins.pyi"), "class object: ...\nclass int: ...\nclass float: ...\nclass str: ...\n").unwrap();
    std::fs::write(lib.join("typing.pyi"), "def overload(f: object) -> object: ...\n").unwrap();
    std::fs::write(
        lib.join("box.pyi"),
        "from typing import overload\nclass A:\n    def run(self) -> int: ...\nclass B:\n    def run(self) -> int: ...\nclass Env:\n    @overload\n    def get(self, key: str, /) -> A: ...\n    @overload\n    def get(self, key: str, /, default: int) -> B: ...\n@overload\ndef conv(x: int) -> A: ...\n@overload\ndef conv(x: str) -> B: ...\n@overload\ndef wide(x: float) -> A: ...\n@overload\ndef wide(x: str) -> B: ...\n",
    )
    .unwrap();
    std::fs::write(ws.join("rerail.env"), "python.typeshed = ../ts\n").unwrap();
    std::fs::write(
        ws.join("main.py"),
        "from box import Env, conv, wide\ndef f(e: Env) -> None:\n    e.get(\"k\", default=1).run()\n    conv(1).run()\n    conv(\"s\").run()\n    wide(1).run()\n",
    )
    .unwrap();
    let out = dir.join("out");
    let _no_session = corpus::SESSION_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    corpus::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(dir.join("store.sqlite")));
    // The overloads each call fits (`lcfn`), by the call's line.
    let text = format!("{}\n{JEDI_SPANS}\n.decl t_fit(c: node, f: symbol)\nt_fit(c, f) :- lcfn(c, _, f), struct__flag(f, \"overload\").\n.output t_fit\n", program("calls_p3"));
    corpus::run(&text, &ws, None, &out, "program", Binding::Cpu, 1 << 18).unwrap();
    let read = |rel: &str| std::fs::read_to_string(out.join(format!("{rel}.csv"))).unwrap();
    let src = std::fs::read_to_string(ws.join("main.py")).unwrap();
    let starts: std::collections::HashMap<String, usize> =
        read("oracle_jedi_at").lines().map(|l| { let v: Vec<&str> = l.split('\t').collect(); (v[0].to_owned(), v[1].parse().unwrap()) }).collect();
    let mut fits: Vec<(u64, String)> = read("t_fit").lines().filter_map(|l| l.split_once('\t')).map(|(c, f)| (line(&src, starts[c]), f.to_owned())).collect();
    fits.sort();
    let want: Vec<(u64, String)> = [(3, "box.Env.get#1"), (4, "box.conv"), (5, "box.conv#1"), (6, "box.wide")].iter().map(|(l, f)| (*l, (*f).to_owned())).collect();
    assert_eq!(fits, want, "{}", read("t_fit"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_analysis_phase_infers_a_library_constructors_type_variables_from_its_arguments() {
    // calls_p3 over a miniature typeshed: `Set(xs)`, where `__new__(cls, xs: Src[_T])` and `Lst(Src[_T])`, is a
    // `Set` of what xs holds. Of a workspace V (`xs: Lst[V]`), a loop over it binds V (`x.run()` targets V.run); of
    // a library str (`names() -> Lst[str]`), `s.upper()` is str's, not open.
    let dir = crate::code_rule_dsl::prefilter::tests::temp().with_extension("p3-ctor");
    let _ = std::fs::remove_dir_all(&dir);
    let (ws, lib) = (dir.join("ws"), dir.join("ts/stdlib"));
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::create_dir_all(&lib).unwrap();
    std::fs::write(lib.join("VERSIONS"), "builtins: 3.0-\ntyping: 3.0-\nbox: 3.0-\n").unwrap();
    std::fs::write(lib.join("builtins.pyi"), "class object: ...\nclass int: ...\nclass str:\n    def upper(self) -> str: ...\n").unwrap();
    std::fs::write(lib.join("typing.pyi"), "class _SpecialForm: ...\nGeneric: _SpecialForm\ndef TypeVar(name: str) -> object: ...\n").unwrap();
    std::fs::write(
        lib.join("box.pyi"),
        "from typing import Generic, TypeVar\n_T = TypeVar(\"_T\")\nclass It(Generic[_T]):\n    def __next__(self) -> _T: ...\nclass Src(Generic[_T]):\n    def __iter__(self) -> It[_T]: ...\nclass Lst(Src[_T]): ...\nclass Set(Generic[_T]):\n    def __new__(cls, xs: Src[_T]) -> Set[_T]: ...\n    def __iter__(self) -> It[_T]: ...\ndef names() -> Lst[str]: ...\n",
    )
    .unwrap();
    std::fs::write(ws.join("rerail.env"), "python.typeshed = ../ts\n").unwrap();
    std::fs::write(
        ws.join("main.py"),
        "from box import Lst, Set, names\nclass V:\n    def run(self) -> int: ...\ndef f(xs: Lst[V]) -> None:\n    for x in Set(xs):\n        x.run()\n    for s in Set(names()):\n        s.upper()\n",
    )
    .unwrap();
    let out = dir.join("out");
    let _no_session = corpus::SESSION_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    corpus::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(dir.join("store.sqlite")));
    let text = format!("{}\n{JEDI_SPANS}", program("calls_p3"));
    corpus::run(&text, &ws, None, &out, "program", Binding::Cpu, 1 << 18).unwrap();
    let read = |rel: &str| std::fs::read_to_string(out.join(format!("{rel}.csv"))).unwrap_or_default();
    let src = std::fs::read_to_string(ws.join("main.py")).unwrap();
    let starts: std::collections::HashMap<String, usize> =
        read("oracle_jedi_at").lines().map(|l| { let v: Vec<&str> = l.split('\t').collect(); (v[0].to_owned(), v[1].parse().unwrap()) }).collect();
    let at_line = |cell: &str| line(&src, starts[cell]);
    let targets = read("call_target");
    let to: Vec<u64> = targets.lines().filter_map(|l| l.split_once('\t')).map(|(c, _)| at_line(c)).collect();
    assert_eq!(to, [6], "V.run, through Set's _T bound by xs: {targets}");
    let open: Vec<u64> = read("call_open").lines().map(|l| at_line(l.split('\t').next().unwrap())).collect();
    assert!(!open.contains(&8), "s.upper() of a Set built from a Lst[str]: {}", read("call_open"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_analysis_phase_binds_a_library_context_managers_type_variable_from_the_wrapped_return() {
    // calls_p3 over a miniature contextlib: `cm(func: Callable[P, It[T]]) -> Callable[P, Cm[T]]`, and Cm's parameter
    // is its `Enter[T, bool | None]` base's (its first base, `Base[Gen[T, S]]`, nests a generic). `opened()` holds a
    // Cm of V (the workspace function returns `It[V]`), so `with opened() as v` binds v to a V and `v.run()` targets
    // V.run.
    let dir = crate::code_rule_dsl::prefilter::tests::temp().with_extension("p3-cm");
    let _ = std::fs::remove_dir_all(&dir);
    let (ws, lib) = (dir.join("ws"), dir.join("ts/stdlib"));
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::create_dir_all(&lib).unwrap();
    std::fs::write(lib.join("VERSIONS"), "builtins: 3.0-\ntyping: 3.0-\ncm: 3.0-\n").unwrap();
    std::fs::write(lib.join("builtins.pyi"), "class object: ...\nclass int: ...\nclass bool: ...\n").unwrap();
    std::fs::write(
        lib.join("typing.pyi"),
        "class _SpecialForm: ...\nGeneric: _SpecialForm\nProtocol: _SpecialForm\nCallable: _SpecialForm\ndef TypeVar(name: str) -> object: ...\ndef ParamSpec(name: str) -> object: ...\n_T = TypeVar(\"_T\")\nclass It(Protocol[_T]):\n    def __next__(self) -> _T: ...\n",
    )
    .unwrap();
    std::fs::write(
        lib.join("cm.pyi"),
        "from typing import Callable, Generic, ParamSpec, Protocol, TypeVar, It\n_T = TypeVar(\"_T\")\n_S = TypeVar(\"_S\")\n_E = TypeVar(\"_E\")\n_P = ParamSpec(\"_P\")\nclass Enter(Protocol[_T, _E]):\n    def __enter__(self) -> _T: ...\nclass Gen(It[_T], Generic[_T, _S]): ...\nclass Base(Generic[_T]): ...\nclass Cm(Base[Gen[_T, _S]], Enter[_T, bool | None]): ...\ndef cm(func: Callable[_P, It[_T]]) -> Callable[_P, Cm[_T]]: ...\n",
    )
    .unwrap();
    std::fs::write(ws.join("rerail.env"), "python.typeshed = ../ts\n").unwrap();
    std::fs::write(
        ws.join("main.py"),
        "from typing import It\nfrom cm import cm\nclass V:\n    def run(self) -> int: ...\n@cm\ndef opened() -> It[V]: ...\ndef f() -> None:\n    with opened() as v:\n        v.run()\n",
    )
    .unwrap();
    let out = dir.join("out");
    let _no_session = corpus::SESSION_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    corpus::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(dir.join("store.sqlite")));
    let text = format!("{}\n{JEDI_SPANS}", program("calls_p3"));
    corpus::run(&text, &ws, None, &out, "program", Binding::Cpu, 1 << 18).unwrap();
    let read = |rel: &str| std::fs::read_to_string(out.join(format!("{rel}.csv"))).unwrap_or_default();
    let src = std::fs::read_to_string(ws.join("main.py")).unwrap();
    let starts: std::collections::HashMap<String, usize> =
        read("oracle_jedi_at").lines().map(|l| { let v: Vec<&str> = l.split('\t').collect(); (v[0].to_owned(), v[1].parse().unwrap()) }).collect();
    let at_line = |cell: &str| line(&src, starts[cell]);
    let targets = read("call_target");
    let to: Vec<u64> = targets.lines().filter_map(|l| l.split_once('\t')).map(|(c, _)| at_line(c)).collect();
    assert!(to.contains(&9), "v.run() through Cm's _T bound by opened's It[V]: {targets}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_analysis_phase_rules_out_an_overload_whose_none_parameter_is_given_a_literal() {
    // calls_p3 over a miniature builtins: `setdefault(self, key, default: None = None) -> _VT | None` and
    // `setdefault(self, key, default: _VT) -> _VT` both fit two arguments, but `[]` is no None, so
    // `d.setdefault(k, []).append(v)` runs the second, holds a `list[V]`, and its `append` is list's, not open.
    let dir = crate::code_rule_dsl::prefilter::tests::temp().with_extension("p3-none");
    let _ = std::fs::remove_dir_all(&dir);
    let (ws, lib) = (dir.join("ws"), dir.join("ts/stdlib"));
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::create_dir_all(&lib).unwrap();
    std::fs::write(lib.join("VERSIONS"), "builtins: 3.0-\ntyping: 3.0-\n").unwrap();
    std::fs::write(
        lib.join("typing.pyi"),
        "class _SpecialForm: ...\nGeneric: _SpecialForm\ndef TypeVar(name: str) -> object: ...\ndef overload(f: object) -> object: ...\n",
    )
    .unwrap();
    std::fs::write(
        lib.join("builtins.pyi"),
        "from typing import Generic, TypeVar, overload\n_T = TypeVar(\"_T\")\n_KT = TypeVar(\"_KT\")\n_VT = TypeVar(\"_VT\")\nclass object: ...\nclass str: ...\nclass list(Generic[_T]):\n    def append(self, x: _T, /) -> None: ...\nclass dict(Generic[_KT, _VT]):\n    @overload\n    def setdefault(self, key: _KT, default: None = None, /) -> _T | None: ...\n    @overload\n    def setdefault(self, key: _KT, default: _VT, /) -> _VT: ...\n",
    )
    .unwrap();
    std::fs::write(ws.join("rerail.env"), "python.typeshed = ../ts\n").unwrap();
    std::fs::write(
        ws.join("main.py"),
        "class V: ...\ndef f(k: str, v: V) -> None:\n    d: dict[str, list[V]] = {}\n    d.setdefault(k, []).append(v)\n",
    )
    .unwrap();
    let out = dir.join("out");
    let _no_session = corpus::SESSION_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    corpus::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(dir.join("store.sqlite")));
    let text = format!("{}\n{JEDI_SPANS}", program("calls_p3"));
    corpus::run(&text, &ws, None, &out, "program", Binding::Cpu, 1 << 18).unwrap();
    let read = |rel: &str| std::fs::read_to_string(out.join(format!("{rel}.csv"))).unwrap_or_default();
    let src = std::fs::read_to_string(ws.join("main.py")).unwrap();
    let starts: std::collections::HashMap<String, usize> =
        read("oracle_jedi_at").lines().map(|l| { let v: Vec<&str> = l.split('\t').collect(); (v[0].to_owned(), v[1].parse().unwrap()) }).collect();
    let open: Vec<u64> = read("call_open").lines().map(|l| line(&src, starts[l.split('\t').next().unwrap()])).collect();
    assert!(!open.contains(&4), "setdefault(k, []) runs the overload with a default, append is list's: {}", read("call_open"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_analysis_phase_reads_a_library_generic_base_as_known() {
    // calls_p3 over a miniature typeshed: `class A(Cm[None])`, a library generic base written as a subscript, is a
    // known base, so `A()` runs A's own `__init__` and `A().go()` targets A.go; neither is open.
    let dir = crate::code_rule_dsl::prefilter::tests::temp().with_extension("p3-gbase");
    let _ = std::fs::remove_dir_all(&dir);
    let (ws, lib) = (dir.join("ws"), dir.join("ts/stdlib"));
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::create_dir_all(&lib).unwrap();
    std::fs::write(lib.join("VERSIONS"), "builtins: 3.0-\ntyping: 3.0-\nbox: 3.0-\n").unwrap();
    std::fs::write(lib.join("builtins.pyi"), "class object: ...\nclass int: ...\n").unwrap();
    std::fs::write(lib.join("typing.pyi"), "class _SpecialForm: ...\nGeneric: _SpecialForm\ndef TypeVar(name: str) -> object: ...\n").unwrap();
    std::fs::write(
        lib.join("box.pyi"),
        "from typing import Generic, TypeVar\n_T = TypeVar(\"_T\")\nclass Cm(Generic[_T]):\n    def __enter__(self) -> _T: ...\n",
    )
    .unwrap();
    std::fs::write(ws.join("rerail.env"), "python.typeshed = ../ts\n").unwrap();
    std::fs::write(
        ws.join("main.py"),
        "from box import Cm\nclass A(Cm[None]):\n    def __init__(self) -> None: ...\n    def go(self) -> int: ...\ndef f() -> None:\n    A().go()\n",
    )
    .unwrap();
    let out = dir.join("out");
    let _no_session = corpus::SESSION_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    corpus::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(dir.join("store.sqlite")));
    let text = format!("{}\n{JEDI_SPANS}", program("calls_p3"));
    corpus::run(&text, &ws, None, &out, "program", Binding::Cpu, 1 << 18).unwrap();
    let read = |rel: &str| std::fs::read_to_string(out.join(format!("{rel}.csv"))).unwrap_or_default();
    assert_eq!(read("call_open"), "", "A() and A().go() of a class with a library generic base");
    assert_eq!(read("call_target").lines().count(), 2, "{}", read("call_target"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_analysis_phase_gives_an_awaited_call_what_the_async_def_returns() {
    // calls_p3: `await load()` holds what `async def load() -> V` declares, `await make()` what make returns (a V),
    // so both `run()` calls target V.run; the calls of load and make, coroutines, are not awaited values.
    let dir = crate::code_rule_dsl::prefilter::tests::temp().with_extension("p3-await");
    let _ = std::fs::remove_dir_all(&dir);
    let (ws, lib) = (dir.join("ws"), dir.join("ts/stdlib"));
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::create_dir_all(&lib).unwrap();
    std::fs::write(lib.join("VERSIONS"), "builtins: 3.0-\n").unwrap();
    std::fs::write(lib.join("builtins.pyi"), "class object: ...\nclass int: ...\n").unwrap();
    std::fs::write(ws.join("rerail.env"), "python.typeshed = ../ts\n").unwrap();
    std::fs::write(
        ws.join("main.py"),
        "class V:\n    def run(self) -> int: ...\nasync def load() -> V: ...\nasync def make():\n    return V()\nasync def f() -> None:\n    v = await load()\n    v.run()\n    w = await make()\n    w.run()\n",
    )
    .unwrap();
    let out = dir.join("out");
    let _no_session = corpus::SESSION_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    corpus::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(dir.join("store.sqlite")));
    let text = format!("{}\n{JEDI_SPANS}", program("calls_p3"));
    corpus::run(&text, &ws, None, &out, "program", Binding::Cpu, 1 << 18).unwrap();
    let read = |rel: &str| std::fs::read_to_string(out.join(format!("{rel}.csv"))).unwrap_or_default();
    let src = std::fs::read_to_string(ws.join("main.py")).unwrap();
    let starts: std::collections::HashMap<String, usize> =
        read("oracle_jedi_at").lines().map(|l| { let v: Vec<&str> = l.split('\t').collect(); (v[0].to_owned(), v[1].parse().unwrap()) }).collect();
    let at_line = |cell: &str| line(&src, starts[cell]);
    let targets = read("call_target");
    let to: Vec<u64> = targets.lines().filter_map(|l| l.split_once('\t')).map(|(c, _)| at_line(c)).collect();
    assert!(to.contains(&8) && to.contains(&10), "v.run() and w.run() through the awaited values: {targets}");
    let open: Vec<u64> = read("call_open").lines().map(|l| at_line(l.split('\t').next().unwrap())).collect();
    assert!(!open.contains(&8) && !open.contains(&10), "{}", read("call_open"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_analysis_phase_awaits_an_async_loops_next_value_and_an_async_withs_entered_value() {
    // calls_p3 (t32 2.5): `async for x in s` binds x to the awaited `s.__aiter__().__anext__()`, `async with c as z` z
    // to the awaited `c.__aenter__()`: the workspace's `async def`s' results, and a stub's `async def __aenter__(self)
    // -> Self` the library instance itself. A plain `__anext__` that returns an awaitable leaves its target open.
    let dir = crate::code_rule_dsl::prefilter::tests::temp().with_extension("p3-async");
    let _ = std::fs::remove_dir_all(&dir);
    let (ws, lib) = (dir.join("ws"), dir.join("ts/stdlib"));
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::create_dir_all(&lib).unwrap();
    std::fs::write(lib.join("VERSIONS"), "builtins: 3.0-\nconn: 3.0-\ntyping: 3.0-\n").unwrap();
    std::fs::write(lib.join("builtins.pyi"), "class object: ...\nclass int: ...\n").unwrap();
    std::fs::write(lib.join("typing.pyi"), "class _SpecialForm: ...\nSelf: _SpecialForm\n").unwrap();
    std::fs::write(lib.join("conn.pyi"), "from typing import Self\nclass Conn:\n    async def __aenter__(self) -> Self: ...\n    def close(self) -> None: ...\n").unwrap();
    std::fs::write(ws.join("rerail.env"), "python.typeshed = ../ts\n").unwrap();
    std::fs::write(
        ws.join("main.py"),
        "from conn import Conn\nclass V:\n    def run(self) -> int: ...\nclass It:\n    async def __anext__(self) -> V: ...\nclass Seq:\n    def __aiter__(self) -> It: ...\nclass Cm:\n    async def __aenter__(self) -> V: ...\nclass Bare:\n    def __anext__(self): ...\nclass Raw:\n    def __aiter__(self) -> Bare: ...\nasync def f(s: Seq, c: Cm, k: Conn, r: Raw) -> None:\n    async for x in s:\n        x.run()\n    async with c as z:\n        z.run()\n    async with k as n:\n        n.close()\n    async for y in r:\n        y.run()\n",
    )
    .unwrap();
    let out = dir.join("out");
    let _no_session = corpus::SESSION_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    corpus::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(dir.join("store.sqlite")));
    let text = format!("{}\n{JEDI_SPANS}", program("calls_p3"));
    corpus::run(&text, &ws, None, &out, "program", Binding::Cpu, 1 << 18).unwrap();
    let read = |rel: &str| std::fs::read_to_string(out.join(format!("{rel}.csv"))).unwrap_or_default();
    let src = std::fs::read_to_string(ws.join("main.py")).unwrap();
    let starts: std::collections::HashMap<String, usize> =
        read("oracle_jedi_at").lines().map(|l| { let v: Vec<&str> = l.split('\t').collect(); (v[0].to_owned(), v[1].parse().unwrap()) }).collect();
    let at_line = |cell: &str| line(&src, starts[cell]);
    let to: Vec<u64> = read("call_target").lines().filter_map(|l| l.split_once('\t')).map(|(c, _)| at_line(c)).collect();
    let open: Vec<u64> = read("call_open").lines().map(|l| at_line(l.split('\t').next().unwrap())).collect();
    assert!(to.contains(&16) && to.contains(&18), "x.run() and z.run() through the awaited values: {}", read("call_target"));
    assert!(!open.contains(&16) && !open.contains(&18) && !open.contains(&20), "open at lines {open:?}");
    assert!(open.contains(&22), "y's `__anext__` returns no declared class: {}", read("call_open"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_session_maintains_the_typing_phase_from_struct_s_delta() {
    // P2 (types.rr) in a session: an edit to one workspace file changes struct's rows for that file only, and the
    // typing phase's run takes that delta: it maintains the strata the delta reaches, not every stratum, and its
    // exports are a full run's.
    use crate::code_rule_dsl::logic::plan::corpus::session;
    let dir = crate::code_rule_dsl::prefilter::tests::temp().with_extension("p2-session");
    let _ = std::fs::remove_dir_all(&dir);
    let (ws, lib) = (dir.join("ws"), dir.join("ts/stdlib"));
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::create_dir_all(&lib).unwrap();
    std::fs::write(lib.join("VERSIONS"), "builtins: 3.0-\ntyping: 3.0-\n").unwrap();
    std::fs::write(lib.join("builtins.pyi"), "class object: ...\nclass int: ...\nclass str:\n    def upper(self) -> str: ...\n").unwrap();
    std::fs::write(lib.join("typing.pyi"), "class _SpecialForm: ...\nGeneric: _SpecialForm\ndef TypeVar(name: str) -> object: ...\n").unwrap();
    std::fs::write(ws.join("rerail.env"), "python.typeshed = ../ts\n").unwrap();
    std::fs::write(ws.join("a.py"), "class A:\n    def run(self) -> int: ...\n").unwrap();
    std::fs::write(ws.join("b.py"), "from a import A\nclass B(A):\n    def go(self) -> str: ...\n").unwrap();
    let reader = r#"
.decl types__binds(m: symbol, x: symbol, d: symbol)
.decl types__anc(c: symbol, a: symbol)
.decl types__attr(c: symbol, x: symbol, d: symbol, k: symbol)
.decl types__rtype(f: symbol, a: symbol, d: symbol)
.decl binds(m: symbol, x: symbol, d: symbol)
binds(m, x, d) :- types__binds(m, x, d).
.decl anc(c: symbol, a: symbol)
anc(c, a) :- types__anc(c, a).
.decl attr(c: symbol, x: symbol, d: symbol, k: symbol)
attr(c, x, d, k) :- types__attr(c, x, d, k).
.decl rtype(f: symbol, a: symbol, d: symbol)
rtype(f, a, d) :- types__rtype(f, a, d).
.output binds, anc, attr, rtype
"#;
    let out = dir.join("out");
    let _one = corpus::SESSION_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    corpus::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(dir.join("store.sqlite")));
    let run = || corpus::run_with_options(&[reader], &ws, None, &out, "program", Binding::Cpu, 1 << 18, &corpus::Typing::default(), &serde_json::json!({})).unwrap();
    let dump = || -> Vec<Vec<String>> {
        ["binds", "anc", "attr", "rtype"].iter().map(|r| {
            let mut v: Vec<String> = std::fs::read_to_string(out.join(format!("{r}.csv"))).unwrap_or_default().lines().map(str::to_owned).collect();
            v.sort();
            v
        }).collect()
    };
    let types = |r: &corpus::Report| -> (Option<(usize, usize)>, (usize, usize, usize)) {
        let p = r.phase_runs.iter().find(|p| p.name == "types").expect("the typing phase ran");
        (p.report.cross_strata, p.report.cross_maintained)
    };
    session::start(&dir.join("session")).unwrap();
    let cold = run();
    println!("P2 cold: {:?}", types(&cold));
    std::fs::write(ws.join("b.py"), "from a import A\nclass B(A):\n    def go(self) -> str: ...\nclass C(B): ...\n").unwrap();
    let edited = run();
    let (strata, maintained) = types(&edited);
    println!("P2 edited: strata {strata:?}, maintained {maintained:?}");
    let kept = dump();
    assert!(kept[1].iter().any(|l| l == "b.C\ta.A"), "C's ancestors through the edit: {:?}", kept[1]);
    // A second edit, then none: the phase's run keeps its relations as the session keeps any run's, so the
    // unchanged run derives no stratum again.
    std::fs::write(ws.join("a.py"), "class A:\n    def run(self) -> int: ...\nclass Z: ...\n").unwrap();
    let _ = run();
    let second = dump();
    assert!(second[1].iter().any(|l| l == "b.C\ta.A") && second[0].iter().any(|l| l.starts_with("a\tZ\t")), "{second:?}");
    let unchanged = run();
    let how: Vec<String> = unchanged.phase_runs.iter().map(|p| format!("{} {:?} {:?}", p.name, p.report.cross_strata, p.report.phases.iter().map(|x| x.0).collect::<Vec<_>>())).collect();
    assert!(unchanged.phase_runs.iter().any(|p| p.name == "types" && p.report.cross_strata.is_none_or(|(ran, _)| ran == 0)), "the typing phase kept as it was: {how:?}");
    assert_eq!(dump(), second, "the unchanged run's exports");
    let full = session::full(run);
    assert_eq!(second, dump(), "the session's maintained typing phase against a full run");
    let _ = full;
    let (run_, all) = strata.expect("the typing phase's run is a session's");
    assert!(maintained.0 > 0, "strata maintained from struct's delta: {maintained:?}");
    assert!(run_ < all, "the edit runs the strata its delta reaches, {run_} of {all}");
    let types_local = edited.phase_runs.iter().find(|p| p.name == "types").unwrap().report.phases.iter().find(|p| p.0 == "local").map_or(0.0, |p| p.1 + p.2);
    assert!(types_local < 0.05, "a phase reading no document has no file-local part: {types_local:.2} s");
    session::stop();
    let _ = std::fs::remove_dir_all(&dir);
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
.decl oracle_jedi_cls(c: node, k: node)
oracle_jedi_cls(c, k) :- callee(c, e), pv(e, \"cls\", k).
.decl oracle_jedi_at(n: node, s: number, e: number)
oracle_jedi_at(n, s, e) :- kind(n, \"call\"), node(_, n, _, s, e).
oracle_jedi_at(f, s, e) :- kind(f, \"function_definition\"), node(_, f, _, s, e).
oracle_jedi_at(f, s, e) :- kind(f, \"class_definition\"), node(_, f, _, s, e).
oracle_jedi_at(f, s, e) :- kind(f, \"lambda\"), node(_, f, _, s, e).
.output oracle_jedi_cls, oracle_jedi_at
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
    // `RERAIL_PYTHON_REUSE=on`: phase 0's cutoff, as the command line's runs have it (`corpus::reuse`).
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
    let report = corpus::run_with_options(&[&text], &root, None, &out, "program", Binding::Cpu, 1 << 18, &corpus::Typing::default(), &serde_json::json!({ "env": settings, "reuse": std::env::var("RERAIL_PYTHON_REUSE").is_ok_and(|v| v == "on") })).unwrap();
    let cpu: f64 = report.phases.iter().map(|p| p.1 + p.2).sum();
    println!("cross-file memo: {} components found, {} derived", report.cross.0, report.cross.1);
    println!("phase 0: {}; {}", report.files_observed, report.phases.first().map_or("", |p| p.0));
    println!("calls.rr over {} files: rounds {:?}; {cpu:.1} s CPU (main thread), peak RSS {:.0} MB", report.files, report.rounds, report.phases.iter().map(|p| p.3).fold(0.0, f64::max));
    // The comparison below is the benchmark's, not the run's: its own span, for the memory sampler's peaks.
    let compare = crate::code_rule_dsl::logic::profile::span(|| "benchmark: compare with Jedi".to_owned());
    let rows = |rel: &str| -> Vec<Vec<String>> {
        std::fs::read_to_string(out.join(format!("{rel}.csv"))).unwrap().lines().map(|l| l.split('\t').map(str::to_owned).collect()).collect()
    };
    let at: BTreeMap<String, (usize, usize)> = rows("oracle_jedi_at").into_iter().map(|r| (r[0].clone(), (r[1].parse().unwrap(), r[2].parse().unwrap()))).collect();
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
    // The analysis phase (calls_p3) names no library node: a call that reaches a library's callable is `call_lib`.
    let lib_calls: BTreeSet<(String, usize, usize)> = if out.join("call_lib.csv").exists() { rows("call_lib").iter().map(|r| key(&r[0])).collect() } else { BTreeSet::new() };
    let mut named: BTreeMap<(String, usize, usize), BTreeSet<(String, u64)>> = BTreeMap::new();
    for r in rows("call_target") {
        let t = target(&r[1]);
        named.entry(key(&r[0])).or_default().insert(t);
    }
    let mut classes: BTreeMap<(String, usize, usize), BTreeSet<(String, u64)>> = BTreeMap::new();
    for r in rows("oracle_jedi_cls") {
        let t = target(&r[1]);
        classes.entry(key(&r[0])).or_default().insert(t);
    }
    // `RERAIL_PYTHON_CALLS_ALL=<file>`: every answer the program gives, `site<TAB>kind<TAB>target`, a site as
    // `path:line:column` and a target as `path:line` (or `<library>`), to compare two layouts of one corpus.
    if let Ok(all) = std::env::var("RERAIL_PYTHON_CALLS_ALL") {
        let mut site = |n: &str| -> String {
            let f = file(n);
            let s = at[n].0;
            let src = sources.entry(f.clone()).or_insert_with(|| std::fs::read_to_string(root.join(&f)).unwrap_or_default());
            let s = s.min(src.len());
            let column = s - src[..s].rfind('\n').map_or(0, |i| i + 1);
            format!("{f}:{}:{column}", line(src, s))
        };
        let mut lines: Vec<String> = Vec::new();
        for (kind, rel) in [("def", "call_target"), ("class", "oracle_jedi_cls")] {
            for r in rows(rel) {
                let s = site(&r[0]);
                let f = file(&r[1]);
                let t = if library(&f) { "<library>".to_owned() } else { site(&r[1]).rsplit_once(':').unwrap().0.to_owned() };
                lines.push(format!("{s}\t{kind}\t{t}"));
            }
        }
        for r in rows("call_open") {
            lines.push(format!("{}\topen\t-", site(&r[0])));
        }
        lines.sort();
        lines.dedup();
        std::fs::write(all, lines.join("\n") + "\n").unwrap();
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
                let reaches_library = named.get(&k).is_some_and(|d| d.contains(&lib)) || classes.get(&k).is_some_and(|c| c.contains(&lib)) || lib_calls.contains(&k);
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
    drop(compare);
    if crate::code_rule_dsl::logic::profile::on() {
        println!("== Memory, the process's at the end\n{}", crate::code_rule_dsl::logic::profile::peaks());
    }
}

/// A probe of the analysis phase: calls_p3 over `RERAIL_P3_PROBE_ROOT` (a small workspace, its `rerail.env` naming a
/// typeshed), with `RERAIL_P3_PROBE_RULES` appended (rules over P3's own relations, each `.output` it adds printed).
/// For asking why one call is open without the corpus's run.
#[test]
#[ignore = "a probe: set RERAIL_P3_PROBE_ROOT and RERAIL_P3_PROBE_RULES"]
fn probe_the_analysis_phase() {
    let root = PathBuf::from(std::env::var("RERAIL_P3_PROBE_ROOT").unwrap());
    let rules = std::env::var("RERAIL_P3_PROBE_RULES").unwrap_or_default();
    let out = crate::code_rule_dsl::prefilter::tests::temp().with_extension("p3-probe");
    let text = format!("{}\n{JEDI_SPANS}\n{rules}\n", program("calls_p3"));
    corpus::run(&text, &root, None, &out, "program", Binding::Cpu, 1 << 18).unwrap();
    let asked: Vec<String> = rules.lines().filter_map(|l| l.trim().strip_prefix(".output ")).flat_map(|l| l.split(',').map(|r| r.trim().to_owned())).collect();
    for rel in ["call_target", "call_open", "call_lib"].into_iter().map(str::to_owned).chain(asked) {
        println!("== {rel}\n{}", std::fs::read_to_string(out.join(format!("{rel}.csv"))).unwrap_or_default());
    }
    let _ = std::fs::remove_dir_all(&out);
}

/// t26's first experiment: which rows of `calls_cx`'s run its outputs read. The liveness rules
/// (`logic::liveness`) are added to the program and run with it over `RERAIL_PYTHON_CALLS_ROOT` with
/// `RERAIL_PYTHON_TYPESHED`. Printed per relation, most dead rows first: rows, live, keys (first column) and live
/// keys, and the rows about loaded documents (typeshed's stubs) and the live ones among them.
#[test]
#[ignore = "a spike: minutes over the 84-file corpus with typeshed"]
fn spike_liveness_of_the_calls_program() {
    use crate::code_rule_dsl::logic::liveness;
    let root = PathBuf::from(std::env::var("RERAIL_PYTHON_CALLS_ROOT").unwrap());
    let settings: Vec<String> = std::env::var("RERAIL_PYTHON_TYPESHED").map(|t| vec![format!("python.typeshed={t}")]).unwrap_or_default();
    let text = format!("{}\n{JEDI_SPANS}", program(&calls_name()));
    let prog = crate::code_rule_dsl::logic::rr::parse(&text).unwrap();
    let live = liveness::program(&prog);
    println!("liveness: {} rules over {} relations", live.rules.len(), live.decls.len());
    let text = format!("{text}\n{}", crate::code_rule_dsl::logic::rr::print(&live).unwrap());
    let out = crate::code_rule_dsl::prefilter::tests::temp().with_extension("liveness");
    let report = corpus::run_with_options(&[&text], &root, None, &out, "program", Binding::Cpu, 1 << 18, &corpus::Typing::default(), &serde_json::json!({ "env": settings })).unwrap();
    let cpu: f64 = report.phases.iter().map(|p| p.1 + p.2).sum();
    println!("{} files: {cpu:.1} s CPU (main thread), peak RSS {:.0} MB", report.files, report.phases.iter().map(|p| p.3).fold(0.0, f64::max));
    let counts = liveness::read_counts(&std::fs::read_to_string(out.join(format!("{}.csv", liveness::COUNTS))).unwrap());
    let _ = std::fs::remove_dir_all(&out);
    let get = |m: &BTreeMap<String, u64>, w: &str| m.get(w).copied().unwrap_or(0);
    let mut rows: Vec<(&String, &BTreeMap<String, u64>)> = counts.iter().collect();
    rows.sort_by_key(|(r, m)| (std::cmp::Reverse(get(m, "rows") - get(m, "live").min(get(m, "rows"))), (*r).clone()));
    println!("{:<28} {:>9} {:>9} {:>6} {:>9} {:>9} {:>9} {:>9}", "relation", "rows", "live", "live%", "keys", "live keys", "library", "lib live");
    let mut total = [0u64; 6];
    for (rel, m) in &rows {
        let v: Vec<u64> = liveness::MEASURES.iter().map(|w| get(m, w)).collect();
        for (t, x) in total.iter_mut().zip(&v) {
            *t += x;
        }
        if v[0] == 0 {
            continue;
        }
        println!("{:<28} {:>9} {:>9} {:>5.1}% {:>9} {:>9} {:>9} {:>9}", rel, v[0], v[1], 100.0 * v[1] as f64 / v[0] as f64, v[2], v[3], v[4], v[5]);
    }
    println!("{:<28} {:>9} {:>9} {:>5.1}% {:>9} {:>9} {:>9} {:>9}", "total", total[0], total[1], 100.0 * total[1] as f64 / total[0].max(1) as f64, total[2], total[3], total[4], total[5]);
}

/// Spike for decision 0083 §1–3: P1, `struct.rr`, as a phase in its own fact space over the workspace and the library
/// modules its imports reach, to its fixed point; a reader takes its exports. Cold, unchanged, then each edit of
/// `RERAIL_SPIKE_EDITS` (E1 a function appended, E2 a comment at the top, E3 a line in a function's body): the
/// phase's CPU, its export rows changed, and freshness (definitions changed, granules changed, the frontier, changes
/// unattributed). `RERAIL_PYTHON_CALLS_ROOT` names the corpus, `RERAIL_PYTHON_TYPESHED` typeshed.
#[test]
#[ignore = "a spike: minutes over the 84-file corpus with typeshed"]
fn spike_phases_struct_over_the_corpus() {
    let root = std::path::PathBuf::from(std::env::var("RERAIL_PYTHON_CALLS_ROOT").expect("RERAIL_PYTHON_CALLS_ROOT"));
    let settings: Vec<String> = std::env::var("RERAIL_PYTHON_TYPESHED").map(|t| vec![format!("python.typeshed={t}")]).unwrap_or_default();
    let out = std::env::temp_dir().join(format!("rerail-phases-{}", std::process::id()));
    // The reader hands every export on as an output, so a maintained run and a full one compare row for row.
    let reader = r#"
.decl struct__unit(s: symbol, h: symbol)
.decl struct__def(s: symbol, k: symbol)
.decl struct__binds(m: symbol, x: symbol, how: symbol, t: symbol)
.decl struct__base(c: symbol, i: number, b: symbol)
.decl struct__member(c: symbol, x: symbol, t: symbol)
.decl struct__param(f: symbol, i: number, x: symbol, a: symbol)
.decl struct__ret(f: symbol, a: symbol)
.decl struct__module_file(m: symbol, p: symbol)
.decl struct__at(s: symbol, p: symbol, l: number)
.decl units(s: symbol, h: symbol)
units(s, h) :- struct__unit(s, h).
.decl modules(m: symbol, p: symbol)
modules(m, p) :- struct__module_file(m, p).
.decl defs(s: symbol, k: symbol)
defs(s, k) :- struct__def(s, k).
.decl imports(m: symbol, x: symbol, t: symbol)
imports(m, x, t) :- struct__binds(m, x, "import", t).
.decl bindings(m: symbol, x: symbol, how: symbol, t: symbol)
bindings(m, x, h, t) :- struct__binds(m, x, h, t).
.decl bases(c: symbol, i: number, b: symbol)
bases(c, i, b) :- struct__base(c, i, b).
.decl members(c: symbol, x: symbol, t: symbol)
members(c, x, t) :- struct__member(c, x, t).
.decl params(f: symbol, i: number, x: symbol, a: symbol)
params(f, i, x, a) :- struct__param(f, i, x, a).
.decl rets(f: symbol, a: symbol)
rets(f, a) :- struct__ret(f, a).
.decl ats(s: symbol, p: symbol, l: number)
ats(s, p, l) :- struct__at(s, p, l).
.output units, modules, defs, imports, bindings, bases, members, params, rets, ats
"#;
    let text = crate::code_rule_dsl::logic::rr::print(&crate::code_rule_dsl::logic::rr::parse(reader).unwrap()).unwrap();
    let run = || corpus::run_with_options(&[&text], &root, None, &out, "program", Binding::Cpu, 1 << 18, &corpus::Typing::default(), &serde_json::json!({ "env": settings, "reuse": std::env::var("RERAIL_PYTHON_REUSE").is_ok_and(|v| v == "on") })).unwrap();
    let count = |rel: &str| std::fs::read_to_string(out.join(format!("{rel}.csv"))).map_or(0, |t| t.lines().count());
    let show = |what: &str, r: &corpus::Report| {
        let p = &r.phase_runs[0];
        let cpu: f64 = p.report.phases.iter().map(|x| x.1 + x.2).sum();
        let rss = p.report.phases.iter().map(|x| x.3).fold(0.0, f64::max);
        println!("PHASE {what}: struct {cpu:.1} s CPU (main thread), peak RSS {rss:.0} MB, {} workspace files; modules {}, definitions {}, imports {}", p.report.files, count("modules"), count("defs"), count("imports"));
        let rows: Vec<String> = p.exports.iter().map(|(rel, n)| format!("{rel} {n}")).collect();
        println!("PHASE {what}: exports: {}; strata {:?}, maintained {:?}", rows.join(", "), p.report.cross_strata, p.report.cross_maintained);
        let parts: Vec<String> = p.report.phases.iter().filter(|x| x.1 + x.2 >= 0.01).map(|x| format!("{} {:.2}", x.0, x.1 + x.2)).collect();
        println!("PHASE {what}: parts: {}", parts.join(", "));
        let outer: Vec<String> = r.phases.iter().filter(|x| x.1 + x.2 >= 0.01).map(|x| format!("{} {:.2}", x.0, x.1 + x.2)).collect();
        println!("PHASE {what}: the reader's run (phase included): {:.2} s: {}", r.phases.iter().map(|x| x.1 + x.2).sum::<f64>(), outer.join(", "));
    };
    let outputs = ["units", "modules", "defs", "imports", "bindings", "bases", "members", "params", "rets", "ats"];
    let dump = || -> BTreeMap<&str, Vec<String>> {
        outputs.iter().map(|rel| {
            let mut lines: Vec<String> = std::fs::read_to_string(out.join(format!("{rel}.csv"))).unwrap_or_default().lines().map(str::to_owned).collect();
            lines.sort();
            (*rel, lines)
        }).collect()
    };
    // The full run each maintained one is checked against: a store of its own, its phase store removed first.
    let kept_store = std::env::var("RERAIL_TAPE_CACHE").expect("RERAIL_TAPE_CACHE: the spike keeps phases in a store");
    let full_store = format!("{kept_store}.full");
    let full = || {
        let _ = std::fs::remove_file(format!("{full_store}.phases"));
        // SAFETY: the spike's thread is the only one running between runs.
        unsafe { std::env::set_var("RERAIL_TAPE_CACHE", &full_store) };
        let r = crate::code_rule_dsl::logic::plan::corpus::session::full(&run);
        unsafe { std::env::set_var("RERAIL_TAPE_CACHE", &kept_store) };
        r
    };
    // `RERAIL_SPIKE_SESSION`: the runs are a session's (decision 0063), as the explorer's are; the full runs are not.
    let session = std::env::var_os("RERAIL_SPIKE_SESSION").is_some();
    if session {
        crate::code_rule_dsl::logic::plan::corpus::session::start(&out.with_extension("session")).unwrap();
    }
    show("cold", &run());
    show("unchanged", &run());
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
        show(edit, &run());
        let kept = dump();
        show(&format!("{edit} full"), &full());
        let derived = dump();
        let differ: Vec<String> = outputs.iter().filter(|r| kept[*r] != derived[*r]).map(|r| format!("{r} ({} kept, {} derived)", kept[r].len(), derived[r].len())).collect();
        println!("PHASE {edit}: maintained vs full: {}", if differ.is_empty() { "the same".to_owned() } else { format!("differ: {}", differ.join(", ")) });
    }
    if session {
        crate::code_rule_dsl::logic::plan::corpus::session::stop();
        let _ = std::fs::remove_dir_all(out.with_extension("session"));
    }
    let _ = std::fs::remove_dir_all(&out);
}

/// t28 step 7 (decision 0083 §2): the typing phase (types.rr) in a session over `RERAIL_PYTHON_CALLS_ROOT`, copied,
/// edited a file at a time (`RERAIL_SPIKE_EDITS`, as O125's). Each edit runs the reader of every types export in the
/// session, then in full; the exports must be equal. Prints each phase's CPU, its strata run and maintained, and where
/// its CPU went.
#[test]
#[ignore = "a spike: minutes over the 84-file corpus with typeshed"]
fn spike_t28_a_session_maintains_the_typing_phase() {
    use crate::code_rule_dsl::logic::plan::corpus::session;
    let given = PathBuf::from(std::env::var("RERAIL_PYTHON_CALLS_ROOT").expect("RERAIL_PYTHON_CALLS_ROOT"));
    let settings: Vec<String> = std::env::var("RERAIL_PYTHON_TYPESHED").map(|t| vec![format!("python.typeshed={t}")]).unwrap_or_default();
    let base = std::env::temp_dir().join(format!("rerail-t28-{}", std::process::id()));
    let (root, out) = (base.join("src"), base.join("out"));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    assert!(std::process::Command::new("cp").arg("-R").arg(&given).arg(&root).status().unwrap().success());
    // The reader: every export of types.rr handed on as an output.
    let types = crate::code_rule_dsl::logic::rr::parse(&program("types")).unwrap();
    let exports = crate::code_rule_dsl::logic::plan::corpus::phases::declared(&types).unwrap().exports;
    let mut reader = String::new();
    for e in &exports {
        let cols = &types.decls[e];
        let decl = cols.iter().map(|(c, t)| format!("{c}: {t}")).collect::<Vec<_>>().join(", ");
        let vars = (0..cols.len()).map(|i| format!("v{i}")).collect::<Vec<_>>().join(", ");
        reader.push_str(&format!(".decl types__{e}({decl})\n.decl out_{e}({decl})\nout_{e}({vars}) :- types__{e}({vars}).\n.output out_{e}\n"));
    }
    let run = || corpus::run_with_options(&[&reader], &root, None, &out, "program", Binding::Cpu, 1 << 18, &corpus::Typing::default(), &serde_json::json!({ "env": settings })).unwrap();
    let dump = || -> BTreeMap<String, Vec<String>> {
        exports.iter().map(|e| {
            let mut v: Vec<String> = std::fs::read_to_string(out.join(format!("out_{e}.csv"))).unwrap_or_default().lines().map(str::to_owned).collect();
            v.sort();
            (e.clone(), v)
        }).collect()
    };
    let show = |what: &str, r: &corpus::Report| {
        for p in &r.phase_runs {
            let cpu: f64 = p.report.phases.iter().map(|x| x.1 + x.2).sum();
            let parts: Vec<String> = p.report.phases.iter().filter(|x| x.1 + x.2 >= 0.01).map(|x| format!("{} {:.2}", x.0, x.1 + x.2)).collect();
            let rows: usize = p.exports.iter().map(|e| e.1).sum();
            println!("T28 {what}: {} {cpu:.2} s CPU; strata {:?} (a new process's {:?}), maintained {:?}; export rows {rows}; parts: {}",
                p.name, p.report.cross_strata, p.report.cross_kept, p.report.cross_maintained, parts.join(", "));
        }
        println!("T28 {what}: the reader's run: {:.2} s", r.phases.iter().map(|x| x.1 + x.2).sum::<f64>());
    };
    // `RERAIL_SPIKE_PROCESS`: no session; each run maintains from what the store kept, as a new process does
    // (t32 4.1; the process's own caches of programs and files stay warm).
    let in_session = std::env::var_os("RERAIL_SPIKE_PROCESS").is_none();
    if in_session {
        session::start(&base.join("session")).unwrap();
    }
    show("cold", &run());
    show("unchanged", &run());
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
        show(edit, &run());
        let kept = dump();
        // The full run: no session, a phase store of its own.
        let store = std::env::var("RERAIL_TAPE_CACHE").expect("RERAIL_TAPE_CACHE");
        let full_store = format!("{store}.full");
        // SAFETY: the spike's thread is the only one running between runs.
        unsafe { std::env::set_var("RERAIL_TAPE_CACHE", &full_store) };
        // Its cross-file part derived, not maintained from what its store kept (t32 4.1).
        unsafe { std::env::set_var("RERAIL_KEPT_PART", "off") };
        let full = session::full(run);
        unsafe { std::env::remove_var("RERAIL_KEPT_PART") };
        unsafe { std::env::set_var("RERAIL_TAPE_CACHE", &store) };
        show(&format!("{edit} full"), &full);
        let derived = dump();
        let differ: Vec<String> = exports.iter().filter(|e| kept[*e] != derived[*e]).map(|e| format!("{e} ({} kept, {} derived)", kept[e].len(), derived[e].len())).collect();
        println!("T28 {edit}: maintained vs full: {}", if differ.is_empty() { "the same".to_owned() } else { format!("differ: {}", differ.join(", ")) });
    }
    if in_session {
        session::stop();
    }
    let _ = std::fs::remove_dir_all(&base);
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
    let run = || corpus::run_with_options(&[&text], &root, None, &out, "program", Binding::Cpu, 1 << 18, &corpus::Typing::default(), &serde_json::json!({ "env": settings, "reuse": std::env::var("RERAIL_PYTHON_REUSE").is_ok_and(|v| v == "on") })).unwrap();
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

/// Spike O125 (decision 0083 §2): `calls.rr`'s cross-file part kept by a session and maintained from deltas
/// (decision 0077's delete-and-rederive) over `RERAIL_PYTHON_CALLS_ROOT`, copied, edited a file at a time. Each run's
/// outputs must be a full run's; prints each run's strata run and maintained and its CPU against the full run's.
#[test]
#[ignore = "calls.rr in a session over a Python corpus, edited (minutes)"]
fn spike_o125_session_maintains_calls() {
    use crate::code_rule_dsl::logic::plan::corpus::session;
    let given = PathBuf::from(std::env::var("RERAIL_PYTHON_CALLS_ROOT").unwrap());
    let base = std::env::temp_dir().join(format!("rerail-o125-{}", std::process::id()));
    let (root, out) = (base.join("src"), base.join("out"));
    std::fs::create_dir_all(&base).unwrap();
    let copy = std::process::Command::new("cp").arg("-R").arg(&given).arg(&root).status().unwrap();
    assert!(copy.success());
    let text = format!("{}\n{JEDI_SPANS}", program("calls"));
    let run = || corpus::run_with_options(&[&text], &root, None, &out, "program", Binding::Cpu, 1 << 18, &corpus::Typing::default(), &serde_json::json!({ "env": [] })).unwrap();
    let rels = ["call_target", "call_open", "type_of", "possible_type", "finding"];
    let outputs = || -> Vec<Vec<String>> {
        rels.iter().map(|r| {
            let mut v: Vec<String> = std::fs::read_to_string(out.join(format!("{r}.csv"))).unwrap_or_default().lines().map(str::to_owned).collect();
            v.sort();
            v
        }).collect()
    };
    let cpu = |r: &corpus::Report| -> f64 { r.phases.iter().map(|p| p.1 + p.2).sum() };
    session::start(&base.join("session")).unwrap();
    let t = std::time::Instant::now();
    let first = run();
    println!("o125 cold session run: {:.1} s CPU (main), {:.1} s wall, strata {:?}", cpu(&first), t.elapsed().as_secs_f64(), first.cross_strata);
    println!("o125 rows {:?}", outputs().iter().map(Vec::len).collect::<Vec<_>>());
    let mut files: Vec<PathBuf> = walk(&root);
    files.sort();
    let pick = |i: usize| files[i * files.len() / 4].clone();
    let exceptions = root.join("l1100_exceptions/exceptions.py");
    let read = |p: &Path| std::fs::read_to_string(p).unwrap();
    let (e0, a0, b0) = (read(&exceptions), read(&pick(1)), read(&pick(2)));
    let edits: Vec<(String, PathBuf, String)> = vec![
        ("comment".into(), exceptions.clone(), format!("{e0}# an edit to a comment\n")),
        ("comment reverted".into(), exceptions.clone(), e0.clone()),
        ("a function added".into(), pick(1), format!("{a0}\n\ndef o125_added(x):\n    return len(x)\n")),
        ("an import added".into(), pick(2), format!("import os\n{b0}")),
        ("a body changed".into(), pick(1), a0.replacen("return", "return None or", 1)),
        ("all reverted".into(), pick(2), b0.clone()),
    ];
    // `O125_EDITS`: which edits, by index (each costs a session run and a full one).
    let wanted: Vec<usize> = std::env::var("O125_EDITS").map(|v| v.split(',').map(|i| i.parse().unwrap()).collect()).unwrap_or_else(|_| (0..edits.len()).collect());
    for (i, (what, path, text)) in edits.into_iter().enumerate() {
        if !wanted.contains(&i) {
            continue;
        }
        std::fs::write(&path, text).unwrap();
        if what == "all reverted" {
            std::fs::write(pick(1), &a0).unwrap();
        }
        let t = std::time::Instant::now();
        let r = run();
        let (wall, got) = (t.elapsed().as_secs_f64(), outputs());
        // `O125_NO_FULL`: skip the full run (its profile would replace the session run's).
        if std::env::var_os("O125_NO_FULL").is_some() {
            println!("o125 {what}: session {:.1} s CPU, {wall:.1} s wall; strata run {:?}, maintained {:?}; derived again {:?}; rows overdeleted, inserted, held {:?}", cpu(&r), r.cross_strata, r.cross_maintained, r.cross_again, r.cross_delta_counts);
            continue;
        }
        let f = session::full(run);
        let same = got == outputs();
        println!(
            "o125 {what} ({}): session {:.1} s CPU (main), {wall:.1} s wall; full {:.1} s CPU; strata run {:?}, maintained (all, recursive) {:?}; files evaluated {:?}; same as full: {same}",
            path.strip_prefix(&root).unwrap().display(), cpu(&r), cpu(&f), r.cross_strata, r.cross_maintained, r.memo
        );
        let ph = |r: &corpus::Report| r.phases.iter().filter(|p| p.1 + p.2 > 0.3).map(|p| format!("{} {:.1}", p.0, p.1 + p.2)).collect::<Vec<_>>().join(", ");
        println!("  maintained rows: over-deleted/inserted/held/input-deltas/truly-deleted/truly-inserted {:?}", r.cross_delta_counts);
        println!("  phases: session [{}]; full [{}]", ph(&r), ph(&f));
        println!("  inputs changed ({}): {:?}; derived again: {:?}", r.cross_changed.len(), r.cross_changed.iter().take(30).collect::<Vec<_>>(), r.cross_again);
        if !same {
            let want = outputs();
            for (i, rel) in rels.iter().enumerate() {
                let (a, b): (BTreeSet<&String>, BTreeSet<&String>) = (got[i].iter().collect(), want[i].iter().collect());
                let (extra, missing) = (a.difference(&b).count(), b.difference(&a).count());
                if extra + missing > 0 {
                    println!("  {rel}: {extra} extra, {missing} missing; e.g. {:?} / {:?}", a.difference(&b).next(), b.difference(&a).next());
                }
            }
        }
    }
    session::stop();
    let _ = std::fs::remove_dir_all(&base);
}

/// Spike O125 over real history: `O125_REPLAY` holds `00` (the files before the oldest commit) and `NN` (the files commit
/// NN changed, with `.deleted`, written by the spike's `replay.sh`). `O125_STEPS=a-b` applies `01..a-1`, runs cold in a
/// session, then applies `a..b` one commit at a time; each step's outputs are compared with a full run's at `b`, or at
/// every step with `O125_FULL_EACH`. With `RERAIL_PROFILE`, each run's `native.tsv` is kept as
/// `<profile>/native-<step>-<cold|session|full>.tsv`. `RERAIL_PYTHON_TYPESHED` loads the standard library's stubs.
#[test]
#[ignore = "calls.rr in a session over real commits (minutes)"]
fn spike_o125_replay() {
    use crate::code_rule_dsl::logic::plan::corpus::session;
    let replay = PathBuf::from(std::env::var("O125_REPLAY").unwrap());
    let (a, b): (usize, usize) = {
        let v = std::env::var("O125_STEPS").unwrap();
        let (a, b) = v.split_once('-').unwrap();
        (a.parse().unwrap(), b.parse().unwrap())
    };
    let each = std::env::var_os("O125_FULL_EACH").is_some();
    let profile = std::env::var_os("RERAIL_PROFILE").map(PathBuf::from);
    let settings: Vec<String> = std::env::var("RERAIL_PYTHON_TYPESHED").map(|t| vec![format!("python.typeshed={t}")]).unwrap_or_default();
    let base = std::env::temp_dir().join(format!("rerail-o125-{}", std::process::id()));
    let (root, out) = (base.join("src"), base.join("out"));
    std::fs::create_dir_all(&base).unwrap();
    let overlay = |step: usize| -> usize {
        let dir = replay.join(format!("{step:02}"));
        let mut n = 0;
        for f in walk(&dir) {
            let to = root.join(f.strip_prefix(&dir).unwrap());
            std::fs::create_dir_all(to.parent().unwrap()).unwrap();
            std::fs::copy(&f, &to).unwrap();
            n += 1;
        }
        for gone in std::fs::read_to_string(dir.join(".deleted")).unwrap_or_default().lines().filter(|l| !l.is_empty()) {
            let _ = std::fs::remove_file(root.join(gone));
            n += 1;
        }
        n
    };
    for step in 0..a {
        overlay(step);
    }
    let text = format!("{}\n{JEDI_SPANS}", program("calls"));
    let run = || corpus::run_with_options(&[&text], &root, None, &out, "program", Binding::Cpu, 1 << 18, &corpus::Typing::default(), &serde_json::json!({ "env": settings, "reuse": std::env::var("RERAIL_PYTHON_REUSE").is_ok_and(|v| v == "on") })).unwrap();
    let rels = ["call_target", "call_open", "type_of", "possible_type"];
    let outputs = || -> Vec<Vec<String>> {
        rels.iter().map(|r| {
            let mut v: Vec<String> = std::fs::read_to_string(out.join(format!("{r}.csv"))).unwrap_or_default().lines().map(str::to_owned).collect();
            v.sort();
            v
        }).collect()
    };
    let keep = |step: usize, what: &str| {
        if let Some(p) = &profile {
            let _ = std::fs::copy(p.join("native.tsv"), p.join(format!("native-{step:02}-{what}.tsv")));
            let _ = std::fs::copy(p.join("spans.tsv"), p.join(format!("spans-{step:02}-{what}.tsv")));
            let _ = std::fs::copy(p.join("profile.txt"), p.join(format!("profile-{step:02}-{what}.txt")));
        }
    };
    let phase = |r: &corpus::Report, name: &str| -> f64 { r.phases.iter().filter(|p| p.0 == name).map(|p| p.1 + p.2).sum() };
    let cpu = |r: &corpus::Report| -> f64 { r.phases.iter().map(|p| p.1 + p.2).sum() };
    // `O125_SPANS`: each step's session spans, and its costliest span names summed over every stratum.
    let spans_on = std::env::var_os("O125_SPANS").is_some();
    if spans_on {
        crate::code_rule_dsl::logic::profile::set(true);
    }
    // `O125_DECISIONS=<topic text>`: each step's decisions under topics holding the text, with how often.
    let decided = std::cell::RefCell::new(crate::code_rule_dsl::logic::profile::decisions());
    let show_spans = |step: usize| {
        if !spans_on {
            return;
        }
        if let Ok(topic) = std::env::var("O125_DECISIONS") {
            let now = crate::code_rule_dsl::logic::profile::decisions_since(&decided.borrow());
            for ((t, text), n) in now.iter().filter(|((t, _), _)| t.contains(topic.as_str())) {
                println!("o125d {step:02} {n:>7} {t}: {text}");
            }
            *decided.borrow_mut() = crate::code_rule_dsl::logic::profile::decisions();
        }
        let spans = crate::code_rule_dsl::logic::profile::spans();
        // `O125_SPANS_TSV=<dir>`: the step's whole session tree, `<dir>/spans-<step>.tsv` (path, calls, CPU s).
        if let Some(dir) = std::env::var_os("O125_SPANS_TSV") {
            let rows: String = spans.iter().filter(|(p, _)| p.contains("session/") || std::env::var_os("O125_SPANS_ALL").is_some()).map(|(p, t)| format!("{p}\t{}\t{:.4}\n", t.calls, t.total)).collect();
            let _ = std::fs::create_dir_all(&dir);
            let _ = std::fs::write(PathBuf::from(dir).join(format!("spans-{step:02}.tsv")), rows);
        }
        let mut by: BTreeMap<String, f64> = BTreeMap::new();
        for (path, t) in spans.iter() {
            let Some((_, rest)) = path.split_once("session/") else { continue };
            if !rest.contains('/') {
                println!("o125s {step:02}   {:>7.3} s  session/{rest}", t.total);
            }
            let last = rest.rsplit('/').next().unwrap_or(rest);
            let name = if last.starts_with("stratum ") { "stratum (all)".to_owned() } else if last.starts_with("plan #") { "plan # (all rules)".to_owned() } else if last.starts_with("cold rule") { "cold rule (all)".to_owned() } else { last.to_owned() };
            *by.entry(name).or_default() += t.total;
        }
        let mut v: Vec<_> = by.into_iter().collect();
        v.sort_by(|a, b| b.1.total_cmp(&a.1));
        for (name, t) in v.into_iter().take(14) {
            println!("o125s {step:02}   {t:>7.3} s  (sum) {name}");
        }
    };
    session::start(&base.join("session")).unwrap();
    // `O125_DERIVATIONS=<dir>` (with `RERAIL_SPIKE_DERIVATIONS`): each run's rows derived and rows new, by relation,
    // `<dir>/<step>-<run>.tsv`: the derivations a counting scheme would keep a count of.
    let derivations = |step: usize, what: &str| {
        let taken = std::mem::take(&mut *crate::code_rule_dsl::logic::native::DERIVATIONS.lock().unwrap());
        if let Some(dir) = std::env::var_os("O125_DERIVATIONS").map(PathBuf::from) {
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join(format!("{step:02}-{what}.tsv")), taken.iter().map(|(r, (d, n))| format!("{r}\t{d}\t{n}\n")).collect::<String>()).unwrap();
        }
    };
    derivations(0, "before");
    let cold = run();
    derivations(a - 1, "cold");
    keep(a - 1, "cold");
    println!("o125r cold at {:02}: {:.1} s CPU, cross {:.1} s; rows {:?}", a - 1, cpu(&cold), phase(&cold, "cross"), outputs().iter().map(Vec::len).collect::<Vec<_>>());
    // `O125_FULL_ONLY`: each step a full run alone (to profile it apart from the session's runs).
    let full_only = std::env::var_os("O125_FULL_ONLY").is_some();
    for step in a..=b {
        let files = overlay(step);
        if full_only {
            let f = session::full(run);
            println!("o125r {step:02} ({files} files): full {:.1} s CPU, cross {:.1} s, local {:.1} s", cpu(&f), phase(&f, "cross"), phase(&f, "local"));
            continue;
        }
        let r = run();
        derivations(step, "session");
        show_spans(step);
        keep(step, "session");
        let got = outputs();
        let mut line = format!(
            "o125r {step:02} ({files} files): session {:.1} s CPU, cross {:.1} s, local {:.1} s; strata {:?}, maintained {:?}, again {:?}; rows over-deleted/inserted/held/input-deltas/truly-deleted/truly-inserted {:?}; inputs changed {}",
            cpu(&r), phase(&r, "cross"), phase(&r, "local"), r.cross_strata, r.cross_maintained, r.cross_again, r.cross_delta_counts, r.cross_changed.len()
        );
        // `O125_SAVE`: each step's outputs, `<dir>/NN.txt`, to compare two modes' runs without a full run each.
        if let Some(dir) = std::env::var_os("O125_SAVE").map(PathBuf::from) {
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join(format!("{step:02}.txt")), got.iter().map(|v| v.join("\n")).collect::<Vec<_>>().join("\n--\n")).unwrap();
        }
        if std::env::var_os("O125_NO_FULL").is_none() && (each || step == b) {
            let f = session::full(run);
            derivations(step, "full");
            keep(step, "full");
            let want = outputs();
            line += &format!("; full {:.1} s CPU, cross {:.1} s; same as full: {}", cpu(&f), phase(&f, "cross"), got == want);
            for (i, rel) in rels.iter().enumerate() {
                let (x, y): (BTreeSet<&String>, BTreeSet<&String>) = (got[i].iter().collect(), want[i].iter().collect());
                let (extra, missing) = (x.difference(&y).count(), y.difference(&x).count());
                if extra + missing > 0 {
                    line += &format!("\n  {rel}: {extra} extra, {missing} missing; e.g. {:?} / {:?}", x.difference(&y).next(), y.difference(&x).next());
                }
            }
        }
        println!("{line}");
    }
    session::stop();
    let _ = std::fs::remove_dir_all(&base);
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() { out.extend(walk(&p)); } else if p.extension().is_some_and(|x| x == "py") { out.push(p); }
    }
    out
}
