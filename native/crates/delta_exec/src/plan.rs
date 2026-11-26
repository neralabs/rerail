//! Planning as operations (*0686: Operations* §8; decisions 0038, 0122, 0123, 0203, 0206, 0207, 0271, 0354, 0361).
//!
//! A stratum's chains are written in their rules' order (`phase`'s planner), and each round plans the order they run
//! in before `seed` starts them. Each step of the planner is an operation over blocks, its host version its
//! reference, as the body's are. What a planner keeps between rounds and runs is the operator's cache (§6.1), the
//! **memo**, a line's entry by its key (`rerail_delta_twins::cache::line`); a round's choices are the plan block.
//!
//! - **lines** (an emit): the round's lines, a chain each: which have no work (a change with no rows), which have one
//!   order (a fact, a chain of one free step), which take their kept order unsearched (0122: a kept order whose last
//!   work, scaled by its first step's growth, is under a search's, `SEARCH_WORK`), and which are priced. K0 declares
//!   no keys, fanouts or algorithms, so no line has a declared default (0361) beyond its one order.
//! - **price** (a map): each priced line's steps, every one in one batch: its rows and each column's distinct values
//!   (a trace's from its layers' sketches, kept as rows come, 0123; a view's read from its documents' rows), with the
//!   exact pairs a slow line asked for (0203 §3). The learned correction (0354), when a model is given, is the
//!   model's one batch over every step and every set of its columns bound (`correct`): the factor its target gives.
//! - **search** (a line a group): the dynamic program over subsets of a line's steps (0038), greedy past `GREEDY`,
//!   keeping the best order ending in each other step as runners-up (0203 §1). A step's fanout is its rows over the
//!   distinct values of each column bound (by a constant, an earlier step or a repeat), corrected; a `not` keeps what
//!   its key misses; a `cmp` keeps a fixed share. An order's price is the rows it makes, step by step.
//! - **choose** (a map): the cheapest candidate, its price corrected by what its order measured when it last ran
//!   (0203 §6); now and then (`EXPLORE_RATE`) an untried runner-up within `MARGIN` of it, for a line priced under
//!   `EXPLORE_ROWS`, by a seeded draw from the line's key and its runs (a pure function of them). At a handoff it
//!   takes the order a trial measured least, where it beats the running one by `TRIAL_MARGIN` (0207).
//! - **emit_plan** (an emit): the chosen chains into the plan block `seed`, `step` and `head` read (each step's
//!   arguments bound again in the new order), and each priced line's choice and runners-up into the memo.
//! - **check** (a map, after each depth): each line's work so far against its limit, at its due points (`SLOW_ROWS`,
//!   then each doubling): past `SLOW_FACTOR` times its price, it is slow (0203 §3).
//! - **stats** (a map, for slow lines): the exact rows of each pair of the line's atoms that share a variable, into
//!   the memo (0203 §3's exact pair statistics), which the next pricing uses.
//! - **note** (a map, at the round's end): what each line measured, into the memo: its work and its first step's
//!   rows (the budget's next test), its order's measured ratio, and each step's sample (its rows in and out against
//!   its price: the learned model's training rows, `samples`).
//!
//! A slow line is planned again from the depth it reached (0206's cut): `stats`, `price` and `search` with the steps
//! it ran as a fixed prefix and its tuples there as the rows in; its candidates are trialled over a sample of those
//! tuples (0207: `TRIAL_ROWS`, each cut at `TRIAL_CUTOFF` times the least measured so far, the sample what
//! `TRIAL_SHARE` of its work pays for), and the one that goes on is given a limit (0271, `limit_again`). Its steps
//! already run keep their places, so the tuples made so far read the same in the new order.
//!
//! The memo is the stratum's operator cache (P§11.3, L5): dropping it changes what is planned, never an answer, and
//! every order a planner emits is a valid order of its chain's steps (a filter after its variables are bound).

use crate::body::{self, Arg, BodyOps, Env, Op, Plan, Side, StepPlan, TermPlan};
use crate::dir::{BlockId, Directory};
use crate::op::{Across, Bound, Found, Independence, Inside, Mode, OpId, Output, Over, ParamsLayout, Phase, Registry, Signature, SlotSpec, Tiling, Unit, Version, run_cpu};
use crate::program::Cmp;
use crate::sym::Names;
use rerail_blocks::block::kind;
use rerail_blocks::gens::{BODY_AT, Table, set_word, word};
use rerail_blocks::region::pin_block;
use rerail_delta_twins::cache as memo_twin;
use rerail_delta_twins::{Ctx, NoParams, Tile, TwinPhase};
use std::collections::BTreeMap;

/// What a search costs, in rows of join work (0122): a kept order that did less, its first step's growth allowed
/// for, goes on unsearched.
pub const SEARCH_WORK: f64 = 250.0;
/// Past this many free steps a chain is ordered greedily, not searched (0038).
pub const GREEDY: usize = 12;
/// The runners-up a search keeps: the best order ending in each other step (0203 §1).
pub const RUNNERS_UP: usize = 4;
/// A line is slow past this many times its price (0203 §3)...
pub const SLOW_FACTOR: f64 = 8.0;
/// ...checked first at this many rows of work, then at each doubling.
pub const SLOW_ROWS: f64 = 100_000.0;
/// The chance a line explores an untried runner-up (0203 §2), within this many times the best's price, for a line
/// priced under `EXPLORE_ROWS` (0354: a wrong order there costs little).
pub const EXPLORE_RATE: f64 = 0.1;
pub const MARGIN: f64 = 2.0;
pub const EXPLORE_ROWS: f64 = 1_000_000.0;
/// Decision 0207's trials: a sample's rows at most, an order's cutoff as a multiple of the least measured so far, the
/// share of the line's work so far the sample may spend, and by how much a trialled order must beat the running one;
/// under `TRIAL_LEAST` tuples there is no sample.
pub const TRIAL_ROWS: usize = 256;
pub const TRIAL_CUTOFF: f64 = 2.0;
pub const TRIAL_SHARE: f64 = 0.1;
pub const TRIAL_MARGIN: f64 = 1.25;
pub const TRIAL_LEAST: usize = 8;
/// The exploration's seed (the old engine's `RERAIL_JOIN_SEED`, fixed: the plan is a function of its inputs).
const SEED: u64 = 0x0686_0203;

/// Decision 0271: the limit of an order that goes on at a cut: past the work `base` it has done, `SLOW_FACTOR` times
/// its work per row `rate` over the `left` rows left; an order that has done no row may double the larger of `base`
/// and the cut's `work`. Always finite: no order goes on unlimited.
pub fn limit_again(rate: f64, base: f64, left: f64, work: f64) -> f64 {
    if rate.is_finite() { base + SLOW_FACTOR * rate * left.max(1.0) } else { 2.0 * base.max(work).max(1.0) }
}

/// What a round's planning did: lines planned, by how; steps priced; orders explored; slow lines, handoffs and the
/// trials' launches; exact pairs measured.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Planned {
    pub lines: u64,
    pub kept: u64,
    pub searched: u64,
    pub priced: u64,
    pub explored: u64,
    pub slow: u64,
    pub handoffs: u64,
    pub trials: u64,
    pub pairs: u64,
}

impl std::ops::AddAssign for Planned {
    fn add_assign(&mut self, o: Planned) {
        self.lines += o.lines;
        self.kept += o.kept;
        self.searched += o.searched;
        self.priced += o.priced;
        self.explored += o.explored;
        self.slow += o.slow;
        self.handoffs += o.handoffs;
        self.trials += o.trials;
        self.pairs += o.pairs;
    }
}

// ---- Words: every planning block holds its length, then its words, at the body.

fn fb(x: f64) -> i64 {
    x.to_bits() as i64
}

fn bf(x: i64) -> f64 {
    f64::from_bits(x as u64)
}

/// A planning block's words, decoded: a few words a line.
fn words_of(b: &[u8]) -> Vec<i64> {
    let n = word::<i64>(b, BODY_AT) as usize;
    (0..n).map(|i| word::<i64>(b, BODY_AT + 8 + 8 * i)).collect() // copy: derived (a planning block's few words, decoded)
}

fn put_words(b: &mut [u8], w: &[i64]) {
    assert!(BODY_AT + 8 + 8 * w.len() <= b.len(), "a planning block sized for its words");
    set_word::<i64>(b, BODY_AT, w.len() as i64);
    for (i, &x) in w.iter().enumerate() {
        set_word::<i64>(b, BODY_AT + 8 + 8 * i, x);
    }
}

/// A cursor over words.
struct Cur<'a> {
    w: &'a [i64],
    at: usize,
}

impl Cur<'_> {
    fn new(w: &[i64]) -> Cur<'_> {
        Cur { w, at: 0 }
    }
    fn next(&mut self) -> i64 {
        self.at += 1;
        self.w[self.at - 1]
    }
    fn f(&mut self) -> f64 {
        bf(self.next())
    }
    fn n(&mut self) -> usize {
        self.next() as usize
    }
    fn vec(&mut self, n: usize) -> Vec<usize> {
        (0..n).map(|_| self.n()).collect()
    }
    fn fs(&mut self, n: usize) -> Vec<f64> {
        (0..n).map(|_| self.f()).collect()
    }
}

/// A plan block's plan and its chains' line keys (`body::plan_block_keyed`).
fn plan_and_keys(b: &[u8]) -> (Plan, Vec<i64>) {
    let w = words_of(b);
    let plan = Plan::of_words(&w);
    let at = plan.words().len();
    let keys = if at < w.len() { w[at + 1..at + 1 + w[at] as usize].to_vec() } else { vec![0; plan.terms.len()] };
    (plan, keys)
}

// ---- A chain as variables: what each step binds, needs and follows.

/// An argument as a variable, a constant, `_`, or a column of a step's row past its arguments (a level).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum V {
    Var(usize),
    Const(i64),
    Wild,
    Row(usize, usize),
}

#[derive(Clone, Debug)]
struct Node {
    op: Op,
    src: usize,
    side: Side,
    args: Vec<V>,
    /// The variables it binds (an atom's every one; a `not`'s own, those no other step reads), those it needs bound
    /// before it, and the steps it reads a row column of.
    binds: u64,
    needs: u64,
    after: u64,
}

/// A chain's steps as variables, in its rule's order, and whether its first step is its change (it stays first).
#[derive(Clone, Debug)]
pub(crate) struct ChainVars {
    nodes: Vec<Node>,
    fixed: bool,
    out: Vec<V>,
    level: Option<Vec<(usize, usize)>>,
}

impl ChainVars {
    /// Chain `t` (its steps in its rule's order) as variables; `None` past 64 variables or steps (it runs as written).
    pub(crate) fn of(t: &TermPlan) -> Option<ChainVars> {
        let mut var_at: BTreeMap<(usize, usize), usize> = BTreeMap::new();
        for (j, s) in t.steps.iter().enumerate() {
            for (i, a) in s.args.iter().enumerate() {
                if *a == Arg::Bind {
                    let n = var_at.len();
                    var_at.insert((j, i), n);
                }
            }
        }
        if var_at.len() > 64 || t.steps.len() > 64 {
            return None;
        }
        let v = |j: usize, a: Arg| match a {
            Arg::Bind => unreachable!("a binding is read at its place"),
            Arg::Same(c) => V::Var(var_at[&(j, c)]),
            Arg::Eq(bj, bc) => var_at.get(&(bj, bc)).map_or(V::Row(bj, bc), |&x| V::Var(x)),
            Arg::Const(x) => V::Const(x),
            Arg::Wild => V::Wild,
        };
        let mut nodes: Vec<Node> = Vec::new();
        for (j, s) in t.steps.iter().enumerate() {
            let args: Vec<V> = s.args.iter().enumerate().map(|(i, &a)| if a == Arg::Bind { V::Var(var_at[&(j, i)]) } else { v(j, a) }).collect();
            nodes.push(Node { op: s.op, src: s.src, side: s.side, args, binds: 0, needs: 0, after: 0 });
        }
        let out: Vec<V> = t.out.iter().map(|&a| v(usize::MAX, a)).collect();
        let mut occ = vec![0u64; var_at.len()];
        for (j, n) in nodes.iter().enumerate() {
            for a in &n.args {
                if let V::Var(x) = a {
                    occ[*x] |= 1 << j;
                }
            }
        }
        let in_out: u64 = out.iter().filter_map(|a| if let V::Var(x) = a { Some(1u64 << x) } else { None }).fold(0, |m, b| m | b);
        for (j, n) in nodes.iter_mut().enumerate() {
            let vars: u64 = n.args.iter().filter_map(|a| if let V::Var(x) = a { Some(1u64 << x) } else { None }).fold(0, |m, b| m | b);
            n.after = n.args.iter().filter_map(|a| if let V::Row(sj, _) = a { Some(1u64 << sj) } else { None }).fold(0, |m, b| m | b);
            match n.op {
                Op::Atom => n.binds = vars,
                Op::Not => {
                    let own = (0..occ.len()).filter(|&x| vars >> x & 1 == 1 && occ[x] == 1 << j && in_out >> x & 1 == 0).fold(0u64, |m, x| m | 1 << x);
                    n.binds = own;
                    n.needs = vars & !own;
                }
                Op::Cmp(_) => n.needs = vars,
            }
        }
        let fixed = t.steps.first().is_some_and(|s| s.side == Side::Delta);
        Some(ChainVars { nodes, fixed, out, level: t.level.clone() })
    }

    fn len(&self) -> usize {
        self.nodes.len()
    }

    /// The variables the atoms of `placed` bind.
    fn bound(&self, placed: u64) -> u64 {
        self.nodes.iter().enumerate().filter(|(j, n)| placed >> j & 1 == 1 && n.op == Op::Atom).fold(0, |m, (_, n)| m | n.binds)
    }

    /// Whether step `j` may run after the steps of `placed`.
    fn ready(&self, placed: u64, j: usize) -> bool {
        let n = &self.nodes[j];
        n.needs & !self.bound(placed) == 0 && n.after & !placed == 0
    }

    /// Whether `order` is one of its valid orders.
    fn valid(&self, order: &[usize]) -> bool {
        let mut placed = 0u64;
        if order.len() != self.len() || (self.fixed && order.first() != Some(&0)) {
            return false;
        }
        for &j in order {
            if j >= self.len() || placed >> j & 1 == 1 || !self.ready(placed, j) {
                return false;
            }
            placed |= 1 << j;
        }
        true
    }

    /// ChainVars `t` with its steps run in `order` (rule-order indices), each argument bound again: a variable's first
    /// step that binds it binds it, the later ones read it there; a row column is read at its step's new place.
    fn rebuild(&self, t: &TermPlan, order: &[usize]) -> TermPlan {
        let mut pos = vec![0usize; self.len()];
        for (p, &j) in order.iter().enumerate() {
            pos[j] = p;
        }
        let nvars = self.nodes.iter().flat_map(|n| &n.args).filter_map(|a| if let V::Var(x) = a { Some(*x + 1) } else { None }).max().unwrap_or(0);
        let mut binder: Vec<Option<(usize, usize)>> = vec![None; nvars];
        let read = |a: V, binder: &[Option<(usize, usize)>]| match a {
            V::Var(x) => {
                let (p, c) = binder[x].expect("a variable bound before it is read");
                Arg::Eq(p, c)
            }
            V::Row(sj, c) => Arg::Eq(pos[sj], c),
            V::Const(v) => Arg::Const(v),
            V::Wild => Arg::Wild,
        };
        let mut steps = Vec::new();
        for (p, &j) in order.iter().enumerate() {
            let n = &self.nodes[j];
            let mut here: BTreeMap<usize, usize> = BTreeMap::new();
            let args = n
                .args
                .iter()
                .enumerate()
                .map(|(i, &a)| match a {
                    V::Var(x) if binder[x].is_none() && n.binds >> x & 1 == 1 => match here.get(&x) {
                        Some(&c0) => Arg::Same(c0),
                        None => {
                            here.insert(x, i);
                            Arg::Bind
                        }
                    },
                    a => read(a, &binder),
                })
                .collect();
            for (x, i) in here {
                binder[x] = Some((p, i));
            }
            steps.push(StepPlan { op: n.op, src: n.src, side: n.side, args });
        }
        let out = self.out.iter().map(|&a| read(a, &binder)).collect();
        let level = self.level.as_ref().map(|l| l.iter().map(|&(j, c)| (pos[j], c)).collect());
        TermPlan { head: t.head, sign: t.sign, parts: t.parts.clone(), steps, out, level }
    }
}

// ---- Prices: a step's rows, its columns' distinct values, the correction at each set of them bound.

#[derive(Clone, Debug, Default, PartialEq)]
struct StepPrice {
    card: f64,
    ndv: Vec<f64>,
    /// The learned correction at each set of its columns bound (bit i: column i), when a model gave one.
    corr: Vec<f64>,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct LinePrice {
    steps: Vec<StepPrice>,
    /// Exact pair statistics a slow line asked for (0203 §3): `(a, b, rows of a joined with b)`.
    pairs: Vec<(usize, usize, f64)>,
}

impl LinePrice {
    fn words(&self, w: &mut Vec<i64>) {
        w.push(self.steps.len() as i64);
        for s in &self.steps {
            w.extend([fb(s.card), s.ndv.len() as i64]);
            w.extend(s.ndv.iter().map(|&x| fb(x)));
            w.push(s.corr.len() as i64);
            w.extend(s.corr.iter().map(|&x| fb(x)));
        }
        w.push(self.pairs.len() as i64);
        w.extend(self.pairs.iter().flat_map(|&(a, b, r)| [a as i64, b as i64, fb(r)]));
    }

    fn read(c: &mut Cur<'_>) -> LinePrice {
        let n = c.n();
        let steps = (0..n)
            .map(|_| {
                let card = c.f();
                let k = c.n();
                let ndv = c.fs(k);
                let m = c.n();
                StepPrice { card, ndv, corr: c.fs(m) }
            })
            .collect();
        let np = c.n();
        let pairs = (0..np).map(|_| (c.n(), c.n(), c.f())).collect();
        LinePrice { steps, pairs }
    }
}

/// Which of step `n`'s columns are bound when it runs after variables `bound`: a constant, a bound variable, a row
/// column, or a repeat of a variable this row binds first.
fn mask_of(n: &Node, bound: u64) -> usize {
    let mut here = 0u64;
    let mut m = 0usize;
    for (i, a) in n.args.iter().enumerate() {
        let b = match *a {
            V::Const(_) | V::Row(..) => true,
            V::Var(x) => bound >> x & 1 == 1 || here >> x & 1 == 1,
            V::Wild => false,
        };
        if let V::Var(x) = a {
            here |= 1 << x;
        }
        if b && i < 64 {
            m |= 1 << i;
        }
    }
    m
}

/// Step `j`'s rows out per row in, run after the steps of `placed`.
fn fan(c: &ChainVars, lp: &LinePrice, placed: u64, j: usize) -> f64 {
    let n = &c.nodes[j];
    let sp = &lp.steps[j];
    let bound = c.bound(placed);
    let mask = mask_of(n, bound);
    let corr = sp.corr.get(mask).copied().unwrap_or(1.0);
    let sel: f64 = (0..n.args.len()).filter(|i| mask >> i & 1 == 1).map(|i| 1.0 / sp.ndv.get(i).copied().unwrap_or(1.0).max(1.0)).product();
    match n.op {
        Op::Atom => {
            // An exact pair: b's bound variables all from one step a before it.
            let need: u64 = n.args.iter().filter_map(|a| if let V::Var(x) = a { (bound >> x & 1 == 1).then_some(1u64 << x) } else { None }).fold(0, |m, b| m | b);
            if need != 0
                && let Some(&(a, _, rows)) = lp.pairs.iter().find(|&&(a, b, _)| b == j && placed >> a & 1 == 1 && need & !c.nodes[a].binds == 0)
            {
                return rows / lp.steps[a].card.max(1.0) * corr;
            }
            sp.card * sel * corr
        }
        Op::Not => (1.0 - (sp.card * sel).min(1.0)).max(0.01) * corr,
        Op::Cmp(cmp) => match cmp {
            Cmp::Eq => 0.1,
            Cmp::Ne => 0.9,
            _ => 1.0 / 3.0,
        },
    }
}

/// The rows a chain's first step makes: an atom's rows under its constants; a filter's one tuple, kept or not.
fn start_rows(c: &ChainVars, lp: &LinePrice, j: usize) -> f64 {
    match c.nodes[j].op {
        Op::Atom => fan(c, lp, 0, j),
        _ => fan(c, lp, 0, j).min(1.0),
    }
}

/// An order's rows after each of its steps, from `rows_in` tuples after its first `from` steps (none: its first
/// step's own rows).
fn rows_along(c: &ChainVars, lp: &LinePrice, order: &[usize], from: usize, rows_in: f64) -> Vec<f64> {
    let mut out = Vec::new();
    let mut placed = 0u64;
    let mut rows = rows_in;
    for (p, &j) in order.iter().enumerate() {
        if p < from {
            placed |= 1 << j;
            out.push(if p + 1 == from { rows_in } else { 0.0 });
            continue;
        }
        rows = if p == 0 { start_rows(c, lp, j) } else { rows * fan(c, lp, placed, j) };
        out.push(rows);
        placed |= 1 << j;
    }
    out
}

// ---- Search: the dynamic program over subsets, greedy past `GREEDY`.

#[derive(Clone, Debug, PartialEq)]
struct Searched {
    order: Vec<usize>,
    price: f64,
    est: Vec<f64>,
    /// The best order ending in each other step, cheapest first.
    runners: Vec<(Vec<usize>, f64)>,
}

/// The cheapest valid order of chain `c` after its `prefix` (its steps already run, `rows_in` tuples there; empty: a
/// round's start, its fixed first step kept first), and the runners-up.
fn search(c: &ChainVars, lp: &LinePrice, prefix: &[usize], rows_in: f64) -> Option<Searched> {
    let n = c.len();
    if n == 0 {
        return None;
    }
    let mut prefix = prefix.to_vec();
    let mut rows0 = rows_in;
    if prefix.is_empty() && c.fixed {
        prefix.push(0);
        rows0 = start_rows(c, lp, 0);
    }
    let pmask = prefix.iter().fold(0u64, |m, &j| m | 1 << j);
    let free = n - prefix.len();
    // An order's price: the rows it makes after each step, its prefix's tuples counted once.
    let priced = |est: &[f64]| if prefix.is_empty() { est.iter().sum() } else { est[prefix.len()..].iter().sum::<f64>() + rows0 };
    let price_of = |order: &[usize]| priced(&rows_along(c, lp, order, prefix.len(), rows0));
    let done = |order: Vec<usize>, runners: Vec<(Vec<usize>, f64)>| {
        let est = rows_along(c, lp, &order, prefix.len(), rows0);
        let price = priced(&est);
        Some(Searched { order, price, est, runners })
    };
    if free == 0 {
        return done(prefix.clone(), Vec::new());
    }
    if free > GREEDY {
        let mut order = prefix.clone();
        let mut placed = pmask;
        let mut rows = rows0;
        while order.len() < n {
            let best = (0..n).filter(|&j| placed >> j & 1 == 0 && c.ready(placed, j)).map(|j| (if order.is_empty() { start_rows(c, lp, j) } else { rows * fan(c, lp, placed, j) }, j)).min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)))?;
            rows = best.0;
            placed |= 1 << best.1;
            order.push(best.1);
        }
        return done(order, Vec::new());
    }
    // The free steps, indexed 0..free; a state is (the free steps placed, the last placed), its cost and rows and
    // the last before it.
    let fs: Vec<usize> = (0..n).filter(|&j| pmask >> j & 1 == 0).collect();
    let full = (1usize << free) - 1;
    let mut st: Vec<Option<(f64, f64, usize)>> = vec![None; (full + 1) * free];
    let placed_of = |s: usize| fs.iter().enumerate().filter(|(i, _)| s >> i & 1 == 1).fold(pmask, |m, (_, &j)| m | 1 << j);
    for (i, &j) in fs.iter().enumerate() {
        if c.ready(pmask, j) {
            let r = if prefix.is_empty() { start_rows(c, lp, j) } else { rows0 * fan(c, lp, pmask, j) };
            st[(1 << i) * free + i] = Some((r, r, usize::MAX));
        }
    }
    for s in 1..=full {
        let placed = placed_of(s);
        for last in 0..free {
            let Some((cost, rows, _)) = st[s * free + last] else { continue };
            for (i, &j) in fs.iter().enumerate() {
                if s >> i & 1 == 1 || !c.ready(placed, j) {
                    continue;
                }
                let r = rows * fan(c, lp, placed, j);
                let to = (s | 1 << i) * free + i;
                if st[to].is_none_or(|(c0, _, _)| cost + r < c0) {
                    st[to] = Some((cost + r, r, last));
                }
            }
        }
    }
    let order_of = |last: usize| {
        let mut out = Vec::new();
        let (mut s, mut l) = (full, last);
        while l != usize::MAX {
            out.push(fs[l]);
            let prev = st[s * free + l].expect("reached").2;
            s &= !(1 << l);
            l = prev;
        }
        out.reverse();
        prefix.iter().copied().chain(out).collect::<Vec<usize>>()
    };
    let mut ends: Vec<(f64, usize)> = (0..free).filter_map(|l| st[full * free + l].map(|(cost, _, _)| (cost, l))).collect();
    ends.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    let (_, best) = *ends.first()?;
    let runners = ends[1..].iter().take(RUNNERS_UP).map(|&(_, l)| {
        let o = order_of(l);
        let p = price_of(&o);
        (o, p)
    });
    let runners: Vec<(Vec<usize>, f64)> = runners.collect();
    done(order_of(best), runners)
}

// ---- The memo: what a line's planning keeps across rounds and runs.

/// A step's sample (decision 0354's training row): its place, its columns bound, its price's rows in and fanout, its
/// rows in and out as run, its relation's rows and its bound columns' most distinct values.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sample {
    pub step: usize,
    pub pos: usize,
    pub mask: u64,
    pub est_in: f64,
    pub est_fan: f64,
    pub actual_in: f64,
    pub actual_out: f64,
    pub card: f64,
    pub key_ndv: f64,
}

/// A line's memo: its order and price, the rows it priced after each step, its last work and its first step's rows
/// then, its runs and how often it ran slow; each order tried with its measured work over its price; the runners-up;
/// the exact pairs measured for it; and its steps' last samples.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Note {
    pub order: Vec<usize>,
    pub price: f64,
    pub est: Vec<f64>,
    pub work: f64,
    pub start: f64,
    pub runs: u64,
    pub slow: u64,
    pub tried: Vec<(Vec<usize>, f64)>,
    pub runners: Vec<(Vec<usize>, f64)>,
    pub pairs: Vec<(usize, usize, f64)>,
    pub samples: Vec<Sample>,
}

impl Note {
    fn words(&self) -> Vec<i64> {
        let n = self.order.len();
        let mut w = vec![n as i64];
        w.extend(self.order.iter().map(|&j| j as i64));
        w.extend([fb(self.price), fb(self.work), fb(self.start), self.runs as i64, self.slow as i64]);
        w.extend(self.est.iter().map(|&x| fb(x)));
        for list in [&self.tried, &self.runners] {
            w.push(list.len() as i64);
            for (o, x) in list {
                w.extend(o.iter().map(|&j| j as i64));
                w.push(fb(*x));
            }
        }
        w.push(self.pairs.len() as i64);
        w.extend(self.pairs.iter().flat_map(|&(a, b, r)| [a as i64, b as i64, fb(r)]));
        w.push(self.samples.len() as i64);
        for s in &self.samples {
            w.extend([s.step as i64, s.pos as i64, s.mask as i64, fb(s.est_in), fb(s.est_fan), fb(s.actual_in), fb(s.actual_out), fb(s.card), fb(s.key_ndv)]);
        }
        w
    }

    fn of_words(w: &[i64]) -> Note {
        let mut c = Cur::new(w);
        let n = c.n();
        let order = c.vec(n);
        let (price, work, start, runs, slow) = (c.f(), c.f(), c.f(), c.next() as u64, c.next() as u64);
        let est = c.fs(n);
        let mut lists = [Vec::new(), Vec::new()];
        for l in &mut lists {
            for _ in 0..c.n() {
                let o = c.vec(n);
                l.push((o, c.f()));
            }
        }
        let [tried, runners] = lists;
        let np = c.n();
        let pairs = (0..np).map(|_| (c.n(), c.n(), c.f())).collect();
        let ns = c.n();
        let samples = (0..ns)
            .map(|_| Sample { step: c.n(), pos: c.n(), mask: c.next() as u64, est_in: c.f(), est_fan: c.f(), actual_in: c.f(), actual_out: c.f(), card: c.f(), key_ndv: c.f() })
            .collect();
        Note { order, price, est, work, start, runs, slow, tried, runners, pairs, samples }
    }

    /// Line `key`'s note in memo `memo`, if it holds one of a chain of `n` steps.
    fn of(memo: &[u8], key: i64, n: usize) -> Option<Note> {
        let note = Note::of_words(memo_twin::line(memo, key as u64)?);
        (note.order.len() == n).then_some(note)
    }

    /// The measured ratio of order `o`, when it ran.
    fn ratio(&self, o: &[usize]) -> Option<f64> {
        self.tried.iter().find(|t| t.0 == o).map(|t| t.1)
    }
}

/// The memo's notes, each line's by its key (a reader's: `Memo`'s body).
pub fn notes(memo: &[u8]) -> Vec<(u64, Note)> {
    memo_twin::entries(memo)
        .filter(|e| e.layer == memo_twin::MEMO)
        .map(|e| {
            let key = e.cols.iter().enumerate().fold(0u64, |k, (i, &c)| k | u64::from(c) << (16 * i));
            (key, Note::of_words(memo_twin::line(memo, key).expect("listed")))
        })
        .collect()
}

/// The memo's steps' samples, as the learned join model trains on them (decision 0354: the label what the run
/// measured; the estimate the statistics', a feature and what the target is a ratio to).
pub fn samples(memo: &[u8]) -> Vec<rerail_accel::join_model::StepSample> {
    notes(memo)
        .into_iter()
        .flat_map(|(key, n)| {
            let steps = n.order.len() as u32;
            n.samples.into_iter().map(move |s| sample_of(&s, key, steps))
        })
        .collect()
}

fn sample_of(s: &Sample, key: u64, steps: u32) -> rerail_accel::join_model::StepSample {
    rerail_accel::join_model::StepSample {
        basis: "statistics".into(),
        access: if s.mask == 0 { "scan" } else { "lookup" }.into(),
        source: "trace".into(),
        bound: s.mask.count_ones(),
        step: s.pos as u32,
        steps,
        est_in: s.est_in,
        est_fan: s.est_fan,
        card: s.card,
        solving: false,
        seed: false,
        literal: format!("{key:x}:{}", s.step),
        key_ndv: s.key_ndv,
        actual_in: s.actual_in,
        actual_out: s.actual_out,
        relation: String::new(),
        bound_mask: s.mask,
        cols: 0,
        origin: "ran".into(),
    }
}

/// The memo as an operator's cache body: its notes read where they lie.
pub struct Memo;

/// `Memo` read where it lies.
pub struct MemoRead<'a>(&'a [u8]);

impl MemoRead<'_> {
    pub fn notes(&self) -> Vec<(u64, Note)> {
        notes(self.0)
    }
}

impl crate::contract::CacheBody for Memo {
    type Read<'a> = MemoRead<'a>;
    fn read(bytes: &[u8]) -> MemoRead<'_> {
        MemoRead(bytes)
    }
}

// ---- The operations' twins.

// A line's status in the lines block.
const NONE: i64 = 0;
const ONE: i64 = 1;
const KEPT: i64 = 2;
const PRICE: i64 = 3;

fn seed_draw(key: i64, runs: u64) -> f64 {
    let mut h = SEED ^ (key as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ runs.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    h ^= h >> 31;
    h = h.wrapping_mul(0x94d0_49bb_1331_11eb);
    h ^= h >> 29;
    (h >> 11) as f64 / (1u64 << 53) as f64
}

/// `lines`: read the plan, atoms, memo · write lines: each chain's status, its first step's rows, its kept order.
fn lines(c: &mut Ctx<'_, '_, '_>, _: Tile) {
    let table = c.table;
    let ([plan, atoms, memo], [out]) = c.views([0, 1, 2], [3]);
    let names = Names::new(atoms);
    let (p, keys) = plan_and_keys(plan);
    let env = Env::new(table, plan, &names);
    let mut w = vec![p.terms.len() as i64];
    for (ti, t) in p.terms.iter().enumerate() {
        let chain = ChainVars::of(t);
        let note = chain.as_ref().and_then(|ch| Note::of(memo, keys[ti], ch.len()));
        let start = |j: usize| {
            let s = &t.steps[j];
            let (rows, _) = env.side_stats(s.src, s.side, 0);
            rows as f64
        };
        let (status, rows, kept) = match &chain {
            None => (ONE, 0.0, Vec::new()),
            Some(ch) if ch.fixed && start(0) == 0.0 => (NONE, 0.0, Vec::new()),
            Some(ch) if ch.len() - usize::from(ch.fixed) <= 1 => (ONE, 0.0, Vec::new()),
            Some(ch) => {
                let first = note.as_ref().map_or(0, |n| n.order[0]);
                let rows = start(if ch.fixed { 0 } else { first });
                match &note {
                    Some(n) if ch.valid(&n.order) && n.work * (rows.max(1.0) / n.start.max(1.0)) < SEARCH_WORK => (KEPT, rows, n.order.clone()),
                    _ => (PRICE, rows, Vec::new()),
                }
            }
        };
        w.extend([status, fb(rows), kept.len() as i64]);
        w.extend(kept.iter().map(|&j| j as i64));
    }
    put_words(out, &w);
}

struct LineInfo {
    status: i64,
    kept: Vec<usize>,
}

fn lines_of(b: &[u8]) -> Vec<LineInfo> {
    let w = words_of(b);
    let mut c = Cur::new(&w);
    (0..c.n())
        .map(|_| {
            let status = c.next();
            let _rows = c.f();
            let k = c.n();
            LineInfo { status, kept: c.vec(k) }
        })
        .collect()
}

/// A handoff's running lines (`running`): each line's tuples at the depth it was cut at, and whether it was.
struct RunningLine {
    work: f64,
    due: f64,
    limit: f64,
    slow: bool,
    cut: usize,
    /// How many times it ran slow this round (a handoff clears `slow`, so the checks go on).
    times: u64,
    counts: Vec<f64>,
}

fn running_of(b: &[u8]) -> Vec<RunningLine> {
    running_of_words(&words_of(b))
}

fn running_words(lines: &[RunningLine], width: usize) -> Vec<i64> {
    let mut w = vec![lines.len() as i64, width as i64];
    for l in lines {
        w.extend([fb(l.work), fb(l.due), fb(l.limit), i64::from(l.slow), l.cut as i64, l.times as i64]);
        w.extend(l.counts.iter().map(|&x| fb(x)));
    }
    w
}

/// The chosen order of each chain (its rule-order indices), its price and the rows priced after each step, and
/// whether it explored.
#[derive(Clone, Debug, PartialEq)]
struct Choice {
    order: Vec<usize>,
    price: f64,
    est: Vec<f64>,
    explored: bool,
}

fn choices_of(b: &[u8]) -> Vec<Choice> {
    let w = words_of(b);
    if w.is_empty() {
        return Vec::new();
    }
    let mut c = Cur::new(&w);
    (0..c.n())
        .map(|_| {
            let n = c.n();
            let order = c.vec(n);
            let price = c.f();
            let est = c.fs(n);
            Choice { order, price, est, explored: c.next() != 0 }
        })
        .collect()
}

fn choice_words(cs: &[Choice]) -> Vec<i64> {
    let mut w = vec![cs.len() as i64];
    for ch in cs {
        w.push(ch.order.len() as i64);
        w.extend(ch.order.iter().map(|&j| j as i64));
        w.push(fb(ch.price));
        w.extend(ch.est.iter().map(|&x| fb(x)));
        w.push(i64::from(ch.explored));
    }
    w
}

/// The lines a round prices: every `PRICE` line, or at a handoff, the slow ones.
fn priced(lines: &[LineInfo], running: &[RunningLine], ti: usize) -> bool {
    if running.is_empty() { lines[ti].status == PRICE } else { running[ti].slow }
}

/// `price`: read the plan, atoms, memo, lines, running · write prices: each priced chain's steps' rows and distinct
/// values, and its exact pairs.
fn price(c: &mut Ctx<'_, '_, '_>, _: Tile) {
    let table = c.table;
    let ([plan, atoms, memo, lines, running], [out]) = c.views([0, 1, 2, 3, 4], [5]);
    let names = Names::new(atoms);
    let (p, keys) = plan_and_keys(plan);
    let env = Env::new(table, plan, &names);
    let (ls, rs) = (lines_of(lines), running_of(running));
    let mut w = vec![p.terms.len() as i64];
    for (ti, t) in p.terms.iter().enumerate() {
        if !priced(&ls, &rs, ti) {
            w.push(0);
            continue;
        }
        w.push(1);
        let steps = t
            .steps
            .iter()
            .map(|s| {
                let (rows, ndv) = env.side_stats(s.src, s.side, s.args.len());
                StepPrice { card: rows as f64, ndv: ndv.into_iter().map(|x| x as f64).collect(), corr: Vec::new() }
            })
            .collect();
        let pairs = Note::of(memo, keys[ti], t.steps.len()).map(|n| n.pairs).unwrap_or_default();
        LinePrice { steps, pairs }.words(&mut w);
    }
    put_words(out, &w);
}

fn prices_of(b: &[u8]) -> Vec<Option<LinePrice>> {
    let w = words_of(b);
    if w.is_empty() {
        return Vec::new();
    }
    let mut c = Cur::new(&w);
    (0..c.n()).map(|_| (c.next() == 1).then(|| LinePrice::read(&mut c))).collect()
}

/// `search`: read the plan, lines, prices, corrections, running, the running choice · write the searched orders.
fn search_op(c: &mut Ctx<'_, '_, '_>, _: Tile) {
    let ([plan, lines, prices, corr, running, now], [out]) = c.views([0, 1, 2, 3, 4, 5], [6]);
    let (p, _) = plan_and_keys(plan);
    let (ls, rs, now) = (lines_of(lines), running_of(running), choices_of(now));
    let mut lps = prices_of(prices);
    // The learned correction, each step's at each set of its columns bound (`correct`).
    let cw = words_of(corr);
    if !cw.is_empty() {
        let mut cc = Cur::new(&cw);
        for lp in lps.iter_mut() {
            let Some(lp) = lp else { continue };
            for s in &mut lp.steps {
                let m = cc.n();
                s.corr = cc.fs(m);
            }
        }
    }
    let mut w = vec![p.terms.len() as i64];
    for (ti, t) in p.terms.iter().enumerate() {
        let found = match (&lps.get(ti), ChainVars::of(t)) {
            (Some(Some(lp)), Some(ch)) if priced(&ls, &rs, ti) => match rs.get(ti) {
                Some(r) => {
                    let prefix = &now[ti].order[..=r.cut];
                    search(&ch, lp, prefix, r.counts[r.cut])
                }
                None => search(&ch, lp, &[], 0.0),
            },
            _ => None,
        };
        match found {
            None => w.push(0),
            Some(s) => {
                w.extend([1, fb(s.price), s.order.len() as i64]);
                w.extend(s.order.iter().map(|&j| j as i64));
                w.extend(s.est.iter().map(|&x| fb(x)));
                w.push(s.runners.len() as i64);
                for (o, x) in &s.runners {
                    w.push(fb(*x));
                    w.extend(o.iter().map(|&j| j as i64));
                }
            }
        }
    }
    put_words(out, &w);
}

fn searched_of(b: &[u8]) -> Vec<Option<Searched>> {
    let w = words_of(b);
    if w.is_empty() {
        return Vec::new();
    }
    let mut c = Cur::new(&w);
    (0..c.n())
        .map(|_| {
            (c.next() == 1).then(|| {
                let price = c.f();
                let n = c.n();
                let order = c.vec(n);
                let est = c.fs(n);
                let runners = (0..c.n())
                    .map(|_| {
                        let x = c.f();
                        (c.vec(n), x)
                    })
                    .collect();
                Searched { order, price, est, runners }
            })
        })
        .collect()
}

/// `choose`: read the plan, lines, search, memo, trials, the running choice · write the choice.
fn choose(c: &mut Ctx<'_, '_, '_>, _: Tile) {
    let ([plan, lines, search, memo, trials, now], [out]) = c.views([0, 1, 2, 3, 4, 5], [6]);
    let (p, keys) = plan_and_keys(plan);
    let (ls, ss, now) = (lines_of(lines), searched_of(search), choices_of(now));
    // A handoff's trials: per line, each candidate's measured work per sample row (candidate 0: the running order).
    let tw = words_of(trials);
    let mut measured: Vec<Vec<f64>> = Vec::new();
    if !tw.is_empty() {
        let mut tc = Cur::new(&tw);
        for _ in 0..tc.n() {
            let k = tc.n();
            measured.push(tc.fs(k));
        }
    }
    let mut cs = Vec::new();
    for (ti, t) in p.terms.iter().enumerate() {
        let rule: Vec<usize> = (0..t.steps.len()).collect();
        let note = Note::of(memo, keys[ti], t.steps.len());
        let ch = match (ss.get(ti).cloned().flatten(), now.get(ti)) {
            // A handoff: the order a trial measured least, where it beats the running one by the margin.
            (Some(s), Some(run)) if !now.is_empty() => {
                let m = measured.get(ti).cloned().unwrap_or_default();
                let cands: Vec<(Vec<usize>, f64)> = std::iter::once((run.order.clone(), run.price)).chain(std::iter::once((s.order.clone(), s.price))).chain(s.runners.iter().cloned()).collect();
                let best = (0..cands.len().min(m.len())).min_by(|&a, &b| m[a].total_cmp(&m[b])).unwrap_or(0);
                if best != 0 && m[best] * TRIAL_MARGIN < m[0] {
                    Choice { order: cands[best].0.clone(), price: cands[best].1, est: s.est.clone(), explored: false }
                } else {
                    run.clone()
                }
            }
            (_, Some(run)) if !now.is_empty() => run.clone(),
            (Some(s), _) => {
                let corrected = |o: &[usize], price: f64| price * note.as_ref().and_then(|n| n.ratio(o)).unwrap_or(1.0);
                let mut cands: Vec<(Vec<usize>, f64, f64)> = std::iter::once((s.order.clone(), s.price)).chain(s.runners.iter().cloned()).map(|(o, x)| (o.clone(), x, corrected(&o, x))).collect();
                cands.sort_by(|a, b| a.2.total_cmp(&b.2));
                let best = cands[0].clone();
                let runs = note.as_ref().map_or(0, |n| n.runs);
                let untried = cands.iter().skip(1).find(|c| c.1 <= MARGIN * best.1 && note.as_ref().is_none_or(|n| n.ratio(&c.0).is_none()));
                match untried {
                    Some(u) if best.1 < EXPLORE_ROWS && seed_draw(keys[ti], runs) < EXPLORE_RATE => Choice { order: u.0.clone(), price: u.1, est: s.est.clone(), explored: true },
                    _ => {
                        let est = if best.0 == s.order { s.est.clone() } else { vec![0.0; best.0.len()] };
                        Choice { order: best.0, price: best.1, est, explored: false }
                    }
                }
            }
            _ => match (ls[ti].status, &note) {
                (KEPT, Some(n)) => Choice { order: ls[ti].kept.clone(), price: n.price, est: n.est.clone(), explored: false },
                _ => Choice { order: rule.clone(), price: 0.0, est: vec![0.0; rule.len()], explored: false },
            },
        };
        cs.push(ch);
    }
    put_words(out, &choice_words(&cs));
}

/// `emit_plan`: read the rule-order plan, choice, search, lines · rw memo · write the planned plan: each chain's
/// steps in its chosen order, and each searched line's note.
fn emit_plan(c: &mut Ctx<'_, '_, '_>, _: Tile) {
    let (p, keys, cs, ss, notes) = {
        let ([plan, choice, search, memo], []) = c.views([0, 1, 2, 4], []);
        let (p, keys) = plan_and_keys(plan);
        let (cs, ss) = (choices_of(choice), searched_of(search));
        let notes: Vec<Option<Note>> = p.terms.iter().enumerate().map(|(ti, t)| Note::of(memo, keys[ti], t.steps.len())).collect();
        (p, keys, cs, ss, notes)
    };
    let mut planned = Plan { srcs: p.srcs.clone(), docs: p.docs.clone(), terms: Vec::new() };
    for (ti, t) in p.terms.iter().enumerate() {
        let ch = ChainVars::of(t);
        let order = &cs[ti].order;
        let term = match &ch {
            Some(ch) if ch.valid(order) => ch.rebuild(t, order),
            _ => t.clone(),
        };
        planned.terms.push(term);
    }
    for (ti, s) in ss.iter().enumerate() {
        let Some(s) = s else { continue };
        let mut note = notes[ti].clone().unwrap_or_default();
        note.order = cs[ti].order.clone();
        note.price = cs[ti].price;
        note.est = if cs[ti].est.len() == note.order.len() { cs[ti].est.clone() } else { s.est.clone() };
        note.runners = s.runners.clone();
        if !memo_twin::keep_line(c, 4, keys[ti] as u64, &note.words()) {
            return;
        }
    }
    let w: Vec<i64> = planned.words().into_iter().chain(std::iter::once(keys.len() as i64)).chain(keys.iter().copied()).collect();
    let ([], [out]) = c.views([], [3]);
    put_words(out, &w);
}

/// `check`: read the plan, choice, frames · rw running (its first word the depth just run): each line's tuples
/// counted at that depth; past its due point, slow when its work passed its limit.
fn check(c: &mut Ctx<'_, '_, '_>, _: Tile) {
    let ([plan, frames], [running]) = c.views([0, 2], [3]);
    let (p, _) = plan_and_keys(plan);
    let mut w = words_of(running);
    let d = w.remove(0) as usize;
    let mut rs = running_of_words(&w);
    let mut n = vec![0f64; rs.len()];
    // A chain's tuples made at depth `d`: a chain done before it passes its tuples on, made then.
    for (ti, _, _) in body::tuples(frames) {
        if d < p.terms[ti].steps.len() {
            n[ti] += 1.0;
        }
    }
    for (ti, r) in rs.iter_mut().enumerate() {
        r.counts[d] = n[ti];
        r.work += n[ti];
        if r.work >= r.due {
            while r.due <= r.work {
                r.due *= 2.0;
            }
            if r.work > r.limit && !r.slow {
                r.slow = true;
                r.times += 1;
                r.cut = d;
            }
        }
    }
    let width = rs.first().map_or(0, |r| r.counts.len());
    let mut out = vec![d as i64];
    out.extend(running_words(&rs, width));
    put_words(running, &out);
}

fn running_of_words(w: &[i64]) -> Vec<RunningLine> {
    if w.is_empty() {
        return Vec::new();
    }
    let mut c = Cur::new(w);
    let (n, width) = (c.n(), c.n());
    (0..n).map(|_| RunningLine { work: c.f(), due: c.f(), limit: c.f(), slow: c.next() != 0, cut: c.n(), times: c.next() as u64, counts: c.fs(width) }).collect()
}

/// `stats`: read the rule-order plan, atoms, running · rw memo: each slow line's exact pairs.
fn stats(c: &mut Ctx<'_, '_, '_>, _: Tile) {
    let table = c.table;
    let notes = {
        let ([plan, atoms, running, memo], []) = c.views([0, 1, 2, 3], []);
        let names = Names::new(atoms);
        let (p, keys) = plan_and_keys(plan);
        let env = Env::new(table, plan, &names);
        let mut w = words_of(running);
        w.remove(0);
        let rs = running_of_words(&w);
        let mut notes = Vec::new();
        for (ti, t) in p.terms.iter().enumerate() {
            if !rs.get(ti).is_some_and(|r| r.slow) {
                continue;
            }
            let Some(ch) = ChainVars::of(t) else { continue };
            let mut note = Note::of(memo, keys[ti], t.steps.len()).unwrap_or_else(|| Note { order: (0..t.steps.len()).collect(), est: vec![0.0; t.steps.len()], ..Note::default() });
            note.pairs.clear();
            for a in 0..ch.len() {
                for b in 0..ch.len() {
                    let (na, nb) = (&ch.nodes[a], &ch.nodes[b]);
                    if a == b || na.op != Op::Atom || nb.op != Op::Atom || na.side == Side::Delta || nb.side == Side::Delta {
                        continue;
                    }
                    let on: Vec<(usize, usize)> = nb.args.iter().enumerate().filter_map(|(ib, x)| if let V::Var(v) = x { na.args.iter().position(|y| y == &V::Var(*v)).map(|ia| (ia, ib)) } else { None }).collect();
                    if on.is_empty() {
                        continue;
                    }
                    note.pairs.push((a, b, env.pair_rows(&t.steps[a], &t.steps[b], &on) as f64));
                }
            }
            notes.push((keys[ti], note));
        }
        notes
    };
    for (key, note) in notes {
        if !memo_twin::keep_line(c, 3, key as u64, &note.words()) {
            return;
        }
    }
}

/// `note`: read the plan (as run), choice, running · rw memo: what each line measured.
fn note(c: &mut Ctx<'_, '_, '_>, _: Tile) {
    let notes = {
        let ([plan, choice, running, memo], []) = c.views([0, 1, 2, 3], []);
        let (p, keys) = plan_and_keys(plan);
        let cs = choices_of(choice);
        let mut w = words_of(running);
        w.remove(0);
        let rs = running_of_words(&w);
        let mut notes = Vec::new();
        for (ti, t) in p.terms.iter().enumerate() {
            let (Some(ch), Some(r)) = (cs.get(ti), rs.get(ti)) else { continue };
            if t.steps.len() < 2 || r.work == 0.0 {
                continue;
            }
            let mut note = Note::of(memo, keys[ti], t.steps.len()).unwrap_or_else(|| Note { order: ch.order.clone(), price: ch.price, est: ch.est.clone(), ..Note::default() });
            note.order = ch.order.clone();
            note.price = ch.price;
            if ch.est.len() == note.order.len() {
                note.est = ch.est.clone();
            }
            note.work = r.work;
            note.start = r.counts[0];
            note.runs += 1;
            note.slow += r.times;
            if ch.price > 0.0 {
                let ratio = r.work / ch.price;
                match note.tried.iter_mut().find(|x| x.0 == ch.order) {
                    Some(x) => x.1 = ratio,
                    None => note.tried.push((ch.order.clone(), ratio)),
                }
            }
            note.samples = (1..ch.order.len())
                .map(|pos| {
                    let est_in = ch.est[pos - 1];
                    Sample { step: ch.order[pos], pos, mask: 0, est_in, est_fan: if est_in > 0.0 { ch.est[pos] / est_in } else { 0.0 }, actual_in: r.counts[pos - 1], actual_out: r.counts[pos], card: 0.0, key_ndv: 0.0 }
                })
                .collect();
            notes.push((keys[ti], note));
        }
        notes
    };
    for (key, note) in notes {
        if !memo_twin::keep_line(c, 3, key as u64, &note.words()) {
            return;
        }
    }
}

fn one(_: &Ctx<'_, '_, '_>) -> u32 {
    1
}

const fn phase(name: &'static str, inside: Inside, output: Output, bounds: &'static [Bound]) -> Phase {
    Phase { name, found: Found::Fixed, unit: Unit::Threadgroup, inside, across: Across::Nothing, output, bounds }
}

static EMIT: [Phase; 1] = [phase("emit", Inside::Nothing, Output::ByScan, &[])];
static MAP: [Phase; 1] = [phase("map", Inside::Nothing, Output::ByIndex, &[])];
/// A partition's program (29): a line a group, its subsets in the group's memory.
static GROUP: [Phase; 1] = [phase("search", Inside::GroupMemory, Output::ByIndex, &[])];
/// A memo's write: within the cache's reservation, grown and run again past its capacity.
static EMIT_MEMO: [Phase; 1] = [phase("emit", Inside::Nothing, Output::WriterState, &[Bound::Capacity(Over::Grow)])];
static MAP_MEMO: [Phase; 1] = [phase("map", Inside::Nothing, Output::WriterState, &[Bound::Capacity(Over::Grow)])];

const fn s(name: &'static str, mode: Mode, kind: u32) -> SlotSpec {
    SlotSpec { name, mode, kind }
}

const R: Mode = Mode::Read;
const W: Mode = Mode::Write;
const RW: Mode = Mode::ReadWrite;
const CTL: u32 = kind::CONTROL;

static LINES_SLOTS: [SlotSpec; 4] = [s("plan", R, CTL), s("atoms", R, kind::ATOMS), s("memo", R, kind::CACHE), s("lines", W, CTL)];
static PRICE_SLOTS: [SlotSpec; 6] = [s("plan", R, CTL), s("atoms", R, kind::ATOMS), s("memo", R, kind::CACHE), s("lines", R, CTL), s("running", R, CTL), s("prices", W, CTL)];
static SEARCH_SLOTS: [SlotSpec; 7] = [s("plan", R, CTL), s("lines", R, CTL), s("prices", R, CTL), s("corrections", R, CTL), s("running", R, CTL), s("running choice", R, CTL), s("searched", W, CTL)];
static CHOOSE_SLOTS: [SlotSpec; 7] = [s("plan", R, CTL), s("lines", R, CTL), s("searched", R, CTL), s("memo", R, kind::CACHE), s("trials", R, CTL), s("running choice", R, CTL), s("choice", W, CTL)];
static EMIT_SLOTS: [SlotSpec; 5] = [s("plan", R, CTL), s("choice", R, CTL), s("searched", R, CTL), s("planned", W, CTL), s("memo", RW, kind::CACHE)];
static CHECK_SLOTS: [SlotSpec; 4] = [s("plan", R, CTL), s("choice", R, CTL), s("frames", R, kind::SCRATCH), s("running", RW, CTL)];
static STATS_SLOTS: [SlotSpec; 4] = [s("plan", R, CTL), s("atoms", R, kind::ATOMS), s("running", R, CTL), s("memo", RW, kind::CACHE)];
static NOTE_SLOTS: [SlotSpec; 4] = [s("plan", R, CTL), s("choice", R, CTL), s("running", R, CTL), s("memo", RW, kind::CACHE)];

/// A planning block's facts, by meaning: its words.
fn words_facts(_: &Table<'_>, _: usize, bytes: &[u8]) -> Vec<Vec<i64>> {
    vec![words_of(bytes)]
}

/// A memo's facts, by meaning: its notes' words, by key.
fn memo_facts(bytes: &[u8]) -> Vec<Vec<i64>> {
    let mut v: Vec<Vec<i64>> = notes(bytes).into_iter().map(|(k, n)| std::iter::once(k as i64).chain(n.words()).collect()).collect();
    v.sort();
    v
}

/// `emit_plan`'s slots' facts: its memo's (slot 4) by note, the rest by words.
fn emit_facts(t: &Table<'_>, slot: usize, bytes: &[u8]) -> Vec<Vec<i64>> {
    if slot == 4 { memo_facts(bytes) } else { words_facts(t, slot, bytes) }
}

/// `stats`' and `note`'s slots' facts: their memo's (slot 3) by note, the rest by words.
fn memo3_facts(t: &Table<'_>, slot: usize, bytes: &[u8]) -> Vec<Vec<i64>> {
    if slot == 3 { memo_facts(bytes) } else { words_facts(t, slot, bytes) }
}

const ALONG: Independence = Independence::Along("lines: a line's plan is its own");

pub static LINES: Signature = Signature::new("lines", &LINES_SLOTS, ParamsLayout::of::<NoParams>(), &EMIT, ALONG, words_facts);
pub static PRICE_OP: Signature = Signature::new("price", &PRICE_SLOTS, ParamsLayout::of::<NoParams>(), &MAP, ALONG, words_facts);
pub static SEARCH: Signature = Signature::new("search", &SEARCH_SLOTS, ParamsLayout::of::<NoParams>(), &GROUP, ALONG, words_facts);
pub static CHOOSE: Signature = Signature::new("choose", &CHOOSE_SLOTS, ParamsLayout::of::<NoParams>(), &MAP, ALONG, words_facts);
pub static EMIT_PLAN: Signature = Signature::new("emit_plan", &EMIT_SLOTS, ParamsLayout::of::<NoParams>(), &EMIT_MEMO, ALONG, emit_facts);
pub static CHECK: Signature = Signature::new("check", &CHECK_SLOTS, ParamsLayout::of::<NoParams>(), &MAP, ALONG, words_facts);
pub static STATS: Signature = Signature::new("stats", &STATS_SLOTS, ParamsLayout::of::<NoParams>(), &MAP_MEMO, ALONG, memo3_facts);
pub static NOTE: Signature = Signature::new("note", &NOTE_SLOTS, ParamsLayout::of::<NoParams>(), &MAP_MEMO, ALONG, memo3_facts);

static LINES_HOST: [TwinPhase; 1] = [TwinPhase { name: "emit", items: one, run: lines, again: None }];
static PRICE_HOST: [TwinPhase; 1] = [TwinPhase { name: "map", items: one, run: price, again: None }];
static SEARCH_HOST: [TwinPhase; 1] = [TwinPhase { name: "search", items: one, run: search_op, again: None }];
static CHOOSE_HOST: [TwinPhase; 1] = [TwinPhase { name: "map", items: one, run: choose, again: None }];
static EMIT_HOST: [TwinPhase; 1] = [TwinPhase { name: "emit", items: one, run: emit_plan, again: None }];
static CHECK_HOST: [TwinPhase; 1] = [TwinPhase { name: "map", items: one, run: check, again: None }];
static STATS_HOST: [TwinPhase; 1] = [TwinPhase { name: "map", items: one, run: stats, again: None }];
static NOTE_HOST: [TwinPhase; 1] = [TwinPhase { name: "map", items: one, run: note, again: None }];

/// The planning operations, declared, their host versions their references.
#[derive(Clone, Copy)]
pub struct PlanOps {
    pub lines: OpId,
    pub price: OpId,
    pub search: OpId,
    pub choose: OpId,
    pub emit: OpId,
    pub check: OpId,
    pub stats: OpId,
    pub note: OpId,
}

impl PlanOps {
    pub fn declare(reg: &mut Registry) -> Result<PlanOps, String> {
        let mut d = |sig: &'static Signature, name: &'static str, host: &'static [TwinPhase], shape: &'static [Phase]| -> Result<OpId, String> {
            let op = reg.declare(sig)?;
            reg.register_reference(Version::host(op, name, host, shape))?;
            Ok(op)
        };
        Ok(PlanOps {
            lines: d(&LINES, "lines on the host", &LINES_HOST, &EMIT)?,
            price: d(&PRICE_OP, "price on the host", &PRICE_HOST, &MAP)?,
            search: d(&SEARCH, "search on the host", &SEARCH_HOST, &GROUP)?,
            choose: d(&CHOOSE, "choose on the host", &CHOOSE_HOST, &MAP)?,
            emit: d(&EMIT_PLAN, "emit_plan on the host", &EMIT_HOST, &EMIT_MEMO)?,
            check: d(&CHECK, "check on the host", &CHECK_HOST, &MAP)?,
            stats: d(&STATS, "stats on the host", &STATS_HOST, &MAP_MEMO)?,
            note: d(&NOTE, "note on the host", &NOTE_HOST, &MAP_MEMO)?,
        })
    }
}

// ---- The round: the thin host loop that launches them.

/// A planning block of room for `n` words, open.
fn words_block(dir: &mut Directory, n: usize) -> BlockId {
    dir.make(kind::CONTROL, 8 + 8 * n)
}

/// A planning block holding `w`, made by the host: an empty input (`[]`), the running lines it starts, a handoff's
/// trials measured by launches, or the model's corrections.
fn given(dir: &mut Directory, w: &[i64]) -> Result<BlockId, String> {
    let id = words_block(dir, w.len().max(1));
    let b = dir.block(id).expect("just made");
    let mut pin = pin_block(&b, "a planning block", true)?;
    put_words(pin.bytes_mut().expect("a write pin"), w);
    Ok(id)
}

fn read_block<T>(dir: &Directory, id: BlockId, f: impl FnOnce(&[u8]) -> T) -> Result<T, String> {
    Ok(f(dir.read("a planning block", &[id])?.block(0)))
}

/// What one round of a stratum's chains runs with: the registry and the operations, the atoms, the memo (the
/// operator's cache block, open for writing) and the learned model.
pub struct Round<'a> {
    pub reg: &'a Registry,
    pub body: &'a BodyOps,
    pub ops: &'a PlanOps,
    pub atoms: BlockId,
    pub memo: BlockId,
    pub join: Option<&'a rerail_accel::join_model::JoinModel>,
}

/// The room a planning block of plan `p` takes: its plan's words, its keys, and a few words a step and mask.
fn room(p: &Plan) -> usize {
    let steps: usize = p.terms.iter().map(|t| t.steps.len()).sum();
    let masks: usize = p.terms.iter().flat_map(|t| &t.steps).map(|s| 1usize << s.args.len().min(12)).sum();
    p.words().len() + 4 * p.terms.len() + steps * (RUNNERS_UP + 8) * 4 + steps * 24 + masks + 64
}

impl Round<'_> {
    /// Plans `plan`'s chains (their steps in their rules' order, each chain's line key in `keys`) and runs them into
    /// the heads' runs `runs`, checking each depth: the probes made, and what the planning did.
    pub fn run(&self, dir: &mut Directory, plan: &Plan, keys: &[i64], runs: &[BlockId]) -> Result<(u64, Planned), String> {
        if plan.terms.is_empty() {
            return Ok((0, Planned::default()));
        }
        let mut made: Vec<BlockId> = Vec::new();
        let ran = self.run_in(dir, plan, keys, runs, &mut made);
        made.iter().for_each(|&b| dir.release(b));
        ran
    }

    fn launch(&self, dir: &mut Directory, op: OpId, slots: &[BlockId]) -> Result<(), String> {
        run_cpu(self.reg, dir, op, slots, &[], Tiling::WHOLE)
    }

    /// `op` into a new planning block of `n` words over `ins` (the out slot at `at`, the rest in order).
    fn into(&self, dir: &mut Directory, made: &mut Vec<BlockId>, op: OpId, ins: &[BlockId], at: usize, n: usize) -> Result<BlockId, String> {
        let out = words_block(dir, n);
        made.push(out);
        let mut slots = ins.to_vec();
        slots.insert(at, out);
        self.launch(dir, op, &slots)?;
        Ok(out)
    }

    fn run_in(&self, dir: &mut Directory, plan: &Plan, keys: &[i64], runs: &[BlockId], made: &mut Vec<BlockId>) -> Result<(u64, Planned), String> {
        let n = room(plan);
        let mut pd = Planned { lines: plan.terms.len() as u64, ..Planned::default() };
        let rule = body::plan_block_keyed(dir, plan, keys)?;
        made.push(rule);
        let empty = given(dir, &[])?;
        made.push(empty);
        let (atoms, memo) = (self.atoms, self.memo);
        let lines = self.into(dir, made, self.ops.lines, &[rule, atoms, memo], 3, n)?;
        let ls = read_block(dir, lines, lines_of)?;
        pd.kept += ls.iter().filter(|l| l.status == KEPT).count() as u64;
        let any = ls.iter().any(|l| l.status == PRICE);
        let (searched, corr) = if any {
            let prices = self.into(dir, made, self.ops.price, &[rule, atoms, memo, lines, empty], 5, n)?;
            pd.priced += read_block(dir, prices, |b| prices_of(b).iter().flatten().map(|lp| lp.steps.len() as u64).sum::<u64>())?;
            let corr = self.correct(dir, made, prices)?;
            let searched = self.into(dir, made, self.ops.search, &[rule, lines, prices, corr, empty, empty], 6, n)?;
            pd.searched += read_block(dir, searched, |b| searched_of(b).iter().flatten().count() as u64)?;
            (searched, corr)
        } else {
            (given(dir, &[0])?, empty)
        };
        made.push(searched);
        let _ = corr;
        let choice = self.into(dir, made, self.ops.choose, &[rule, lines, searched, memo, empty, empty], 6, n)?;
        pd.explored += read_block(dir, choice, |b| choices_of(b).iter().filter(|c| c.explored).count() as u64)?;
        let planned_b = self.planned(dir, made, rule, choice, searched, n)?;
        let planned = read_block(dir, planned_b, |b| plan_and_keys(b).0)?;
        // The run, checked after each depth.
        let width = planned.width();
        let cs = read_block(dir, choice, choices_of)?;
        let start: Vec<RunningLine> = cs.iter().map(|c| RunningLine { work: 0.0, due: SLOW_ROWS, limit: (SLOW_FACTOR * c.price).max(SLOW_ROWS), slow: false, cut: 0, times: 0, counts: vec![0.0; width] }).collect();
        let mut rw = vec![0i64];
        rw.extend(running_words(&start, width));
        let running = given(dir, &rw)?;
        made.push(running);
        let mut choice = choice;
        let probes = {
            let mut between = |dir: &mut Directory, d: usize, frames: BlockId, pb: BlockId| -> Result<BlockId, String> {
                {
                    let b = dir.block(running).expect("made");
                    let mut pin = pin_block(&b, "the running lines", true)?;
                    set_word::<i64>(pin.bytes_mut().expect("a write pin"), BODY_AT + 8, d as i64);
                }
                self.launch(dir, self.ops.check, &[pb, choice, frames, running])?;
                let rs = read_block(dir, running, |b| running_of_words(&words_of(b)[1..]))?;
                let slow: Vec<usize> = rs.iter().enumerate().filter(|(_, r)| r.slow && r.cut == d && d + 1 < width).map(|(ti, _)| ti).collect();
                if slow.is_empty() {
                    return Ok(pb);
                }
                pd.slow += slow.len() as u64;
                Ok(match self.handoff(dir, made, rule, choice, running, frames, &slow, d, n, &mut pd)? {
                    Some((nb, nc)) => {
                        choice = nc;
                        nb
                    }
                    None => pb,
                })
            };
            body::run_block(self.reg, self.body, dir, atoms, &planned, planned_b, runs, &mut between)?
        };
        // What each line measured, into the memo.
        self.launch(dir, self.ops.note, &[rule, choice, running, memo])?;
        Ok((probes, pd))
    }

    /// `emit_plan` into a new plan block, which then refers to every block its chains read and is frozen.
    fn planned(&self, dir: &mut Directory, made: &mut Vec<BlockId>, rule: BlockId, choice: BlockId, searched: BlockId, n: usize) -> Result<BlockId, String> {
        let out = words_block(dir, n);
        made.push(out);
        self.launch(dir, self.ops.emit, &[rule, choice, searched, out, self.memo])?;
        let p = read_block(dir, out, |b| plan_and_keys(b).0)?;
        for (b, at) in p.reads() {
            dir.refer(out, b, at)?;
        }
        dir.freeze(out)?;
        Ok(out)
    }

    /// The learned correction (0354), when a model is given: each priced step at every set of its columns bound, in
    /// one batch of the model's; the factor each target gives, a step's list in its columns' sets' order.
    fn correct(&self, dir: &mut Directory, made: &mut Vec<BlockId>, prices: BlockId) -> Result<BlockId, String> {
        let Some(model) = self.join else {
            let b = given(dir, &[])?;
            made.push(b);
            return Ok(b);
        };
        let lps = read_block(dir, prices, prices_of)?;
        let mut asked = Vec::new();
        for lp in lps.iter().flatten() {
            for (j, s) in lp.steps.iter().enumerate() {
                let k = s.ndv.len().min(12);
                for mask in 0..1u64 << k {
                    let sel: f64 = (0..k).filter(|i| mask >> i & 1 == 1).map(|i| 1.0 / s.ndv[i].max(1.0)).product();
                    let key_ndv = (0..k).filter(|i| mask >> i & 1 == 1).map(|i| s.ndv[i]).fold(0.0, f64::max);
                    let sample = Sample { step: j, pos: j, mask, est_in: 1.0, est_fan: s.card * sel, actual_in: 0.0, actual_out: 0.0, card: s.card, key_ndv };
                    asked.push(sample_of(&sample, 0, lp.steps.len() as u32));
                }
            }
        }
        let targets = model.fanouts(&asked.iter().collect::<Vec<_>>())?;
        let mut w = Vec::new();
        let mut at = 0;
        for lp in lps.iter().flatten() {
            for s in &lp.steps {
                let m = 1usize << s.ndv.len().min(12);
                w.push(m as i64);
                w.extend(targets[at..at + m].iter().map(|t| fb(t.clamp(-30.0, 30.0).exp())));
                at += m;
            }
        }
        let b = given(dir, &w)?;
        made.push(b);
        Ok(b)
    }

    /// A handoff (0206) at depth `d` for the slow lines `slow`: their exact pairs, their prices and searches from the
    /// steps they ran, their candidates trialled over a sample of their tuples there (0207), the choice, and a new
    /// plan block when any line's order changed (`None`: none did).
    #[allow(clippy::too_many_arguments)]
    fn handoff(&self, dir: &mut Directory, made: &mut Vec<BlockId>, rule: BlockId, choice: BlockId, running: BlockId, frames: BlockId, slow: &[usize], d: usize, n: usize, pd: &mut Planned) -> Result<Option<(BlockId, BlockId)>, String> {
        let (atoms, memo) = (self.atoms, self.memo);
        // The running block as `price` and `search` read it: without its depth word.
        let rs = read_block(dir, running, |b| running_of_words(&words_of(b)[1..]))?;
        let width = rs.first().map_or(0, |r| r.counts.len());
        let only_slow: Vec<RunningLine> = rs.iter().enumerate().map(|(ti, r)| RunningLine { slow: slow.contains(&ti), cut: d, counts: r.counts.clone(), ..*r }).collect();
        let rblock = given(dir, &running_words(&only_slow, width))?;
        made.push(rblock);
        let mut sw = vec![0i64];
        sw.extend(running_words(&only_slow, width));
        let sblock = given(dir, &sw)?;
        made.push(sblock);
        self.launch(dir, self.ops.stats, &[rule, atoms, sblock, memo])?;
        pd.pairs += read_block(dir, memo, |b| notes(b).iter().map(|(_, n)| n.pairs.len() as u64).sum::<u64>())?;
        let empty = given(dir, &[])?;
        made.push(empty);
        let lines = given(dir, &[0])?;
        made.push(lines);
        let lw = {
            let p = read_block(dir, rule, |b| plan_and_keys(b).0)?;
            let mut w = vec![p.terms.len() as i64];
            for _ in &p.terms {
                w.extend([PRICE, fb(0.0), 0]);
            }
            w
        };
        let lines = {
            let b = given(dir, &lw)?;
            made.push(b);
            b
        };
        let prices = self.into(dir, made, self.ops.price, &[rule, atoms, memo, lines, rblock], 5, n)?;
        let corr = self.correct(dir, made, prices)?;
        let searched = self.into(dir, made, self.ops.search, &[rule, lines, prices, corr, rblock, choice], 6, n)?;
        // The trials: each line's running order and its candidates over a sample of its tuples at depth `d`.
        let ss = read_block(dir, searched, searched_of)?;
        let cs = read_block(dir, choice, choices_of)?;
        let rp = read_block(dir, rule, plan_and_keys)?;
        let mut tw = vec![rp.0.terms.len() as i64];
        for (ti, t) in rp.0.terms.iter().enumerate() {
            let (Some(Some(s)), true) = (ss.get(ti), slow.contains(&ti)) else {
                tw.push(0);
                continue;
            };
            let here = rs[ti].counts[d] as usize;
            let share = (TRIAL_SHARE * rs[ti].work / (rs[ti].work / here.max(1) as f64).max(1.0)) as usize;
            let sample = here.min(TRIAL_ROWS).min(share.max(TRIAL_LEAST));
            if sample < TRIAL_LEAST || (s.order == cs[ti].order && s.runners.is_empty()) {
                tw.push(0);
                continue;
            }
            let cands: Vec<Vec<usize>> = std::iter::once(cs[ti].order.clone()).chain(std::iter::once(s.order.clone())).chain(s.runners.iter().map(|r| r.0.clone())).collect();
            let Some(ch) = ChainVars::of(t) else {
                tw.push(0);
                continue;
            };
            let mut least = f64::INFINITY;
            let mut m = Vec::new();
            for (k, o) in cands.iter().enumerate() {
                if !ch.valid(o) {
                    m.push(f64::INFINITY);
                    continue;
                }
                // The running order again (the search's best, or a runner-up): measured once.
                if k > 0 && *o == cands[0] {
                    m.push(m[0]);
                    continue;
                }
                let w = self.trial(dir, &rp.0, ti, &ch, o, frames, d, sample, TRIAL_CUTOFF * least * sample as f64)?;
                pd.trials += 1;
                let per = w / sample as f64;
                least = least.min(per);
                m.push(per);
            }
            tw.push(m.len() as i64);
            tw.extend(m.iter().map(|&x| fb(x)));
        }
        let trials = given(dir, &tw)?;
        made.push(trials);
        let next = self.into(dir, made, self.ops.choose, &[rule, lines, searched, memo, trials, choice], 6, n)?;
        let nc = read_block(dir, next, choices_of)?;
        if nc.iter().zip(&cs).all(|(a, b)| a.order == b.order) {
            return Ok(None);
        }
        pd.handoffs += nc.iter().zip(&cs).filter(|(a, b)| a.order != b.order).count() as u64;
        // Each line that goes on with a new order gets its limit again (0271), from what it did so far.
        {
            let rs_now = read_block(dir, running, |b| running_of_words(&words_of(b)[1..]))?;
            let mut out = vec![d as i64];
            let lines2: Vec<RunningLine> = rs_now
                .iter()
                .enumerate()
                .map(|(ti, r)| {
                    if nc[ti].order == cs[ti].order {
                        return RunningLine { counts: r.counts.clone(), ..*r };
                    }
                    let rate = r.work / r.counts[d].max(1.0);
                    RunningLine { slow: false, due: r.work * 2.0, limit: limit_again(rate, r.work, r.counts[d], r.work), counts: r.counts.clone(), ..*r }
                })
                .collect();
            out.extend(running_words(&lines2, width));
            let b = dir.block(running).expect("made");
            let mut pin = pin_block(&b, "the running lines", true)?;
            put_words(pin.bytes_mut().expect("a write pin"), &out);
        }
        let nb = self.planned(dir, made, rule, next, searched, n)?;
        Ok(Some((nb, next)))
    }

    /// The work order `o` of chain `ti` does over the first `sample` of its tuples in `frames` (depth `d` run), its
    /// remaining steps launched for that chain alone, cut once past `cutoff` (none while infinite).
    #[allow(clippy::too_many_arguments)]
    fn trial(&self, dir: &mut Directory, p: &Plan, ti: usize, ch: &ChainVars, o: &[usize], frames: BlockId, d: usize, sample: usize, cutoff: f64) -> Result<f64, String> {
        let mut tp = p.clone();
        tp.terms[ti] = ch.rebuild(&p.terms[ti], o);
        let pb = body::plan_block(dir, &tp)?;
        let width = tp.width();
        let mut room = sample.max(16);
        let mut work = 0.0;
        let mut cur: Option<BlockId> = None;
        let ran = (|| {
            for depth in d + 1..width {
                let from = cur.unwrap_or(frames);
                let only = Some((ti, if depth == d + 1 { sample } else { 0 }));
                let (f, _) = body::emit_only(self.reg, self.body.step, dir, pb, self.atoms, Some(from), depth as i64, width, &mut room, only)?;
                if let Some(c) = cur.replace(f) {
                    dir.release(c);
                }
                work += read_block(dir, f, |b| body::tuples(b).count() as f64)?;
                if cutoff.is_finite() && work > cutoff {
                    break;
                }
            }
            Ok(work)
        })();
        if let Some(c) = cur {
            dir.release(c);
        }
        dir.release(pb);
        ran
    }
}

#[cfg(test)]
#[path = "plan_tests.rs"]
mod tests;
