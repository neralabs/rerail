//! The planning track (*0686: Operations* §8, §13 P1–P4): chains bound again in another order, the search, the
//! budget and the memo, the learned correction, exploration, and a slow line's cut, trials and handoff. Every planned
//! round answers as its chains in their rules' order do.

use super::*;
use crate::body::{BodyOps, Src, run_plan};
use crate::cache::OpCache;
use crate::contract::push_layer;
use crate::sym::Atoms;
use crate::trace::{RollUp, Trace, TraceOps, open_runs};
use rerail_blocks::layer::{ColType, Runs, Shape};

struct B {
    reg: Registry,
    body: BodyOps,
    plan: PlanOps,
    traces: TraceOps,
    dir: Directory,
    atoms: Atoms,
}

type Rows = Vec<(Vec<i64>, i32)>;

impl B {
    fn new() -> B {
        let mut reg = Registry::default();
        let body = BodyOps::declare(&mut reg).unwrap();
        let plan = PlanOps::declare(&mut reg).unwrap();
        let traces = TraceOps::declare(&mut reg).unwrap();
        let mut dir = Directory::default();
        let atoms = Atoms::new(&mut dir);
        B { reg, body, plan, traces, dir, atoms }
    }

    fn trace(arity: usize) -> Trace {
        Trace::new(&Shape::run(vec![ColType::I64; arity], (0..arity as u16).collect(), None)).unwrap().with_roll_up(RollUp::Never)
    }

    /// A trace holding `rows`, its head.
    fn rel(&mut self, arity: usize, rows: impl IntoIterator<Item = Vec<i64>>) -> BlockId {
        let mut t = B::trace(arity);
        let pairs: Rows = rows.into_iter().map(|r| (r, 1)).collect();
        push_layer(&self.reg, &self.traces, &mut self.dir, &mut t, &pairs).unwrap()
    }

    /// The rows `go` writes into a runs block per head (`arities`), read back sorted.
    fn rows(&mut self, arities: &[usize], go: impl FnOnce(&mut B, &[BlockId])) -> Vec<Rows> {
        let runs: Vec<BlockId> = arities.iter().map(|&a| open_runs(&mut self.dir, B::trace(a).shape(), 20_000)).collect();
        go(self, &runs);
        let out = runs
            .iter()
            .map(|&r| {
                let l = self.dir.read("t", &[r]).unwrap();
                let rr = Runs::new(l.block(0));
                assert_eq!(rr.needed(), 0, "the runs' room");
                let mut v: Rows = (0..rr.len()).map(|i| ((0..rr.head.shape.arity()).map(|c| rr.get(i, c)).collect(), rr.diff(i))).collect();
                v.sort();
                v
            })
            .collect();
        runs.iter().for_each(|&r| self.dir.release(r));
        out
    }

    /// `plan`'s chains in their rules' order.
    fn ruled(&mut self, plan: &Plan, arities: &[usize]) -> Vec<Rows> {
        self.rows(arities, |b, runs| {
            run_plan(&b.reg, &b.body, &mut b.dir, b.atoms.0, plan, runs).unwrap();
        })
    }

    /// `plan`'s chains planned (line keys `keys`, memo `memo`, model `join`): their rows and what the planning did.
    fn planned(&mut self, plan: &Plan, keys: &[i64], memo: BlockId, join: Option<&rerail_accel::join_model::JoinModel>, arities: &[usize]) -> (Vec<Rows>, Planned) {
        let mut pd = Planned::default();
        let rows = self.rows(arities, |b, runs| {
            let round = Round { reg: &b.reg, body: &b.body, ops: &b.plan, atoms: b.atoms.0, memo, join };
            pd = round.run(&mut b.dir, plan, keys, runs).unwrap().1;
        });
        (rows, pd)
    }

    fn memo(&mut self) -> BlockId {
        OpCache::new(&mut self.dir).0
    }

    fn notes(&self, memo: BlockId) -> Vec<(u64, Note)> {
        notes(self.dir.read("t", &[memo]).unwrap().block(0))
    }
}

fn atom(src: usize, side: Side, args: Vec<Arg>) -> StepPlan {
    StepPlan { op: Op::Atom, src, side, args }
}

fn term(steps: Vec<StepPlan>, out: Vec<Arg>) -> TermPlan {
    TermPlan { head: 0, sign: 1, parts: Vec::new(), steps, out, level: None }
}

fn tr(arity: usize, after: BlockId) -> Src {
    Src::Trace { arity, before: None, after: Some(after) }
}

/// `path(x, w) :- e(x, y), f(y, z), g(z, w)` over sources 0, 1, 2, in its rule's order.
fn three_hop() -> TermPlan {
    term(
        vec![atom(0, Side::After, vec![Arg::Bind, Arg::Bind]), atom(1, Side::After, vec![Arg::Eq(0, 1), Arg::Bind]), atom(2, Side::After, vec![Arg::Eq(1, 1), Arg::Bind])],
        vec![Arg::Eq(0, 0), Arg::Eq(2, 1)],
    )
}

/// A chain bound again in its own order is itself, a filter never runs before its variables are bound, and every
/// valid order answers as the rule's order.
#[test]
fn a_chain_bound_again_in_its_own_order_is_itself_and_every_valid_order_answers_the_same() {
    let mut b = B::new();
    let e = b.rel(2, (0..35).map(|i| vec![i % 7, i % 5]));
    let m = b.rel(1, [vec![2], vec![3]]);
    let r = b.rel(3, (0..12).map(|i| vec![i % 4, i % 6, i % 3]));
    // lone(x, y) :- e(x, y), !m(x), x < y.  path(x, w) :- e(x, y), e(y, z), e(z, w).
    // reach(x, z) :- reach(x, y), e(y, z), its level one more than reach(x, y)'s.  same(x, x) :- e(x, x), e(x, x).
    let lone = term(
        vec![atom(0, Side::After, vec![Arg::Bind, Arg::Bind]), StepPlan { op: Op::Not, src: 1, side: Side::After, args: vec![Arg::Eq(0, 0)] }, StepPlan { op: Op::Cmp(Cmp::Lt), src: 0, side: Side::After, args: vec![Arg::Eq(0, 0), Arg::Eq(0, 1)] }],
        vec![Arg::Eq(0, 0), Arg::Eq(0, 1)],
    );
    let mut path = three_hop();
    path.steps.iter_mut().for_each(|s| s.src = 0);
    let reach = TermPlan { level: Some(vec![(0, 2)]), ..term(vec![atom(2, Side::After, vec![Arg::Bind, Arg::Bind, Arg::Wild]), atom(0, Side::After, vec![Arg::Eq(0, 1), Arg::Bind])], vec![Arg::Eq(0, 0), Arg::Eq(1, 1)]) };
    let same = term(vec![atom(0, Side::After, vec![Arg::Bind, Arg::Same(0)]), atom(0, Side::After, vec![Arg::Eq(0, 0), Arg::Eq(0, 1)])], vec![Arg::Eq(0, 0), Arg::Eq(1, 1)]);
    let srcs = vec![tr(2, e), tr(1, m), tr(3, r)];
    for t in [&lone, &path, &reach, &same] {
        let ch = ChainVars::of(t).unwrap();
        let rule: Vec<usize> = (0..t.steps.len()).collect();
        assert!(ch.valid(&rule));
        assert_eq!(ch.rebuild(t, &rule), *t, "the identity order");
    }
    let lc = ChainVars::of(&lone).unwrap();
    assert!(!lc.valid(&[1, 0, 2]) && !lc.valid(&[2, 0, 1]), "a filter after its variables are bound");
    assert!(lc.valid(&[0, 2, 1]));
    let arity = |t: &TermPlan| t.out.len() + usize::from(t.level.is_some());
    for t in [&lone, &path, &reach, &same] {
        let ch = ChainVars::of(t).unwrap();
        let p = |t: TermPlan| Plan { srcs: srcs.clone(), docs: Vec::new(), terms: vec![t] };
        let want = b.ruled(&p(t.clone()), &[arity(t)]);
        assert!(!want[0].is_empty());
        let n = t.steps.len();
        let mut orders = vec![(0..n).rev().collect::<Vec<_>>(), (1..n).chain([0]).collect(), [n - 1].into_iter().chain(0..n - 1).collect()];
        orders.retain(|o| ch.valid(o));
        for o in orders {
            assert_eq!(b.ruled(&p(ch.rebuild(t, &o)), &[arity(t)]), want, "order {o:?}");
        }
    }
}

fn step(card: f64, ndv: &[f64]) -> StepPrice {
    StepPrice { card, ndv: ndv.to_vec(), corr: Vec::new() }
}

/// The dynamic program takes the cheapest order (a small relation first, the big one probed by it) and keeps the
/// other ending as its runner-up; past `GREEDY` free steps the order is greedy, and still valid.
#[test]
fn the_search_takes_the_cheapest_order_and_keeps_the_runners_up() {
    // q(x, y) :- big(x, y), small(y).
    let t = term(vec![atom(0, Side::After, vec![Arg::Bind, Arg::Bind]), atom(1, Side::After, vec![Arg::Eq(0, 1)])], vec![Arg::Eq(0, 0), Arg::Eq(0, 1)]);
    let ch = ChainVars::of(&t).unwrap();
    let lp = LinePrice { steps: vec![step(1e6, &[1e6, 10.0]), step(1.0, &[1.0])], pairs: Vec::new() };
    let s = search(&ch, &lp, &[], 0.0).unwrap();
    assert_eq!(s.order, vec![1, 0]);
    assert_eq!(s.price, 1.0 + 1e5);
    assert_eq!(s.runners, vec![(vec![0, 1], 2e6)]);
    // An exact pair (0203 §3) overrides the independence estimate.
    let lp2 = LinePrice { pairs: vec![(1, 0, 3.0)], ..lp.clone() };
    assert_eq!(search(&ch, &lp2, &[], 0.0).unwrap().price, 1.0 + 3.0);
    // A chain of 14 steps, each joined to the last: greedy, valid, and the small one first.
    let n = 14;
    let steps: Vec<StepPlan> = (0..n).map(|j| atom(0, Side::After, if j == 0 { vec![Arg::Bind, Arg::Bind] } else { vec![Arg::Eq(j - 1, 1), Arg::Bind] })).collect();
    let long = term(steps, vec![Arg::Eq(0, 0), Arg::Eq(n - 1, 1)]);
    let ch = ChainVars::of(&long).unwrap();
    let mut prices: Vec<StepPrice> = (0..n).map(|_| step(1000.0, &[100.0, 100.0])).collect();
    prices[5] = step(2.0, &[2.0, 2.0]);
    let s = search(&ch, &LinePrice { steps: prices, pairs: Vec::new() }, &[], 0.0).unwrap();
    assert!(ch.valid(&s.order) && s.runners.is_empty(), "{:?}", s.order);
    assert_eq!(s.order[0], 5);
}

/// A round plans its line, its order answers as the rule's, and its memo holds what it chose and measured; the next
/// round takes its kept order under the budget (0122) unsearched; a dropped memo plans again, the same rows (L5).
#[test]
fn a_round_plans_keeps_its_order_under_the_budget_and_a_dropped_memo_answers_the_same() {
    let mut b = B::new();
    let e = b.rel(2, (0..6).map(|i| vec![i, i % 3]));
    let f = b.rel(2, (0..3).map(|i| vec![i, 10 + i]));
    let g = b.rel(2, [vec![11, 100]]);
    let plan = Plan { srcs: vec![tr(2, e), tr(2, f), tr(2, g)], docs: Vec::new(), terms: vec![three_hop()] };
    let want = b.ruled(&plan, &[2]);
    assert_eq!(want[0], vec![(vec![1, 100], 1), (vec![4, 100], 1)]);
    let memo = b.memo();
    let key = 0x5eed_0001;
    let (got, pd) = b.planned(&plan, &[key], memo, None, &[2]);
    assert_eq!(got, want);
    assert_eq!((pd.lines, pd.searched, pd.kept, pd.priced, pd.explored), (1, 1, 0, 3, 0));
    let notes = b.notes(memo);
    assert_eq!(notes.len(), 1);
    let (k, note) = &notes[0];
    assert_eq!(*k, key as u64);
    assert_eq!(note.order[0], 2, "g, one row, first");
    assert_eq!((note.runs, note.tried.len()), (1, 1));
    assert!(note.work > 0.0 && note.work < SEARCH_WORK);
    assert_eq!(note.samples.len(), 2);
    let (got, pd) = b.planned(&plan, &[key], memo, None, &[2]);
    assert_eq!(got, want);
    assert_eq!((pd.searched, pd.kept), (0, 1), "the kept order, unsearched");
    assert_eq!(b.notes(memo)[0].1.runs, 2);
    assert!(!samples(b.dir.read("t", &[memo]).unwrap().block(0)).is_empty(), "the model's training rows");
    let fresh = b.memo();
    let (got, pd) = b.planned(&plan, &[key], fresh, None, &[2]);
    assert_eq!((got, pd.searched), (want, 1));
}

/// A fake learned model: every scan of step `step` is `exp(by)` times its estimate.
#[derive(Debug)]
struct Fake {
    step: usize,
    by: f64,
}

impl rerail_accel::models::LearnedJoin for Fake {
    fn fanout(&self, s: &rerail_accel::join_model::StepSample) -> Result<f64, String> {
        Ok(if s.bound == 0 && s.literal.ends_with(&format!(":{}", self.step)) { self.by } else { 0.0 })
    }
    fn fanouts(&self, steps: &[&rerail_accel::join_model::StepSample]) -> Result<Vec<f64>, String> {
        steps.iter().map(|s| self.fanout(s)).collect()
    }
    fn on_device(&self) -> bool {
        false
    }
    fn counts(&self) -> [u64; 3] {
        [0; 3]
    }
    fn cpu(&self) -> [f64; 3] {
        [0.0; 3]
    }
    fn trained_on(&self) -> usize {
        0
    }
    fn device_rows(&self) -> usize {
        0
    }
    fn costs(&self) -> (f64, f64) {
        (0.0, 0.0)
    }
}

/// The learned correction (0354) moves the choice: told `small`'s scan makes far more rows than its statistics say,
/// the planner starts from `big`; the answer is the same.
#[test]
fn a_learned_correction_changes_the_order_chosen_not_the_answer() {
    let mut b = B::new();
    let big = b.rel(2, (0..1000).map(|i| vec![i, i % 10]));
    let small = b.rel(1, [vec![3]]);
    let t = term(vec![atom(0, Side::After, vec![Arg::Bind, Arg::Bind]), atom(1, Side::After, vec![Arg::Eq(0, 1)])], vec![Arg::Eq(0, 0), Arg::Eq(0, 1)]);
    let plan = Plan { srcs: vec![tr(2, big), tr(1, small)], docs: Vec::new(), terms: vec![t] };
    let want = b.ruled(&plan, &[2]);
    assert_eq!(want[0].len(), 100);
    let key = 0x5eed_0002;
    let order = |b: &mut B, join: Option<&rerail_accel::join_model::JoinModel>| {
        let memo = b.memo();
        let (got, pd) = b.planned(&plan, &[key], memo, join, &[2]);
        assert_eq!((got, pd.explored), (want.clone(), 0));
        b.notes(memo)[0].1.order.clone()
    };
    assert_eq!(order(&mut b, None), vec![1, 0]);
    let model: rerail_accel::join_model::JoinModel = std::sync::Arc::new(Fake { step: 1, by: 10.0 });
    assert_eq!(order(&mut b, Some(&model)), vec![0, 1]);
}

/// Exploration (0203 §2) is a seeded draw from the line's key and its runs: a line whose draw falls under
/// `EXPLORE_RATE` runs its untried runner-up within `MARGIN`, and answers the same.
#[test]
fn exploration_is_a_seeded_draw_and_runs_a_runner_up() {
    let key = (0..10_000i64).find(|&k| seed_draw(k, 0) < EXPLORE_RATE).unwrap();
    assert_eq!(seed_draw(key, 0), seed_draw(key, 0));
    let mut b = B::new();
    // Both orders about as cheap: a(x, y), c(y) with a of 4 rows and c of 2.
    let a = b.rel(2, (0..4).map(|i| vec![i, i % 2]));
    let c = b.rel(1, [vec![0], vec![1]]);
    let t = term(vec![atom(0, Side::After, vec![Arg::Bind, Arg::Bind]), atom(1, Side::After, vec![Arg::Eq(0, 1)])], vec![Arg::Eq(0, 0), Arg::Eq(0, 1)]);
    let plan = Plan { srcs: vec![tr(2, a), tr(1, c)], docs: Vec::new(), terms: vec![t] };
    let want = b.ruled(&plan, &[2]);
    let memo = b.memo();
    let (got, pd) = b.planned(&plan, &[key], memo, None, &[2]);
    assert_eq!((got, pd.explored), (want, 1));
}

/// A slow line (0203 §3, 0206, 0207): its estimate misled by skew, it passes its limit at a depth; its exact pairs
/// are measured, it is searched again from the steps it ran, its candidates are trialled over a sample of its
/// tuples, and it goes on in the order the trials measured least. The answer is the rule order's.
#[test]
fn a_slow_line_is_cut_trialled_and_handed_off_to_a_better_order() {
    let mut b = B::new();
    // δa(x, y): 200 rows, y = 0. bb(y, z): 501 rows of y = 0 (z 0..=500), and 4499 of their own y. c(z, v): ten
    // rows of z = 7, ten each of z 1000..1100. d(z, w): four each of z 0..1000.
    let a = b.rel(2, (0..200).map(|x| vec![x, 0]));
    let bb = b.rel(2, (0..=500).map(|z| vec![0, z]).chain((1..4500).map(|y| vec![y, 100_000 + y])));
    let c = b.rel(2, (0..10).map(|v| vec![7, v]).chain((1000..1100).flat_map(|z| (0..10).map(move |v| vec![z, v]))));
    let d = b.rel(2, (0..1000).flat_map(|z| (0..4).map(move |w| vec![z, w])));
    // q(x, v, w) :- δa(x, y), bb(y, z), c(z, v), d(z, w).
    let t = term(
        vec![atom(0, Side::Delta, vec![Arg::Bind, Arg::Bind]), atom(1, Side::After, vec![Arg::Eq(0, 1), Arg::Bind]), atom(2, Side::After, vec![Arg::Eq(1, 1), Arg::Bind]), atom(3, Side::After, vec![Arg::Eq(1, 1), Arg::Bind])],
        vec![Arg::Eq(0, 0), Arg::Eq(2, 1), Arg::Eq(3, 1)],
    );
    let plan = Plan { srcs: vec![tr(2, a), tr(2, bb), tr(2, c), tr(2, d)], docs: Vec::new(), terms: vec![t] };
    let want = b.ruled(&plan, &[3]);
    assert_eq!(want[0].len(), 8000);
    let memo = b.memo();
    let (got, pd) = b.planned(&plan, &[0x5eed_0003], memo, None, &[3]);
    assert_eq!(got, want);
    assert_eq!((pd.slow, pd.handoffs), (1, 1), "{pd:?}");
    assert!(pd.trials >= 2 && pd.pairs > 0, "{pd:?}");
    let note = &b.notes(memo)[0].1;
    assert_eq!(note.order, vec![0, 1, 2, 3], "c before d, after the handoff");
    assert_eq!(note.slow, 1);
    assert!(note.work < 200_000.0, "d before c would have made 400800 tuples: {}", note.work);
}

/// `limit_again` (0271): finite always; past the work done, eight times the rate over the rows left.
#[test]
fn a_handed_off_order_is_limited_again() {
    assert_eq!(limit_again(2.0, 100.0, 10.0, 50.0), 100.0 + 8.0 * 2.0 * 10.0);
    assert_eq!(limit_again(f64::INFINITY, 100.0, 10.0, 300.0), 600.0);
    assert!(limit_again(f64::NAN, 0.0, 0.0, 0.0).is_finite());
}
