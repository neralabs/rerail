//! The rebuild's tests (decision 0686): at M0, the pieces that are already the final shape. Each test deleted with
//! the first build comes back at the milestone the rebuild plan names for it.

use crate::sym::Symbols;
use crate::tape::{Names, TapeView, View};
use crate::{L0, k0, kform::RELATIONS, text};
use rerail_blocks::bistate::{ColumnIndex, Read};
use rerail_blocks::table::Table;
use std::collections::BTreeSet;

type Rows = BTreeSet<Vec<i64>>;

/// Column `col` of each row of `t` that agrees with `key` on `cols`, found by the view's probe.
fn probe(t: &TapeView<'_>, cols: &[u16], key: &[i64], col: u16) -> Vec<i64> {
    let cols: Vec<ColumnIndex> = cols.iter().map(|&c| ColumnIndex::new(c)).collect();
    let mut out = Vec::new();
    Table::probe(t, &cols, key, &mut |id| {
        out.push(Read::get(t, id, ColumnIndex::new(col)));
        true
    });
    out
}

/// The views over a three-line `.rerail` tape, read where the tape lies: the joins three rules would make of them,
/// walked by the views' probes (a bound node read where it lies, a bound parent's children walked).
#[test]
fn views_over_a_three_line_tape() {
    let sym = Symbols::new();
    let names = Names::new(&sym);
    let t = text("a(1, x).\n// a comment\nb(2) :- a(2, _).\nc(\"x\", 3).\n");
    let (path, module) = (sym.intern("three.rerail"), sym.intern("three"));
    let v = |view| TapeView::new(view, 1, path, module, &t, &sym, &names);
    let (node, kind, field, child, txt, value, doc) = (
        v(View::Node),
        v(View::Kind),
        v(View::Field),
        v(View::Child),
        v(View::Text),
        v(View::Value),
        v(View::Doc),
    );
    let s = |x: &str| sym.intern(x);
    let clauses = probe(&kind, &[0, 2], &[1, s("clause")], 1);
    assert_eq!(clauses.len(), 3);
    // name(d, n) :- kind(d, c, "clause"), field(d, c, "head", h), field(d, h, "relation", r), text(d, r, n).
    let mut heads = BTreeSet::new();
    // second(d, a) :- kind(d, c, "clause"), field(d, c, "head", h), field(d, h, "arguments", l), child(d, l, a, 1).
    let mut second = 0;
    for &c in &clauses {
        for h in probe(&field, &[0, 1, 2], &[1, c, s("head")], 3) {
            for r in probe(&field, &[0, 1, 2], &[1, h, s("relation")], 3) {
                for n in probe(&txt, &[0, 1], &[1, r], 2) {
                    heads.insert(sym.name(n));
                }
            }
            for l in probe(&field, &[0, 1, 2], &[1, h, s("arguments")], 3) {
                second += probe(&child, &[0, 1, 3], &[1, l, 1], 2).len();
            }
        }
    }
    assert_eq!(heads, ["a", "b", "c"].map(String::from).into());
    // `child` counts named children that are no comment: the second argument of `a` and of `c`.
    assert_eq!(second, 2);
    // num(d, v) :- kind(d, t, "number"), value(d, t, v).
    let nums: BTreeSet<i64> = probe(&kind, &[0, 2], &[1, s("number")], 1)
        .into_iter()
        .flat_map(|n| probe(&value, &[0, 1], &[1, n], 2))
        .collect();
    assert_eq!(nums, [1, 2, 3].into());
    assert_eq!(probe(&doc, &[0], &[1], 1), vec![path]);
    assert_eq!(probe(&doc, &[0, 1], &[1, path], 2), vec![module]);
    // node(d, n, k, start, end): each clause's kind and its span, the clause's own text.
    for &c in &clauses {
        assert_eq!(probe(&node, &[0, 1], &[1, c], 2), vec![s("clause")]);
        let (start, end) = (probe(&node, &[0, 1], &[1, c], 3)[0], probe(&node, &[0, 1], &[1, c], 4)[0]);
        assert_eq!(probe(&txt, &[0, 1], &[1, c], 2), vec![s(&t.src[start as usize..end as usize])]);
    }
    assert_eq!(probe(&node, &[0, 2], &[1, s("clause")], 1), clauses, "node's kind column answers as kind's");
    // Another document's key finds nothing here.
    assert!(probe(&kind, &[0], &[2], 1).is_empty());
}

/// The K0 pass's rows for a five-line file, against rows written by hand (node ids read off the tape's dump).
#[test]
fn k0_pass_rows_by_hand() {
    let sym = Symbols::new();
    let src = "e(1, 2).\n\
               e(2, 3).\n\
               // a comment\n\
               p(x, z) :- e(x, y), e(y, z), !q(y), x != z.\n\
               q(\"s\") :- e(_, 3).\n";
    let t = text(src);
    let rows = k0::rows(&t, 7, &sym).expect("K0");
    let r = crate::tape::Reads::of(&t);
    let mut dump = String::new();
    for i in 0..r.len() {
        let (k, f, x) = r.node(i);
        dump += &format!(
            "{i}: {} {} {x} {:?}\n",
            rerail_rr_syntax::tape::KIND_NAMES
                .get(k as usize)
                .unwrap_or(&"?"),
            rerail_rr_syntax::tape::FIELD_NAMES[f as usize],
            r.text(i)
        );
    }
    let s = |x: &str| sym.intern(x);
    let got = |i: usize| -> Rows { rows[i].iter().map(|r| r.to_vec()).collect() };
    let want: [Rows; 7] = HAND(&s);
    for (i, w) in want.iter().enumerate() {
        assert_eq!(&got(i), w, "{}\n{dump}", RELATIONS[i].0);
    }
    assert!(got(7).is_empty(), "k_order is boot's, not the pass's");
}

/// K-form's rows stored as traces of their shapes (`kform::shape`): 32-bit columns but a number's value, each row read
/// back as written.
#[test]
fn k_form_rows_are_stored_in_32_bits() {
    use crate::dir::Directory;
    use crate::op::{Registry, Tiling};
    use crate::trace::{Trace, TraceOps};
    use rerail_blocks::layer::{ColType, RunsW};
    use rerail_blocks::region::pin_block;
    let sym = Symbols::new();
    let t = text("e(1, 2).\np(x, z) :- e(x, y), e(y, z), !q(y), x != z.\nq(\"s\") :- e(_, -3).\n");
    let rows = k0::rows(&t, 7, &sym).expect("K0");
    let mut reg = Registry::default();
    let ops = TraceOps::declare(&mut reg).unwrap();
    let mut dir = Directory::default();
    for (i, (name, arity)) in RELATIONS.iter().enumerate() {
        let shape = crate::kform::shape(i);
        let wide: Vec<usize> = (0..*arity).filter(|&c| shape.columns[c] == ColType::I64).collect();
        assert_eq!(wide, if *name == "k_num" { vec![2] } else { vec![] }, "{name}: only a number's value is 64 bits");
        let mut tr = Trace::new(&shape).unwrap();
        let want: Rows = rows[i].iter().map(|r| r.to_vec()).collect();
        let runs = tr.open(&mut dir, want.len().max(1));
        {
            let b = dir.block(runs).unwrap();
            let pin = pin_block(&b, "k0", true).unwrap();
            let w = RunsW::new(&pin);
            for r in &want {
                w.push(r, 1).unwrap();
            }
        }
        let h = tr.commit(&reg, &ops, &mut dir, runs, Tiling::WHOLE).unwrap();
        let l = tr.read(&dir, &[Some(h)]).unwrap();
        let table = l.table();
        let st = tr.state(&table, Some(h)).unwrap();
        let got: Rows = st.scan().map(|id| ColumnIndex::all(*arity).map(|c| Read::get(&st, id, c)).collect()).collect();
        assert_eq!(got, want, "{name}");
    }
}

/// The hand-written rows of `k0_pass_rows_by_hand`'s file, module 7.
#[allow(non_snake_case)]
fn HAND(s: &dyn Fn(&str) -> i64) -> [Rows; 7] {
    let m = 7;
    let (atom, not, cmp) = (s("atom"), s("not"), s("cmp"));
    [
        // k_rule
        [
            vec![m, 1, s("e")],
            vec![m, 7, s("e")],
            vec![m, 14, s("p")],
            vec![m, 40, s("q")],
        ]
        .into(),
        // k_step
        [
            vec![m, 14, 0, -1, atom, s("e")],
            vec![m, 14, 1, -1, atom, s("e")],
            vec![m, 14, 2, -1, not, s("q")],
            vec![m, 14, 3, -1, cmp, s("!=")],
            vec![m, 40, 0, -1, atom, s("e")],
        ]
        .into(),
        // k_arg
        [
            vec![m, 1, -1, -1, 0, 5],
            vec![m, 1, -1, -1, 1, 6],
            vec![m, 7, -1, -1, 0, 11],
            vec![m, 7, -1, -1, 1, 12],
            vec![m, 14, -1, -1, 0, 18],
            vec![m, 14, -1, -1, 1, 19],
            vec![m, 14, 0, -1, 0, 24],
            vec![m, 14, 0, -1, 1, 25],
            vec![m, 14, 1, -1, 0, 29],
            vec![m, 14, 1, -1, 1, 30],
            vec![m, 14, 2, -1, 0, 35],
            vec![m, 14, 3, -1, 0, 37],
            vec![m, 14, 3, -1, 1, 39],
            vec![m, 40, -1, -1, 0, 44],
            vec![m, 40, 0, -1, 0, 49],
            vec![m, 40, 0, -1, 1, 50],
        ]
        .into(),
        // k_var
        [
            vec![m, 18, s("x")],
            vec![m, 19, s("z")],
            vec![m, 24, s("x")],
            vec![m, 25, s("y")],
            vec![m, 29, s("y")],
            vec![m, 30, s("z")],
            vec![m, 35, s("y")],
            vec![m, 37, s("x")],
            vec![m, 39, s("z")],
        ]
        .into(),
        // k_str
        [vec![m, 44, s("s")]].into(),
        // k_num
        [
            vec![m, 5, 1],
            vec![m, 6, 2],
            vec![m, 11, 2],
            vec![m, 12, 3],
            vec![m, 50, 3],
        ]
        .into(),
        // k_wild
        [vec![m, 49]].into(),
    ]
}

/// The K0 pass refuses what is not K0: an aggregate, a disjunction.
#[test]
fn k0_pass_refuses_an_aggregate() {
    let sym = Symbols::new();
    let t = text("n(c) :- c = count : { e(_, _) }.\n");
    let e = k0::rows(&t, 1, &sym).expect_err("an aggregate is not K0");
    assert!(e.why.contains("not K0"), "{e}");
    let t = text("n(x) :- e(x, _) ; e(_, x).\n");
    assert!(k0::rows(&t, 1, &sym).is_err());
}


/// `bootstrap-l0.rerail` is K0: the pass reads every rule of it, and each writes a K-form relation.
#[test]
fn l0_is_k0() {
    let sym = Symbols::new();
    let rows = k0::rows(&text(L0), 1, &sym).expect("l0 is K0");
    let heads: BTreeSet<String> = rows[0].iter().map(|r| sym.name(r[2])).collect();
    let kform: BTreeSet<String> = RELATIONS.iter().map(|(n, _)| n.to_string()).collect();
    assert!(heads.is_subset(&kform), "{heads:?}");
    assert_eq!(rows[0].len(), 14, "13 reader rules and k_order's");
}
