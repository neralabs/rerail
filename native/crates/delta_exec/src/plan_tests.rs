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
        let runs: Vec<BlockId> = arities.iter().map(|&a| open_runs(&mut self.dir, B::trace(a).shape(), 1 << 18)).collect();
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
    let s = search(&ch, &lp, &[], 0.0, GREEDY).unwrap();
    assert_eq!(s.order, vec![1, 0]);
    assert_eq!(s.price, 1.0 + 1e5);
    assert_eq!(s.runners, vec![(vec![0, 1], 2e6)]);
    // An exact pair (0203 §3) overrides the independence estimate.
    let lp2 = LinePrice { pairs: vec![(1, 0, 3.0)], ..lp.clone() };
    assert_eq!(search(&ch, &lp2, &[], 0.0, GREEDY).unwrap().price, 1.0 + 3.0);
    // A chain of 14 steps, each joined to the last: greedy, valid, and the small one first.
    let n = 14;
    let steps: Vec<StepPlan> = (0..n).map(|j| atom(0, Side::After, if j == 0 { vec![Arg::Bind, Arg::Bind] } else { vec![Arg::Eq(j - 1, 1), Arg::Bind] })).collect();
    let long = term(steps, vec![Arg::Eq(0, 0), Arg::Eq(n - 1, 1)]);
    let ch = ChainVars::of(&long).unwrap();
    let mut prices: Vec<StepPrice> = (0..n).map(|_| step(1000.0, &[100.0, 100.0])).collect();
    prices[5] = step(2.0, &[2.0, 2.0]);
    let s = search(&ch, &LinePrice { steps: prices, pairs: Vec::new() }, &[], 0.0, GREEDY).unwrap();
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

/// Exploration (0203 §2, §7) is a seeded draw from the line's key and its runs, and only for a line that matters:
/// one whose draw falls under `EXPLORE_RATE` does not explore while it matters to nothing; once the run's critical
/// path gives it its share, its next plan runs its untried runner-up within `MARGIN`, and answers the same.
#[test]
fn exploration_is_a_seeded_draw_and_runs_a_runner_up() {
    let key = (0..100_000i64).find(|&k| seed_draw(k, 0) < EXPLORE_RATE && seed_draw(k, 1) < EXPLORE_RATE).unwrap();
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
    assert_eq!((got, pd.explored), (want.clone(), 0), "a line that matters to nothing");
    // A line left off the path keeps its order.
    let other = b.memo();
    b.planned(&plan, &[key], other, None, &[2]);
    let (got, pd) = b.planned(&plan, &[key], other, None, &[2]);
    assert_eq!((got, pd.kept, pd.explored), (want.clone(), 1, 0));
    // On the path, the whole of it: it matters.
    let marked = b.plan.critical_path(&b.reg, &mut b.dir, &[memo], &[Vec::new()]).unwrap();
    assert_eq!(marked, 0, "under CRITICAL_ROWS of work: no wider search asked");
    let note = &b.notes(memo)[0].1;
    assert_eq!((note.critical, note.fresh, note.ran, note.wants_search), (1.0, false, 0.0, false));
    let (got, pd) = b.planned(&plan, &[key], memo, None, &[2]);
    assert_eq!((got, pd.explored), (want, 1));
}

/// The random connected orders of a wider search (0203 §5): distinct, valid, a fixed first step first, each next
/// atom joined to those placed; drawn the same from the same key.
#[test]
fn random_orders_are_distinct_valid_and_connected() {
    let n = 14;
    // A star: δs(x, y) first, every other step joined to it on y.
    let steps: Vec<StepPlan> = (0..n).map(|j| atom(0, if j == 0 { Side::Delta } else { Side::After }, if j == 0 { vec![Arg::Bind, Arg::Bind] } else { vec![Arg::Eq(0, 1), Arg::Bind] })).collect();
    let ch = ChainVars::of(&term(steps, vec![Arg::Eq(0, 0), Arg::Eq(n - 1, 1)])).unwrap();
    let os = random_orders(&ch, RANDOM_ORDERS, 7);
    assert_eq!(os.len(), RANDOM_ORDERS, "13! orders to draw from");
    assert_eq!(os, random_orders(&ch, RANDOM_ORDERS, 7));
    for (i, o) in os.iter().enumerate() {
        assert!(ch.valid(o) && o[0] == 0 && !os[..i].contains(o), "{o:?}");
        let mut placed = 1u64;
        for &j in &o[1..] {
            assert!(ch.nodes[j].binds & ch.bound(placed) != 0, "{o:?}: {j} joins nothing placed");
            placed |= 1 << j;
        }
    }
}

/// The wider search (0203 §5): exact past a round's `GREEDY` (up to `WIDE_GREEDY`), so on a 14-step chain its best is
/// no dearer than the round's greedy one; at most `PROPOSALS`, distinct and valid, cheapest first.
#[test]
fn a_wider_search_is_exact_past_the_rounds_bound_and_proposes_distinct_orders() {
    let n = 14;
    let steps: Vec<StepPlan> = (0..n).map(|j| atom(0, Side::After, if j == 0 { vec![Arg::Bind, Arg::Bind] } else { vec![Arg::Eq(j - 1, 1), Arg::Bind] })).collect();
    let ch = ChainVars::of(&term(steps, vec![Arg::Eq(0, 0), Arg::Eq(n - 1, 1)])).unwrap();
    let mut prices: Vec<StepPrice> = (0..n).map(|j| step(1000.0 + 37.0 * j as f64, &[100.0, 10.0 + j as f64])).collect();
    prices[9] = step(2.0, &[2.0, 2.0]);
    prices[3] = step(5.0, &[5.0, 3.0]);
    let lp = LinePrice { steps: prices, pairs: Vec::new() };
    let greedy = search(&ch, &lp, &[], 0.0, GREEDY).unwrap();
    let props = wide(&ch, &lp, 11);
    assert!(!props.is_empty() && props.len() <= PROPOSALS);
    assert!(props[0].1 <= greedy.price, "{} over the greedy {}", props[0].1, greedy.price);
    for (i, (o, x)) in props.iter().enumerate() {
        assert!(ch.valid(o) && !props[..i].iter().any(|p| &p.0 == o));
        assert!(i == 0 || props[i - 1].1 <= *x);
    }
    assert!(wide_cost(n) > SEARCH_WORK, "a wider search costs more than a round's");
}

/// The step misses (0203 §3a): judged only once a line has done `SLOW_FACTOR` searches' work; its widest step blown
/// up, or a step over-estimated with enough bindings entering it.
#[test]
fn step_misses_are_judged_past_eight_searches_of_work() {
    let est = [100.0, 1000.0, 500.0];
    let early = SLOW_FACTOR * SEARCH_WORK - 1.0;
    let late = SLOW_FACTOR * SEARCH_WORK;
    // Blown up: 9000 tuples where the widest priced is 1000.
    assert!(!step_miss(early, &[100.0, 9000.0, 0.0], &est, 1));
    assert!(step_miss(late, &[100.0, 9000.0, 0.0], &est, 1));
    assert!(!step_miss(late, &[100.0, 7000.0, 0.0], &est, 1), "within eight times");
    // Over-estimated: step 1 priced at 2000 rows per... 20 per binding; 100 bindings made 200, 2 per binding.
    let est = [100.0, 2000.0, 2000.0];
    assert!(step_miss(late, &[100.0, 200.0, 0.0], &est, 1));
    assert!(!step_miss(late, &[100.0, 400.0, 0.0], &est, 1), "a fifth of its price: within the factor");
    assert!(!step_miss(late, &[50.0, 10.0, 0.0], &est, 1), "too few bindings to judge");
    assert!(!step_miss(late, &[100.0, 200.0, 0.0], &[0.0; 3], 1), "unpriced");
}

/// The skewed line of the slow-line tests: δa(x, y), 200 rows of y = 0 (`delta` of them, at `y`). bb(y, z): 501 rows of
/// y = 0 (z 0..=500), and 4499 of their own y. c(z, v): ten rows of z = 7, ten each of z 1000..1100. d(z, w): four
/// each of z 0..1000. q(x, v, w) :- δa(x, y), bb(y, z), c(z, v), d(z, w).
fn skewed(b: &mut B, delta: i64, y: i64) -> Plan {
    let a = b.rel(2, (0..delta).map(|x| vec![x, y]));
    let bb = b.rel(2, (0..=500).map(|z| vec![0, z]).chain((1..4500).map(|y| vec![y, 100_000 + y])));
    let c = b.rel(2, (0..10).map(|v| vec![7, v]).chain((1000..1100).flat_map(|z| (0..10).map(move |v| vec![z, v]))));
    let d = b.rel(2, (0..1000).flat_map(|z| (0..4).map(move |w| vec![z, w])));
    let t = term(
        vec![atom(0, Side::Delta, vec![Arg::Bind, Arg::Bind]), atom(1, Side::After, vec![Arg::Eq(0, 1), Arg::Bind]), atom(2, Side::After, vec![Arg::Eq(1, 1), Arg::Bind]), atom(3, Side::After, vec![Arg::Eq(1, 1), Arg::Bind])],
        vec![Arg::Eq(0, 0), Arg::Eq(2, 1), Arg::Eq(3, 1)],
    );
    Plan { srcs: vec![tr(2, a), tr(2, bb), tr(2, c), tr(2, d)], docs: Vec::new(), terms: vec![t] }
}

/// A slow line on the critical path (0274): run first on a small change that reaches none of the skew (y = 1) and
/// given the path's whole share, then on the big one: it passes its limit at a depth; its exact pairs are measured, it is searched again from the steps it
/// ran, its candidates are trialled over a sample of its tuples (0207), and it goes on in the order the trials
/// measured least. The answer is the rule order's.
#[test]
fn a_slow_line_on_the_critical_path_is_trialled_and_handed_off_to_a_better_order() {
    let mut b = B::new();
    let key = 0x5eed_0003;
    assert!(seed_draw(key, 1) >= EXPLORE_RATE, "no exploration at its second plan");
    let memo = b.memo();
    let small = skewed(&mut b, 2, 1);
    let (got, pd) = b.planned(&small, &[key], memo, None, &[3]);
    assert_eq!((got, pd.slow, pd.trials), (b.ruled(&small, &[3]), 0, 0));
    b.plan.critical_path(&b.reg, &mut b.dir, &[memo], &[Vec::new()]).unwrap();
    assert_eq!(b.notes(memo)[0].1.critical, 1.0);
    let plan = skewed(&mut b, 200, 0);
    let want = b.ruled(&plan, &[3]);
    assert_eq!(want[0].len(), 8000);
    let (got, pd) = b.planned(&plan, &[key], memo, None, &[3]);
    assert_eq!(got, want);
    assert_eq!((pd.slow, pd.handoffs), (1, 1), "{pd:?}");
    assert!(pd.trials >= 2 && pd.pairs > 0, "{pd:?}");
    let note = &b.notes(memo)[0].1;
    assert_eq!(note.order, vec![0, 1, 2, 3], "c before d, after the handoff");
    assert!(note.trialled && note.slow == 1);
}

/// A slow line off the critical path (0274) is bounded, not trialled: its exact pairs measured, it is planned again
/// from the steps it ran and goes on in the search's order there, under the handoff's limit (0271). Then a slow line
/// asks for a wider search, made inline where its work pays for it. The answer is the rule order's.
#[test]
fn a_slow_line_is_cut_trialled_and_handed_off_to_a_better_order() {
    let mut b = B::new();
    let plan = skewed(&mut b, 200, 0);
    let want = b.ruled(&plan, &[3]);
    assert_eq!(want[0].len(), 8000);
    let memo = b.memo();
    let (got, pd) = b.planned(&plan, &[0x5eed_0003], memo, None, &[3]);
    assert_eq!(got, want);
    assert_eq!((pd.slow, pd.handoffs, pd.trials), (1, 1, 0), "{pd:?}");
    assert!(pd.pairs > 0, "{pd:?}");
    let note = &b.notes(memo)[0].1;
    assert_eq!(note.order, vec![0, 1, 2, 3], "c before d, planned again from the cut");
    assert_eq!((note.slow, note.trialled), (1, false));
    assert!(note.work < 200_000.0, "d before c would have made 400800 tuples: {}", note.work);
    // A slow line asks for a wider search (0203 §3); its work pays for it, so its next plan makes it inline (§2):
    // its proposals kept, the ask answered, the rows the same.
    assert!(note.wants_search && note.proposals.is_empty());
    assert!(note.work >= WIDE_PAYS * wide_cost(3), "{}", note.work);
    let (got, pd) = b.planned(&plan, &[0x5eed_0003], memo, None, &[3]);
    assert_eq!((got, pd.wide), (want.clone(), 1), "{pd:?}");
    let note = b.notes(memo)[0].1.clone();
    assert!((1..=PROPOSALS).contains(&note.proposals.len()), "{note:?}");
    assert!(note.proposals.iter().all(|(o, _)| o[0] == 0), "δa stays first");
    // The search's answer taken, the ask stands again only where the run went slow again (its skew is in no price).
    assert_eq!(note.wants_search, note.slow > 1, "{note:?}");
    // The geometric mean of its order's measured ratios over the runs (0203 §6).
    assert_eq!(note.tried.iter().filter(|t| t.0 == note.order).count(), 1);
    let (got, pd) = b.planned(&plan, &[0x5eed_0003], memo, None, &[3]);
    assert_eq!((got, pd.wide), (want, u64::from(note.wants_search)));
}

/// A line on the critical path is trialled at its first due point though it is not slow, and once only (0207 §2):
/// its next run, as accurate, trials nothing; a line off the path is never trialled.
#[test]
fn a_critical_line_is_trialled_once_at_its_first_check() {
    // q(x, v, w) :- δa(x, y), bb(y, z), c(z, v), d(z, w): every estimate exact (bb's y has one value), 300 K rows.
    let line = |b: &mut B, delta: i64| {
        let a = b.rel(2, (0..delta).map(|x| vec![x, 0]));
        let bb = b.rel(2, (0..=500).map(|z| vec![0, z]));
        let c = b.rel(2, (0..=500).map(|z| vec![z, z]));
        let d = b.rel(2, (0..=500).map(|z| vec![z, z]));
        let t = term(
            vec![atom(0, Side::Delta, vec![Arg::Bind, Arg::Bind]), atom(1, Side::After, vec![Arg::Eq(0, 1), Arg::Bind]), atom(2, Side::After, vec![Arg::Eq(1, 1), Arg::Bind]), atom(3, Side::After, vec![Arg::Eq(1, 1), Arg::Bind])],
            vec![Arg::Eq(0, 0), Arg::Eq(2, 1), Arg::Eq(3, 1)],
        );
        Plan { srcs: vec![tr(2, a), tr(2, bb), tr(2, c), tr(2, d)], docs: Vec::new(), terms: vec![t] }
    };
    let mut b = B::new();
    let key = (0..10_000i64).find(|&k| (1..4).all(|r| seed_draw(k, r) >= EXPLORE_RATE)).unwrap();
    let (on, off) = (b.memo(), b.memo());
    let small = line(&mut b, 2);
    for m in [on, off] {
        b.planned(&small, &[key], m, None, &[3]);
    }
    b.plan.critical_path(&b.reg, &mut b.dir, &[on], &[Vec::new()]).unwrap();
    let big = line(&mut b, 200);
    let want = b.ruled(&big, &[3]);
    let (got, pd) = b.planned(&big, &[key], on, None, &[3]);
    assert_eq!(got, want);
    assert_eq!(pd.slow, 0, "{pd:?}");
    assert!(pd.trials >= 2, "{pd:?}");
    assert!(b.notes(on)[0].1.trialled);
    let (got, pd) = b.planned(&big, &[key], on, None, &[3]);
    assert_eq!((got, pd.trials), (want.clone(), 0), "trialled once");
    let (got, pd) = b.planned(&big, &[key], off, None, &[3]);
    assert_eq!((got, pd.trials), (want, 0), "off the path");
}

/// The run's critical path (0203 §4): the longest chain of producers by the strata's work. A line on it with its
/// share and `CRITICAL_ROWS` of work is marked critical and, slow and unproposed, asks for a wider search; lines off
/// it get no share; each line's run work is let go after.
#[test]
fn the_critical_path_marks_the_lines_of_its_heaviest_chain() {
    let mut b = B::new();
    let a = b.rel(2, (0..200).map(|x| vec![x, 0]));
    let bb = b.rel(2, (0..=60).map(|z| vec![0, z]));
    let t = term(vec![atom(0, Side::Delta, vec![Arg::Bind, Arg::Bind]), atom(1, Side::After, vec![Arg::Eq(0, 1), Arg::Bind])], vec![Arg::Eq(0, 0), Arg::Eq(1, 1)]);
    let heavy = Plan { srcs: vec![tr(2, a), tr(2, bb)], docs: Vec::new(), terms: vec![t.clone()] };
    let small = b.rel(2, (0..3).map(|x| vec![x, 0]));
    let one = b.rel(2, [vec![0, 1]]);
    let light = Plan { srcs: vec![tr(2, small), tr(2, one)], docs: Vec::new(), terms: vec![t] };
    let memos = [b.memo(), b.memo(), b.memo()];
    b.planned(&heavy, &[1], memos[0], None, &[2]);
    b.planned(&light, &[2], memos[1], None, &[2]);
    b.planned(&light, &[3], memos[2], None, &[2]);
    let ran0 = b.notes(memos[0])[0].1.ran;
    assert!(ran0 >= CRITICAL_ROWS, "{ran0}");
    // Stratum 2 reads stratum 1's: their chain is lighter than stratum 0 alone.
    let marked = b.plan.critical_path(&b.reg, &mut b.dir, &memos, &[Vec::new(), Vec::new(), vec![1]]).unwrap();
    assert_eq!(marked, 1);
    let n0 = b.notes(memos[0])[0].1.clone();
    assert_eq!((n0.critical, n0.fresh, n0.ran, n0.wants_search), (1.0, false, 0.0, true));
    for m in &memos[1..] {
        let n = &b.notes(*m)[0].1;
        assert_eq!((n.critical, n.fresh, n.ran), (0.0, false, 0.0));
    }
    // The path again with nothing run since: no line ran, none marked, the shares kept.
    assert_eq!(b.plan.critical_path(&b.reg, &mut b.dir, &memos, &[Vec::new(), Vec::new(), vec![1]]).unwrap(), 0);
    assert_eq!(b.notes(memos[0])[0].1.critical, 1.0);
}

/// `limit_again` (0271): finite always; past the work done, eight times the rate over the rows left.
#[test]
fn a_handed_off_order_is_limited_again() {
    assert_eq!(limit_again(2.0, 100.0, 10.0, 50.0), 100.0 + 8.0 * 2.0 * 10.0);
    assert_eq!(limit_again(f64::INFINITY, 100.0, 10.0, 300.0), 600.0);
    assert!(limit_again(f64::NAN, 0.0, 0.0, 0.0).is_finite());
}
