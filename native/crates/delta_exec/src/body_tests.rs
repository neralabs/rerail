//! The rule body's operations over hand-written plans (B§10 step 1): `seed`, `step` and `head` over traces, on each
//! side and on a change; and `tally`, a head row moving only where its count crosses 0.

use super::*;
use crate::contract::{push_layer, rows_at};
use crate::sym::Atoms;
use crate::tally::{self, TallyOps};
use crate::trace::{RollUp, Trace, TraceOps, open_runs};
use rerail_blocks::layer::{ColType, Shape};

struct B {
    reg: Registry,
    body: BodyOps,
    tally: TallyOps,
    traces: TraceOps,
    dir: Directory,
    atoms: Atoms,
}

impl B {
    fn new() -> B {
        let mut reg = Registry::default();
        let body = BodyOps::declare(&mut reg).unwrap();
        let tally = TallyOps::declare(&mut reg).unwrap();
        let traces = TraceOps::declare(&mut reg).unwrap();
        let mut dir = Directory::default();
        let atoms = Atoms::new(&mut dir);
        B { reg, body, tally, traces, dir, atoms }
    }

    fn trace(arity: usize) -> Trace {
        Trace::new(&Shape::run(vec![ColType::I64; arity], (0..arity as u16).collect(), None)).unwrap().with_roll_up(RollUp::Never)
    }

    fn push(&mut self, t: &mut Trace, pairs: &[(Vec<i64>, i32)]) -> BlockId {
        push_layer(&self.reg, &self.traces, &mut self.dir, t, pairs).unwrap()
    }

    /// `plan`'s rows into a runs block a head of `arities`, read back: each head's `(row, diff)`s, sorted.
    fn run(&mut self, plan: &Plan, arities: &[usize]) -> Vec<Vec<(Vec<i64>, i32)>> {
        let runs: Vec<BlockId> = arities.iter().map(|&a| open_runs(&mut self.dir, B::trace(a).shape(), 64)).collect();
        run_plan(&self.reg, &self.body, &mut self.dir, self.atoms.0, plan, &runs).unwrap();
        runs.iter()
            .map(|&r| {
                let l = self.dir.read("t", &[r]).unwrap();
                let rr = Runs::new(l.block(0));
                let mut v: Vec<(Vec<i64>, i32)> = (0..rr.len()).map(|i| ((0..rr.head.shape.arity()).map(|c| rr.get(i, c)).collect(), rr.diff(i))).collect();
                v.sort();
                v
            })
            .collect()
    }
}

fn atom(src: usize, side: Side, args: Vec<Arg>) -> StepPlan {
    StepPlan { op: Op::Atom, src, side, args }
}

/// `two(x, z) :- e(x, y), e(y, z)`, its two delta terms by hand (O1): e's change first, then the other e on the
/// before side; and e's after side, then the change. Their sum is the two-hop relation's change.
#[test]
fn a_hand_written_plan_runs_a_join_on_each_side_and_on_a_change() {
    let mut b = B::new();
    let mut e = B::trace(2);
    let h0 = b.push(&mut e, &[(vec![1, 2], 1), (vec![2, 3], 1)]);
    let h1 = b.push(&mut e, &[(vec![2, 3], -1), (vec![2, 4], 1), (vec![3, 5], 1), (vec![4, 1], 1)]);
    let out = vec![Arg::Eq(0, 0), Arg::Eq(1, 1)];
    // Cold, after: every two-hop path of e at h1.
    let cold = |before, after, side| Plan {
        srcs: vec![Src::Trace { arity: 2, before, after }],
        docs: Vec::new(),
        decls: Decls::default(),
        terms: vec![TermPlan { head: 0, sign: 1, parts: Vec::new(), steps: vec![atom(0, side, vec![Arg::Bind, Arg::Bind]), atom(0, side, vec![Arg::Eq(0, 1), Arg::Bind])], out: out.clone(), level: None }],
    };
    assert_eq!(b.run(&cold(Some(h0), Some(h1), Side::After), &[2]), vec![vec![(vec![1, 4], 1), (vec![2, 1], 1), (vec![4, 2], 1)]]);
    assert_eq!(b.run(&cold(Some(h0), Some(h1), Side::Before), &[2]), vec![vec![(vec![1, 3], 1)]]);
    // The change: δe ⋈ e_before + e_after ⋈ δe.
    let terms = vec![
        TermPlan { head: 0, sign: 1, parts: Vec::new(), steps: vec![atom(0, Side::Delta, vec![Arg::Bind, Arg::Bind]), atom(0, Side::Before, vec![Arg::Eq(0, 1), Arg::Bind])], out: out.clone(), level: None },
        TermPlan { head: 0, sign: 1, parts: Vec::new(), steps: vec![atom(0, Side::Delta, vec![Arg::Bind, Arg::Bind]), atom(0, Side::After, vec![Arg::Bind, Arg::Eq(0, 0)])], out: vec![Arg::Eq(1, 0), Arg::Eq(0, 1)], level: None },
    ];
    let plan = Plan { srcs: vec![Src::Trace { arity: 2, before: Some(h0), after: Some(h1) }], docs: Vec::new(), decls: Decls::default(), terms };
    let mut net: std::collections::BTreeMap<Vec<i64>, i32> = std::collections::BTreeMap::new();
    for (r, d) in b.run(&plan, &[2]).remove(0) {
        *net.entry(r).or_default() += d;
    }
    net.retain(|_, d| *d != 0);
    assert_eq!(net, [(vec![1, 3], -1), (vec![1, 4], 1), (vec![2, 1], 1), (vec![4, 2], 1)].into_iter().collect());
    // The plan as words, read back, is the plan.
    assert_eq!(Plan::of_words(&plan.words()), plan);
}

/// A `not` and a `cmp` in a chain, a fact (no steps), and a level written (0364 §2.4: one more than the highest level
/// read, 0 with none).
#[test]
fn filters_facts_and_levels_in_a_hand_written_plan() {
    let mut b = B::new();
    let mut e = B::trace(2);
    let mut m = B::trace(1);
    let he = b.push(&mut e, &[(vec![1, 2], 1), (vec![2, 3], 1), (vec![3, 3], 1)]);
    let hm = b.push(&mut m, &[(vec![2], 1)]);
    let mut p = B::trace(3);
    let hp = b.push(&mut p, &[(vec![1, 2, 0], 1), (vec![1, 3, 4], 1)]);
    let plan = Plan {
        srcs: vec![Src::Trace { arity: 2, before: None, after: Some(he) }, Src::Trace { arity: 1, before: None, after: Some(hm) }, Src::Trace { arity: 3, before: None, after: Some(hp) }],
        docs: Vec::new(),
        decls: Decls::default(),
        terms: vec![
            // lone(x, y) :- e(x, y), !m(x), x < y.
            TermPlan {
                head: 0,
                sign: 1,
                parts: Vec::new(),
                steps: vec![atom(0, Side::After, vec![Arg::Bind, Arg::Bind]), StepPlan { op: Op::Not, src: 1, side: Side::After, args: vec![Arg::Eq(0, 0)] }, StepPlan { op: Op::Cmp(Cmp::Lt), src: 0, side: Side::After, args: vec![Arg::Eq(0, 0), Arg::Eq(0, 1)] }],
                out: vec![Arg::Eq(0, 0), Arg::Eq(0, 1)],
                level: None,
            },
            // lone(7, 7).
            TermPlan { head: 0, sign: 1, parts: Vec::new(), steps: Vec::new(), out: vec![Arg::Const(7), Arg::Const(7)], level: None },
            // reach(x, z) :- reach(x, y), e(y, z), its level one more than reach(x, y)'s.
            TermPlan {
                head: 1,
                sign: 1,
                parts: Vec::new(),
                steps: vec![atom(2, Side::After, vec![Arg::Bind, Arg::Bind, Arg::Wild]), atom(0, Side::After, vec![Arg::Eq(0, 1), Arg::Bind])],
                out: vec![Arg::Eq(0, 0), Arg::Eq(1, 1)],
                level: Some(vec![(0, 2)]),
            },
            // reach(5, 5), of level 0.
            TermPlan { head: 1, sign: 1, parts: Vec::new(), steps: Vec::new(), out: vec![Arg::Const(5), Arg::Const(5)], level: Some(Vec::new()) },
        ],
    };
    let got = b.run(&plan, &[2, 3]);
    assert_eq!(got[0], vec![(vec![1, 2], 1), (vec![7, 7], 1)], "e(2, 3)'s x is marked; e(3, 3) fails x < y");
    assert_eq!(got[1], vec![(vec![1, 3, 1], 1), (vec![1, 3, 5], 1), (vec![5, 5, 0], 1)]);
}

/// `tally` (0678 step 9): derivations gained and lost move a row's count; the head moves where it crosses 0. A count
/// below 0 is refused.
#[test]
fn tally_moves_a_head_row_only_where_its_count_crosses_zero() {
    let mut b = B::new();
    let mut counts = B::trace(2);
    let mut head = B::trace(1);
    type Moved = Vec<(Vec<i64>, i32)>;
    let step = |b: &mut B, counts: &mut Trace, head: &mut Trace, derivs: &[(Vec<i64>, i32)]| -> Result<(u64, Moved), String> {
        let d = open_runs(&mut b.dir, B::trace(1).shape(), 16);
        {
            let bl = b.dir.block(d).unwrap();
            let pin = pin_block(&bl, "t", true).unwrap();
            let w = rerail_blocks::layer::RunsW::new(&pin);
            for (r, x) in derivs {
                w.push(r, *x).unwrap();
            }
        }
        let (cr, hr) = (open_runs(&mut b.dir, counts.shape(), 16), open_runs(&mut b.dir, head.shape(), 16));
        let n = tally::run(&b.reg, &b.tally, &mut b.dir, counts.head(), 1, d, cr, hr);
        b.dir.release(d);
        let n = match n {
            Ok(n) => n,
            Err(e) => {
                b.dir.release(cr);
                b.dir.release(hr);
                return Err(e);
            }
        };
        let moved = {
            let l = b.dir.read("t", &[hr]).unwrap();
            let rr = Runs::new(l.block(0));
            let mut v: Vec<(Vec<i64>, i32)> = (0..rr.len()).map(|i| (vec![rr.get(i, 0)], rr.diff(i))).collect();
            v.sort();
            v
        };
        counts.commit(&b.reg, &b.traces, &mut b.dir, cr, crate::op::Tiling::WHOLE).unwrap();
        if moved.is_empty() {
            b.dir.release(hr);
        } else {
            head.commit(&b.reg, &b.traces, &mut b.dir, hr, crate::op::Tiling::WHOLE).unwrap();
        }
        Ok((n, moved))
    };
    assert_eq!(step(&mut b, &mut counts, &mut head, &[(vec![1], 1), (vec![1], 1), (vec![2], 1)]).unwrap(), (2, vec![(vec![1], 1), (vec![2], 1)]));
    assert_eq!(rows_at(&b.dir, &counts, counts.head()).unwrap(), [vec![1, 2], vec![2, 1]].into_iter().collect());
    assert_eq!(step(&mut b, &mut counts, &mut head, &[(vec![1], -1)]).unwrap(), (0, vec![]), "row 1 still has a derivation");
    assert_eq!(step(&mut b, &mut counts, &mut head, &[(vec![1], -1), (vec![2], 1)]).unwrap(), (1, vec![(vec![1], -1)]));
    assert_eq!(rows_at(&b.dir, &counts, counts.head()).unwrap(), [vec![2, 2]].into_iter().collect());
    assert_eq!(rows_at(&b.dir, &head, head.head()).unwrap(), [vec![2]].into_iter().collect());
    let e = step(&mut b, &mut counts, &mut head, &[(vec![3], -1)]).unwrap_err();
    assert!(e.contains("below 0"), "{e}");
}
