//! A group's first by an order (t50 step 2, decision 0515): the rules' `X, !later` self-join on one side, W's loop
//! that keeps the first of each group (`Graph::lookup` over the order) on the other.
use super::*;
use crate::code_rule_dsl::logic::rr::lower::first::{First, Order, recognize};
use crate::content_store::ContentStore;

fn statements(src: &str) -> Vec<rerail_rr_syntax::syntax::Statement> {
    rerail_rr_syntax::syntax::Parser::new(src).and_then(|mut p| p.statements()).unwrap_or_else(|e| panic!("{e:?}"))
}

const SHORTLEX: &str = "
.decl x(c: symbol, f: symbol, r: symbol)
.input x
.decl first(c: symbol, f: symbol, r: symbol)
.output first
first(c, f, r) :- x(c, f, r), !later(c, f, r).
.decl later(c: symbol, f: symbol, r: symbol)
later(c, f, r) :- x(c, f, r), x(c, f, s), @strlen(s) < @strlen(r).
later(c, f, r) :- x(c, f, r), x(c, f, s), @strlen(s) = @strlen(r), s < r.
";

/// references.rerail's shortlex shape (`rf_method_first`), pytest-collect's max with a literal on the keys
/// (`pyc_own`), and a min written the other way round are each the first of their group.
#[test]
fn a_rows_no_other_row_precedes_is_recognized_with_its_order() {
    assert_eq!(recognize(&statements(SHORTLEX)), vec![First { first: "first".into(), later: "later".into(), of: "x".into(), order: Order::Shortlex, drop_later: true }]);
    let max = ".decl m(q: number, x: symbol, f: number)\n.input m\n.decl t(x: symbol)\n.input t\n.decl own(q: number, x: symbol, f: number)\n.output own\nown(q, x, f) :- m(q, x, f), t(x), !later(q, x, f).\n.decl later(q: number, x: symbol, f: number)\nlater(q, x, f) :- m(q, x, f), m(q, x, g), g > f.\n";
    assert_eq!(recognize(&statements(max)).iter().map(|f| f.order).collect::<Vec<_>>(), vec![Order::Max]);
    let min = max.replace("g > f", "f > g");
    assert_eq!(recognize(&statements(&min)).iter().map(|f| f.order).collect::<Vec<_>>(), vec![Order::Min]);
    // A column X has besides the keys and the value, read as `_` in every use (`rf_field_first`).
    let wild = SHORTLEX.replace(".decl x(c: symbol, f: symbol, r: symbol)", ".decl x(c: symbol, f: symbol, r: symbol, t: symbol)").replace("x(c, f, r)", "x(c, f, r, _)").replace("x(c, f, s)", "x(c, f, s, _)");
    assert_eq!(recognize(&statements(&wild)).iter().map(|f| f.order).collect::<Vec<_>>(), vec![Order::Shortlex], "{wild}");
    // Not when the wildcards differ between X's uses.
    let apart = wild.replacen("x(c, f, s, _), @strlen(s) < @strlen(r)", "x(c, _, s, f), @strlen(s) < @strlen(r)", 1);
    assert!(recognize(&statements(&apart)).is_empty(), "{apart}");
}

/// Shapes that are not the first of a group are left as written: a literal reading the value, a later-rule over
/// another relation, half the shortlex order, symbols under `<` (text order, which no lattice keeps), and a
/// later-relation something else reads (recognized, kept).
#[test]
fn other_shapes_are_left_as_written() {
    for (what, from, to) in [
        ("a literal reads the value", "!later(c, f, r).", "!later(c, f, r), r != \"\"."),
        ("another relation", "x(c, f, s), @strlen(s) = @strlen(r)", "y(c, f, s), @strlen(s) = @strlen(r)"),
        ("half the order", "later(c, f, r) :- x(c, f, r), x(c, f, s), @strlen(s) = @strlen(r), s < r.\n", ""),
        ("text order", "later(c, f, r) :- x(c, f, r), x(c, f, s), @strlen(s) < @strlen(r).\n", ""),
    ] {
        let src = format!("{}\n.decl y(c: symbol, f: symbol, r: symbol)\n.input y\n", SHORTLEX.replace(from, to));
        assert!(recognize(&statements(&src)).is_empty(), "{what}");
    }
    let read = format!("{SHORTLEX}.decl seen(c: symbol)\n.output seen\nseen(c) :- later(c, _, _).\n");
    assert_eq!(recognize(&statements(&read)).iter().map(|f| f.drop_later).collect::<Vec<_>>(), vec![false]);
}

/// The rows: on a group of 10⁴ routes and 300 smaller groups, the native engine's `first` is each group's shortlex
/// least, as W's loop finds it, and the later-relation is gone from the program.
#[test]
fn the_first_of_each_group_is_ws_loops_on_a_large_group() {
    let mut rows: Vec<(String, String, String)> = Vec::new();
    for i in 0..10_000u64 {
        let r = format!("{:0w$}.", (i * 7919) % 100_003, w = 1 + (i % 5) as usize);
        rows.push(("Big".into(), "m".into(), r));
    }
    for g in 0..300u64 {
        for j in 0..(1 + g % 7) {
            rows.push((format!("C{g}"), format!("f{}", j % 2), format!("{}.", (g * 31 + j * 17) % (10 + j * 1000))));
        }
    }
    let mut src = SHORTLEX.replace(".input x\n", "");
    for (c, f, r) in &rows {
        src.push_str(&format!("x(\"{c}\", \"{f}\", \"{r}\").\n"));
    }
    let mut want: BTreeMap<(String, String), String> = BTreeMap::new();
    for (c, f, r) in &rows {
        let e = want.entry((c.clone(), f.clone())).or_insert_with(|| r.clone());
        if (r.len(), r.as_bytes()) < (e.len(), e.as_bytes()) {
            *e = r.clone();
        }
    }
    let want: Vec<Vec<String>> = want.into_iter().map(|((c, f), r)| vec![c, f, r]).collect();

    let front = crate::code_rule_dsl::logic::rr::parse(&src).unwrap_or_else(|e| panic!("{e}"));
    assert!(!front.rules.iter().any(|r| r.head.rel == "later"), "the self-join is not made");
    assert_eq!(front.lattice.get("first").and_then(|l| l.merge.as_deref()), Some("shortlex_min"), "first keeps a group's least");
    let p = crate::code_rule_dsl::logic::rr::parse_version(&src).unwrap_or_else(|e| panic!("{e}"));
    let path = crate::code_rule_dsl::fixtures::temp();
    let store = ContentStore::open(&path).unwrap();
    let db = Connection::open(&path).unwrap();
    super::super::native::register(&db).unwrap();
    super::super::native::register_symbols(&db).unwrap();
    // `shortlex_min<>` orders the symbols by their text: the store's atoms, as a run declares it (`corpus`). More
    // than it holds in memory: 10⁴ routes, read from the store (`ShortlexMin::lub`).
    super::super::native::declare("shortlex_min", Arc::new(super::super::native::ShortlexMin(store.atoms_handle())));
    let python = crate::code_rule_dsl::language::dialect_by_label("python").unwrap();
    let lowered = lower::lowered(p, python, &mut |s| store.intern_atoms(s).map_err(|e| Error::Plan(e.to_string()))).unwrap();
    // The native engine, then SQLite (the fallback every stratum has).
    for modes in ["program", ""] {
        let got = run_atoms_native_over(&lowered, &db, "main", Stats::empty_of_lowered(&lowered).unwrap(), BTreeSet::new(), modes).unwrap_or_else(|e| panic!("{modes:?}: {e}"));
        let mut rows = got.outputs.get("first").map(|r| r.shown()).unwrap_or_default();
        rows.sort();
        assert_eq!(rows.len(), want.len(), "{modes:?}: one row a group");
        assert_eq!(rows, want, "{modes:?}");
        if modes == "program" {
            assert!(got.native.iter().any(|k| k.split('/').any(|r| r == "first")), "first ran native: {:?}", got.native);
        }
    }
}
