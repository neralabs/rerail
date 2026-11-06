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
    let src = format!("{program}\n{SPANS}");
    let report = corpus::run(&src, project, None, &out, "program", Binding::Cpu, 1 << 18).unwrap();
    println!("cross-file rounds {:?}", report.rounds);
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

fn program(name: &str) -> String {
    std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("src/code_rule_dsl/typing/programs/{name}.rr"))).unwrap()
}

/// Where `calls.rr` answers otherwise than CPython ran, and why (2026-11-06: 83 agree, these 10 depart, none
/// wrong). The native resolver departs at 17 calls on the same oracle; the program resolves `super()`, a local
/// holding an instance, a call's result and an instance's `__call__`, which the native resolver leaves unknown.
const PYTHON_DEPARTURES: &[(&str, Departure, &str)] = &[
    (
        "main.py:10 `pkg.exported_helper(2)`",
        Departure::NoEdge,
        "O41: the re-export needs a third file crossing (main, pkg/__init__, pkg/util); at k = 2 the rounds stop \
         unconverged and the missing row is no edge, not unknown. With k = 6 they converge in 5 rounds and it agrees",
    ),
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
    let (agree, departures, never_ran) = score_python(&program("calls"));
    println!("calls.rr: {agree} agree, {} depart", departures.len());
    assert!(never_ran.is_empty(), "{never_ran:#?}");
    let listed: BTreeMap<String, Departure> = PYTHON_DEPARTURES.iter().map(|(at, d, _)| ((*at).to_owned(), *d)).collect();
    assert_eq!(departures, listed, "calls where calls.rr answers otherwise than CPython ran");
    assert!(agree >= 83, "{agree} agree");
}
