//! The rebuild's tests (decision 0686): at M0, the pieces that are already the final shape. Each test deleted with
//! the first build comes back at the milestone the rebuild plan names for it.

use crate::dir::Directory;
use crate::files::{BlobId, BlobRows, DocIds, Placed, doc_of, reads};
use crate::intern::{InternOps, intern};
use crate::cache::CacheOps;
use crate::contract::{Cx, Global, apply, push_layer};
use crate::k0::K0Ops;
use crate::l1::{L1, tape_row, tape_shape};
use crate::op::Registry;
use crate::trace::{RollUp, Trace, TraceOps};
use crate::sym::{Atoms, Names};
use crate::tape::{Reads, TapeView, View};
use crate::{L0, kform::RELATIONS};
use rerail_blocks::bistate::{ColumnIndex, Read};
use rerail_blocks::table::Table;
use std::collections::BTreeSet;

type Rows = BTreeSet<Vec<i64>>;

/// A registry with `intern`, `k0_pass` and the trace's and the cache's operations, a directory and its atoms:
/// documents of sources in hand, interned, read by L1.
pub(crate) struct Fx {
    pub reg: Registry,
    pub ops: InternOps,
    pub k0: K0Ops,
    pub drops: crate::drop::DropOps,
    pub traces: TraceOps,
    pub caches: CacheOps,
    pub answers: crate::answer::AnswerOps,
    pub dir: Directory,
    pub blobs: BlobRows,
    pub atoms: Atoms,
}

/// A document in hand: its blobs and its symbols blob.
pub(crate) struct D {
    pub ids: DocIds,
    pub syms: BlobId,
}

impl Fx {
    pub fn new() -> Fx {
        let mut reg = Registry::default();
        let ops = InternOps::declare(&mut reg).unwrap();
        let k0 = K0Ops::declare(&mut reg).unwrap();
        let drops = crate::drop::DropOps::declare(&mut reg).unwrap();
        let traces = TraceOps::declare(&mut reg).unwrap();
        let caches = CacheOps::declare(&mut reg).unwrap();
        let answers = crate::answer::AnswerOps::declare(&mut reg).unwrap();
        let mut dir = Directory::default();
        let atoms = Atoms::new(&mut dir);
        Fx { reg, ops, k0, drops, traces, caches, answers, dir, blobs: BlobRows::new(&traces), atoms }
    }

    pub fn l1(&self) -> L1 {
        L1 { ops: self.k0, drops: self.drops }
    }

    /// A document of `src` answered at `path`, interned: its blobs and its symbols blob.
    pub fn doc(&mut self, src: &str, path: &str) -> D {
        let ids = doc_of(&self.reg, &self.answers, &mut self.dir, &mut self.blobs, src).expect("a document");
        let module = path.strip_suffix(".rerail").unwrap_or(path);
        let (s, t) = (self.place(&ids.source), self.place(&ids.tape));
        let syms = intern(&self.reg, &self.ops, &mut self.dir, &mut self.blobs, self.atoms, s, t, path, module).expect("interned");
        D { ids, syms }
    }

    /// Where blob `id` lies.
    pub fn place(&self, id: &BlobId) -> Placed {
        self.blobs.get(&self.dir, id).unwrap().expect("a blob housed")
    }

    /// `f` given document `d`'s reads, its symbols block's bytes and the language's names, each read where it lies.
    pub fn read<R>(&self, d: &D, f: impl FnOnce(Reads<'_>, &[u8], &Names) -> R) -> R {
        let (s, t, y) = (self.place(&d.ids.source), self.place(&d.ids.tape), self.place(&d.syms));
        let (tl, sl, al) = (self.dir.read("t", &[s.block, t.block]).unwrap(), self.dir.read("t", &[y.block]).unwrap(), self.dir.read("t", &[self.atoms.0]).unwrap());
        let names = Names::new(al.block(0));
        f(reads(tl.block(0), s, tl.block(1), t), sl.block(0), &names)
    }

    /// The symbol of `x`, which must be interned.
    pub fn s(&self, x: &str) -> i64 {
        self.atoms.find(&self.dir, x).unwrap_or_else(|| panic!("{x:?} not interned"))
    }

    /// L1's rows of `doc` as module `d`, by relation: its plain function over a tape port of the one document.
    pub fn rows(&mut self, doc: &D, d: i64) -> Result<[Rows; 8], String> {
        let mut t = Trace::new(&tape_shape())?.with_roll_up(RollUp::Never);
        let ht = push_layer(&self.reg, &self.traces, &mut self.dir, &mut t, &[(tape_row(d, d, 0, &doc.ids, &doc.syms), 1)])?;
        let atoms = Global { block: self.atoms.0, count: u64::from(self.atoms.count(&self.dir)) };
        let l1 = self.l1();
        let mut cx = Cx { reg: &self.reg, dir: &mut self.dir, blobs: &self.blobs, traces: &self.traces, caches: &self.caches };
        let rows = apply(&l1, &mut cx, &[(&t, Some(ht))], &[atoms])?;
        Ok(rows.try_into().expect("eight relations"))
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
    let doc = fx.doc("a(1, x).\n// a comment\nb(2) :- a(2, _).\nc(\"x\", 3).\n", "three.rerail");
    let src_len = fx.place(&doc.ids.source).len;
    fx.read(&doc, |r, syms, names| {
    let (path, module) = (fx.s("three.rerail"), fx.s("three"));
    let v = |view| TapeView::new(view, 1, r, syms, names);
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
        assert!(start < end && end as usize <= src_len);
        assert!(probe(&txt, &[0, 1], &[1, c], 2).is_empty(), "a clause is no leaf");
    }
    assert_eq!(probe(&node, &[0, 2], &[1, s("clause")], 1), clauses, "node's kind column answers as kind's");
    // Another document's key finds nothing here.
    assert!(probe(&kind, &[0], &[2], 1).is_empty());
    });
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
    let doc = fx.doc(src, "five.rerail");
    let rows = fx.rows(&doc, 7).expect("K0");
    let mut dump = String::new();
    fx.read(&doc, |r, _, _| for i in 0..r.len() {
        let (k, f, x) = r.node(i);
        dump += &format!(
            "{i}: {} {} {x} {:?}\n",
            rerail_rr_syntax::tape::KIND_NAMES
                .get(k as usize)
                .unwrap_or(&"?"),
            rerail_rr_syntax::tape::FIELD_NAMES[f as usize],
            r.text(i)
        );
    });
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
    use rerail_blocks::layer::{ColType, RunsW};
    use rerail_blocks::region::pin_block;
    let mut fx = Fx::new();
    let doc = fx.doc("e(1, 2).\np(x, z) :- e(x, y), e(y, z), !q(y), x != z.\nq(\"s\") :- e(_, -3).\n", "k.rerail");
    let rows = fx.rows(&doc, 7).expect("K0");
    let (reg, ops, dir) = (&fx.reg, &fx.traces, &mut fx.dir);
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
        let h = tr.commit(reg, ops, dir, runs, Tiling::WHOLE).unwrap();
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
    let doc = fx.doc("n(c) :- c = count : { e(_, _) }.\n", "agg.rerail");
    let e = fx.rows(&doc, 1).expect_err("an aggregate is not K0");
    assert!(e.contains("not K0"), "{e}");
    let doc = fx.doc("n(x) :- e(x, _) ; e(_, x).\n", "or.rerail");
    assert!(fx.rows(&doc, 1).is_err());
}


/// `bootstrap-l0.rerail` is K0: the pass reads every rule of it, and each writes a K-form relation.
#[test]
fn l0_is_k0() {
    let mut fx = Fx::new();
    let doc = fx.doc(L0, "bootstrap-l0.rerail");
    let rows = fx.rows(&doc, 1).expect("l0 is K0");
    let heads: BTreeSet<String> = rows[0].iter().map(|r| fx.atoms.name(&fx.dir, r[2])).collect();
    let kform: BTreeSet<String> = RELATIONS.iter().map(|(n, _)| n.to_string()).collect();
    assert!(heads.is_subset(&kform), "{heads:?}");
    assert_eq!(rows[0].len(), 14, "13 reader rules and k_order's");
}

/// A document gone is retracted from the rows L1 kept, read in its output traces where they lie: its old tape is not
/// read (here no blob holds it), and the run's output is the new document's cold rows.
#[test]
fn a_gone_document_is_retracted_from_the_kept_rows_not_its_old_tape() {
    use crate::contract::{GlobalIn, Instance, oracle_rows};
    let mut fx = Fx::new();
    let a = fx.doc("e(1, 2).\np(x) :- e(x, _).\n", "m.rerail");
    let b = fx.doc("e(1, 3).\n", "m.rerail");
    let d = fx.s("m");
    let want = fx.rows(&b, d).unwrap();
    let mut t = Trace::new(&tape_shape()).unwrap().with_roll_up(RollUp::Never);
    let (ra, rb) = (tape_row(d, d, 0, &a.ids, &a.syms), tape_row(d, d, 0, &b.ids, &b.syms));
    let h1 = push_layer(&fx.reg, &fx.traces, &mut fx.dir, &mut t, &[(ra.clone(), 1)]).unwrap();
    let h2 = push_layer(&fx.reg, &fx.traces, &mut fx.dir, &mut t, &[(ra, -1), (rb, 1)]).unwrap();
    let n = u64::from(fx.atoms.count(&fx.dir));
    let atoms = [GlobalIn { block: fx.atoms.0, before: Some(n), after: n }];
    let l1 = fx.l1();
    let mut inst = Instance::new(&l1, &mut fx.dir).unwrap();
    let mut cx = Cx { reg: &fx.reg, dir: &mut fx.dir, blobs: &fx.blobs, traces: &fx.traces, caches: &fx.caches };
    inst.step(&l1, &mut cx, &[(&t, None, Some(h1))], &atoms, None).unwrap();
    let kept: usize = inst.outs.iter().map(|o| oracle_rows(&fx.dir, o, o.head()).unwrap().len()).sum();
    assert!(kept > 0);
    // The new document's blobs alone.
    let mut only_b = BlobRows::new(&fx.traces);
    for id in [b.ids.source, b.ids.tape, b.syms] {
        let p = fx.place(&id);
        only_b.house(&fx.reg, &mut fx.dir, p).unwrap();
    }
    let mut cx = Cx { reg: &fx.reg, dir: &mut fx.dir, blobs: &only_b, traces: &fx.traces, caches: &fx.caches };
    let work = inst.step(&l1, &mut cx, &[(&t, Some(h1), Some(h2))], &atoms, None).unwrap();
    let added: usize = want.iter().map(BTreeSet::len).sum();
    assert_eq!(work.rows_written as usize, kept + added, "the kept rows retracted, the new ones placed");
    for (k, o) in inst.outs.iter().enumerate() {
        assert_eq!(oracle_rows(&fx.dir, o, o.head()).unwrap(), want[k], "{}", RELATIONS[k].0);
    }
}

/// Document `d` for `k0_pass` as module `module`.
fn k0_doc(fx: &Fx, d: &D, module: i64) -> crate::k0::K0Doc {
    crate::k0::K0Doc { module, src: fx.place(&d.ids.source), tape: fx.place(&d.ids.tape), syms: fx.place(&d.syms).block }
}

/// `k0_pass`'s rows over `docs`, one call, by relation: each `(row, diff)`, sorted.
fn k0_rows(fx: &mut Fx, docs: &[crate::k0::K0Doc]) -> Result<Vec<Vec<Vec<i64>>>, crate::kform::Refused> {
    let runs: Vec<crate::dir::BlockId> = (0..RELATIONS.len()).map(|k| Trace::new(&crate::kform::shape(k)).unwrap().open(&mut fx.dir, 4096)).collect();
    let d = crate::k0::Docs::of(&mut fx.dir, docs).map_err(|why| crate::kform::Refused { module: -1, rule: -1, why })?;
    let ran = crate::k0::k0_pass(&fx.reg, &fx.k0, &mut fx.dir, fx.atoms.0, &d, None, 1, &runs, None);
    d.release(&mut fx.dir);
    let got = runs
        .iter()
        .enumerate()
        .map(|(k, &r)| {
            let l = fx.dir.read("t", &[r]).unwrap();
            crate::k0::k0_facts(&l.table(), rerail_delta_twins::k0::RUNS + k, l.block(0))
        })
        .collect();
    runs.into_iter().for_each(|r| fx.dir.release(r));
    ran.map(|_| got)
}

/// One `k0_pass` launch over 20 documents (two docs blocks, chained by reference): its rows are those of each
/// document read alone, its phases launched once each, and a refusal names its own document.
#[test]
fn one_k0_pass_launch_reads_every_document() {
    let mut fx = Fx::new();
    let docs: Vec<D> = (0..20).map(|i| fx.doc(&format!("e({i}, {}).\np(x) :- e(x, _), !q(x).\n", i + 1), &format!("m{i}.rerail"))).collect();
    assert!(docs.len() > rerail_delta_twins::k0::DOCS_PER_BLOCK, "past one docs block");
    let all: Vec<crate::k0::K0Doc> = docs.iter().enumerate().map(|(i, d)| k0_doc(&fx, d, i as i64 + 1)).collect();
    let one = k0_rows(&mut fx, &all).unwrap();
    assert_eq!(fx.dir.launches(), 4, "count, scan, place, keep: one launch");
    let mut each = vec![Vec::new(); RELATIONS.len()];
    for d in &all {
        for (k, r) in k0_rows(&mut fx, std::slice::from_ref(d)).unwrap().into_iter().enumerate() {
            each[k].extend(r);
        }
    }
    each.iter_mut().for_each(|r| r.sort());
    assert_eq!(one, each);
    assert!(one[0].len() == 40, "two rules a document");
    // A refusal names its own document.
    let bad = fx.doc("n(c) :- c = count : { e(_, _) }.\n", "bad.rerail");
    let mut with_bad = all.clone();
    with_bad.insert(17, k0_doc(&fx, &bad, 99));
    let e = k0_rows(&mut fx, &with_bad).unwrap_err();
    assert_eq!(e.module, 99, "{e}");
    assert!(e.why.contains("not K0"), "{e}");
}

/// `k0_pass` keeps each document's sizes in L1's cache, the rows it gave it by relation, and lets go of a gone
/// document's entry; the next document takes its room (Operations §6.1: a file that goes takes its entries with it).
#[test]
fn k0_pass_keeps_each_documents_sizes_and_a_gone_one_takes_its_entry() {
    use rerail_delta_twins::cache as c;
    let mut fx = Fx::new();
    let a = fx.doc("e(1, 2).\np(x) :- e(x, _), !q(x).\n", "a.rerail");
    let b = fx.doc("e(3, 4).\n", "b.rerail");
    let cache = crate::cache::OpCache::new(&mut fx.dir).0;
    let runs: Vec<crate::dir::BlockId> = (0..RELATIONS.len()).map(|k| Trace::new(&crate::kform::shape(k)).unwrap().open(&mut fx.dir, 4096)).collect();
    let pass = |fx: &mut Fx, docs: &[crate::k0::K0Doc], gone: &[i64]| {
        let d = crate::k0::Docs::of(&mut fx.dir, docs).unwrap();
        let g = crate::drop::Parts::write(&mut fx.dir, gone.len(), |put| gone.iter().copied().for_each(put)).unwrap();
        let n = crate::k0::k0_pass(&fx.reg, &fx.k0, &mut fx.dir, fx.atoms.0, &d, Some(&g), 1, &runs, Some(cache)).unwrap();
        d.release(&mut fx.dir);
        g.release(&mut fx.dir);
        n
    };
    let (da, db) = (k0_doc(&fx, &a, 7), k0_doc(&fx, &b, 8));
    pass(&mut fx, &[da, db], &[]);
    let rows = |fx: &mut Fx, d: crate::k0::K0Doc| -> Vec<u32> { k0_rows(fx, &[d]).unwrap().iter().map(|r| r.len() as u32).collect() };
    let (want_a, want_b) = (rows(&mut fx, da), rows(&mut fx, db));
    let read = |fx: &Fx| fx.dir.read("t", &[cache]).unwrap().block(0).to_vec(); // copy: not block data (the test's look at the cache)
    let bytes = read(&fx);
    assert_eq!(c::sizes(&bytes, 7).unwrap(), &want_a[..]);
    assert_eq!(c::sizes(&bytes, 8).unwrap(), &want_b[..]);
    let used = c::used(&bytes);
    // Module 7 gone, with no document read: its entry let go; then module 9 takes its room.
    pass(&mut fx, &[], &[7]);
    let bytes = read(&fx);
    assert!(c::sizes(&bytes, 7).is_none() && c::sizes(&bytes, 8).is_some());
    let d9 = k0_doc(&fx, &a, 9);
    pass(&mut fx, &[d9], &[]);
    let bytes = read(&fx);
    assert_eq!(c::sizes(&bytes, 9).unwrap(), &want_a[..]);
    assert_eq!(c::used(&bytes), used, "the gone document's room, taken");
    runs.into_iter().for_each(|r| fx.dir.release(r));
}
