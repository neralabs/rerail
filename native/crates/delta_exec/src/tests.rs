//! The rebuild's tests (decision 0686): at M0, the pieces that are already the final shape. Each test deleted with
//! the first build comes back at the milestone the rebuild plan names for it.

use crate::dir::{BlockId, Directory};
use crate::files::{DocBlob, doc_of};
use crate::intern::{InternOps, intern};
use crate::kform::Refused;
use crate::op::Registry;
use crate::sym::{Atoms, Names};
use crate::tape::{TapeView, View};
use crate::{L0, k0, kform::RELATIONS};
use rerail_blocks::bistate::{ColumnIndex, Read};
use rerail_blocks::table::Table;
use std::collections::BTreeSet;

type Rows = BTreeSet<Vec<i64>>;

/// A registry with `intern`, a directory and its atoms: documents of sources in hand, interned.
pub(crate) struct Fx {
    pub reg: Registry,
    pub ops: InternOps,
    pub dir: Directory,
    pub atoms: Atoms,
}

impl Fx {
    pub fn new() -> Fx {
        let mut reg = Registry::default();
        let ops = InternOps::declare(&mut reg).unwrap();
        let mut dir = Directory::default();
        let atoms = Atoms::new(&mut dir);
        Fx { reg, ops, dir, atoms }
    }

    /// A document of `src` answered at `path`, interned: its blob and its symbols block.
    pub fn doc(&mut self, src: &str, path: &str) -> (DocBlob, BlockId) {
        let d = doc_of(&mut self.dir, src).expect("a document");
        let module = path.strip_suffix(".rerail").unwrap_or(path);
        let s = intern(&self.reg, &self.ops, &mut self.dir, self.atoms, &d, path, module).expect("interned");
        (d, s)
    }

    /// The symbol of `x`, which must be interned.
    pub fn s(&self, x: &str) -> i64 {
        self.atoms.find(&self.dir, x).unwrap_or_else(|| panic!("{x:?} not interned"))
    }

    /// The K0 pass's rows of `doc` as module `d`.
    pub fn rows(&self, doc: &DocBlob, syms: BlockId, d: i64) -> Result<[Rows; 8], Refused> {
        let (s, a) = (self.dir.read("rows", &[syms]).unwrap(), self.dir.read("rows", &[self.atoms.0]).unwrap());
        k0::rows(&doc.text, d, s.block(0), a.block(0))
    }
}

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
    let mut fx = Fx::new();
    let (doc, syms) = fx.doc("a(1, x).\n// a comment\nb(2) :- a(2, _).\nc(\"x\", 3).\n", "three.rerail");
    let t = &doc.text;
    let (sl, al) = (fx.dir.read("views", &[syms]).unwrap(), fx.dir.read("views", &[fx.atoms.0]).unwrap());
    let names = Names::new(al.block(0));
    let (path, module) = (fx.s("three.rerail"), fx.s("three"));
    let v = |view| TapeView::new(view, 1, t, sl.block(0), &names);
    let (node, kind, field, child, txt, value, doc) = (
        v(View::Node),
        v(View::Kind),
        v(View::Field),
        v(View::Child),
        v(View::Text),
        v(View::Value),
        v(View::Doc),
    );
    let s = |x: &str| fx.s(x);
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
                    heads.insert(fx.atoms.name(&fx.dir, n));
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
    // node(d, n, k, start, end): each clause's kind and its span; `text` is a leaf's (B5: identifiers and strings
    // are interned, not a clause's whole text).
    for &c in &clauses {
        assert_eq!(probe(&node, &[0, 1], &[1, c], 2), vec![s("clause")]);
        let (start, end) = (probe(&node, &[0, 1], &[1, c], 3)[0], probe(&node, &[0, 1], &[1, c], 4)[0]);
        assert!(start < end && end as usize <= t.src.as_str().len());
        assert!(probe(&txt, &[0, 1], &[1, c], 2).is_empty(), "a clause is no leaf");
    }
    assert_eq!(probe(&node, &[0, 2], &[1, s("clause")], 1), clauses, "node's kind column answers as kind's");
    // Another document's key finds nothing here.
    assert!(probe(&kind, &[0], &[2], 1).is_empty());
}

/// The K0 pass's rows for a five-line file, against rows written by hand (node ids read off the tape's dump).
#[test]
fn k0_pass_rows_by_hand() {
    let mut fx = Fx::new();
    let src = "e(1, 2).\n\
               e(2, 3).\n\
               // a comment\n\
               p(x, z) :- e(x, y), e(y, z), !q(y), x != z.\n\
               q(\"s\") :- e(_, 3).\n";
    let (doc, syms) = fx.doc(src, "five.rerail");
    let rows = fx.rows(&doc, syms, 7).expect("K0");
    let r = crate::tape::reads(&doc.text);
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
    let s = |x: &str| fx.s(x);
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
    use crate::op::Tiling;
    use crate::trace::{Trace, TraceOps};
    use rerail_blocks::layer::{ColType, RunsW};
    use rerail_blocks::region::pin_block;
    let mut fx = Fx::new();
    let (doc, syms) = fx.doc("e(1, 2).\np(x, z) :- e(x, y), e(y, z), !q(y), x != z.\nq(\"s\") :- e(_, -3).\n", "k.rerail");
    let rows = fx.rows(&doc, syms, 7).expect("K0");
    let ops = TraceOps::declare(&mut fx.reg).unwrap();
    let (reg, dir) = (&fx.reg, &mut fx.dir);
    for (i, (name, arity)) in RELATIONS.iter().enumerate() {
        let shape = crate::kform::shape(i);
        let wide: Vec<usize> = (0..*arity).filter(|&c| shape.columns[c] == ColType::I64).collect();
        assert_eq!(wide, if *name == "k_num" { vec![2] } else { vec![] }, "{name}: only a number's value is 64 bits");
        let mut tr = Trace::new(&shape).unwrap();
        let want: Rows = rows[i].iter().map(|r| r.to_vec()).collect();
        let runs = tr.open(dir, want.len().max(1));
        {
            let b = dir.block(runs).unwrap();
            let pin = pin_block(&b, "k0", true).unwrap();
            let w = RunsW::new(&pin);
            for r in &want {
                w.push(r, 1).unwrap();
            }
        }
        let h = tr.commit(reg, &ops, dir, runs, Tiling::WHOLE).unwrap();
        let l = tr.read(dir, &[Some(h)]).unwrap();
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
    let mut fx = Fx::new();
    let (doc, syms) = fx.doc("n(c) :- c = count : { e(_, _) }.\n", "agg.rerail");
    let e = fx.rows(&doc, syms, 1).expect_err("an aggregate is not K0");
    assert!(e.why.contains("not K0"), "{e}");
    let (doc, syms) = fx.doc("n(x) :- e(x, _) ; e(_, x).\n", "or.rerail");
    assert!(fx.rows(&doc, syms, 1).is_err());
}


/// `bootstrap-l0.rerail` is K0: the pass reads every rule of it, and each writes a K-form relation.
#[test]
fn l0_is_k0() {
    let mut fx = Fx::new();
    let (doc, syms) = fx.doc(L0, "bootstrap-l0.rerail");
    let rows = fx.rows(&doc, syms, 1).expect("l0 is K0");
    let heads: BTreeSet<String> = rows[0].iter().map(|r| fx.atoms.name(&fx.dir, r[2])).collect();
    let kform: BTreeSet<String> = RELATIONS.iter().map(|(n, _)| n.to_string()).collect();
    assert!(heads.is_subset(&kform), "{heads:?}");
    assert_eq!(rows[0].len(), 14, "13 reader rules and k_order's");
}
