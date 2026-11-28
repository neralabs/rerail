//! Callable per-file relations (the "Callable Per-file Relations" page's spike): the split's programs, the calls'
//! answers against the relation made whole, batching by file, the memo, and what is refused at load.

use super::{CallCounts, Calls, FileState};
use crate::code_rule_dsl::facts::vtab::Source;
use crate::code_rule_dsl::logic::front::{self, Value};
use crate::code_rule_dsl::logic::native::component::Asked;
use crate::code_rule_dsl::logic::plan::corpus::preparation::callable::{self, Split};
use crate::code_rule_dsl::logic::plan::{Stats, body, lower, run_atoms_native_answering, run_atoms_native_over};
use crate::code_rule_dsl::logic::program_image::fields::FieldsMut as _;
use crate::code_rule_dsl::logic::program_image::{Builder, Version};
use crate::code_rule_dsl::logic::rr::parse_version;
use rerail_blocks::vector::Vector;
use rusqlite::Connection;
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

/// Two functions that touch a file-scope name in common, compared only where they run in one block.
const THREADS: &str = ".decl touches(p: symbol, f: symbol, v: symbol)\n.input touches\n\
    .decl runs_in(b: symbol, p: symbol, f: symbol)\n\
    .decl same_globals(p: symbol, f: symbol, g: symbol, v: symbol)\n.callable same_globals bbbf\n\
    same_globals(p, f, g, v) :- touches(p, f, v), touches(p, g, v), f != g.\n\
    .decl unsafe_pair(b: symbol, f: symbol, g: symbol, v: symbol)\n.output unsafe_pair\n\
    unsafe_pair(b, f, g, v) :- runs_in(b, p, f), runs_in(b, p, g), same_globals(p, f, g, v).\n";

fn s(x: &str) -> Value {
    Value::Str(x.into())
}

/// Three files of `n` functions each: an even one touches `x`, each its own `y<i>`. Blocks: two in a.py (f0 with
/// f2; f0 with f4), one in b.py (f1 with f3, which share nothing); none in c.py.
fn corpus(n: usize) -> (Vec<Vec<Value>>, Vec<Vec<Value>>) {
    let mut touches = Vec::new();
    for p in ["a.py", "b.py", "c.py"] {
        for i in 0..n {
            if i % 2 == 0 {
                touches.push(vec![s(p), s(&format!("f{i}")), s("x")]);
            }
            touches.push(vec![s(p), s(&format!("f{i}")), s(&format!("y{i}"))]);
        }
    }
    let runs = [("blk1", "a.py", "f0"), ("blk1", "a.py", "f2"), ("blk2", "a.py", "f0"), ("blk2", "a.py", "f4"), ("blk3", "b.py", "f1"), ("blk3", "b.py", "f3")];
    (touches, runs.iter().map(|(b, p, f)| vec![s(b), s(p), s(f)]).collect())
}

fn list(rows: &[Vec<Value>]) -> Vec<front::List<Value>> {
    rows.iter().map(|r| front::List::from(r.clone())).collect()
}

fn lowered(v: &Arc<Version>) -> lower::Lowered {
    lower::Lowered::of_placed(body::Placed::whole(v.clone()), lower::domains_of(v).unwrap(), [0; 3], crate::code_rule_dsl::language::GrammarDialect::Python)
}

/// `v` with facts stated for `rel`.
fn with_facts(v: &Arc<Version>, facts: &[(&str, &[Vec<Value>])]) -> Arc<Version> {
    let mut b = Builder::on(v);
    for (rel, rows) in facts {
        b.fields().set_facts(rel, &list(rows));
    }
    b.finish().unwrap().0
}

/// The files' rows of `touches`, each read where it lies: by its path's atom.
struct Files(HashMap<i64, Arc<dyn Source>>, std::cell::RefCell<Vec<i64>>);

impl FileState for Files {
    fn holder(&self, at: i64) -> Option<u64> {
        self.0.contains_key(&at).then_some(at as u64)
    }
    fn rows(&self, holder: u64, rels: &[String]) -> crate::code_rule_dsl::logic::plan::Result<Vec<(String, Arc<dyn Source>)>> {
        assert_eq!(rels, ["touches"], "a call reads its file's touches alone");
        self.1.borrow_mut().push(holder as i64);
        Ok(vec![("touches".to_owned(), Arc::clone(&self.0[&(holder as i64)]))])
    }
}

fn files(store: &crate::content_store::ContentStore, touches: &[Vec<Value>]) -> Files {
    let mut by: HashMap<i64, Arc<dyn Source>> = HashMap::new();
    for p in ["a.py", "b.py", "c.py"] {
        let rows: Vec<Vec<Value>> = touches.iter().filter(|r| r[0] == s(p)).cloned().collect();
        let source = super::super::atoms_source(store, 3, rows.iter()).unwrap();
        by.insert(source.value(0, 0), source);
    }
    Files(by, Default::default())
}

/// The program run whole, and run with its callable called: each run's `unsafe_pair`, the calls' counts, the files
/// entered in order, and the split.
fn both(n: usize) -> (BTreeSet<Vec<String>>, BTreeSet<Vec<String>>, CallCounts, Vec<i64>, Split, usize) {
    let (touches, runs) = corpus(n);
    let v = parse_version(THREADS).unwrap_or_else(|e| panic!("{e}"));
    let path = crate::code_rule_dsl::fixtures::temp();
    let store = crate::content_store::ContentStore::open(&path).unwrap();
    let db = Connection::open(&path).unwrap();

    // Whole: same_globals for every pair of a file's functions.
    let whole_v = with_facts(&v, &[("runs_in", &runs)]);
    let whole_l = lowered(&whole_v);
    let state = files(&store, &touches);
    let all_touches = super::super::atoms_source(&store, 3, touches.iter()).unwrap();
    let mut whole_stats = Stats::empty_of_lowered(&whole_l).unwrap();
    super::measured(&mut whole_stats, "touches", all_touches.as_ref());
    crate::code_rule_dsl::facts::vtab::Modules::default().bind(&db, "touches", all_touches).unwrap();
    let whole = run_atoms_native_over(&whole_l, &db, "main", whole_stats, BTreeSet::from(["same_globals".to_owned()]), "program").unwrap();
    let made_whole = crate::code_rule_dsl::logic::native::component::derived(&db, "same_globals").map_or(0, |h| h.len());
    let whole_rows: BTreeSet<Vec<String>> = whole.outputs["unsafe_pair"].shown().into_iter().collect();

    // Called.
    let split = callable::split(&v).unwrap().expect("a callable");
    let caller = with_facts(&split.caller, &[("runs_in", &runs)]);
    let caller_l = lowered(&caller);
    let mut calls = Calls::of(&split, Connection::open(&path).unwrap(), "program").unwrap();
    let mut answer = |rel: &str, rows: &Asked| calls.answer(rel, rows, &state);
    let db2 = Connection::open(&path).unwrap();
    let called = run_atoms_native_answering(&caller_l, &db2, "main", Stats::empty_of_lowered(&caller_l).unwrap(), BTreeSet::new(), "program", &mut answer, None, None, None).unwrap();
    let called_rows: BTreeSet<Vec<String>> = called.outputs["unsafe_pair"].shown().into_iter().collect();
    let entered = state.1.borrow().clone();
    drop(answer);
    (whole_rows, called_rows, calls.counts.clone(), entered, split, made_whole)
}

/// A callable relation is never made whole: the caller has no rule for it, its rows are only those the readers'
/// asks answer, and the findings are the whole program's.
#[test]
fn a_callable_relation_is_answered_in_its_readers_join() {
    let (whole, called, counts, _, split, made_whole) = both(30);
    assert_eq!(called, whole, "the same findings");
    assert_eq!(whole.len(), 4, "f0 and f2, f0 and f4, each way: {whole:?}");
    // 15 even functions in each of 3 files: 15 × 14 × 3 pairs touching x.
    assert_eq!(made_whole, 630, "made whole, every pair of every file");
    assert_eq!(counts.answers, 4, "called, the asked pairs that share a name: {counts:?}");
    assert!(!split.caller.rules().any(|r| split.caller.rel_name(r.head().rel()) == "same_globals"), "the caller makes no same_globals");
}

/// Calls are batched: each file is entered once a batch, whatever its requests; a file nothing asks is not entered.
#[test]
fn a_callable_enters_each_file_once_a_batch() {
    let (_, _, counts, entered, _, _) = both(30);
    // Each block asks its 4 ordered pairs, a function with itself included (the request leaves `f != g` to the
    // callee): 12, of which (a.py, f0, f0) twice, by blk1 and blk2. All in one batch.
    assert_eq!(counts.asked, 11, "{counts:?}");
    assert_eq!(counts.batches, 1, "{counts:?}");
    let distinct: BTreeSet<i64> = entered.iter().copied().collect();
    assert_eq!(distinct.len(), 2, "a.py and b.py, not c.py: {entered:?}");
    assert_eq!(entered.len(), counts.batches * 2, "once a batch each: {entered:?}, {counts:?}");
    assert_eq!(counts.entered, entered.len());
    assert_eq!(counts.files, 2, "{counts:?}");
}

/// The memo holds what was asked, its keys alone: a batch asked again enters no file and adds no rows, for its
/// answers already lie in the caller's R (decision 0353).
#[test]
fn a_callables_memo_holds_what_was_asked() {
    let (touches, _) = corpus(6);
    let v = parse_version(THREADS).unwrap();
    let path = crate::code_rule_dsl::fixtures::temp();
    let store = crate::content_store::ContentStore::open(&path).unwrap();
    let state = files(&store, &touches);
    let split = callable::split(&v).unwrap().unwrap();
    let mut calls = Calls::of(&split, Connection::open(&path).unwrap(), "program").unwrap();
    // (a.py, f0, f2) and (a.py, f2, f4): ids by the store's atoms.
    let ids = store.intern_atoms(&["a.py", "f0", "f2", "f4"]).unwrap();
    let id = |k: usize| i64::from(ids[k]);
    let rows: Vec<Vector<i64>> = vec![[id(0), id(1), id(2)].into_iter().collect(), [id(0), id(2), id(3)].into_iter().collect()];
    let first = calls.answer("same_globals__call", &Asked::rows(rows.clone()), &state).unwrap();
    assert_eq!(calls.counts.entered, 1);
    assert_eq!(calls.asked(), 2, "the memo holds the two asked");
    assert_eq!(first[0].1.len(), 2, "both pairs share x");
    let again = calls.answer("same_globals__call", &Asked::rows(rows), &state).unwrap();
    assert_eq!(calls.counts.entered, 1, "asked again, no file entered: {:?}", calls.counts);
    assert_eq!(calls.counts.memo_hits, 2);
    assert!(again[0].1.is_empty(), "nothing added: the answers are the caller's already");
}

/// The callee's only rules are the callable's: a call writes nothing of its file's. Its inputs are its request and
/// the relations the callable reads.
#[test]
fn a_call_leaves_its_files_state_as_it_was() {
    let v = parse_version(THREADS).unwrap();
    let split = callable::split(&v).unwrap().unwrap();
    let callee = &split.callees[0];
    let heads: BTreeSet<&str> = callee.program.rules().map(|r| callee.program.rel_name(r.head().rel())).collect();
    assert_eq!(heads, BTreeSet::from(["same_globals"]));
    let inputs: Vec<&str> = callee.program.inputs().map(|a| callee.program.text(a)).collect();
    assert_eq!(inputs, ["same_globals__call", "touches"]);
    assert!(callee.program.rules().all(|r| callee.program.show_rule(r).contains("same_globals__call(")), "each rule reads its request first");
    let asks: Vec<String> = split.caller.rules().filter(|r| split.caller.rel_name(r.head().rel()) == "same_globals__call").map(|r| split.caller.show_rule(r)).collect();
    assert_eq!(asks.len(), 1, "one request rule, from unsafe_pair: {asks:?}");
    assert!(asks[0].contains("runs_in(") && !asks[0].contains("same_globals("), "{asks:?}");
}

fn refused(src: &str) -> String {
    let v = parse_version(src).unwrap_or_else(|e| panic!("{e}"));
    match callable::split(&v) {
        Err(e) => e,
        Ok(_) => panic!("not refused:\n{src}"),
    }
}

/// A callable answering a node is refused, naming its column: a call's answers are values (decision 0677).
#[test]
fn a_callable_answering_a_node_is_refused() {
    let e = refused(".decl k(n: node, p: symbol)\n.input k\n.decl r(p: symbol, n: node)\n.callable r bf\nr(p, n) :- k(n, p).\n.decl o(p: symbol)\n.output o\no(p) :- k(_, p), r(p, _).\n");
    assert!(e.contains("column `n` of r is a node"), "{e}");
}

/// A reader that does not bind a bound column before its call is refused, naming the rule and the pattern.
#[test]
fn a_call_with_its_pattern_unbound_is_refused() {
    let src = THREADS.replace("unsafe_pair(b, f, g, v) :- runs_in(b, p, f), runs_in(b, p, g), same_globals(p, f, g, v).", "unsafe_pair(b, f, g, v) :- runs_in(b, _, f), runs_in(b, _, g), same_globals(p, f, g, v).");
    let e = refused(&src);
    assert!(e.contains("column 0 of callable same_globals is bound in its pattern bbbf"), "{e}");
    assert!(e.contains("unsafe_pair("), "names the rule: {e}");
}

/// A callable whose rules read what calls answer is refused: its file's state would not stand while it answers.
#[test]
fn a_callable_reading_an_incomplete_cross_result_is_refused() {
    let src = THREADS.replace("same_globals(p, f, g, v) :- touches(p, f, v), touches(p, g, v), f != g.", "same_globals(p, f, g, v) :- touches(p, f, v), touches(p, g, v), f != g, !unsafe_pair(_, f, g, v).");
    let e = refused(&src);
    assert!(e.contains("made from calls' answers"), "{e}");
}

/// The directive's own lint: its pattern's length and letters, and column 0 bound.
#[test]
fn a_callables_pattern_is_linted() {
    for (pattern, says) in [("bbb", "has 4 columns"), ("bbxf", "has 4 columns"), ("fbbf", "column 0 names the file")] {
        let e = refused(&THREADS.replace(".callable same_globals bbbf", &format!(".callable same_globals {pattern}")));
        assert!(e.contains(says), "{pattern}: {e}");
    }
}
